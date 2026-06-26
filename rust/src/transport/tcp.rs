// ============================================================
// transport/tcp.rs — TCP/IP implementation
// ============================================================
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use super::Transport;
use crate::errors::{PrinterError, Result};

pub struct TcpTransport {
    host: String,
    port: u16,
    timeout: Duration,
    stream: Option<TcpStream>,
    max_retries: u8,
}

impl TcpTransport {
    pub fn new(host: impl Into<String>, port: u16, timeout_ms: u64, max_retries: u8) -> Self {
        Self {
            host: host.into(),
            port,
            timeout: Duration::from_millis(timeout_ms),
            stream: None,
            max_retries,
        }
    }

    fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Attempts to connect with retries and exponential backoff.
    async fn try_connect(&self) -> Result<TcpStream> {
        let addr = self.addr();
        let mut last_err = PrinterError::ConnectionFailed("no attempts made".into());

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let backoff = Duration::from_millis(200 * 2u64.pow(attempt as u32 - 1));
                warn!(attempt, ?backoff, "Retrying TCP connection...");
                tokio::time::sleep(backoff).await;
            }

            match timeout(self.timeout, TcpStream::connect(&addr)).await {
                Ok(Ok(stream)) => {
                    stream.set_nodelay(true)?;
                    info!(addr = %addr, "TCP connected (attempt {})", attempt + 1);
                    return Ok(stream);
                }
                Ok(Err(e)) => {
                    last_err = PrinterError::ConnectionFailed(e.to_string());
                    error!(error = %e, "TCP connection failed");
                }
                Err(_) => {
                    last_err = PrinterError::Timeout;
                    error!("Timeout connecting to {}", addr);
                }
            }
        }

        Err(last_err)
    }
}

#[async_trait]
impl Transport for TcpTransport {
    async fn connect(&mut self) -> Result<()> {
        let stream = self.try_connect().await?;
        self.stream = Some(stream);
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            PrinterError::TransportUnavailable("TCP: writing without active connection".into())
        })?;

        debug!(bytes = data.len(), "Sending ESC/POS buffer via TCP");

        timeout(self.timeout, stream.write_all(data))
            .await
            .map_err(|_| PrinterError::Timeout)?
            .map_err(PrinterError::Io)?;

        timeout(self.timeout, stream.flush())
            .await
            .map_err(|_| PrinterError::Timeout)?
            .map_err(PrinterError::Io)?;

        debug!(bytes = data.len(), "Buffer sent successfully");
        Ok(())
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            PrinterError::TransportUnavailable("TCP: reading without active connection".into())
        })?;

        let n = timeout(self.timeout, stream.read(buf))
            .await
            .map_err(|_| PrinterError::Timeout)?
            .map_err(PrinterError::Io)?;

        debug!(bytes_read = n, "Bytes read from printer");
        Ok(n)
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut stream) = self.stream.take() {
            let _ = stream.shutdown().await;
            info!("TCP disconnected from {}", self.addr());
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    fn transport_name(&self) -> &'static str {
        "TcpTransport"
    }

    fn preferred_chunk_size(&self) -> usize {
        16384 // 16KB for TCP stream
    }
}

impl Drop for TcpTransport {
    fn drop(&mut self) {
        // Async cleanup should be done via disconnect().
        // Drop ensures the stream is not left open indefinitely.
        self.stream.take();
    }
}
