import 'printer_enums.dart';

/// Style configuration for text printing.
class PosStyles {
  const PosStyles({
    this.align = PrinterAlign.left,
    this.bold = false,
    this.underline = false,
    this.reverse = false,
    this.height = PrinterSize.standard,
    this.width = PrinterSize.standard,
  });

  /// Shorthand for creating centered text.
  const PosStyles.center()
      : align = PrinterAlign.center,
        bold = false,
        underline = false,
        reverse = false,
        height = PrinterSize.standard,
        width = PrinterSize.standard;

  /// Shorthand for creating bold text.
  const PosStyles.bold()
      : align = PrinterAlign.left,
        bold = true,
        underline = false,
        reverse = false,
        height = PrinterSize.standard,
        width = PrinterSize.standard;

  final PrinterAlign align;
  final bool bold;
  final bool underline;
  final bool reverse;
  final PrinterSize height;
  final PrinterSize width;

  PosStyles copyWith({
    PrinterAlign? align,
    bool? bold,
    bool? underline,
    bool? reverse,
    PrinterSize? height,
    PrinterSize? width,
  }) {
    return PosStyles(
      align: align ?? this.align,
      bold: bold ?? this.bold,
      underline: underline ?? this.underline,
      reverse: reverse ?? this.reverse,
      height: height ?? this.height,
      width: width ?? this.width,
    );
  }
}

/// Column definition using the 12-column grid system.
class PosColumn {
  const PosColumn({
    required this.text,
    required this.width,
    this.styles = const PosStyles(),
  }) : assert(width >= 1 && width <= 12, 'Width must be between 1 and 12');

  final String text;

  /// Width in 12-column grid units (1-12). All columns in a row must sum to 12.
  final int width;

  final PosStyles styles;
}
