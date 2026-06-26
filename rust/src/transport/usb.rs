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
use nusb::{Error, ErrorKind};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use super::Transport;
use crate::errors::{PrinterError, Result};

/// Typical OUT endpoint for ESC/POS printers.
/// May vary by model — detect with `lsusb -v` or USBDeview.
const USB_ENDPOINT_OUT: u8 = 0x01;
const USB_INTERFACE: u8 = 0;

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
            kernel_driver_detached: false,
        }
    }

    fn out_endpoint(&self) -> Result<u8> {
        Ok(self.out_endpoint)
    }

    async fn teardown_inflight(&mut self) {
        self.interface.take();
        self.device.take();
        self.kernel_driver_detached = false;
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

        info!(
            vendor_id = format!("{:04x}", self.vendor_id),
            product_id = format!("{:04x}", self.product_id),
            "USB connected"
        );
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let address = self.out_endpoint()?;
        let iface = self.interface.as_ref().ok_or_else(|| {
            PrinterError::TransportUnavailable("USB: writing without active connection".into())
        })?;

        let mut ep = iface.endpoint::<Bulk, Out>(address).map_err(map_nusb_err)?;

        ep.submit(data.to_vec().into());

        let write_fut = tokio::time::timeout(self.timeout, ep.next_complete());

        tokio::select! {
            res = write_fut => match res {
                Ok(completion) => match completion.into_result() {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        self.teardown_inflight().await;
                        Err(map_transfer_err(e))
                    }
                },
                Err(_elapsed) => {
                    self.teardown_inflight().await;
                    Err(PrinterError::Timeout)
                }
            },
            _ = self.cancel.cancelled() => {
                self.teardown_inflight().await;
                Err(PrinterError::JobCancelled)
            }
        }
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
        4096
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
