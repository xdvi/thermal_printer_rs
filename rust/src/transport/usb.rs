// ============================================================
// transport/usb.rs — USB Transport (desktop only, feature = "usb")
// ============================================================
//
// IMPORTANT PLATFORM WARNING:
//   - Linux:   requires udev rule for non-root access:
//              SUBSYSTEM=="usb", ATTR{idVendor}=="04b8", MODE="0666", GROUP="plugdev"
//   - Windows: requires WinUSB driver installed via Zadig (https://zadig.akeo.ie/)
//   - macOS:   works directly (may need user permission)
//   - Android: libusb DOES NOT work. Use JNI + android.hardware.usb.UsbManager
//   - iOS:     LOCKED by Apple. MFi / AirPrint / BLE only.

use std::time::Duration;

use async_trait::async_trait;
use nusb::transfer::{Bulk, Out, TransferError};
use nusb::{Endpoint, Error, ErrorKind};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use super::Transport;
use crate::errors::{PrinterError, Result};

/// Typical OUT endpoint for ESC/POS printers.
/// May vary by model — detect with `lsusb -v` or USBDeview.
const USB_ENDPOINT_OUT: u8 = 0x01;
const USB_INTERFACE: u8 = 0;

/// Sub-chunk size used for pipelined bulk OUT transfers. Kept well below
/// common per-URB limits (Linux usbfs default 16-64KB, WinUSB drivers vary)
/// so a single submit never risks a driver-level rejection.
const PIPELINE_CHUNK: usize = 16 * 1024;
/// Number of transfers kept in flight simultaneously. Submitting ahead of
/// completion is what actually saturates the bus instead of paying one
/// submit+wait round-trip per chunk.
const PIPELINE_WINDOW: usize = 4;

fn map_nusb_error_kind(kind: ErrorKind, message: impl ToString) -> PrinterError {
    let message = message.to_string();
    match kind {
        ErrorKind::NotFound => PrinterError::PrinterNotFound(message),
        ErrorKind::PermissionDenied | ErrorKind::Busy => PrinterError::PermissionDenied(message),
        ErrorKind::Disconnected => PrinterError::ConnectionFailed(message),
        _ => PrinterError::ConnectionFailed(message),
    }
}

fn map_nusb_err(e: Error) -> PrinterError {
    map_nusb_error_kind(e.kind(), e)
}

fn map_transfer_err(e: TransferError) -> PrinterError {
    PrinterError::ConnectionFailed(e.to_string())
}

pub struct UsbTransport {
    vendor_id: u16,
    product_id: u16,
    timeout: Duration,
    cancel: CancellationToken,
    out_endpoint: u8,
    device: Option<nusb::Device>,
    interface: Option<nusb::Interface>,
    /// Opened OUT endpoint, cached at connect time so each `write()` reuses
    /// it instead of re-resolving the descriptor and re-claiming the queue.
    endpoint: Option<Endpoint<Bulk, Out>>,
    kernel_driver_detached: bool,
}

impl UsbTransport {
    pub fn new(
        vendor_id: u16,
        product_id: u16,
        timeout_ms: u64,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            vendor_id,
            product_id,
            timeout: Duration::from_millis(timeout_ms),
            cancel,
            out_endpoint: USB_ENDPOINT_OUT,
            device: None,
            interface: None,
            endpoint: None,
            kernel_driver_detached: false,
        }
    }

    fn out_endpoint(&self) -> Result<u8> {
        Ok(self.out_endpoint)
    }

    async fn teardown_inflight(&mut self) {
        self.endpoint.take();
        self.interface.take();
        self.device.take();
        self.kernel_driver_detached = false;
    }

    /// Submit `data` as a pipelined sequence of bulk OUT transfers, keeping
    /// up to `PIPELINE_WINDOW` in flight so the host controller always has a
    /// pending request instead of idling between a submit and its await.
    /// Bulk transfers on one endpoint complete in submission order, so chunks
    /// arrive at the device in order despite the overlap.
    async fn write_pipelined(&mut self, data: &[u8]) -> Result<()> {
        let timeout = self.timeout;
        let cancel = self.cancel.clone();
        let ep = self
            .endpoint
            .as_mut()
            .expect("checked by caller before write_pipelined");

        let mut chunks = data.chunks(PIPELINE_CHUNK);
        let mut in_flight = 0usize;

        for chunk in chunks.by_ref().take(PIPELINE_WINDOW) {
            ep.submit(chunk.into());
            in_flight += 1;
        }

        let mut result: Result<()> = Ok(());
        while in_flight > 0 {
            let wait = tokio::time::timeout(timeout, ep.next_complete());
            let completion = tokio::select! {
                res = wait => match res {
                    Ok(completion) => completion,
                    Err(_elapsed) => {
                        result = Err(PrinterError::Timeout);
                        break;
                    }
                },
                _ = cancel.cancelled() => {
                    result = Err(PrinterError::JobCancelled);
                    break;
                }
            };
            in_flight -= 1;

            if let Err(e) = completion.status {
                result = Err(map_transfer_err(e));
                break;
            }

            if let Some(chunk) = chunks.next() {
                ep.submit(chunk.into());
                in_flight += 1;
            }
        }

        if result.is_err() {
            // Cancel and drain whatever is still in flight so the endpoint
            // isn't left with dangling completions before teardown.
            ep.cancel_all();
            while ep.pending() > 0 {
                let _ = ep.next_complete().await;
            }
        }

        result
    }
}

#[async_trait]
impl Transport for UsbTransport {
    async fn connect(&mut self) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Err(PrinterError::JobCancelled);
        }

        let mut devices = nusb::list_devices()
            .await
            .map_err(|e| PrinterError::ConnectionFailed(format!("list_devices: {e}")))?;

        let device_info = devices
            .find(|d| d.vendor_id() == self.vendor_id && d.product_id() == self.product_id)
            .ok_or_else(|| {
                PrinterError::PrinterNotFound(format!(
                    "USB {:04x}:{:04x} not found. Is it connected and do you have permissions?",
                    self.vendor_id, self.product_id
                ))
            })?;

        let device = tokio::select! {
            res = device_info.open() => res.map_err(map_nusb_err)?,
            _ = self.cancel.cancelled() => return Err(PrinterError::JobCancelled),
        };

        if self.cancel.is_cancelled() {
            return Err(PrinterError::JobCancelled);
        }

        #[cfg(target_os = "linux")]
        {
            match device.detach_and_claim_interface(USB_INTERFACE).await {
                Ok(iface) => {
                    self.kernel_driver_detached = true;
                    self.device = Some(device);
                    self.interface = Some(iface);
                }
                Err(_e) => {
                    let iface = device
                        .claim_interface(USB_INTERFACE)
                        .await
                        .map_err(map_nusb_err)?;
                    self.device = Some(device);
                    self.interface = Some(iface);
                }
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let iface = device
                .claim_interface(USB_INTERFACE)
                .await
                .map_err(map_nusb_err)?;
            self.device = Some(device);
            self.interface = Some(iface);
        }

        let address = self.out_endpoint()?;
        let iface = self.interface.as_ref().expect("just assigned above");
        self.endpoint = Some(iface.endpoint::<Bulk, Out>(address).map_err(map_nusb_err)?);

        info!(
            vendor_id = format!("{:04x}", self.vendor_id),
            product_id = format!("{:04x}", self.product_id),
            "USB connected"
        );
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.endpoint.is_none() {
            return Err(PrinterError::TransportUnavailable(
                "USB: writing without active connection".into(),
            ));
        }

        let result = self.write_pipelined(data).await;
        if result.is_err() {
            self.teardown_inflight().await;
        }
        result
    }

    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Err(PrinterError::TransportUnavailable(
            "USB: read not supported (write-only thermal printer)".into(),
        ))
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.teardown_inflight().await;
        info!(
            "USB disconnected ({:04x}:{:04x})",
            self.vendor_id, self.product_id
        );
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.interface.is_some()
    }

    async fn check_liveness(&mut self) -> bool {
        if self.interface.is_none() {
            return false;
        }
        match nusb::list_devices().await {
            Ok(mut devices) => devices
                .any(|d| d.vendor_id() == self.vendor_id && d.product_id() == self.product_id),
            Err(e) => {
                debug!("USB liveness list_devices failed: {e}");
                false
            }
        }
    }

    fn transport_name(&self) -> &'static str {
        "UsbTransport"
    }

    fn preferred_chunk_size(&self) -> usize {
        // No outer chunking: write_pipelined() slices into PIPELINE_CHUNK
        // sub-transfers internally and keeps several in flight, so the full
        // job buffer can be handed over in one Transport::write() call.
        usize::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nusb::ErrorKind;

    #[test]
    fn map_nusb_err_not_found() {
        let e = map_nusb_error_kind(ErrorKind::NotFound, "not found");
        assert!(matches!(e, PrinterError::PrinterNotFound(_)));
    }

    #[test]
    fn map_nusb_err_permission() {
        let e = map_nusb_error_kind(ErrorKind::PermissionDenied, "denied");
        assert!(matches!(e, PrinterError::PermissionDenied(_)));
    }

    #[test]
    fn map_nusb_err_busy() {
        let e = map_nusb_error_kind(ErrorKind::Busy, "busy");
        assert!(matches!(e, PrinterError::PermissionDenied(_)));
    }

    #[test]
    fn map_nusb_err_other() {
        let e = map_nusb_error_kind(ErrorKind::Other, "other");
        assert!(matches!(e, PrinterError::ConnectionFailed(_)));
    }

    #[tokio::test]
    async fn connect_returns_job_cancelled_when_token_already_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut transport = UsbTransport::new(0x04b8, 0x0202, 1000, cancel);
        let result = transport.connect().await;
        assert!(matches!(result, Err(PrinterError::JobCancelled)));
    }
}
