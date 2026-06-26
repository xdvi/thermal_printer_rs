// ============================================================
// jobs.rs — Background job processor
//
// Manages an async queue (MPSC) to process print jobs without
// blocking the main thread or UI.
// ============================================================

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::printer::PrintService;

/// Commands supported by the background worker.
pub enum PrintCommand {
    /// Print a pre-built ESC/POS buffer.
    Print(Vec<u8>),
    /// Explicitly connect the transport.
    Connect,
    /// Explicitly disconnect the transport.
    Disconnect,
    /// Drains all pending jobs in the queue.
    ClearQueue,
}

/// Represents the high-level state of the background worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Disconnected,
    Connecting,
    Connected,
    Printing,
    Error,
}

/// The worker that consumes commands from the receiver.
pub struct PrintWorker {
    service: Arc<PrintService>,
    receiver: mpsc::Receiver<PrintCommand>,
    state_tx: tokio::sync::watch::Sender<WorkerState>,
}

impl PrintWorker {
    pub fn new(
        service: Arc<PrintService>,
        receiver: mpsc::Receiver<PrintCommand>,
        state_tx: tokio::sync::watch::Sender<WorkerState>,
    ) -> Self {
        Self {
            service,
            receiver,
            state_tx,
        }
    }

    fn set_state(&self, state: WorkerState) {
        let _ = self.state_tx.send(state);
    }

    /// Starts the background loop.
    pub async fn run(mut self) {
        info!("Background PrintWorker started");
        self.set_state(WorkerState::Disconnected);

        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                PrintCommand::Print(buf) => {
                    info!(bytes = buf.len(), "Processing background print job");
                    self.set_state(WorkerState::Printing);
                    // buf is already owned here — pass directly to avoid an extra allocation.
                    if let Err(e) = self.service.send_buffer_owned_retrying(buf).await {
                        error!(error = %e, "Background print job failed");
                        self.set_state(WorkerState::Error);
                    } else {
                        self.set_state(WorkerState::Connected);
                    }
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
                    warn!("Clearing background print queue");
                    while self.receiver.try_recv().is_ok() {}
                }
            }
        }

        self.set_state(WorkerState::Disconnected);
        info!("Background PrintWorker stopped");
    }
}
