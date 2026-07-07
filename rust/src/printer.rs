// ============================================================
// printer.rs — PrintService: Main orchestrator
//
// Refactored to use a background IO task for zero-blocking
// and minimized memory copies.
// ============================================================

use std::sync::atomic::{AtomicBool, Ordering};
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

/// Outcome of an `IoCommand::Write`. On error the untouched buffer is handed
/// back (when recoverable) so the caller can retry without re-cloning it.
pub(crate) type WriteOutcome = std::result::Result<usize, (Option<Vec<u8>>, PrinterError)>;

enum IoCommand {
    Connect {
        resp: oneshot::Sender<Result<()>>,
    },
    Write {
        data: Vec<u8>,
        resp: oneshot::Sender<WriteOutcome>,
    },
    Read {
        bytes: usize,
        timeout_ms: u64,
        resp: oneshot::Sender<Result<Vec<u8>>>,
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
    /// Cheap connected flag maintained by the IO task. Replaces a per-print
    /// liveness round-trip (which on TCP did a 200ms `writable()` probe and on
    /// USB re-enumerated the whole bus).
    connected: Arc<AtomicBool>,
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
        let connected = Arc::new(AtomicBool::new(false));

        // Spawn the owner task for the transport
        tokio::spawn(io_task(transport, cmd_rx, connected.clone()));

        Ok(Self {
            cmd_tx,
            adapter: EscposAdapter::new(paper_width),
            config,
            session,
            connected,
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
        let connected = Arc::new(AtomicBool::new(false));

        tokio::spawn(io_task(transport, cmd_rx, connected.clone()));

        Self {
            cmd_tx,
            adapter: EscposAdapter::new(paper_width),
            config,
            session,
            connected,
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
        self.send_buffer_owned(buf).await.map_err(|(_, e)| e)
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
        self.send_buffer_owned(buf).await.map_err(|(_, e)| e)
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
            return self.send_buffer_owned(buf).await.map_err(|(_, e)| e);
        }

        // First attempt: hand over the original buffer with NO clone. On error
        // the untouched buffer comes back to us (when recoverable) for retries.
        let mut data;
        let mut last_err = match self.send_buffer_owned(buf).await {
            Ok(n) => return Ok(n),
            Err((Some(buf), e)) => {
                error!(error = %e, "Send buffer failed (attempt 1)");
                data = buf;
                e
            }
            // Buffer unrecoverable (IO task gone) — nothing to retry with.
            Err((None, e)) => return Err(e),
        };

        for attempt in 1..=self.config.max_retries {
            if cancel.is_cancelled() {
                return Err(PrinterError::JobCancelled);
            }

            let backoff = std::time::Duration::from_millis(500 * 2u64.pow(attempt as u32 - 1));
            warn!(attempt, ?backoff, "Retrying buffer send...");
            tokio::select! {
                _ = cancel.cancelled() => return Err(PrinterError::JobCancelled),
                _ = tokio::time::sleep(backoff) => {}
            }

            if !self.is_connected() {
                let _ = self.connect().await;
            }

            // Move `data` in (no clone); get it back via the recovered buffer.
            match self.send_buffer_owned(data).await {
                Ok(n) => return Ok(n),
                Err((Some(buf), e)) => {
                    error!(error = %e, "Send buffer failed (attempt {})", attempt + 1);
                    data = buf;
                    last_err = e;
                }
                Err((None, e)) => {
                    error!(error = %e, "Send buffer failed (attempt {})", attempt + 1);
                    // Buffer lost (IO task dropped) — cannot retry further.
                    return Err(e);
                }
            }
        }

        Err(last_err)
    }

    pub fn adapter(&self) -> &EscposAdapter {
        &self.adapter
    }

    // ── Private ──────────────────────────────────────────────────

    /// Cheap connected check: reads the atomic flag maintained by the IO task.
    /// No channel round-trip, no liveness probe — reconnect happens lazily on a
    /// real write error via the retry path.
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn ensure_connected(&self) -> Result<()> {
        if !self.is_connected() {
            self.connect().await?;
        }
        Ok(())
    }

    /// Sends the buffer and returns bytes sent. On error the untouched buffer
    /// is returned in the `Option` (when the IO task still had it) so callers
    /// can retry without re-cloning. `None` means the buffer was lost (channel
    /// dropped / task panicked) and is not recoverable.
    pub(crate) async fn send_buffer_owned(&self, buf: Vec<u8>) -> WriteOutcome {
        let (tx, rx) = oneshot::channel();

        // `buf` is moved into the command here; if the send fails the buffer
        // is gone (IO task is gone), hence `None`.
        if self
            .cmd_tx
            .send(IoCommand::Write { data: buf, resp: tx })
            .await
            .is_err()
        {
            return Err((None, PrinterError::ConnectionFailed("IO task dropped".into())));
        }

        match rx.await {
            Ok(outcome) => outcome,
            // Receiver dropped (IO task panicked) — buffer already consumed there.
            Err(_) => Err((
                None,
                PrinterError::ConnectionFailed("IO task panicked".into()),
            )),
        }
    }
}

// ── IO Task ──────────────────────────────────────────────────────

async fn io_task(
    mut transport: Box<dyn Transport>,
    mut cmd_rx: mpsc::Receiver<IoCommand>,
    connected: Arc<AtomicBool>,
) {
    info!("IO Task started");

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            IoCommand::Connect { resp } => {
                if transport.is_connected() {
                    connected.store(true, Ordering::Relaxed);
                    let _ = resp.send(Ok(()));
                } else {
                    let res = transport.connect().await;
                    connected.store(res.is_ok(), Ordering::Relaxed);
                    let _ = resp.send(res);
                }
            }
            IoCommand::Write { data, resp } => {
                let chunk_size = transport.preferred_chunk_size();
                let mut total_sent = 0;
                let mut res: std::result::Result<(), PrinterError> = Ok(());

                if data.len() <= chunk_size {
                    res = transport.write(&data).await;
                    if res.is_ok() {
                        total_sent = data.len();
                    }
                } else {
                    let delay = transport.chunk_delay();
                    let num_chunks = data.len().div_ceil(chunk_size);
                    for (i, chunk) in data.chunks(chunk_size).enumerate() {
                        match transport.write(chunk).await {
                            Ok(()) => total_sent += chunk.len(),
                            Err(e) => {
                                res = Err(e);
                                break;
                            }
                        }
                        // Apply the inter-chunk delay (used by BLE flow control)
                        // only between chunks — never after the last one.
                        if !delay.is_zero() && i + 1 < num_chunks {
                            tokio::time::sleep(delay).await;
                        }
                        // No explicit yield: transport.write().await already
                        // yields to the runtime, so back-to-back chunks on
                        // no-delay transports (TCP/USB) run without extra
                        // scheduler round-trips per chunk.
                    }
                }
                // On error hand the untouched buffer back so the caller can
                // retry without re-cloning. `data` is only borrowed above.
                // Update the connected flag: a failed write usually means the
                // link is down, so the next ensure_connected() reconnects.
                let outcome = match res {
                    Ok(()) => {
                        connected.store(true, Ordering::Relaxed);
                        Ok(total_sent)
                    }
                    Err(e) => {
                        connected.store(false, Ordering::Relaxed);
                        Err((Some(data), e))
                    }
                };
                let _ = resp.send(outcome);
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
            IoCommand::Disconnect { resp } => {
                connected.store(false, Ordering::Relaxed);
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
