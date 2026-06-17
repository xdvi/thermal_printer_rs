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

use async_trait::async_trait;
use rusb::{Context, DeviceHandle, UsbContext};
use tracing::{debug, info, warn};

use crate::errors::{PrinterError, Result};
use super::Transport;

/// Typical OUT endpoint for ESC/POS printers.
/// May vary by model — detect with `lsusb -v` or USBDeview.
const USB_ENDPOINT_OUT: u8 = 0x01;
const USB_ENDPOINT_IN:  u8 = 0x81;
const USB_INTERFACE:    u8 = 0;

pub struct UsbTransport {
    vendor_id:  u16,
    product_id: u16,
    timeout:    std::time::Duration,
    handle:     Option<DeviceHandle<Context>>,
    context:    Option<Context>,
}

impl UsbTransport {
    pub fn new(vendor_id: u16, product_id: u16, timeout_ms: u64) -> Self {
        Self {
            vendor_id,
            product_id,
            timeout:  std::time::Duration::from_millis(timeout_ms),
            handle:   None,
            context:  None,
        }
    }
}

#[async_trait]
impl Transport for UsbTransport {
    async fn connect(&mut self) -> Result<()> {
        let ctx = Context::new()
            .map_err(|e| PrinterError::ConnectionFailed(format!("libusb context: {e}")))?;

        let handle = ctx
            .open_device_with_vid_pid(self.vendor_id, self.product_id)
            .ok_or_else(|| {
                PrinterError::PrinterNotFound(format!(
                    "USB {:04x}:{:04x} not found. Is it connected and do you have permissions?",
                    self.vendor_id, self.product_id
                ))
            })?;

        // Detach kernel driver if active (Linux)
        match handle.kernel_driver_active(USB_INTERFACE) {
            Ok(true) => {
                warn!("Kernel driver active on interface {}. Detaching...", USB_INTERFACE);
                handle.detach_kernel_driver(USB_INTERFACE)
                    .map_err(|e| PrinterError::PermissionDenied(
                        format!("Could not detach kernel driver: {e}. Try running as root?")
                    ))?;
            }
            Ok(false) => {}
            Err(e) => debug!("kernel_driver_active not supported on this platform: {e}"),
        }

        handle.claim_interface(USB_INTERFACE)
            .map_err(|e| PrinterError::PermissionDenied(
                format!("claim_interface failed: {e}")
            ))?;

        info!(
            vendor_id  = format!("{:04x}", self.vendor_id),
            product_id = format!("{:04x}", self.product_id),
            "USB connected"
        );

        self.context = Some(ctx);
        self.handle  = Some(handle);
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let handle = self.handle.as_ref().ok_or_else(|| {
            PrinterError::TransportUnavailable("USB: writing without active connection".into())
        })?;

        handle
            .write_bulk(USB_ENDPOINT_OUT, data, self.timeout)
            .map_err(|e| PrinterError::ConnectionFailed(format!("USB write: {e}")))?;

        Ok(())
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let handle = self.handle.as_ref().ok_or_else(|| {
            PrinterError::TransportUnavailable("USB: reading without active connection".into())
        })?;

        let n = handle
            .read_bulk(USB_ENDPOINT_IN, buf, self.timeout)
            .map_err(|e| PrinterError::ConnectionFailed(format!("USB read: {e}")))?;

        Ok(n)
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            let _ = handle.release_interface(USB_INTERFACE);
            info!("USB disconnected ({:04x}:{:04x})", self.vendor_id, self.product_id);
        }
        self.context.take();
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.handle.is_some()
    }

    fn transport_name(&self) -> &'static str {
        "UsbTransport"
    }

    fn preferred_chunk_size(&self) -> usize {
        4096 // 4KB for libusb stability
    }
}
