// ============================================================
// ios_transport.dart — Dart client for the iOS CoreBluetooth bridge.
//
// Communicates with ThermalPrinterRsPlugin.swift via MethodChannel.
// Provides BLE scanning, connection, and printing on iOS.
//
// WHAT WORKS ON iOS:
//   TCP    — Handled by Rust/FRB. Use ThermalPrinterRs.initTcp().
//   BLE    — This file. Use IosBleTransport or ThermalPrinterRs.initIosBle().
//   USB    — BLOCKED by Apple (MFi required). Not implemented.
//   SPP    — BLOCKED by Apple (MFi required). Not implemented.
// ============================================================

import 'package:flutter/services.dart';

/// Native method channel for the iOS BLE bridge.
const _channel = MethodChannel('thermal_printer_rs/ios');

// ── Device model ─────────────────────────────────────────────────

/// Represents a BLE peripheral discovered during scan.
class BlePeripheralInfo {
  /// CoreBluetooth NSUUID string (e.g., "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX").
  final String uuid;

  /// Advertised name (may be null for some devices).
  final String name;

  const BlePeripheralInfo({required this.uuid, required this.name});

  @override
  String toString() => 'BlePeripheralInfo(name: $name, uuid: $uuid)';
}

// ══════════════════════════════════════════════════════════════════
// IosBleTransport
// ══════════════════════════════════════════════════════════════════

/// iOS CoreBluetooth BLE transport for ESC/POS thermal printing.
///
/// NOTE: Only valid on iOS. On Android, use [AndroidBluetoothTransport]
/// for BT Classic (SPP) or [ThermalPrinterRs.initBluetooth()] for BLE.
///
/// Usage:
/// ```dart
/// // 1. Scan for nearby BLE printers
/// final devices = await IosBleTransport.scan(timeoutMs: 5000);
/// final printer = devices.firstWhere((d) => d.name.contains('Printer'));
///
/// // 2. Connect (use custom UUIDs if your printer is not Peripage/Goojprt)
/// final transport = IosBleTransport();
/// await transport.connect(printer.uuid);
///
/// // 3. Send ESC/POS data (build buffer with ThermalPrinterRs adapter)
/// await transport.write(escPosBuffer);
///
/// // 4. Disconnect
/// await transport.disconnect();
/// ```
class IosBleTransport {
  bool _connected = false;

  bool get isConnected => _connected;

  // ── Discovery ─────────────────────────────────────────────────

  /// Scans for nearby BLE peripherals.
  ///
  /// [timeoutMs] — Scan duration in milliseconds (default: 8000).
  ///
  /// Returns a list of discovered [BlePeripheralInfo].
  /// Note: iOS does not expose MAC addresses for privacy reasons;
  /// use [uuid] (NSUUID) to identify and reconnect to devices.
  static Future<List<BlePeripheralInfo>> scan({int timeoutMs = 8000}) async {
    final List raw = await _channel.invokeMethod('ble_scan', {
      'timeoutMs': timeoutMs,
    }) as List;

    return raw.map((e) {
      final m = Map<String, String>.from(e as Map);
      return BlePeripheralInfo(
        uuid: m['uuid'] ?? '',
        name: m['name'] ?? 'Unknown',
      );
    }).toList();
  }

  /// Returns the current CoreBluetooth state as a string.
  /// Possible values: 'poweredOn', 'poweredOff', 'unsupported', 'unauthorized', 'unknown'.
  static Future<String> bluetoothState() async {
    return await _channel.invokeMethod<String>('ble_state') ?? 'unknown';
  }

  // ── Connection ─────────────────────────────────────────────────

  /// Connects to a BLE peripheral by its CoreBluetooth NSUUID.
  ///
  /// [peripheralUuid] — UUID returned from [scan()].
  /// [serviceUuid]    — Custom GATT service UUID. Uses Peripage/Goojprt default if omitted.
  /// [characteristicUuid] — Custom write characteristic UUID. Uses default if omitted.
  ///
  /// Throws [PlatformException] if connection or service discovery fails.
  Future<void> connect(
    String peripheralUuid, {
    String? serviceUuid,
    String? characteristicUuid,
  }) async {
    await _channel.invokeMethod<bool>('ble_connect', {
      'uuid':               peripheralUuid,
      'serviceUuid':        serviceUuid,
      'characteristicUuid': characteristicUuid,
    });
    _connected = true;
  }

  // ── Data transfer ──────────────────────────────────────────────

  /// Sends raw ESC/POS bytes to the connected BLE printer.
  ///
  /// Data is automatically split into MTU-safe chunks by the Swift layer.
  /// Throws [StateError] if not connected. Throws [PlatformException] on write error.
  Future<void> write(List<int> data) async {
    if (!_connected) {
      throw StateError('IosBleTransport: not connected. Call connect() first.');
    }
    await _channel.invokeMethod<bool>('ble_write', {
      'data': Uint8List.fromList(data),
    });
  }

  // ── Lifecycle ─────────────────────────────────────────────────

  /// Disconnects from the BLE peripheral and releases CoreBluetooth resources.
  Future<void> disconnect() async {
    await _channel.invokeMethod<bool>('ble_disconnect');
    _connected = false;
  }
}
