// ============================================================
// api/simple.rs — Public API exposed to flutter_rust_bridge
//
// Optimized for zero-blocking IO and direct async usage.
// ============================================================

use flutter_rust_bridge::frb;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::task::AbortHandle;
use tracing::{error, info};

use crate::{
    config::{CharEncoding, PrinterConfig, TransportKind},
    errors::PrinterError,
    jobs::{PrintCommand, PrintWorker},
    printer::PrintService,
    session::SessionControl,
};
use tokio::sync::{mpsc, oneshot};

// ── Thread-safe service singleton ───────────────────────────
static SERVICE: Lazy<Mutex<Option<Arc<PrintService>>>> = Lazy::new(|| Mutex::new(None));

// ── Background job channel ──────────────────────────────────────
static COMMAND_SENDER: Lazy<Mutex<Option<mpsc::Sender<PrintCommand>>>> =
    Lazy::new(|| Mutex::new(None));

// ══════════════════════════════════════════════════════════════════
// DTOs — serializable types for flutter_rust_bridge
// ══════════════════════════════════════════════════════════════════

/// Transport type for connection
#[frb]
#[derive(Debug, Clone)]
pub enum TransportTypeDto {
    Tcp,
    Usb,
    Bluetooth,
}

/// Printer configuration (passed from Dart)
#[frb]
#[derive(Debug, Clone)]
pub struct PrinterConfigDto {
    pub transport: TransportTypeDto,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub ble_address: Option<String>,
    pub timeout_ms: u64,
    pub paper_width: u8,
    pub max_retries: u8,
}

impl Default for PrinterConfigDto {
    fn default() -> Self {
        Self {
            transport: TransportTypeDto::Tcp,
            host: Some("192.168.1.100".into()),
            port: Some(9100),
            vendor_id: None,
            product_id: None,
            ble_address: None,
            timeout_ms: 5000,
            paper_width: 48,
            max_retries: 3,
        }
    }
}

/// Result of a print operation
#[frb]
#[derive(Debug, Clone)]
pub struct PrintResultDto {
    pub success: bool,
    pub bytes_sent: u32,
    pub error_message: Option<String>,
}

impl PrintResultDto {
    fn ok(bytes: usize) -> Self {
        Self {
            success: true,
            bytes_sent: bytes as u32,
            error_message: None,
        }
    }
    fn err(msg: impl ToString) -> Self {
        Self {
            success: false,
            bytes_sent: 0,
            error_message: Some(msg.to_string()),
        }
    }
}

/// Receipt item (label + value/price)
#[frb]
#[derive(Debug, Clone)]
pub struct ReceiptLineDto {
    pub label: String,
    pub value: String,
}

// ══════════════════════════════════════════════════════════════════
// Public bridge functions
// ══════════════════════════════════════════════════════════════════

/// Printer state tracked by the background worker
#[frb]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterStateDto {
    Disconnected,
    Connecting,
    Connected,
    Printing,
    Error,
}

impl From<crate::jobs::WorkerState> for PrinterStateDto {
    fn from(state: crate::jobs::WorkerState) -> Self {
        match state {
            crate::jobs::WorkerState::Disconnected => Self::Disconnected,
            crate::jobs::WorkerState::Connecting => Self::Connecting,
            crate::jobs::WorkerState::Connected => Self::Connected,
            crate::jobs::WorkerState::Printing => Self::Printing,
            crate::jobs::WorkerState::Error => Self::Error,
        }
    }
}

// ── State channel ──────────────────────────────────────────────────
static STATE_RECEIVER: Lazy<Mutex<Option<tokio::sync::watch::Receiver<crate::jobs::WorkerState>>>> =
    Lazy::new(|| Mutex::new(None));

static WORKER_HANDLE: Lazy<Mutex<Option<tokio::task::JoinHandle<()>>>> =
    Lazy::new(|| Mutex::new(None));

static STATE_STREAM_ABORT: Lazy<Mutex<Option<AbortHandle>>> = Lazy::new(|| Mutex::new(None));

static SESSION: Lazy<Mutex<Option<Arc<SessionControl>>>> = Lazy::new(|| Mutex::new(None));

async fn shutdown_internal() {
    if let Some(session) = SESSION.lock().take() {
        session.signal_cancel();
        session.queue_budget.reset();
    }
    if let Some(abort) = STATE_STREAM_ABORT.lock().take() {
        abort.abort();
    }

    let sender = COMMAND_SENDER.lock().take();
    if let Some(tx) = sender {
        let _ = tx.send(PrintCommand::Disconnect).await;
    }

    let handle = WORKER_HANDLE.lock().take();
    if let Some(handle) = handle {
        let _ = handle.await;
    }

    let service = SERVICE.lock().take();
    if let Some(service) = service {
        let _ = service.disconnect().await;
    }

    *STATE_RECEIVER.lock() = None;
}

async fn dispatch_print_await(buf: Vec<u8>) -> Result<usize, String> {
    let session = SESSION
        .lock()
        .clone()
        .ok_or("PrintService not initialized")?;
    let len = buf.len();
    session
        .queue_budget
        .try_reserve(len)
        .map_err(|e| e.to_string())?;

    let sender = COMMAND_SENDER
        .lock()
        .clone()
        .ok_or("Background worker not running")?;

    let (tx, rx) = oneshot::channel();
    if sender
        .send(PrintCommand::PrintAwait { buf, resp: tx })
        .await
        .is_err()
    {
        session.queue_budget.release(len);
        return Err("Background worker not running".into());
    }

    match rx.await {
        Ok(Ok(n)) => Ok(n),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            session.queue_budget.release(len);
            Err("Background worker dropped".into())
        }
    }
}

fn enqueue_with_budget(bytes: Vec<u8>) -> Result<(), String> {
    let session = SESSION
        .lock()
        .clone()
        .ok_or("PrintService not initialized")?;
    let len = bytes.len();
    session
        .queue_budget
        .try_reserve(len)
        .map_err(|e| e.to_string())?;

    let tx = COMMAND_SENDER
        .lock()
        .as_ref()
        .ok_or("Background worker not running")?
        .clone();

    match tx.try_send(PrintCommand::Print(bytes)) {
        Ok(()) => Ok(()),
        Err(e) => {
            session.queue_budget.release(len);
            Err(format!("Failed to enqueue: {e}"))
        }
    }
}

/// Returns the current printer state as a synchronous snapshot.
/// Useful for polling or checking state before an operation.
#[frb(sync)]
pub fn get_printer_state() -> PrinterStateDto {
    let guard = STATE_RECEIVER.lock();
    match guard.as_ref() {
        Some(rx) => PrinterStateDto::from(*rx.borrow()),
        None => PrinterStateDto::Disconnected,
    }
}

/// Subscribes to the printer state stream (push model).
///
/// The `StreamSink` type here is the newtype generated by FRB's
/// `frb_generated_boilerplate_io!()` macro. Accessed via the private
/// `frb_generated` module. FRB codegen constructs this type and passes
/// it here automatically — do not call this function from Dart directly.
pub fn create_state_stream(
    sink: crate::frb_generated::StreamSink<
        PrinterStateDto,
        flutter_rust_bridge::for_generated::SseCodec,
    >,
) -> Result<(), String> {
    let rx = {
        let guard = STATE_RECEIVER.lock();
        guard
            .as_ref()
            .cloned()
            .ok_or("PrintService not initialized")?
    };

    if let Some(abort) = STATE_STREAM_ABORT.lock().take() {
        abort.abort();
    }

    let task = tokio::spawn(async move {
        let mut watch_rx = rx;
        let _ = sink.add(PrinterStateDto::from(*watch_rx.borrow()));
        while watch_rx.changed().await.is_ok() {
            let state = *watch_rx.borrow();
            if sink.add(PrinterStateDto::from(state)).is_err() {
                break;
            }
        }
    });
    *STATE_STREAM_ABORT.lock() = Some(task.abort_handle());

    Ok(())
}

/// Initializes the print service.
pub async fn init_printer(config: PrinterConfigDto) -> Result<(), String> {
    shutdown_internal().await;

    let printer_config = build_config(config).map_err(|e| e.to_string())?;
    let session = Arc::new(SessionControl::new());
    let service = PrintService::new(printer_config, session.clone()).map_err(|e| e.to_string())?;
    let service_arc = Arc::new(service);

    let (tx, rx) = mpsc::channel(256);
    let (state_tx, state_rx) = tokio::sync::watch::channel(crate::jobs::WorkerState::Disconnected);

    let worker = PrintWorker::new(service_arc.clone(), rx, state_tx, session.clone());
    let worker_handle = tokio::spawn(worker.run());

    *SESSION.lock() = Some(session);
    *SERVICE.lock() = Some(service_arc);
    *COMMAND_SENDER.lock() = Some(tx);
    *STATE_RECEIVER.lock() = Some(state_rx);
    *WORKER_HANDLE.lock() = Some(worker_handle);

    info!("PrintService initialized (Phase 1 IO task active, Phase 5 State tracking active)");
    Ok(())
}

/// Explicitly connects to the transport.
pub async fn connect_printer() -> Result<(), String> {
    let tx = {
        let guard = COMMAND_SENDER.lock();
        guard
            .as_ref()
            .cloned()
            .ok_or("PrintService not initialized")?
    };
    tx.send(PrintCommand::Connect)
        .await
        .map_err(|e| e.to_string())
}

/// Prints simple text.
pub async fn print_text(text: String) -> PrintResultDto {
    let buf = {
        let guard = SERVICE.lock();
        let svc = match guard.as_ref() {
            Some(s) => s,
            None => return PrintResultDto::err("PrintService not initialized"),
        };
        match svc.adapter().build_text(&text) {
            Ok(b) => b,
            Err(e) => return PrintResultDto::err(e),
        }
    };

    match dispatch_print_await(buf).await {
        Ok(n) => PrintResultDto::ok(n),
        Err(e) => PrintResultDto::err(e),
    }
}

/// Prints a complete receipt.
pub async fn print_receipt(
    title: String,
    lines: Vec<ReceiptLineDto>,
    total: String,
    qr_data: Option<String>,
) -> PrintResultDto {
    let buf = {
        let guard = SERVICE.lock();
        let svc = match guard.as_ref() {
            Some(s) => s,
            None => return PrintResultDto::err("PrintService not initialized"),
        };
        let pairs: Vec<(&str, &str)> = lines
            .iter()
            .map(|l| (l.label.as_str(), l.value.as_str()))
            .collect();
        match svc
            .adapter()
            .build_receipt_pairs(&title, &pairs, &total, qr_data.as_deref())
        {
            Ok(b) => b,
            Err(e) => return PrintResultDto::err(e),
        }
    };

    match dispatch_print_await(buf).await {
        Ok(n) => PrintResultDto::ok(n),
        Err(e) => PrintResultDto::err(e),
    }
}

/// Disconnects the transport and stops the worker.
pub async fn disconnect_printer() -> Result<(), String> {
    shutdown_internal().await;
    info!("PrintService disconnected");
    Ok(())
}

/// Drains all pending jobs in the background worker.
pub async fn clear_print_queue() -> Result<(), String> {
    if let Some(session) = SESSION.lock().as_ref() {
        session.signal_cancel();
    }

    let tx = {
        let guard = COMMAND_SENDER.lock();
        guard.as_ref().cloned().ok_or("Worker not running")?
    };
    tx.send(PrintCommand::ClearQueue)
        .await
        .map_err(|e| format!("Failed to send clear command: {}", e))?;
    Ok(())
}

/// Sends raw bytes and awaits until the hardware confirms the write.
///
/// This is the blocking counterpart to [enqueue_write_bytes].
/// Use this when the caller needs to know the bytes were actually transmitted
/// before proceeding (e.g., before cutting paper or opening a drawer).
pub async fn write_raw_bytes(bytes: Vec<u8>) -> Result<(), String> {
    dispatch_print_await(bytes).await.map(|_| ())
}

/// Reads raw bytes from the transport.
/// Used for querying printer status via `DLE EOT`.
pub async fn read_raw_bytes(bytes: u32, timeout_ms: u64) -> Result<Vec<u8>, String> {
    let service = {
        let guard = SERVICE.lock();
        guard
            .as_ref()
            .cloned()
            .ok_or("PrintService not initialized")?
    };
    service
        .read(bytes as usize, timeout_ms)
        .await
        .map_err(|e| e.to_string())
}

/// Enqueues a raw bytes payload (fire-and-forget).
///
/// Returns immediately after placing the job in the background queue.
/// The bytes will be sent by the background worker — there is no
/// acknowledgement that the hardware has actually received them.
/// Use [write_raw_bytes] if you need confirmed delivery.
pub fn enqueue_write_bytes(bytes: Vec<u8>) -> Result<(), String> {
    enqueue_with_budget(bytes)
}

/// Enqueues a text print job (non-blocking).
pub fn enqueue_print_text(text: String) -> Result<(), String> {
    let buf = {
        let guard = SERVICE.lock();
        let svc = guard.as_ref().ok_or("PrintService not initialized")?;
        svc.adapter().build_text(&text).map_err(|e| e.to_string())?
    };

    enqueue_write_bytes(buf)
}

/// Enqueues a receipt print job (non-blocking).
pub fn enqueue_print_receipt(
    title: String,
    lines: Vec<ReceiptLineDto>,
    total: String,
    qr_data: Option<String>,
) -> Result<(), String> {
    let buf = {
        let guard = SERVICE.lock();
        let svc = guard.as_ref().ok_or("PrintService not initialized")?;

        let receipt_lines: Vec<crate::escpos_adapter::ReceiptLine> = lines
            .into_iter()
            .map(|l| crate::escpos_adapter::ReceiptLine {
                label: l.label,
                value: l.value,
            })
            .collect();

        svc.adapter()
            .build_receipt(&title, &receipt_lines, &total, qr_data.as_deref())
            .map_err(|e| e.to_string())?
    };

    enqueue_write_bytes(buf)
}

/// Encodes raw RGBA pixel data into an ESC/POS GS v 0 raster image command
/// using Floyd-Steinberg dithering.
///
/// The heavy computation is moved to Tokio's blocking thread pool so the
/// async executor (and therefore the Flutter UI) is never starved.
///
/// Returns the complete ESC/POS byte sequence, ready to send to the printer.
pub async fn encode_raster_image(rgba: Vec<u8>, width: i64, height: i64) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    match tokio::task::spawn_blocking(move || _dither_and_encode(&rgba, width, height)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(e)) => {
            error!(error = %e, "Raster encode validation failed");
            Vec::new()
        }
        Err(e) => {
            error!(error = %e, "Raster encode task failed");
            Vec::new()
        }
    }
}

// ── Raster image internals (not exposed to Dart) ─────────────────────────

fn _dither_and_encode(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, PrinterError> {
    if width == 0 || height == 0 {
        return Err(PrinterError::InvalidConfig(
            "Raster image width and height must be > 0".into(),
        ));
    }
    let required = width
        .checked_mul(height)
        .and_then(|p| p.checked_mul(4))
        .ok_or_else(|| PrinterError::InvalidConfig("Raster image dimensions overflow".into()))?;
    if rgba.len() < required {
        return Err(PrinterError::InvalidConfig(format!(
            "RGBA buffer too small: need {required} bytes, got {}",
            rgba.len()
        )));
    }

    let n = width * height;
    let mut gray = vec![0i32; n];

    // RGBA → grayscale with alpha-compositing over a white background.
    // All integer math: avoids FPU setup cost per pixel.
    for (i, gray_val) in gray.iter_mut().enumerate().take(n) {
        let b = i * 4;
        let r = rgba[b] as i32;
        let g = rgba[b + 1] as i32;
        let bv = rgba[b + 2] as i32;
        let a = rgba[b + 3] as i32;
        let ia = 255 - a;

        // Blend over white: channel_out = (channel * a + 255 * (255-a)) / 255
        let br = (r * a + 255 * ia + 127) / 255;
        let bg = (g * a + 255 * ia + 127) / 255;
        let bb = (bv * a + 255 * ia + 127) / 255;

        // Luminosity weights (BT.601, integer approximation × 1000)
        *gray_val = (299 * br + 587 * bg + 114 * bb) / 1000;
    }

    // Floyd-Steinberg dithering.
    // Skip diffusion when err == 0 (pure white or pure black pixels).
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let old = gray[idx];
            let neo = if old < 128 { 0i32 } else { 255i32 };
            gray[idx] = neo;
            let err = old - neo;
            if err == 0 {
                continue;
            }

            if x + 1 < width {
                let i = idx + 1;
                gray[i] = (gray[i] + err * 7 / 16).clamp(0, 255);
            }
            if y + 1 < height {
                if x > 0 {
                    let i = idx + width - 1;
                    gray[i] = (gray[i] + err * 3 / 16).clamp(0, 255);
                }
                {
                    let i = idx + width;
                    gray[i] = (gray[i] + err * 5 / 16).clamp(0, 255);
                }
                if x + 1 < width {
                    let i = idx + width + 1;
                    gray[i] = (gray[i] + err / 16).clamp(0, 255);
                }
            }
        }
    }

    // Build GS v 0 raster bit-image command.
    // Single pre-allocated Vec — no reallocations during assembly.
    let bpl = width.div_ceil(8); // bytes per raster line
    let mut cmd = Vec::with_capacity(8 + bpl * height);

    cmd.extend_from_slice(&[0x1D, 0x76, 0x30, 0x00]);
    cmd.push((bpl & 0xFF) as u8);
    cmd.push(((bpl >> 8) & 0xFF) as u8);
    cmd.push((height & 0xFF) as u8);
    cmd.push(((height >> 8) & 0xFF) as u8);

    for y in 0..height {
        let mut bit = 0u8;
        let mut cur = 0u8;
        for x in 0..width {
            if gray[y * width + x] == 0 {
                cur |= 1 << (7 - bit);
            }
            bit += 1;
            if bit == 8 {
                cmd.push(cur);
                cur = 0;
                bit = 0;
            }
        }
        if bit > 0 {
            cmd.push(cur);
        }
    }

    Ok(cmd)
}

#[frb(sync)]
pub fn is_printer_ready() -> bool {
    SERVICE.lock().is_some()
}

// ── Helpers ──────────────────────────────────────────────────────

fn build_config(dto: PrinterConfigDto) -> crate::errors::Result<PrinterConfig> {
    let transport = match dto.transport {
        TransportTypeDto::Tcp => {
            let host = dto
                .host
                .ok_or_else(|| PrinterError::InvalidConfig("TCP requires 'host'".into()))?;
            TransportKind::Tcp {
                host,
                port: dto.port.unwrap_or(9100),
            }
        }
        TransportTypeDto::Usb => {
            let vid = dto
                .vendor_id
                .ok_or_else(|| PrinterError::InvalidConfig("USB requires 'vendor_id'".into()))?;
            let pid = dto
                .product_id
                .ok_or_else(|| PrinterError::InvalidConfig("USB requires 'product_id'".into()))?;
            TransportKind::Usb {
                vendor_id: vid,
                product_id: pid,
            }
        }
        TransportTypeDto::Bluetooth => {
            let addr = dto
                .ble_address
                .ok_or_else(|| PrinterError::InvalidConfig("BLE requires 'ble_address'".into()))?;
            TransportKind::Ble { address: addr }
        }
    };

    Ok(PrinterConfig {
        transport,
        timeout_ms: dto.timeout_ms,
        paper_width: dto.paper_width,
        encoding: CharEncoding::default(),
        max_retries: dto.max_retries,
    })
}
