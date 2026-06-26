# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-06-26

### Added

- `SessionControl` with `CancellationToken` and a 16 MiB print-queue byte budget.
- `clear_print_queue()` cancels in-flight retries and drains pending jobs, releasing queue budget.
- Sync print path routed through the background worker (`PrintAwait`) for unified job ordering.
- TCP and BLE liveness probes; BLE skips a full scan when the target device is already visible.
- Desktop USB mid-transfer cancellation via async `nusb` and `tokio::select!`.
- `UsbTransport::teardown_inflight()` on cancel, timeout, or I/O failure.
- Cancellable USB `connect()` (honours session cancel token during open/claim).
- `ThermalPrinterRs.dispose()`, `writeTicket()`, and status-poll backoff on the Dart side.
- `dispose()` on Android/iOS transports with error-safe connect/disconnect.
- Integration tests for IO-task disconnect, retry cancellation, and zero-copy send.

### Changed

- Version bump `0.0.1` → `0.1.0` (no Dart/FRB API breaking changes).
- Centralized printer shutdown: worker join, state-stream abort, IO-task transport disconnect.
- Zero-copy buffer send when `max_retries == 0`; clone backup only when retries are enabled.
- BLE default chunk size reduced to 20 bytes for low-MTU printers.
- Desktop USB migrated from blocking `rusb` to async `nusb` (Tokio feature).
- `PrintService` passes `session.cancel_token()` into `UsbTransport::new`.
- USB `check_liveness()` uses `list_devices` instead of a `bulk_in` probe.
- USB `read()` is explicitly write-only (typical thermal printers have no IN endpoint).
- `build_receipt_pairs` avoids per-line `String` allocations in Rust.
- `isConnected` reads live state via `getPrinterState()` instead of a stale flag.
- Managed `stateStream` lifecycle; `init*` methods disconnect before re-initializing.
- `TicketBuilder`: deduplicated size ESC commands and optimized `feed()`.

### Fixed

- Resource leaks and stale connection state across Rust shutdown and Dart transport lifecycle.
- IO task blocked for seconds by in-flight USB writes when the print queue was cleared.
- Raster encode validates dimensions; Dart throws on empty encode results.
- Android/iOS `_connected` flag reset on failed connect/disconnect paths.

## [0.0.1] - 2026-06-25

### Added

- Initial release of **ThermalPrinterRs** — cross-platform ESC/POS thermal printing for Flutter via `flutter_rust_bridge`.
- Rust core: `PrintService`, `EscposAdapter`, background print worker (Tokio), and `Transport` trait.
- Transports: TCP/IP (all platforms), USB desktop (`rusb`/libusb), BLE desktop (`btleplug`), in-memory `MockTransport`.
- Native Android bridges: USB and Bluetooth Classic (SPP) via JNI / `MethodChannel`.
- iOS BLE transport via CoreBluetooth (`MethodChannel`).
- High-level Dart API: `ThermalPrinterRs`, `TicketBuilder`, printer status polling (DLE EOT).
- ESC/POS features: text, receipts, QR/barcodes, raster graphics (feature flags).
- GitHub Actions CI matrix: Linux, Windows, macOS, Android; Rust fmt/clippy/test; Flutter analyze/test.
- Benchmarks workflow and `BENCHMARKS.md`.

[0.1.0]: https://github.com/xdvi/thermal_printer_rs/compare/428db34...v0.1.0
[0.0.1]: https://github.com/xdvi/thermal_printer_rs/releases/tag/v0.0.1