// ============================================================
// transport/mock.rs — MockTransport for testing without hardware
//
// Simulates a thermal printer in memory.
// Captures all ESC/POS bytes sent for inspection in tests.
// Configurable to simulate connection failures, timeouts, etc.
// ============================================================

use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use super::Transport;
use crate::errors::{PrinterError, Result};

/// Behavior the mock transport should simulate.
#[derive(Debug, Clone, Default)]
pub struct MockConfig {
    /// If true, connect() returns an error.
    pub fail_on_connect: bool,
    /// If true, write() returns an error.
    pub fail_on_write: bool,
    /// If true, the transport starts already connected.
    pub starts_connected: bool,
}

/// In-memory transport for unit and integration tests.
///
/// Usage example:
/// ```rust,no_run
/// # use std::sync::{Arc, Mutex};
/// # use thermal_printer_rs::transport::mock::MockTransport;
/// let buffer = Arc::new(Mutex::new(Vec::new()));
/// let transport = MockTransport::new_with_buffer(buffer.clone());
/// // ... use transport ...
/// let received = buffer.lock().unwrap();
/// assert!(!received.is_empty());
/// ```
pub struct MockTransport {
    config: MockConfig,
    connected: bool,
    /// Shared buffer where written bytes are captured.
    buffer: Arc<Mutex<Vec<u8>>>,
    /// Simulated data to return on read().
    read_data: Vec<u8>,
}

impl MockTransport {
    /// Creates a mock transport with default (success) behavior.
    pub fn new() -> Self {
        Self {
            config: MockConfig::default(),
            connected: false,
            buffer: Arc::new(Mutex::new(Vec::new())),
            read_data: vec![],
        }
    }

    /// Creates a mock transport that shares an external buffer for inspection.
    pub fn new_with_buffer(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self {
            config: MockConfig::default(),
            connected: false,
            buffer,
            read_data: vec![],
        }
    }

    /// Configures failure behavior.
    pub fn with_config(mut self, config: MockConfig) -> Self {
        self.connected = config.starts_connected;
        self.config = config;
        self
    }

    /// Sets data to return on read() (e.g., simulated printer status).
    pub fn with_read_data(mut self, data: Vec<u8>) -> Self {
        self.read_data = data;
        self
    }

    /// Returns a clone of all bytes written so far.
    pub fn written_bytes(&self) -> Vec<u8> {
        self.buffer.lock().unwrap().clone()
    }

    /// Clears the capture buffer.
    pub fn clear(&self) {
        self.buffer.lock().unwrap().clear();
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn connect(&mut self) -> Result<()> {
        if self.config.fail_on_connect {
            warn!("MockTransport: simulating connection failure");
            return Err(PrinterError::ConnectionFailed(
                "Simulated connection failure (MockTransport)".into(),
            ));
        }
        info!("MockTransport: connected");
        self.connected = true;
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        if !self.connected {
            return Err(PrinterError::TransportUnavailable(
                "MockTransport: write attempted without active connection".into(),
            ));
        }
        if self.config.fail_on_write {
            warn!("MockTransport: simulating write failure");
            return Err(PrinterError::ConnectionFailed(
                "Simulated write failure (MockTransport)".into(),
            ));
        }
        debug!(
            bytes = data.len(),
            "MockTransport: capturing {} bytes",
            data.len()
        );
        self.buffer.lock().unwrap().extend_from_slice(data);
        Ok(())
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.read_data.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(self.read_data.len());
        buf[..n].copy_from_slice(&self.read_data[..n]);
        Ok(n)
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("MockTransport: disconnected");
        self.connected = false;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn transport_name(&self) -> &'static str {
        "Mock"
    }
}

// ============================================================
// Unit tests for MockTransport
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_connects_and_writes() {
        let mut transport = MockTransport::new();
        transport.connect().await.unwrap();
        assert!(transport.is_connected());

        transport.write(b"ESC/POS data").await.unwrap();
        assert_eq!(transport.written_bytes(), b"ESC/POS data");
    }

    #[tokio::test]
    async fn test_mock_fail_on_connect() {
        let mut transport = MockTransport::new().with_config(MockConfig {
            fail_on_connect: true,
            ..Default::default()
        });
        let result = transport.connect().await;
        assert!(result.is_err());
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn test_mock_fail_on_write() {
        let mut transport = MockTransport::new().with_config(MockConfig {
            fail_on_write: true,
            starts_connected: true,
            ..Default::default()
        });
        let result = transport.write(b"data").await;
        assert!(result.is_err());
        assert!(transport.written_bytes().is_empty());
    }

    #[tokio::test]
    async fn test_mock_write_without_connect_fails() {
        let mut transport = MockTransport::new();
        let result = transport.write(b"data").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_shared_buffer() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let mut transport = MockTransport::new_with_buffer(buffer.clone());
        transport.connect().await.unwrap();
        transport.write(b"hello").await.unwrap();
        transport.write(b" world").await.unwrap();

        let data = buffer.lock().unwrap();
        assert_eq!(*data, b"hello world");
    }

    #[tokio::test]
    async fn test_mock_disconnect_resets_state() {
        let mut transport = MockTransport::new();
        transport.connect().await.unwrap();
        assert!(transport.is_connected());
        transport.disconnect().await.unwrap();
        assert!(!transport.is_connected());
    }
}
