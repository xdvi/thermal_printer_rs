// ============================================================
// session.rs — Per-session cancellation and queue memory budget
// ============================================================

use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::errors::{PrinterError, Result};

/// Default cap for queued print payload bytes (~16 MiB).
pub const DEFAULT_QUEUE_BYTE_BUDGET: usize = 16 * 1024 * 1024;

/// Tracks total bytes currently held in the background print queue.
#[derive(Debug)]
pub struct QueueBudget {
    max_bytes: usize,
    current: AtomicUsize,
}

impl QueueBudget {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            current: AtomicUsize::new(0),
        }
    }

    pub fn try_reserve(&self, bytes: usize) -> Result<()> {
        loop {
            let current = self.current.load(Ordering::Acquire);
            let next = current.saturating_add(bytes);
            if next > self.max_bytes {
                return Err(PrinterError::TransportUnavailable(format!(
                    "Print queue memory budget exceeded ({next} > {} bytes)",
                    self.max_bytes
                )));
            }
            if self
                .current
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    pub fn release(&self, bytes: usize) {
        loop {
            let current = self.current.load(Ordering::Acquire);
            let next = current.saturating_sub(bytes);
            if self
                .current
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
    }

    pub fn reset(&self) {
        self.current.store(0, Ordering::Release);
    }
}

/// Shared session controls: cancellation for in-flight work and queue budgeting.
#[derive(Debug)]
pub struct SessionControl {
    cancel: Mutex<CancellationToken>,
    pub queue_budget: QueueBudget,
}

impl Default for SessionControl {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionControl {
    pub fn new() -> Self {
        Self {
            cancel: Mutex::new(CancellationToken::new()),
            queue_budget: QueueBudget::new(DEFAULT_QUEUE_BYTE_BUDGET),
        }
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.lock().clone()
    }

    pub fn signal_cancel(&self) {
        self.cancel.lock().cancel();
    }

    pub fn reset_cancel(&self) {
        *self.cancel.lock() = CancellationToken::new();
    }
}
