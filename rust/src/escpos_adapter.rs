// ============================================================
// escpos_adapter.rs — escpos crate wrapper
//
// Implements a custom MemoryDriver that implements the escpos Driver trait
// capturing bytes into a Vec<u8> in memory.
// ============================================================

use std::cell::RefCell;
use std::rc::Rc;

use escpos::{
    driver::Driver, errors::Result as EscposResult, printer::Printer,
    printer_options::PrinterOptions, utils::*,
};
use tracing::debug;

use crate::errors::{PrinterError, Result};

// ──────────────────────────────────────────────────────────────────
// MemoryDriver: lightweight implementation of the escpos Driver trait
// capturing all bytes into a Vec<u8> without Arc/Mutex overhead.
// ──────────────────────────────────────────────────────────────────
#[derive(Clone, Default)]
pub struct MemoryDriver {
    pub buffer: Rc<RefCell<Vec<u8>>>,
}

impl MemoryDriver {
    pub fn new() -> Self {
        Self {
            buffer: Rc::new(RefCell::new(Vec::with_capacity(1024))),
        }
    }

    pub fn take_buffer(&self) -> Vec<u8> {
        let mut buf = self.buffer.borrow_mut();
        std::mem::take(&mut *buf)
    }
}

impl Driver for MemoryDriver {
    fn name(&self) -> String {
        "MemoryDriver".to_string()
    }

    fn write(&self, data: &[u8]) -> EscposResult<()> {
        self.buffer.borrow_mut().extend_from_slice(data);
        Ok(())
    }

    fn read(&self, _buf: &mut [u8]) -> EscposResult<usize> {
        Ok(0)
    }

    fn flush(&self) -> EscposResult<()> {
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────
// ReceiptLine — receipt item
// ──────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct ReceiptLine {
    pub label: String,
    pub value: String,
}

// ──────────────────────────────────────────────────────────────────
// EscposAdapter — builds ESC/POS buffers in memory
// ──────────────────────────────────────────────────────────────────
pub struct EscposAdapter {
    paper_width: u8,
    /// Precomputed separator line (constant for a given paper width) so receipts
    /// don't re-allocate it on every build.
    separator: String,
}

impl EscposAdapter {
    pub fn new(paper_width: u8) -> Self {
        Self {
            paper_width,
            separator: "-".repeat(paper_width as usize),
        }
    }

    /// Generates an ESC/POS buffer for simple text with paper cut.
    pub fn build_text(&self, text: &str) -> Result<Vec<u8>> {
        let buf = self.with_printer(|p| {
            p.init()?
                .justify(JustifyMode::LEFT)?
                .writeln(text)?
                .feed()?
                .print_cut()
        })?;
        debug!(bytes = buf.len(), "Text buffer generated");
        Ok(buf)
    }

    /// Generates a complete receipt from string pairs (zero-copy labels/values).
    pub fn build_receipt_pairs(
        &self,
        title: &str,
        lines: &[(&str, &str)],
        total: &str,
        qr_data: Option<&str>,
    ) -> Result<Vec<u8>> {
        let sep = self.separator();

        let buf = self.with_printer(|p| {
            // Reused line buffer — one allocation per receipt, not per line.
            let mut row = String::with_capacity(self.paper_width as usize);

            p.init()?
                .justify(JustifyMode::CENTER)?
                .bold(true)?
                .size(1, 1)?
                .writeln(title)?
                .bold(false)?
                .writeln(sep)?
                .justify(JustifyMode::LEFT)?;

            for (label, value) in lines {
                self.format_line_into(&mut row, label, value);
                p.writeln(&row)?;
            }

            // ── Separator and total ────────────────────────────────────
            p.writeln(sep)?
                .justify(JustifyMode::RIGHT)?
                .bold(true)?
                .writeln(&format!("TOTAL: {}", total))?
                .bold(false)?
                .feed()?;

            // ── Optional QR Code (requires feature codes_2d) ──────────
            #[cfg(feature = "codes_2d")]
            if let Some(qr) = qr_data {
                p.justify(JustifyMode::CENTER)?
                    .qrcode_option(
                        qr,
                        QRCodeOption::new(QRCodeModel::Model2, 5, QRCodeCorrectionLevel::M),
                    )?
                    .feed()?;
            }
            #[cfg(not(feature = "codes_2d"))]
            let _ = qr_data;

            p.print_cut()
        })?;

        debug!(bytes = buf.len(), "Complete receipt buffer generated");
        Ok(buf)
    }

    /// Generates a complete receipt: title, lines, total and optional QR.
    pub fn build_receipt(
        &self,
        title: &str,
        lines: &[ReceiptLine],
        total: &str,
        qr_data: Option<&str>,
    ) -> Result<Vec<u8>> {
        let pairs: Vec<(&str, &str)> = lines
            .iter()
            .map(|l| (l.label.as_str(), l.value.as_str()))
            .collect();
        self.build_receipt_pairs(title, &pairs, total, qr_data)
    }

    /// Generates a centered QR code with paper cut.
    #[cfg(feature = "codes_2d")]
    pub fn build_qr(&self, data: &str) -> Result<Vec<u8>> {
        self.with_printer(|p| {
            p.init()?
                .justify(JustifyMode::CENTER)?
                .qrcode_option(
                    data,
                    QRCodeOption::new(QRCodeModel::Model2, 6, QRCodeCorrectionLevel::M),
                )?
                .feed()?
                .print_cut()
        })
    }

    /// Generates an EAN13 barcode.
    #[cfg(feature = "barcodes")]
    pub fn build_ean13(&self, data: &str) -> Result<Vec<u8>> {
        self.with_printer(|p| {
            p.init()?
                .justify(JustifyMode::CENTER)?
                .ean13_option(
                    data,
                    BarcodeOption::new(
                        BarcodeWidth::M,
                        BarcodeHeight::M,
                        BarcodeFont::A,
                        BarcodePosition::Below,
                    ),
                )?
                .feed()?
                .print_cut()
        })
    }

    /// Paper cut only.
    pub fn build_cut(&self) -> Result<Vec<u8>> {
        self.with_printer(|p| p.init()?.cut())
    }

    // ── Internal helpers ──────────────────────────────────────────

    fn with_printer<F>(&self, f: F) -> Result<Vec<u8>>
    where
        F: FnOnce(&mut Printer<MemoryDriver>) -> EscposResult<&mut Printer<MemoryDriver>>,
    {
        let driver = MemoryDriver::new();
        let driver_clone = driver.clone(); // shares the Arc<Mutex<Vec<u8>>>

        let mut printer =
            Printer::new(driver, Protocol::default(), Some(PrinterOptions::default()));

        f(&mut printer).map_err(|e| PrinterError::EscposError(e.to_string()))?;

        Ok(driver_clone.take_buffer())
    }

    fn separator(&self) -> &str {
        &self.separator
    }

    /// Formats `label ... value` into `buf` (reused across lines), padded to the
    /// paper width. Avoids a fresh `String` allocation per receipt line.
    fn format_line_into(&self, buf: &mut String, label: &str, value: &str) {
        buf.clear();
        let width = self.paper_width as usize;
        let available = width.saturating_sub(label.len() + value.len());
        buf.push_str(label);
        for _ in 0..available {
            buf.push(' ');
        }
        buf.push_str(value);
    }
}

// ──────────────────────────────────────────────────────────────────
// Unit tests (no physical printer)
// ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> EscposAdapter {
        EscposAdapter::new(48)
    }

    #[test]
    fn test_build_text_returns_nonempty_buffer() {
        let buf = adapter().build_text("Hello World").unwrap();
        assert!(!buf.is_empty(), "ESC/POS buffer should not be empty");
    }

    #[test]
    fn test_buffer_contains_esc_init() {
        let buf = adapter().build_text("Test").unwrap();
        // ESC/POS always starts with ESC @ (0x1B 0x40)
        assert!(
            buf.windows(2).any(|w| w == [0x1B, 0x40]),
            "Buffer should contain ESC @ (init command)"
        );
    }

    #[test]
    fn test_build_receipt_nonempty() {
        let lines = vec![
            ReceiptLine {
                label: "Coffee".into(),
                value: "$45.00".into(),
            },
            ReceiptLine {
                label: "Water".into(),
                value: "$20.00".into(),
            },
        ];
        let buf = adapter()
            .build_receipt("TICKET", &lines, "$65.00", None)
            .unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_build_receipt_with_qr() {
        let buf = adapter()
            .build_receipt("TEST", &[], "$0.00", Some("https://example.com"))
            .unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_memory_driver_captures_bytes() {
        let driver = MemoryDriver::new();
        driver.write(b"hello").unwrap();
        driver.write(b" world").unwrap();
        let buf = driver.take_buffer();
        assert_eq!(buf, b"hello world");
    }
}
