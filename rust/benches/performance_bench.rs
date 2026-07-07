use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::{Arc, Mutex};
use thermal_printer_rs::api::simple::encode_raster_image;
use thermal_printer_rs::config::PrinterConfig;
use thermal_printer_rs::escpos_adapter::{EscposAdapter, ReceiptLine};
use thermal_printer_rs::printer::PrintService;
use thermal_printer_rs::session::SessionControl;
use thermal_printer_rs::transport::mock::{MockConfig, MockTransport};
use tokio::runtime::Runtime;

// ── Helpers ──────────────────────────────────────────────────────

fn create_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn create_mock_service(rt: &Runtime) -> (PrintService, Arc<Mutex<Vec<u8>>>) {
    let _guard = rt.enter();
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let transport = Box::new(MockTransport::new_with_buffer(buffer.clone()).with_config(
        MockConfig {
            starts_connected: true,
            ..Default::default()
        },
    ));
    let config = PrinterConfig::default();
    let service =
        PrintService::new_with_transport(config, transport, Arc::new(SessionControl::new()));
    (service, buffer)
}

// ── Benchmarks ───────────────────────────────────────────────────

fn bench_adapter_generation(c: &mut Criterion) {
    let adapter = EscposAdapter::new(48);

    // Case 1: Simple text (2 KiB approx)
    let text_2k = "A".repeat(2048);
    c.bench_function("adapter_build_text_2k", |b| {
        b.iter(|| {
            let _ = adapter.build_text(black_box(&text_2k)).unwrap();
        })
    });

    // Case 2: Receipt (8 KiB approx)
    let lines: Vec<ReceiptLine> = (0..100)
        .map(|i| ReceiptLine {
            label: format!("Product Item {:03}", i),
            value: format!("${}.99", i),
        })
        .collect();

    c.bench_function("adapter_build_receipt_8k", |b| {
        b.iter(|| {
            let _ = adapter
                .build_receipt(
                    black_box("BENCHMARK TICKET"),
                    black_box(&lines),
                    black_box("$9999.99"),
                    black_box(Some("https://example.com/qr")),
                )
                .unwrap();
        })
    });
}

fn bench_transport_throughput(c: &mut Criterion) {
    let rt = create_runtime();
    let (service, _buffer) = create_mock_service(&rt);

    // Case: Send 128 KiB bitmap (simulated raw buffer)
    let data_128k = vec![0u8; 128 * 1024];

    c.bench_function("service_send_buffer_128k", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = service
                .send_buffer_owned_retrying(black_box(data_128k.clone()))
                .await
                .unwrap();
        })
    });
}

fn bench_full_pipeline(c: &mut Criterion) {
    let rt = create_runtime();
    let (service, _buffer) = create_mock_service(&rt);
    let text_2k = "B".repeat(2048);

    c.bench_function("service_print_text_2k_pipeline", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = service.print_text(black_box(&text_2k)).await.unwrap();
        })
    });
}

fn bench_raster_encode(c: &mut Criterion) {
    let rt = create_runtime();
    let _guard = rt.enter();
    // 576px (standard 58mm paper) × 800 rows ≈ a typical receipt image.
    let (w, h): (usize, usize) = (576, 800);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for i in 0..(w * h) {
        let v = ((i * 7) % 256) as u8;
        rgba.extend_from_slice(&[v, 255 - v, v / 2, 255]);
    }

    c.bench_function("raster_encode_576x800", |b| {
        b.to_async(&rt).iter(|| {
            let data = black_box(rgba.clone());
            async move { encode_raster_image(data, w as i64, h as i64).await }
        })
    });
}

criterion_group!(
    benches,
    bench_adapter_generation,
    bench_transport_throughput,
    bench_full_pipeline,
    bench_raster_encode
);
criterion_main!(benches);
