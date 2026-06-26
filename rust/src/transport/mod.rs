// ============================================================
// transport/mod.rs — Transport trait + re-exports
// ============================================================
pub mod mock;
pub mod tcp;

#[cfg(feature = "usb")]
pub mod usb;

#[cfg(feature = "ble")]
pub mod ble;

use crate::errors::Result;
use async_trait::async_trait;

/// Transport abstraction.
/// All implementations must be Send + Sync for use in multi-threaded async contexts.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Establishes the connection with the printer.
    async fn connect(&mut self) -> Result<()>;

    /// Sends the complete ESC/POS buffer to the printer.
    /// Implementations must guarantee all bytes are sent (semantic write_all).
    async fn write(&mut self, data: &[u8]) -> Result<()>;

    /// Reads response bytes from the printer (for status queries).
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Closes the connection cleanly.
    async fn disconnect(&mut self) -> Result<()>;

    /// Indicates if the connection is currently active.
    fn is_connected(&self) -> bool;

    /// Descriptive name of the transport for logging.
    fn transport_name(&self) -> &'static str;

    /// Preferred size for data chunks.
    /// TCP: 64KB (let OS handle it), USB: 4KB, BLE: MTU-based.
    fn preferred_chunk_size(&self) -> usize {
        8192 // Default 8KB
    }

    /// Optional delay between chunks.
    /// Useful for slow BLE/BT Classic devices.
    fn chunk_delay(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}
