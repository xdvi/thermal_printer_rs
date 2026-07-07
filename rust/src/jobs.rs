// ============================================================
// jobs.rs — Background job processor
// ============================================================

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::errors::{PrinterError, Result};
use crate::printer::PrintService;
use crate::session::SessionControl;

/// Commands supported by the background worker.
pub enum PrintCommand {
    /// Fire-and-forget print job.
    Print(Vec<u8>),
    /// Print job that reports completion to the caller (serialized through the worker).
    PrintAwait {
        buf: Vec<u8>,
        resp: oneshot::Sender<Result<usize>>,
    },
    Connect,
    Disconnect,
    ClearQueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Disconnected,
    Connecting,
    Connected,
    Printing,
    Error,
}

pub struct PrintWorker {
    service: Arc<PrintService>,
    receiver: mpsc::Receiver<PrintCommand>,
    state_tx: tokio::sync::watch::Sender<WorkerState>,
    session: Arc<SessionControl>,
}

impl PrintWorker {
    pub fn new(
        service: Arc<PrintService>,
        receiver: mpsc::Receiver<PrintCommand>,
        state_tx: tokio::sync::watch::Sender<WorkerState>,
        session: Arc<SessionControl>,
    ) -> Self {
        Self {
            service,
            receiver,
            state_tx,
            session,
        }
    }

    fn set_state(&self, state: WorkerState) {
        let _ = self.state_tx.send(state);
    }

    async fn handle_print(&self, buf: Vec<u8>) -> Result<usize> {
        let len = buf.len();
        self.set_state(WorkerState::Printing);
        let result = self.service.send_buffer_owned_retrying(buf).await;
        match &result {
            Ok(_) => self.set_state(WorkerState::Connected),
            Err(PrinterError::JobCancelled) => {
                warn!("Background print job cancelled");
                self.set_state(WorkerState::Connected);
            }
            Err(_) => {
                error!(error = ?result.as_ref().err(), "Background print job failed");
                self.set_state(WorkerState::Error);
            }
        }
        self.session.queue_budget.release(len);
        result
    }

    fn drain_pending(&mut self) {
        while let Ok(cmd) = self.receiver.try_recv() {
            match cmd {
                PrintCommand::Print(buf) => {
                    self.session.queue_budget.release(buf.len());
                }
                PrintCommand::PrintAwait { buf, resp } => {
                    self.session.queue_budget.release(buf.len());
                    let _ = resp.send(Err(PrinterError::JobCancelled));
                }
                _ => {}
            }
        }
        self.session.queue_budget.reset();
    }

    pub async fn run(mut self) {
        info!("Background PrintWorker started");
        self.set_state(WorkerState::Disconnected);

        while let Some(cmd) = self.receiver.recv().await {
            if self.session.cancel_token().is_cancelled() {
                self.session.reset_cancel();
            }

            match cmd {
                PrintCommand::Print(buf) => {
                    debug!(bytes = buf.len(), "Processing background print job");
                    let _ = self.handle_print(buf).await;
                }
                PrintCommand::PrintAwait { buf, resp } => {
                    debug!(bytes = buf.len(), "Processing awaited print job");
                    let result = self.handle_print(buf).await;
                    let _ = resp.send(result);
                }
                PrintCommand::Connect => {
                    info!("Background connection requested");
                    self.set_state(WorkerState::Connecting);
                    if let Err(e) = self.service.connect().await {
                        error!(error = %e, "Background connection failed");
                        self.set_state(WorkerState::Error);
                    } else {
                        self.set_state(WorkerState::Connected);
                    }
                }
                PrintCommand::Disconnect => {
                    info!("Background disconnection requested");
                    let _ = self.service.disconnect().await;
                    self.set_state(WorkerState::Disconnected);
                }
                PrintCommand::ClearQueue => {
                    warn!("Clearing background print queue and cancelling in-flight job");
                    self.session.signal_cancel();
                    self.drain_pending();
                    self.session.reset_cancel();
                    self.set_state(WorkerState::Connected);
                }
            }
        }

        self.set_state(WorkerState::Disconnected);
        info!("Background PrintWorker stopped");
    }
}
