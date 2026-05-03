// ============================================================
// android_transport.dart — Dart client for the Android native bridge.
//
// Communicates with ThermalPrinterRsPlugin.kt via MethodChannel.
// Provides USB and Bluetooth Classic (SPP) transports for Android
// that cannot be implemented in cross-platform Rust.
// ============================================================

import 'package:flutter/services.dart';
import 'printer_enums.dart';
import 'printer_status_poller.dart';

/// Native method channel for Android-specific transports.
const _channel = MethodChannel('thermal_printer_rs/android');

// ── Device discovery ─────────────────────────────────────────────

/// Describes a paired Bluetooth device.
class BluetoothDeviceInfo {
  final String name;
  final String address;
  final String type; // CLASSIC | BLE | DUAL | UNKNOWN

  const BluetoothDeviceInfo({
    required this.name,
    required this.address,
    required this.type,
  });

  bool get isClassic => type == 'CLASSIC' || type == 'DUAL';

  @override
  String toString() =>
      'BluetoothDeviceInfo(name: $name, address: $address, type: $type)';
}

/// Describes a connected USB device.
class UsbDeviceInfo {
  final String name;
  final int vendorId;
  final int productId;
  final String? manufacturer;

  const UsbDeviceInfo({
    required this.name,
    required this.vendorId,
    required this.productId,
    this.manufacturer,
  });

  @override
  String toString() =>
      'UsbDeviceInfo(name: $name, vid: 0x${vendorId.toRadixString(16)}, pid: 0x${productId.toRadixString(16)})';
}

// ══════════════════════════════════════════════════════════════════
// Bluetooth Classic (SPP) Transport
// ══════════════════════════════════════════════════════════════════

/// Dart wrapper for the Android Bluetooth Classic (SPP) native transport.
///
/// NOTE: Only valid on Android. On other platforms, use BleTransport
/// or TcpTransport instead.
///
/// Usage:
/// ```dart
/// final transport = AndroidBluetoothTransport();
/// await transport.connect('AA:BB:CC:DD:EE:FF');
/// await transport.write(escPosBuffer);
/// await transport.disconnect();
/// ```
class AndroidBluetoothTransport {
  bool _connected = false;

  bool get isConnected => _connected;

  /// Checks if the Bluetooth adapter is powered on.
  static Future<bool> get isBluetoothOn async {
    final result = await _channel.invokeMethod<bool>('bt_is_on');
    return result ?? false;
  }

  /// Opens the system Bluetooth settings.
  static Future<void> openSettings() async {
    await _channel.invokeMethod('open_settings');
  }

  /// Returns all paired Bluetooth devices.
  /// Use [isClassic] to filter printers that support SPP.
  static Future<List<BluetoothDeviceInfo>> listPairedDevices() async {
    final List raw = await _channel.invokeMethod('bt_list_paired') as List;
    return raw.map((e) {
      final m = Map<String, String>.from(e as Map);
      return BluetoothDeviceInfo(
        name: m['name'] ?? 'Unknown',
        address: m['address'] ?? '',
        type: m['type'] ?? 'UNKNOWN',
      );
    }).toList();
  }

  /// Connects to a Bluetooth Classic (SPP) printer.
  ///
  /// [address] — Bluetooth MAC address (e.g., 'AA:BB:CC:DD:EE:FF').
  ///
  /// Throws [PlatformException] if connection fails.
  Future<void> connect(String address) async {
    await _channel.invokeMethod<bool>('bt_connect', {'address': address});
    _connected = true;
  }

  /// Sends raw ESC/POS bytes to the printer.
  Future<void> write(Uint8List data) async {
    if (!_connected) {
      throw StateError(
        'AndroidBluetoothTransport: not connected. Call connect() first.',
      );
    }
    await _channel.invokeMethod<bool>('bt_write', {'data': data});
  }

  /// Reads raw bytes from the printer.
  /// Used primarily for status polling (e.g. `DLE EOT n`).
  ///
  /// Blocks up to [timeoutMs] waiting for the specified number of [bytes].
  /// Returns the bytes read, or throws if a timeout occurs.
  Future<Uint8List> read(int bytes, {int timeoutMs = 1000}) async {
    if (!_connected) {
      throw StateError('AndroidBluetoothTransport: not connected.');
    }
    final result = await _channel.invokeMethod<Uint8List>('bt_read', {
      'bytes': bytes,
      'timeoutMs': timeoutMs,
    });
    return result ?? Uint8List(0);
  }

  /// Queries the physical hardware status (paper out, cover open, etc.).
  /// Works by sending ESC/POS status commands and reading the response.
  Future<HardwareStatus> checkStatus() async {
    return PrinterStatusPoller.checkStatus(
      write: (data) => write(Uint8List.fromList(data)),
      read: (bytes) => read(bytes, timeoutMs: 1500),
    );
  }

  /// Disconnects and releases the Bluetooth socket.
  Future<void> disconnect() async {
    await _channel.invokeMethod<bool>('bt_disconnect');
    _connected = false;
  }
}

// ══════════════════════════════════════════════════════════════════
// USB Transport (Android)
// ══════════════════════════════════════════════════════════════════

/// Dart wrapper for the Android USB (UsbManager) native transport.
///
/// NOTE: Only valid on Android. On Linux/Windows/macOS, the Rust UsbTransport
/// is used instead (via libusb/rusb).
///
/// Usage:
/// ```dart
/// final devices = await AndroidUsbTransport.listDevices();
/// final transport = AndroidUsbTransport();
/// await transport.connect(vendorId: devices[0].vendorId, productId: devices[0].productId);
/// await transport.write(escPosBuffer);
/// await transport.disconnect();
/// ```
class AndroidUsbTransport {
  bool _connected = false;

  bool get isConnected => _connected;

  /// Returns all currently connected USB devices.
  static Future<List<UsbDeviceInfo>> listDevices() async {
    final List raw = await _channel.invokeMethod('usb_list') as List;
    return raw.map((e) {
      final m = Map<String, dynamic>.from(e as Map);
      return UsbDeviceInfo(
        name: m['name'] as String,
        vendorId: m['vendorId'] as int,
        productId: m['productId'] as int,
        manufacturer: m['manufacturer'] as String?,
      );
    }).toList();
  }

  /// Connects to a USB printer by [vendorId] and [productId].
  ///
  /// Will trigger an OS permission dialog if permission has not been granted.
  /// Throws [PlatformException] if connection fails or permission is denied.
  Future<void> connect({required int vendorId, required int productId}) async {
    await _channel.invokeMethod<bool>('usb_connect', {
      'vendorId': vendorId,
      'productId': productId,
    });
    _connected = true;
  }

  /// Sends raw ESC/POS bytes to the connected USB printer.
  Future<void> write(Uint8List data) async {
    if (!_connected) {
      throw StateError(
        'AndroidUsbTransport: not connected. Call connect() first.',
      );
    }
    await _channel.invokeMethod<bool>('usb_write', {'data': data});
  }

  /// Reads raw bytes from the USB printer's IN endpoint.
  /// Used primarily for status polling (e.g. `DLE EOT n`).
  ///
  /// Blocks up to [timeoutMs] waiting for the specified number of [bytes].
  /// Returns the bytes read, or throws if a timeout occurs.
  Future<Uint8List> read(int bytes, {int timeoutMs = 1000}) async {
    if (!_connected) {
      throw StateError('AndroidUsbTransport: not connected.');
    }
    final result = await _channel.invokeMethod<Uint8List>('usb_read', {
      'bytes': bytes,
      'timeoutMs': timeoutMs,
    });
    return result ?? Uint8List(0);
  }

  /// Queries the physical hardware status (paper out, cover open, etc.).
  /// Works by sending ESC/POS status commands and reading the response.
  Future<HardwareStatus> checkStatus() async {
    return PrinterStatusPoller.checkStatus(
      write: (data) => write(Uint8List.fromList(data)),
      read: (bytes) => read(bytes, timeoutMs: 1000),
    );
  }

  /// Releases the USB interface and closes the connection.
  Future<void> disconnect() async {
    await _channel.invokeMethod<bool>('usb_disconnect');
    _connected = false;
  }
}
