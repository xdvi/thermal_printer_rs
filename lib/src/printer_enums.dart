/// Alignment options for printing.
enum PrinterAlign {
  left,
  center,
  right,
}

/// Alias compatible with thermal_printer_plus naming.
typedef PosAlign = PrinterAlign;

/// Font size options for printing.
enum PrinterSize {
  standard,
  medium,
  large,
  extraLarge,
}

/// Alias compatible with thermal_printer_plus naming.
typedef PosSize = PrinterSize;

/// Text size options with finer granularity (1-8x magnification).
enum PosTextSize {
  size1,
  size2,
  size3,
  size4,
  size5,
  size6,
  size7,
  size8,
}

/// Paper size definitions with dot and character widths.
enum PaperSize {
  /// 58mm paper (commonly used in portable printers).
  mm58(384, 32),

  /// 80mm paper (commonly used in POS printers).
  mm80(576, 48);

  const PaperSize(this.dots, this.chars);

  /// Width in dots.
  final int dots;

  /// Maximum characters per line (using standard font).
  final int chars;
}

/// Connection status of the printer.
enum PrinterStatus {
  disconnected,
  connected,
  connecting,
}

/// Available connection types.
enum ConnectionType {
  bluetooth,
  tcp,
  usb,
}

/// Printer hardware or protocol events.
enum PrinterEvent {
  paperEnd,
  overheat,
  lidOpen,
  cutterError,
  connectionFailed,
  writeFailed,
  timeout,
  permissionDenied,
  deviceNotFound,
  unknown,
}

/// Represents the physical status snapshot of the printer based on ESC/POS polling.
class HardwareStatus {
  final PrinterEvent? event;
  final bool isPaperOut;
  final bool isCoverOpen;
  final bool isDrawerOpen;

  const HardwareStatus({
    this.event,
    this.isPaperOut = false,
    this.isCoverOpen = false,
    this.isDrawerOpen = false,
  });

  @override
  String toString() =>
      'HardwareStatus(event: $event, paperOut: $isPaperOut, coverOpen: $isCoverOpen, drawerOpen: $isDrawerOpen)';
}
