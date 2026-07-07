// ============================================================
// config.rs — Configuration structures
// ============================================================

/// Transport type to use for the connection.
#[derive(Debug, Clone)]
pub enum TransportKind {
    /// TCP/IP — Network printers (typical port 9100)
    Tcp { host: String, port: u16 },
    /// USB — Desktop only (Linux, Windows, macOS)
    Usb { vendor_id: u16, product_id: u16 },
    /// Bluetooth Low Energy — Desktop and mobile (limited)
    Ble { address: String },
}

/// Complete configuration for the print service.
#[derive(Debug, Clone)]
pub struct PrinterConfig {
    pub transport: TransportKind,
    pub timeout_ms: u64,
    /// Paper width in characters (typical: 32 or 48)
    pub paper_width: u8,
    /// Character encoding (default: PC437)
    pub encoding: CharEncoding,
    /// Number of automatic reconnection attempts
    pub max_retries: u8,
    /// BLE only: override the write chunk size (bytes). `None` (default)
    /// auto-detects from the negotiated MTU at connect time (falls back to
    /// 20 if the platform can't report one). Only set this for a printer
    /// that needs a value other than mtu - 3.
    pub ble_chunk_size: Option<usize>,
    /// BLE only: override the inter-chunk delay. `None` keeps the 20ms default.
    /// Set to 0 when the transport's own write backpressure suffices.
    pub ble_chunk_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub enum CharEncoding {
    #[default]
    Pc437,
    Utf8,
    Iso8859_1,
}

impl Default for PrinterConfig {
    fn default() -> Self {
        Self {
            transport: TransportKind::Tcp {
                host: "127.0.0.1".into(),
                port: 9100,
            },
            timeout_ms: 5000,
            paper_width: 48,
            encoding: CharEncoding::Pc437,
            max_retries: 3,
            ble_chunk_size: None,
            ble_chunk_delay_ms: None,
        }
    }
}
