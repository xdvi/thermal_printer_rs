// ============================================================
// printer.rs — PrintService: Main orchestrator
//
// Refactored to use a background IO task for zero-blocking
// and minimized memory copies.
// ============================================================

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, instrument, warn};

use crate::{
    config::{PrinterConfig, TransportKind},
    errors::{PrinterError, Result},
    escpos_adapter::EscposAdapter,
    session::SessionControl,
    transport::{tcp::TcpTransport, Transport},
};

#[cfg(feature = "usb")]
use crate::transport::usb::UsbTransport;

#[cfg(feature = "ble")]
use crate::transport::ble::BleTransport;

// ── IO Commands ──────────────────────────────────────────────────

enum IoCommand {
    Connect {
        resp: oneshot::Sender<Result<()>>,
    },
    Write {
        data: Vec<u8>,
        resp: oneshot::Sender<Result<usize>>,
    },
    Read {
        bytes: usize,
        timeout_ms: u64,
        resp: oneshot::Sender<Result<Vec<u8>>>,
    },
    IsConnected {
        resp: oneshot::Sender<bool>,
    },
    Disconnect {
        resp: oneshot::Sender<Result<()>>,
    },
}

// ── PrintService ─────────────────────────────────────────────────

pub struct PrintService {
    cmd_tx: mpsc::Sender<IoCommand>,
    adapter: EscposAdapter,
    config: PrinterConfig,
    session: Arc<SessionControl>,
}

impl PrintService {
    /// Creates a PrintService from configuration and spawns the IO task.
    pub fn new(config: PrinterConfig, session: Arc<SessionControl>) -> Result<Self> {
        let transport: Box<dyn Transport> = match &config.transport {
            TransportKind::Tcp { host, port } => Box::new(TcpTransport::new(
                host.clone(),
                *port,
                config.timeout_ms,
                config.max_retries,
            )),

            #[cfg(feature = "usb")]
            TransportKind::Usb {
                vendor_id,
                product_id,
            } => Box::new(UsbTransport::new(
                *vendor_id,
                *product_id,
                config.timeout_ms,
                session.cancel_token(),
            )),

            #[cfg(feature = "ble")]
            TransportKind::Ble { address } => {
                Box::new(BleTransport::new(address.clone(), config.timeout_ms))
            }

            #[allow(unreachable_patterns)]
            _ => return Err(PrinterError::PlatformNotSupported),
        };

        let (cmd_tx, cmd_rx) = mpsc::channel(100);
        let paper_width = config.paper_width;

        // Spawn the owner task for the transport
        tokio::spawn(io_task(transport, cmd_rx));

        Ok(Self {
            cmd_tx,
            adapter: EscposAdapter::new(paper_width),
            config,
            session,
        })
    }

    /// Creates a PrintService with a pre-built transport instance.
    pub fn new_with_transport(
        config: PrinterConfig,
        transport: Box<dyn Transport>,
        session: Arc<SessionControl>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(100);
        let paper_width = config.paper_width;

        tokio::spawn(io_task(transport, cmd_rx));

        Self {
            cmd_tx,
            adapter: EscposAdapter::new(paper_width),
            config,
            session,
        }
    }

    /// Connects to the transport.
    #[instrument(skip(self))]
    pub async fn connect(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(IoCommand::Connect { resp: tx })
            .await
            .map_err(|_| PrinterError::ConnectionFailed("IO task dropped".into()))?;

        rx.await
            .map_err(|_| PrinterError::ConnectionFailed("IO task panicked".into()))?
    }

    /// Prints simple text with paper cut.
    pub async fn print_text(&self, text: &str) -> Result<usize> {
        self.ensure_connected().await?;
        let buf = self.adapter.build_text(text)?;
        self.send_buffer_owned(buf).await
    }

    /// Prints a complete receipt.
    pub async fn print_receipt(
        &self,
        title: &str,
        lines: &[(&str, &str)],
        total: &str,
        qr_data: Option<&str>,
    ) -> Result<usize> {
        self.ensure_connected().await?;
        let buf = self
            .adapter
            .build_receipt_pairs(title, lines, total, qr_data)?;
        self.send_buffer_owned(buf).await
    }

    /// Disconnects the transport.
    pub async fn disconnect(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(IoCommand::Disconnect { resp: tx }).await;
        rx.await
            .map_err(|_| PrinterError::ConnectionFailed("IO task panicked".into()))?
    }

    /// Reads bytes from the transport.
    pub async fn read(&self, bytes: usize, timeout_ms: u64) -> Result<Vec<u8>> {
        self.ensure_connected().await?;
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(IoCommand::Read {
                bytes,
                timeout_ms,
                resp: tx,
            })
            .await
            .map_err(|_| PrinterError::ConnectionFailed("IO task dropped".into()))?;
        rx.await
            .map_err(|_| PrinterError::ConnectionFailed("IO task panicked".into()))?
    }

    /// Sends an owned buffer with automatic reconnection and retries.
    /// Callers that already own a Vec should use this to avoid any copies.
    pub async fn send_buffer_owned_retrying(&self, buf: Vec<u8>) -> Result<usize> {
        let cancel = self.session.cancel_token();
        if cancel.is_cancelled() {
            return Err(PrinterError::JobCancelled);
        }

        if self.config.max_retries == 0 {
            return self.send_buffer_owned(buf).await;
        }

        let backup = buf.clone();
        match self.send_buffer_owned(buf).await {
            Ok(n) => Ok(n),
            Err(e) => {
                error!(error = %e, "Send buffer failed (attempt 1)");
                let mut last_err = e;

                for attempt in 1..=self.config.max_retries {
                    if cancel.is_cancelled() {
                        return Err(PrinterError::JobCancelled);
                    }

                    let backoff =
                        std::time::Duration::from_millis(500 * 2u64.pow(attempt as u32 - 1));
                    warn!(attempt, ?backoff, "Retrying buffer send...");
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(PrinterError::JobCancelled),
                        _ = tokio::time::sleep(backoff) => {}
                    }

                    if !self.is_connected().await {
                        let _ = self.connect().await;
                    }

                    match self.send_buffer_owned(backup.clone()).await {
                        Ok(n) => return Ok(n),
                        Err(e) => {
                            error!(error = %e, "Send buffer failed (attempt {})", attempt + 1);
                            last_err = e;
                        }
                    }
                }

                Err(last_err)
            }
        }
    }

    pub fn adapter(&self) -> &EscposAdapter {
        &self.adapter
    }

    // ── Private ──────────────────────────────────────────────────

    async fn is_connected(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(IoCommand::IsConnected { resp: tx })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    async fn ensure_connected(&self) -> Result<()> {
        if !self.is_connected().await {
            self.connect().await?;
        }
        Ok(())
    }

    /// Sends the buffer and returns the number of bytes sent.
    /// Takes ownership of the Vec to avoid cloning inside the IO task.
    pub(crate) async fn send_buffer_owned(&self, buf: Vec<u8>) -> Result<usize> {
        let _len = buf.len();
        let (tx, rx) = oneshot::channel();

        self.cmd_tx
            .send(IoCommand::Write {
                data: buf,
                resp: tx,
            })
            .await
            .map_err(|_| PrinterError::ConnectionFailed("IO task dropped".into()))?;

        rx.await
            .map_err(|_| PrinterError::ConnectionFailed("IO task panicked".into()))?
    }
}

// ── IO Task ──────────────────────────────────────────────────────

async fn io_task(mut transport: Box<dyn Transport>, mut cmd_rx: mpsc::Receiver<IoCommand>) {
    info!("IO Task started");

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            IoCommand::Connect { resp } => {
                if transport.is_connected() {
                    let _ = resp.send(Ok(()));
                } else {
                    let res = transport.connect().await;
                    let _ = resp.send(res);
                }
            }
            IoCommand::Write { data, resp } => {
                let chunk_size = transport.preferred_chunk_size();
                let mut total_sent = 0;
                let mut res = Ok(());

                if data.len() <= chunk_size {
                    res = transport.write(&data).await;
                    total_sent = data.len();
                } else {
                    let delay = transport.chunk_delay();
                    for chunk in data.chunks(chunk_size) {
                        if let Err(e) = transport.write(chunk).await {
                            res = Err(e);
                            break;
                        }
                        total_sent += chunk.len();

                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        } else {
                            tokio::task::yield_now().await;
                        }
                    }
                }
                let _ = resp.send(res.map(|_| total_sent));
            }
            IoCommand::Read {
                bytes,
                timeout_ms,
                resp,
            } => {
                let mut buf = vec![0u8; bytes];
                // Transport read might not respect timeout natively, so wrap it
                let read_res = match tokio::time::timeout(
                    std::time::Duration::from_millis(timeout_ms),
                    transport.read(&mut buf),
                )
                .await
                {
                    Ok(Ok(n)) => {
                        buf.truncate(n);
                        Ok(buf)
                    }
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(PrinterError::Timeout),
                };
                let _ = resp.send(read_res);
            }
            IoCommand::IsConnected { resp } => {
                let alive = transport.check_liveness().await;
                let _ = resp.send(alive);
            }
            IoCommand::Disconnect { resp } => {
                let res = transport.disconnect().await;
                let _ = resp.send(res);
            }
        }
    }

    if let Err(e) = transport.disconnect().await {
        warn!(error = %e, "IO task disconnect on shutdown failed");
    }

    info!("IO Task stopped");
}
