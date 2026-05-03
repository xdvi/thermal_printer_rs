// ============================================================
// errors.rs — Typed errors with thiserror
// ============================================================
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PrinterError {
    #[error("Transport unavailable: {0}")]
    TransportUnavailable(String),

    #[error("Connection error: {0}")]
    ConnectionFailed(String),

    #[error("Timeout while communicating with the printer")]
    Timeout,

    #[error("Error building ESC/POS command: {0}")]
    EscposError(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Print job cancelled")]
    JobCancelled,

    #[error("Transport not supported on this platform")]
    PlatformNotSupported,

    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Printer not found: {0}")]
    PrinterNotFound(String),

    #[error("Configuration error: {0}")]
    InvalidConfig(String),
}

pub type Result<T> = std::result::Result<T, PrinterError>;
