/// Domain models for the thermal_printer_rs Dart/Flutter layer.
library;

import 'package:thermal_printer_rs/src/rust/api/simple.dart';

export 'package:thermal_printer_rs/src/rust/api/simple.dart'
    show TransportTypeDto, PrinterConfigDto, PrintResultDto, ReceiptLineDto;

/// Result of a print operation with additional business logic.
class PrintResult {
  final bool success;
  final int bytesSent;
  final String? errorMessage;

  const PrintResult({
    required this.success,
    required this.bytesSent,
    this.errorMessage,
  });

  factory PrintResult.fromDto(PrintResultDto dto) => PrintResult(
        success: dto.success,
        bytesSent: dto.bytesSent,
        errorMessage: dto.errorMessage,
      );

  bool get isFailure => !success;

  @override
  String toString() => success
      ? 'PrintResult(ok, $bytesSent bytes)'
      : 'PrintResult(error: $errorMessage)';
}
