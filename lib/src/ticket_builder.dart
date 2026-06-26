import 'dart:convert';
import 'dart:typed_data';
import 'printer_enums.dart';
import 'print_column.dart';
import 'rust/api/simple.dart' as rust;


/// A complete ESC/POS ticket builder for thermal printers.
///
/// Provides full API parity with `thermal_printer_plus` including:
/// - Styled text with bold, underline, reverse, alignment, and size
/// - Multi-column rows using a 12-column grid system
/// - QR codes, barcodes, images (RGBA), separator lines
/// - Hardware commands: cut, drawer, beep, reset
///
/// ## Key design decisions
/// - Uses [BytesBuilder] internally to avoid repeated heap reallocations
///   while assembling the command stream.
/// - Heavy image processing (Floyd-Steinberg dithering) runs on a dedicated
///   [Isolate] so the Flutter UI never drops frames.
///
/// ## Usage
/// ```dart
/// final ticket = TicketBuilder(paperSize: PaperSize.mm80)
///   ..reset()
///   ..text('MY STORE', styles: PosStyles(bold: true, align: PrinterAlign.center))
///   ..hr()
///   ..row([
///     PosColumn(text: 'Coffee', width: 8),
///     PosColumn(text: '\$45.00', width: 4, styles: PosStyles(align: PrinterAlign.right)),
///   ])
///   ..feed(2)
///   ..cut();
///
/// await ThermalPrinterRs.enqueueWriteBytes(ticket.build());
/// ```
///
/// For tickets with images, [imageRgba] and [image] are `async`:
/// ```dart
/// final ticket = TicketBuilder();
/// await ticket.imageRgba(rgbaBytes, 384, 200);
/// ticket.cut();
/// await ThermalPrinterRs.enqueueWriteBytes(ticket.build());
/// ```
class TicketBuilder {
  TicketBuilder({this.paperSize = PaperSize.mm58});

  final PaperSize paperSize;

  // BytesBuilder(copy: false) holds chunk references without copying them
  // internally; a single allocation happens only at build() time.
  final BytesBuilder _builder = BytesBuilder(copy: false);

  // Style state to suppress redundant ESC command emissions
  bool _currentBold = false;
  bool _currentUnderline = false;
  bool _currentReverse = false;
  PrinterSize _currentHeight = PrinterSize.standard;
  PrinterSize _currentWidth = PrinterSize.standard;

  // ── ESC/POS constants ─────────────────────────────────────────────
  // ignore: constant_identifier_names
  static const int _ESC = 0x1B;
  // ignore: constant_identifier_names
  static const int _GS = 0x1D;
  // ignore: constant_identifier_names
  static const int _LF = 0x0A;

  // ── Output ────────────────────────────────────────────────────────

  /// Returns the constructed ESC/POS commands as a [Uint8List].
  ///
  /// The builder is **not** cleared after calling this — call [clear] to reuse.
  Uint8List build() => _builder.toBytes();

  /// Appends raw bytes directly to the buffer.
  void rawBytes(List<int> bytes) => _builder.add(bytes);

  /// Clears the builder for reuse without re-allocating.
  void clear() {
    _builder.clear();
    _resetInternalState();
  }

  // ── Initialization ────────────────────────────────────────────────

  /// Initializes/resets the printer to default state.
  void reset() {
    _builder.add(const [_ESC, 0x40]);
    _resetInternalState();
  }

  // ── Spacing ───────────────────────────────────────────────────────

  /// Feeds [n] blank lines.
  void feed([int n = 1]) {
    for (var i = 0; i < n; i++) {
      _builder.addByte(_LF);
    }
  }

  /// Alias for [feed].
  void emptyLines(int n) => feed(n);

  // ── Text ──────────────────────────────────────────────────────────

  /// Prints text with optional styles.
  ///
  /// ```dart
  /// ticket.text('Hello', styles: PosStyles(bold: true, align: PrinterAlign.center));
  /// ```
  void text(
    String text, {
    PosStyles styles = const PosStyles(),
    Encoding encoding = utf8,
    bool lf = true,
  }) {
    _applyStyles(styles);
    _builder.add(encoding.encode(text));
    if (lf) _builder.addByte(_LF);
    _resetStyles();
  }

  /// Prints a row of columns using the 12-column grid system.
  ///
  /// ```dart
  /// ticket.row([
  ///   PosColumn(text: 'Item', width: 8),
  ///   PosColumn(text: '\$10.00', width: 4, styles: PosStyles(align: PrinterAlign.right)),
  /// ]);
  /// ```
  void row(List<PosColumn> columns) {
    final charsPerUnit = paperSize.chars / 12;
    final buf = StringBuffer();
    for (final col in columns) {
      final w = (charsPerUnit * col.width).floor();
      if (w > 0) buf.write(_padText(col.text, w, col.styles.align));
    }
    _builder.add(utf8.encode(buf.toString()));
    _builder.addByte(_LF);
  }

  /// Prints a full-width separator line.
  void hr({String ch = '-'}) => text(ch * paperSize.chars, lf: true);

  /// Prints a decorative separator.
  void separator({String pattern = '*'}) =>
      text(pattern * paperSize.chars, styles: const PosStyles(), lf: true);

  // ── Explicit Style Control ─────────────────────────────────────────

  /// Sets text alignment for subsequent calls.
  void setAlign(PrinterAlign align) {
    _builder.add([_ESC, 0x61, align.index]);
  }

  /// Enables or disables bold text.
  void setBold(bool on) {
    _builder.add([_ESC, 0x45, on ? 1 : 0]);
    _currentBold = on;
  }

  /// Enables or disables underline.
  void setUnderline(bool on) {
    _builder.add([_ESC, 0x2D, on ? 1 : 0]);
    _currentUnderline = on;
  }

  /// Enables or disables reverse (white on black).
  void setReverse(bool on) {
    _builder.add([_GS, 0x42, on ? 1 : 0]);
    _currentReverse = on;
  }

  /// Sets the text size.
  void setSize(PrinterSize size) {
    final int v;
    switch (size) {
      case PrinterSize.medium:    v = 0x10; break;
      case PrinterSize.large:     v = 0x20; break;
      case PrinterSize.extraLarge: v = 0x30; break;
      default:                    v = 0x00; break;
    }
    _builder.add([_ESC, 0x21, v]);
  }

  // ── QR Code ───────────────────────────────────────────────────────

  /// Prints a QR code.
  ///
  /// - [size]: module size 1–16 (default 4)
  /// - [level]: error correction 48=L, 49=M, 50=Q, 51=H
  void qrcode(
    String data, {
    int size = 4,
    int level = 48,
    PrinterAlign align = PrinterAlign.center,
  }) {
    setAlign(align);
    final s = size.clamp(1, 16);
    final l = level.clamp(48, 51);

    _builder.add([_GS, 0x28, 0x6B, 0x04, 0x00, 0x31, 0x41, 0x32, 0x00]);
    _builder.add([_GS, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x43, s]);
    _builder.add([_GS, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x45, l]);

    final payload = utf8.encode(data);
    final pL = (payload.length + 3) % 256;
    final pH = (payload.length + 3) ~/ 256;
    _builder.add([_GS, 0x28, 0x6B, pL, pH, 0x31, 0x50, 0x30]);
    _builder.add(payload);
    _builder.add([_GS, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x51, 0x30]);
  }

  // ── Barcode ───────────────────────────────────────────────────────

  /// Prints a barcode.
  ///
  /// Common types: 65=UPC-A, 67=EAN13, 68=EAN8, 73=CODE128 (default)
  void barcode(
    String data, {
    int type = 73,
    int width = 2,
    int height = 100,
    int font = 0,
    int position = 2,
  }) {
    _builder.add([_GS, 0x77, width.clamp(2, 6)]);
    _builder.add([_GS, 0x68, height.clamp(1, 255)]);
    _builder.add([_GS, 0x66, font]);
    _builder.add([_GS, 0x48, position]);

    final payload = ascii.encode(data);
    _builder.add([_GS, 0x6B, type, payload.length]);
    _builder.add(payload);
  }

  // ── Image Printing ────────────────────────────────────────────────

  /// Prints an image from an `image` package [Image] object.
  ///
  /// The entire dithering computation runs on a background [Isolate]
  /// so the Flutter UI thread is never blocked. This method is `async`.
  ///
  /// ```dart
  /// import 'package:image/image.dart' as img;
  /// final decoded = img.decodeImage(bytes)!;
  /// await ticket.image(decoded);
  /// ```
  Future<void> image(dynamic imgImage,
      {PrinterAlign align = PrinterAlign.center}) async {
    try {
      final w = imgImage.width as int;
      final h = imgImage.height as int;

      final rgba = Uint8List(w * h * 4);
      int i = 0;
      for (var y = 0; y < h; y++) {
        for (var x = 0; x < w; x++) {
          final pixel = imgImage.getPixel(x, y);
          int r, g, b, a;
          if (pixel is int) {
            a = (pixel >> 24) & 0xFF;
            b = (pixel >> 16) & 0xFF;
            g = (pixel >> 8) & 0xFF;
            r = pixel & 0xFF;
          } else {
            r = (pixel.r as num).toInt();
            g = (pixel.g as num).toInt();
            b = (pixel.b as num).toInt();
            a = (pixel.a as num).toInt();
          }
          rgba[i++] = r; rgba[i++] = g; rgba[i++] = b; rgba[i++] = a;
        }
      }
      await imageRgba(rgba, w, h, align: align);
    } catch (e) {
      throw ArgumentError(
          'Failed to process image. Pass an img.Image from the image package. Error: $e');
    }
  }

  /// Prints an image from raw RGBA bytes.
  ///
  /// The Floyd-Steinberg dithering runs on a separate [Isolate] to keep
  /// the UI at full frame rate. This method is `async`.
  ///
  /// The Floyd-Steinberg dithering and raster encoding run entirely in **Rust**
  /// on Tokio's blocking thread pool — native speed, zero Dart VM overhead,
  /// no GC pressure. The UI never drops a frame.
  ///
  /// - [rgbaBytes]: raw RGBA data (4 bytes per pixel, row-major)
  /// - [width] / [height]: image dimensions in pixels
  Future<void> imageRgba(
    Uint8List rgbaBytes,
    int width,
    int height, {
    PrinterAlign align = PrinterAlign.center,
  }) async {
    if (rgbaBytes.length < width * height * 4) {
      throw ArgumentError(
          'rgbaBytes length does not match width × height × 4');
    }
    setAlign(align);

    // Floyd-Steinberg dithering + ESC/POS raster encoding in native Rust.
    // Runs on Tokio's blocking thread pool — zero Dart VM overhead.
    final command = await rust.encodeRasterImage(
      rgba: rgbaBytes,
      width: width,
      height: height,
    );
    if (command.isEmpty) {
      throw StateError(
        'Raster encode failed — check image dimensions and RGBA buffer size.',
      );
    }
    _builder.add(command);
  }

  // ── Hardware Commands ─────────────────────────────────────────────

  /// Performs a paper cut.
  void cut({bool partial = false}) {
    if (partial) {
      _builder.add(const [_GS, 0x56, 66, 0x00]);
    } else {
      _builder.add(const [_GS, 0x56, 0x00]);
    }
  }

  /// Opens the cash drawer connected to pin 2.
  void openDrawer() {
    _builder.add(const [_ESC, 0x70, 0x00, 0x3C, 0x78]);
  }

  /// Opens the cash drawer connected to pin 5.
  void openDrawerPin5() {
    _builder.add(const [_ESC, 0x70, 0x01, 0x3C, 0x78]);
  }

  /// Plays a beep sound (if supported by printer).
  void beep({int times = 1, int duration = 3}) {
    _builder.add([_ESC, 0x42, times.clamp(1, 9), duration.clamp(1, 9)]);
  }

  // ── Private ───────────────────────────────────────────────────────

  void _resetInternalState() {
    _currentBold = false;
    _currentUnderline = false;
    _currentReverse = false;
    _currentHeight = PrinterSize.standard;
    _currentWidth = PrinterSize.standard;
  }

  void _applyStyles(PosStyles styles) {
    _builder.add([_ESC, 0x61, styles.align.index]);

    if (styles.bold != _currentBold) {
      _builder.add([_ESC, 0x45, styles.bold ? 1 : 0]);
      _currentBold = styles.bold;
    }
    if (styles.underline != _currentUnderline) {
      _builder.add([_ESC, 0x2D, styles.underline ? 1 : 0]);
      _currentUnderline = styles.underline;
    }
    if (styles.reverse != _currentReverse) {
      _builder.add([_GS, 0x42, styles.reverse ? 1 : 0]);
      _currentReverse = styles.reverse;
    }
    _applySize(styles.height, styles.width);
  }

  void _applySize(PrinterSize height, PrinterSize width) {
    if (height == _currentHeight && width == _currentWidth) {
      return;
    }

    var v = 0;
    if (height == PrinterSize.medium || height == PrinterSize.large) {
      v |= 0x10;
    } else if (height == PrinterSize.extraLarge) {
      v |= 0x30;
    }
    if (width == PrinterSize.medium || width == PrinterSize.large) {
      v |= 0x20;
    } else if (width == PrinterSize.extraLarge) {
      v |= 0x30;
    }
    _builder.add([_ESC, 0x21, v]);
    _currentHeight = height;
    _currentWidth = width;
  }

  void _resetStyles() {
    if (_currentBold) {
      _builder.add(const [_ESC, 0x45, 0]);
      _currentBold = false;
    }
    if (_currentUnderline) {
      _builder.add(const [_ESC, 0x2D, 0]);
      _currentUnderline = false;
    }
    if (_currentReverse) {
      _builder.add(const [_GS, 0x42, 0]);
      _currentReverse = false;
    }
    if (_currentHeight != PrinterSize.standard || _currentWidth != PrinterSize.standard) {
      _builder.add(const [_ESC, 0x21, 0]);
      _currentHeight = PrinterSize.standard;
      _currentWidth = PrinterSize.standard;
    }
  }

  String _padText(String text, int width, PrinterAlign align) {
    if (text.length >= width) return text.substring(0, width);
    final pad = width - text.length;
    switch (align) {
      case PrinterAlign.center:
        final l = pad ~/ 2;
        return '${' ' * l}$text${' ' * (pad - l)}';
      case PrinterAlign.right:
        return '${' ' * pad}$text';
      case PrinterAlign.left:
        return '$text${' ' * pad}';
    }
  }
}
