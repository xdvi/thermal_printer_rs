// ============================================================
// tests/integration_test.rs — Integration tests for PrintService
//
// These tests verify the full pipeline:
//   EscposAdapter -> PrintService -> MockTransport
//
// No physical printer required. All bytes are captured in memory
// and validated for correctness.
// ============================================================

use std::sync::{Arc, Mutex};

use thermal_printer_rs::config::{CharEncoding, PrinterConfig, TransportKind};
use thermal_printer_rs::printer::PrintService;
use thermal_printer_rs::session::SessionControl;
use thermal_printer_rs::transport::mock::{MockConfig, MockTransport};

// Helper: creates a PrintService wired to a MockTransport.
// Returns (service, captured_buffer).
fn make_mock_service(config: MockConfig) -> (PrintService, Arc<Mutex<Vec<u8>>>) {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let transport = Box::new(MockTransport::new_with_buffer(buffer.clone()).with_config(config));
    let printer_config = PrinterConfig {
        transport: TransportKind::Tcp {
            host: "127.0.0.1".into(),
            port: 9100,
        },
        timeout_ms: 1000,
        paper_width: 48,
        encoding: CharEncoding::default(),
        max_retries: 0,
    };
    let service = PrintService::new_with_transport(
        printer_config,
        transport,
        Arc::new(SessionControl::new()),
    );
    (service, buffer)
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

// ── Connectivity ────────────────────────────────────────────────────

#[test]
fn test_service_connect_and_disconnect() {
    rt().block_on(async {
        let (service, _buf) = make_mock_service(MockConfig::default());
        service.connect().await.expect("Connect should succeed");
        service
            .disconnect()
            .await
            .expect("Disconnect should succeed");
    });
}

#[test]
fn test_connect_fails_when_configured_to_fail() {
    rt().block_on(async {
        let (service, _buf) = make_mock_service(MockConfig {
            fail_on_connect: true,
            ..Default::default()
        });
        let result = service.connect().await;
        assert!(
            result.is_err(),
            "Connect should fail when configured to fail"
        );
    });
}

// ── print_text ─────────────────────────────────────────────────────

#[test]
fn test_print_text_sends_escpos_bytes() {
    rt().block_on(async {
        let (service, buffer) = make_mock_service(MockConfig::default());
        let bytes = service
            .print_text("Hello World")
            .await
            .expect("print_text should succeed");

        assert!(bytes > 0, "Should report bytes sent");
        let captured = buffer.lock().unwrap();
        assert!(!captured.is_empty(), "Buffer should contain ESC/POS bytes");
        // ESC @ = printer init command
        assert!(
            captured.windows(2).any(|w| w == [0x1B, 0x40]),
            "Buffer must contain ESC @ (init command)"
        );
    });
}

#[test]
fn test_print_text_auto_connects() {
    rt().block_on(async {
        let (service, buffer) = make_mock_service(MockConfig::default());
        // No explicit connect() call — service should auto-connect
        service
            .print_text("Auto-connect test")
            .await
            .expect("Service should auto-connect and print");
        assert!(!buffer.lock().unwrap().is_empty());
    });
}

#[test]
fn test_print_text_fails_when_write_fails() {
    rt().block_on(async {
        let (service, _buffer) = make_mock_service(MockConfig {
            starts_connected: true,
            fail_on_write: true,
            ..Default::default()
        });
        let result = service.print_text("This should fail").await;
        assert!(
            result.is_err(),
            "Should fail when write is configured to fail"
        );
    });
}

// ── print_receipt ──────────────────────────────────────────────────

#[test]
fn test_print_receipt_sends_bytes() {
    rt().block_on(async {
        let (service, buffer) = make_mock_service(MockConfig::default());
        let lines = [("Americano Coffee", "$45.00"), ("Mixed Sandwich", "$89.00")];
        let bytes = service
            .print_receipt("SALE TICKET", &lines, "$134.00", None)
            .await
            .expect("print_receipt should succeed");

        assert!(bytes > 0);
        let captured = buffer.lock().unwrap();
        assert!(!captured.is_empty());
        assert!(captured.windows(2).any(|w| w == [0x1B, 0x40]));
    });
}

#[test]
fn test_print_receipt_with_qr_is_larger() {
    rt().block_on(async {
        let lines = [("Item A", "$10.00")];

        let (svc_plain, _) = make_mock_service(MockConfig::default());
        let bytes_plain = svc_plain
            .print_receipt("TEST", &lines, "$10.00", None)
            .await
            .unwrap();

        let (svc_qr, _) = make_mock_service(MockConfig::default());
        let bytes_qr = svc_qr
            .print_receipt("TEST", &lines, "$10.00", Some("https://example.com"))
            .await
            .unwrap();

        assert!(
            bytes_qr > bytes_plain,
            "Receipt with QR ({bytes_qr}B) should be larger than without QR ({bytes_plain}B)"
        );
    });
}

// ── Adapter access ─────────────────────────────────────────────────

#[test]
fn test_adapter_builds_text_buffer() {
    rt().block_on(async {
        let (service, _) = make_mock_service(MockConfig::default());
        let buf = service
            .adapter()
            .build_text("Adapter test")
            .expect("Adapter should build ESC/POS buffer");
        assert!(!buf.is_empty());
        assert!(buf.windows(2).any(|w| w == [0x1B, 0x40]));
    });
}

// ── send_buffer_owned (background worker path) ──────────────────

#[test]
fn test_send_buffer_owned_writes_raw_bytes() {
    rt().block_on(async {
        let (service, buffer) = make_mock_service(MockConfig {
            starts_connected: true,
            ..Default::default()
        });
        let raw = b"raw escpos data";
        let bytes = service
            .send_buffer_owned_retrying(raw.to_vec())
            .await
            .expect("send_buffer_owned should succeed");
        assert_eq!(bytes, raw.len());
        assert_eq!(*buffer.lock().unwrap(), raw);
    });
}

#[test]
fn test_clear_queue_cancels_retry_backoff() {
    rt().block_on(async {
        let session = Arc::new(SessionControl::new());
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let transport = Box::new(MockTransport::new_with_buffer(buffer.clone()).with_config(
            MockConfig {
                fail_on_write: true,
                ..Default::default()
            },
        ));
        let printer_config = PrinterConfig {
            transport: TransportKind::Tcp {
                host: "127.0.0.1".into(),
                port: 9100,
            },
            timeout_ms: 1000,
            paper_width: 48,
            encoding: CharEncoding::default(),
            max_retries: 3,
        };
        let service = PrintService::new_with_transport(printer_config, transport, session.clone());

        service.connect().await.unwrap();
        session.signal_cancel();

        let result = service.send_buffer_owned_retrying(vec![0x1b, 0x40]).await;
        assert!(
            matches!(
                result,
                Err(thermal_printer_rs::errors::PrinterError::JobCancelled)
            ),
            "expected JobCancelled after signal_cancel, got {result:?}"
        );
    });
}

#[test]
fn test_send_buffer_without_retries_succeeds() {
    rt().block_on(async {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let transport = Box::new(MockTransport::new_with_buffer(buffer.clone()).with_config(
            MockConfig {
                starts_connected: true,
                ..Default::default()
            },
        ));
        let printer_config = PrinterConfig {
            transport: TransportKind::Tcp {
                host: "127.0.0.1".into(),
                port: 9100,
            },
            timeout_ms: 1000,
            paper_width: 48,
            encoding: CharEncoding::default(),
            max_retries: 0,
        };
        let service = PrintService::new_with_transport(
            printer_config,
            transport,
            Arc::new(SessionControl::new()),
        );
        let payload = vec![0xAA; 4096];
        let sent = service
            .send_buffer_owned_retrying(payload.clone())
            .await
            .expect("send without retries should succeed");
        assert_eq!(sent, payload.len());
        assert_eq!(*buffer.lock().unwrap(), payload);
    });
}
