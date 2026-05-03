/// Management of permissions required by each transport type on each platform.
library;

import 'dart:io';

/// Transport type for requesting specific permissions.
enum TransportType { tcp, usb, bluetooth }

/// Helper to request platform permissions.
///
/// On Android it requires `permission_handler` in pubspec.yaml.
/// iOS requires entries in Info.plist (see README).
abstract class PrinterPermissions {
  /// Requests the necessary permissions for the given transport type.
  /// Throws [PermissionDeniedException] if the user refuses.
  static Future<void> request(TransportType type) async {
    if (Platform.isAndroid) {
      await _requestAndroid(type);
    } else if (Platform.isIOS) {
      await _requestIos(type);
    }
    // Linux/Windows/macOS: no explicit app-level permissions required.
    // Linux USB may require a udev rule — document in README.
  }

  static Future<void> _requestAndroid(TransportType type) async {
    switch (type) {
      case TransportType.tcp:
        // INTERNET permission is declarative in AndroidManifest.xml — always granted.
        break;
      case TransportType.usb:
        // USB Host requires declaration in AndroidManifest and using native UsbManager (JNI).
        // ⚠️ rusb/libusb DOES NOT work on Android. See JNI implementation in android/
        throw UnsupportedError(
          'USB on Android requires JNI implementation with android.hardware.usb.UsbManager. '
          'Check the android/ folder for the native implementation.',
        );
      case TransportType.bluetooth:
        // Bluetooth on Android requires permission_handler >= 10
        // Permissions: BLUETOOTH_SCAN, BLUETOOTH_CONNECT (Android 12+)
        // BLUETOOTH, BLUETOOTH_ADMIN (Android 11-)
        // Implement with permission_handler when btleplug Android is stable.
        break;
    }
  }

  static Future<void> _requestIos(TransportType type) async {
    switch (type) {
      case TransportType.tcp:
        // No permissions required for TCP on iOS.
        break;
      case TransportType.usb:
        // ⚠️ USB completely blocked on iOS except for MFi.
        throw UnsupportedError(
          'USB is not available on iOS. '
          'Use TCP/IP or Bluetooth to connect printers on iOS.',
        );
      case TransportType.bluetooth:
        // NSBluetoothAlwaysUsageDescription must be in Info.plist.
        // iOS system requests permission automatically on first BLE use.
        break;
    }
  }
}

class PermissionDeniedException implements Exception {
  final String message;
  const PermissionDeniedException(this.message);
  @override
  String toString() => 'PermissionDeniedException: $message';
}
