/// Represents various errors that can occur during printer operations.
enum PrinterError {
  /// Bluetooth is not available on the device.
  bluetoothUnavailable,

  /// Bluetooth or the required service is turned off.
  serviceOff,

  /// Failed to connect to the printer.
  connectionFailed,

  /// Not connected to any printer.
  notConnected,

  /// The specified device was not found (USB or Bluetooth).
  deviceNotFound,

  /// Permission was denied by the user.
  permissionDenied,

  /// An error occurred during the write/print operation.
  writeError,

  /// An unknown or unexpected error occurred.
  unknown;

  /// Parses an error string returned from Rust/native bridges into a typed enum.
  static PrinterError fromMessage(String message) {
    final m = message.toLowerCase();
    if (m.contains('permission')) return PrinterError.permissionDenied;
    if (m.contains('not found') || m.contains('not_found')) return PrinterError.deviceNotFound;
    if (m.contains('not connected') || m.contains('not_connected')) return PrinterError.notConnected;
    if (m.contains('connect')) return PrinterError.connectionFailed;
    if (m.contains('write') || m.contains('send')) return PrinterError.writeError;
    if (m.contains('bluetooth')) return PrinterError.bluetoothUnavailable;
    return PrinterError.unknown;
  }
}

/// A typed Result wrapper for printer operations.
/// [S] is the Success type, [E] is the Error type.
class Result<S, E> {
  final S? _success;
  final E? _error;

  Result._(this._success, this._error);

  factory Result.success(S data) => Result._(data, null);
  factory Result.error(E error) => Result._(null, error);

  bool get isSuccess => _success != null;
  bool get isError => _error != null;

  S get success => _success as S;
  E get error => _error as E;

  void fold(void Function(S data) onSuccess, void Function(E error) onError) {
    if (isSuccess) {
      onSuccess(_success as S);
    } else {
      onError(_error as E);
    }
  }
}

/// A typed exception wrapping [PrinterError].
class PrinterException implements Exception {
  final PrinterError code;
  final String message;

  const PrinterException(this.code, this.message);

  @override
  String toString() => 'PrinterException(${code.name}): $message';
}
