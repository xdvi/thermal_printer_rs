/// thermal_printer_rs — High-level public API for Flutter.
///
/// Usage example:
/// ```dart
/// // Initialize
/// await ThermalPrinterRs.initTcp(host: '192.168.1.100', port: 9100);
///
/// // Print simple text
/// await ThermalPrinterRs.printText('Hello World');
///
/// // Print complete receipt
/// await ThermalPrinterRs.printReceipt(
///   title: 'SALE TICKET',
///   lines: [('American Coffee', '\$45.00'), ('Sandwich', '\$89.00')],
///   total: '\$134.00',
///   qrData: 'https://mystore.com/invoice/00123',
/// );
///
/// // Disconnect
/// await ThermalPrinterRs.disconnect();
/// ```
library;

import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'src/rust/api/simple.dart' as rust;
import 'src/rust/frb_generated.dart';
import 'src/models.dart';
import 'src/permissions.dart';
import 'src/android_transport.dart';
import 'src/ios_transport.dart';
import 'src/ticket_builder.dart';
import 'src/printer_enums.dart';
import 'src/printer_error.dart';
import 'src/print_column.dart';
import 'src/printer_status_poller.dart';

export 'src/models.dart';
export 'src/ticket_builder.dart';
export 'src/print_column.dart';
export 'src/printer_enums.dart';
export 'src/printer_error.dart' show PrinterError, PrinterException, Result;
export 'src/permissions.dart' show PrinterPermissions, TransportType;
export 'src/android_transport.dart'
    show
        AndroidBluetoothTransport,
        AndroidUsbTransport,
        BluetoothDeviceInfo,
        UsbDeviceInfo;
export 'src/ios_transport.dart' show IosBleTransport, BlePeripheralInfo;
export 'src/rust/api/simple.dart' show PrinterStateDto;

/// High-level API for cross-platform ESC/POS thermal printing.
///
/// Internally uses Rust + escpos to generate ESC/POS commands and
/// transports them to the device via TCP, USB, or BLE.
class ThermalPrinterRs {
  ThermalPrinterRs._();

  static bool _rustInitialized = false;

  // ── Capability detection ──────────────────────────────────────────

  static List<ConnectionType>? _cachedCapabilities;

  /// Returns the [ConnectionType]s compiled into this build.
  static List<ConnectionType> get capabilities {
    return _cachedCapabilities ??= _buildCapabilities();
  }

  static List<ConnectionType> _buildCapabilities() {
    final caps = <ConnectionType>[ConnectionType.tcp];
    if (Platform.isLinux ||
        Platform.isMacOS ||
        Platform.isWindows ||
        Platform.isAndroid) {
      caps.add(ConnectionType.usb);
    }
    caps.add(ConnectionType.bluetooth);
    return List.unmodifiable(caps);
  }

  /// Returns the [ConnectionType]s currently usable at runtime on this device.
  static List<ConnectionType> get runtimeCapabilities {
    return capabilities;
  }

  /// Whether a specific [ConnectionType] is compiled into this build.
  static bool isCapable(ConnectionType type) => capabilities.contains(type);

  /// Ensures the Rust library is initialized.
  static Future<void> _ensureRustInit() async {
    if (!_rustInitialized) {
      await RustLib.init();
      _rustInitialized = true;
    }
  }

  /// Ensures Rust is ready and shuts down any prior session before re-init.
  static Future<void> _prepareInit() async {
    await _ensureRustInit();
    if (rust.isPrinterReady()) {
      await disconnect();
    }
  }

  // ── Initialization ────────────────────────────────────────────────

  /// Initializes with a TCP/IP network printer.
  ///
  /// [host] — IP or hostname of the printer (e.g., "192.168.1.100")
  /// [port] — Port (default: 9100)
  /// [timeoutMs] — Timeout in ms (default: 5000)
  /// [paperWidth] — Paper width in characters (32 or 48, default: 48)
  static Future<void> initTcp({
    required String host,
    int port = 9100,
    int timeoutMs = 5000,
    int paperWidth = 48,
    int maxRetries = 3,
  }) async {
    await _prepareInit();
    if (!Platform.isAndroid && !Platform.isIOS) {
      await PrinterPermissions.request(TransportType.tcp);
    }
    await rust.initPrinter(
      config: rust.PrinterConfigDto(
        transport: rust.TransportTypeDto.tcp,
        host: host,
        port: port,
        vendorId: null,
        productId: null,
        bleAddress: null,
        timeoutMs: BigInt.from(timeoutMs),
        paperWidth: paperWidth,
        maxRetries: maxRetries,
      ),
    );
  }

  /// Initializes with a USB printer (Desktop only: Linux, Windows, macOS).
  ///
  /// ⚠️ Not available directly on Android/iOS.
  ///    Android requires native JNI implementation.
  ///    iOS does not support USB without MFi.
  ///
  /// [vendorId] — Manufacturer Vendor ID (e.g., 0x04b8 for Epson = 1208)
  /// [productId] — Model Product ID
  static Future<void> initUsb({
    required int vendorId,
    required int productId,
    int timeoutMs = 3000,
    int paperWidth = 48,
  }) async {
    await _prepareInit();
    await PrinterPermissions.request(TransportType.usb);
    await rust.initPrinter(
      config: rust.PrinterConfigDto(
        transport: rust.TransportTypeDto.usb,
        host: null,
        port: null,
        vendorId: vendorId,
        productId: productId,
        bleAddress: null,
        timeoutMs: BigInt.from(timeoutMs),
        paperWidth: paperWidth,
        maxRetries: 1,
      ),
    );
  }

  /// Initializes with a Bluetooth Low Energy (BLE) printer.
  ///
  /// ⚠️ Only for BLE printers. Most economic printers
  ///    use BT Classic (SPP), not BLE. Verify with the manufacturer.
  ///
  /// [bleAddress] — MAC address of the BLE device
  ///               (e.g., "AA:BB:CC:DD:EE:FF")
  static Future<void> initBluetooth({
    required String bleAddress,
    int timeoutMs = 8000,
    int paperWidth = 48,
    int maxRetries = 2,
  }) async {
    await _prepareInit();
    await PrinterPermissions.request(TransportType.bluetooth);
    await rust.initPrinter(
      config: rust.PrinterConfigDto(
        transport: rust.TransportTypeDto.bluetooth,
        host: null,
        port: null,
        vendorId: null,
        productId: null,
        bleAddress: bleAddress,
        timeoutMs: BigInt.from(timeoutMs),
        paperWidth: paperWidth,
        maxRetries: maxRetries,
      ),
    );
  }

  /// [ANDROID ONLY] Connects directly to an Android USB printer using the native
  /// UsbManager bridge (does not go through Rust — incompatible with libusb on Android).
  ///
  /// Returns the native transport instance for direct write/disconnect control.
  ///
  /// Example:
  /// ```dart
  /// final devices = await AndroidUsbTransport.listDevices();
  /// final transport = await ThermalPrinterRs.initAndroidUsb(
  ///   vendorId:  devices[0].vendorId,
  ///   productId: devices[0].productId,
  /// );
  /// final buf = buildEscPosBuffer(); // build with printText, etc.
  /// await transport.write(buf);
  /// ```
  static Future<AndroidUsbTransport> initAndroidUsb({
    required int vendorId,
    required int productId,
  }) async {
    assert(
      Platform.isAndroid,
      'initAndroidUsb() is only available on Android.',
    );
    final transport = AndroidUsbTransport();
    await transport.connect(vendorId: vendorId, productId: productId);
    return transport;
  }

  /// [ANDROID ONLY] Connects to an Android Bluetooth Classic (SPP) printer
  /// using the native BluetoothSocket bridge.
  ///
  /// Most low-cost thermal printers use BT Classic (SPP), NOT BLE.
  /// Use this method for those printers on Android.
  ///
  /// [address] — MAC address (e.g., 'AA:BB:CC:DD:EE:FF').
  ///
  /// Example:
  /// ```dart
  /// final paired = await AndroidBluetoothTransport.listPairedDevices();
  /// final printers = paired.where((d) => d.isClassic).toList();
  /// final transport = await ThermalPrinterRs.initAndroidBluetooth(
  ///   address: printers[0].address,
  /// );
  /// await transport.write(escPosBuffer);
  /// ```
  static Future<AndroidBluetoothTransport> initAndroidBluetooth({
    required String address,
  }) async {
    assert(
      Platform.isAndroid,
      'initAndroidBluetooth() is only available on Android.',
    );
    final transport = AndroidBluetoothTransport();
    await transport.connect(address);
    return transport;
  }

  /// [iOS ONLY] Connects to a BLE printer via CoreBluetooth.
  ///
  /// First use [IosBleTransport.scan()] to discover nearby printers and
  /// obtain the [peripheralUuid] (CoreBluetooth NSUUID).
  ///
  /// NOTE: iOS does not expose MAC addresses for privacy reasons.
  /// The uuid returned by scan() is stable per device but different from the MAC address.
  ///
  /// [peripheralUuid]     — UUID from [IosBleTransport.scan()].
  /// [serviceUuid]        — Custom GATT service UUID. Defaults to Peripage/Goojprt UUID.
  /// [characteristicUuid] — Custom write characteristic UUID. Defaults to Peripage/Goojprt UUID.
  ///
  /// Example:
  /// ```dart
  /// final devices = await IosBleTransport.scan(timeoutMs: 6000);
  /// final printer = devices.firstWhere((d) => d.name.contains('Printer'));
  /// final transport = await ThermalPrinterRs.initIosBle(printer.uuid);
  /// await transport.write(escPosBuffer);
  /// ```
  static Future<IosBleTransport> initIosBle(
    String peripheralUuid, {
    String? serviceUuid,
    String? characteristicUuid,
  }) async {
    assert(Platform.isIOS, 'initIosBle() is only available on iOS.');
    final transport = IosBleTransport();
    await transport.connect(
      peripheralUuid,
      serviceUuid: serviceUuid,
      characteristicUuid: characteristicUuid,
    );
    return transport;
  }

  // ── Print operations ──────────────────────────────────────

  /// Prints simple text with a paper cut.
  ///
  /// Automatically connects if there is no active connection.
  ///
  /// Example:
  /// ```dart
  /// await ThermalPrinterRs.printText('Hello World');
  /// ```
  static Future<PrintResult> printText(String text) async {
    _checkInit();
    final dto = await rust.printText(text: text);
    final result = PrintResult.fromDto(dto);
    if (result.isFailure) {
      final msg = result.errorMessage ?? 'Unknown error';
      throw PrinterException(PrinterError.fromMessage(msg), msg);
    }
    return result;
  }

  /// Prints a complete receipt with title, items, total and optional QR.
  ///
  /// [title] — Receipt header
  /// [lines] — List of tuples (description, price)
  /// [total] — Total text
  /// [qrData] — Data for the QR code (URL, text, etc). Optional.
  ///
  /// Example:
  /// ```dart
  /// await ThermalPrinterRs.printReceipt(
  ///   title: 'SALE TICKET',
  ///   lines: [
  ///     ('American Coffee', '\$45.00'),
  ///     ('Mixed Sandwich', '\$89.00'),
  ///   ],
  ///   total: '\$134.00',
  ///   qrData: 'https://mystore.com/invoice/001',
  /// );
  /// ```
  static Future<PrintResult> printReceipt({
    required String title,
    required List<(String, String)> lines,
    required String total,
    String? qrData,
  }) async {
    _checkInit();
    final dto = await rust.printReceipt(
      title: title,
      lines: lines
          .map((e) => rust.ReceiptLineDto(label: e.$1, value: e.$2))
          .toList(),
      total: total,
      qrData: qrData,
    );
    final result = PrintResult.fromDto(dto);
    if (result.isFailure) {
      final msg = result.errorMessage ?? 'Unknown error';
      throw PrinterException(PrinterError.fromMessage(msg), msg);
    }
    return result;
  }

  /// Enqueues a text print job (non-blocking).
  ///
  /// The job will be processed by a background worker in Rust.
  /// This allows the UI to continue immediately without waiting for the print to finish.
  static Future<void> enqueueText(String text) async {
    _checkInit();
    await rust.enqueuePrintText(text: text);
  }

  /// Enqueues a complete receipt print job (non-blocking).
  static Future<void> enqueueReceipt({
    required String title,
    required List<(String, String)> lines,
    required String total,
    String? qrData,
  }) async {
    _checkInit();
    await rust.enqueuePrintReceipt(
      title: title,
      lines: lines
          .map((e) => rust.ReceiptLineDto(label: e.$1, value: e.$2))
          .toList(),
      total: total,
      qrData: qrData,
    );
  }

  /// Enqueues a custom raw ESC/POS byte payload (non-blocking).
  ///
  /// Useful when combined with [TicketBuilder] to generate custom receipts
  /// with images, custom alignments, and barcodes.
  static Future<void> enqueueWriteBytes(List<int> bytes) async {
    _checkInit();
    await rust.enqueueWriteBytes(bytes: bytes);
  }

  /// Sends raw ESC/POS bytes and **awaits** confirmed delivery to the transport.
  ///
  /// The [Future] completes only after the Rust IO task has written the bytes
  /// to the hardware (or exhausted its retries). Use this when you need
  /// guaranteed ordering — for example, before calling [paperCut] or [drawerPin2].
  ///
  /// For fire-and-forget (non-blocking), use [enqueueWriteBytes] instead.
  static Future<void> writeBytes(List<int> bytes) async {
    _checkInit();
    await rust.writeRawBytes(bytes: bytes);
  }

  /// Queries the physical hardware status (paper out, cover open, etc.) using the Rust backend.
  /// Works by sending ESC/POS status commands and reading the response.
  /// Note: Only works if the configured transport supports bidirectional communication (e.g. TCP, Linux USB).
  static Future<HardwareStatus> checkStatus() async {
    _checkInit();
    return PrinterStatusPoller.checkStatus(
      write: (data) => writeBytes(data),
      read: (bytes) async {
        final result = await rust.readRawBytes(
          bytes: bytes,
          timeoutMs: BigInt.from(1500),
        );
        return Uint8List.fromList(result); // FRB returns growable List<int>
      },
    );
  }

  // ── Verbose convenience API ────────────────────────────────────────

  /// Prints text with custom size and alignment.
  ///
  /// **PERFORMANCE WARNING:**
  /// Calling this method directly crosses the Dart-Rust FFI bridge and triggers
  /// an individual IO task. For building complete receipts, use [TicketBuilder]
  /// to batch all commands into a single payload and cross the bridge once.
  static Future<void> printCustom(
    String message,
    PrinterSize size,
    PrinterAlign align, {
    PaperSize paperSize = PaperSize.mm80,
  }) {
    return writeTicket((ticket) {
      ticket.text(
        message,
        styles: PosStyles(height: size, align: align),
      );
    }, paperSize: paperSize);
  }

  /// Prints styled text.
  ///
  /// **PERFORMANCE WARNING:**
  /// Calling this method directly crosses the Dart-Rust FFI bridge and triggers
  /// an individual IO task. For building complete receipts, use [TicketBuilder]
  /// to batch all commands into a single payload and cross the bridge once.
  static Future<void> printStyledText(
    String message, {
    PrinterSize size = PrinterSize.standard,
    PrinterAlign align = PrinterAlign.left,
    bool bold = false,
    bool underline = false,
    PaperSize paperSize = PaperSize.mm80,
  }) {
    return writeTicket((ticket) {
      ticket.text(
        message,
        styles: PosStyles(
          height: size,
          align: align,
          bold: bold,
          underline: underline,
        ),
      );
    }, paperSize: paperSize);
  }

  /// Prints a QR code.
  ///
  /// **PERFORMANCE WARNING:**
  /// Use [TicketBuilder] instead to batch this command with the rest of your
  /// receipt to avoid multiple FFI crossings and fragmented IO writes.
  static Future<void> printQRcode(String data, {int size = 4}) {
    return writeTicket((ticket) => ticket.qrcode(data, size: size));
  }

  /// Prints a barcode. Type codes: 65=UPC-A, 67=EAN13, 68=EAN8, 73=CODE128
  ///
  /// **PERFORMANCE WARNING:**
  /// Use [TicketBuilder] instead to batch this command with the rest of your
  /// receipt to avoid multiple FFI crossings and fragmented IO writes.
  static Future<void> printBarcode(
    String data, {
    int type = 73,
    int width = 2,
    int height = 100,
  }) {
    return writeTicket(
      (ticket) => ticket.barcode(data, type: type, width: width, height: height),
    );
  }

  /// Performs a paper cut.
  ///
  /// **PERFORMANCE WARNING:**
  /// Calling this independently crosses the FFI bridge just to send 3 bytes.
  /// It is much more efficient to call `ticket.cut()` in your [TicketBuilder]
  /// before sending the final byte payload.
  static Future<void> paperCut() {
    return writeTicket((ticket) => ticket.cut());
  }

  /// Opens the cash drawer (pin 2).
  ///
  /// **PERFORMANCE WARNING:**
  /// Use [TicketBuilder.openDrawer] instead if this is part of a larger job.
  static Future<void> drawerPin2() {
    return writeTicket((ticket) => ticket.openDrawer());
  }

  /// Opens the cash drawer (pin 5).
  ///
  /// **PERFORMANCE WARNING:**
  /// Use [TicketBuilder.openDrawerPin5] instead if this is part of a larger job.
  static Future<void> drawerPin5() {
    return writeTicket((ticket) => ticket.openDrawerPin5());
  }

  /// Prints a new blank line.
  ///
  /// **PERFORMANCE WARNING:**
  /// Use [TicketBuilder.feed] instead.
  static Future<void> printNewLine() {
    return writeBytes([0x0A]);
  }

  /// Resets the printer to factory defaults.
  ///
  /// **PERFORMANCE WARNING:**
  /// Use [TicketBuilder.reset] instead.
  static Future<void> resetPrinter() {
    return writeBytes([0x1B, 0x40]);
  }

  // ── Observability ────────────────────────────────────────────────

  static StreamController<rust.PrinterStateDto>? _stateController;
  static StreamSubscription<rust.PrinterStateDto>? _stateSubscription;

  /// A reactive stream of the background worker's state.
  ///
  /// Emits transitions: Disconnected -> Connecting -> Connected -> Printing -> Idle
  /// Use this to update your UI (e.g., showing a loading spinner while connecting).
  /// Cancel your subscription in [State.dispose] or call [disconnect].
  static Stream<rust.PrinterStateDto> get stateStream {
    _checkInit();
    _ensureStateStream();
    return _stateController!.stream;
  }

  static void _ensureStateStream() {
    if (_stateController != null && !_stateController!.isClosed) {
      return;
    }

    _stateController = StreamController<rust.PrinterStateDto>.broadcast();
    _stateSubscription = rust.createStateStream().listen(
      (state) {
        final controller = _stateController;
        if (controller != null && !controller.isClosed) {
          controller.add(state);
        }
      },
      onError: (Object error, StackTrace stackTrace) {
        final controller = _stateController;
        if (controller != null && !controller.isClosed) {
          controller.addError(error, stackTrace);
        }
      },
      onDone: () {
        unawaited(_closeStateStream());
      },
    );
  }

  static Future<void> _closeStateStream() async {
    await _stateSubscription?.cancel();
    _stateSubscription = null;
    final controller = _stateController;
    if (controller != null && !controller.isClosed) {
      await controller.close();
    }
    _stateController = null;
  }

  /// Clears all pending jobs in the queue.
  static Future<void> clearQueue() async {
    _checkInit();
    await rust.clearPrintQueue();
  }

  // ── Connection management ────────────────────────────────────────

  /// Checks if the Bluetooth adapter is powered on (Android only).
  /// Always returns false on non-Android platforms currently.
  static Future<bool> get isBluetoothOn async {
    if (Platform.isAndroid) {
      return AndroidBluetoothTransport.isBluetoothOn;
    }
    return false;
  }

  /// Opens the system Bluetooth settings (Android only).
  static Future<void> openBluetoothSettings() async {
    if (Platform.isAndroid) {
      await AndroidBluetoothTransport.openSettings();
    }
  }

  /// Explicitly connects to the transport.
  /// Not required before printing — the connection is established automatically.
  static Future<void> connect() async {
    _checkInit();
    await rust.connectPrinter();
  }

  /// Disconnects the transport and releases all resources.
  static Future<void> disconnect() async {
    await _closeStateStream();
    if (_rustInitialized && rust.isPrinterReady()) {
      await rust.disconnectPrinter();
    }
  }

  /// Tears down the Rust runtime and releases all plugin resources.
  ///
  /// Call when the printer subsystem is no longer needed (e.g. app logout).
  /// After [dispose], call an `init*` method again before printing.
  static Future<void> dispose() async {
    await disconnect();
    if (_rustInitialized) {
      RustLib.dispose();
      _rustInitialized = false;
      _cachedCapabilities = null;
    }
  }

  /// Builds a [TicketBuilder], sends it in a single FFI crossing.
  static Future<void> writeTicket(
    void Function(TicketBuilder ticket) build, {
    PaperSize paperSize = PaperSize.mm80,
  }) async {
    final ticket = TicketBuilder(paperSize: paperSize);
    build(ticket);
    await writeBytes(ticket.build());
  }

  /// Whether the service is initialized (not necessarily connected).
  static bool get isReady {
    if (!_rustInitialized) return false;
    return rust.isPrinterReady();
  }

  /// Whether the transport is currently connected.
  ///
  /// Reads the live worker state from Rust. For reactive updates, listen to
  /// [stateStream] and cancel the subscription when done.
  static bool get isConnected {
    if (!_rustInitialized || !rust.isPrinterReady()) return false;
    final state = rust.getPrinterState();
    return state == rust.PrinterStateDto.connected ||
        state == rust.PrinterStateDto.printing;
  }

  // ── Private helpers ─────────────────────────────────────────────

  static void _checkInit() {
    if (!_rustInitialized) {
      throw StateError(
        'ThermalPrinterRs not initialized. '
        'Call initTcp(), initUsb() or initBluetooth() first.',
      );
    }
  }
}
