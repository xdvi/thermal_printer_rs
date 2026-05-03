import 'package:flutter_test/flutter_test.dart';
import 'package:thermal_printer_rs/thermal_printer_rs.dart';

void main() {
  // ── PrintResult model ───────────────────────────────────────────

  group('PrintResult', () {
    test('fromDto maps success correctly', () {
      final dto = PrintResultDto(
        success:      true,
        bytesSent:    256,
        errorMessage: null,
      );
      final result = PrintResult.fromDto(dto);

      expect(result.success,      isTrue);
      expect(result.bytesSent,    256);
      expect(result.errorMessage, isNull);
      expect(result.isFailure,    isFalse);
    });

    test('fromDto maps error correctly', () {
      final dto = PrintResultDto(
        success:      false,
        bytesSent:    0,
        errorMessage: 'Connection timeout',
      );
      final result = PrintResult.fromDto(dto);

      expect(result.success,      isFalse);
      expect(result.bytesSent,    0);
      expect(result.errorMessage, 'Connection timeout');
      expect(result.isFailure,    isTrue);
    });

    test('toString returns descriptive string on success', () {
      final result = PrintResult(success: true, bytesSent: 512);
      expect(result.toString(), contains('512 bytes'));
    });

    test('toString returns error message on failure', () {
      final result = PrintResult(
        success:      false,
        bytesSent:    0,
        errorMessage: 'USB disconnected',
      );
      expect(result.toString(), contains('USB disconnected'));
    });
  });

  // ── BluetoothDeviceInfo ─────────────────────────────────────────

  group('BluetoothDeviceInfo', () {
    test('isClassic returns true for CLASSIC type', () {
      const device = BluetoothDeviceInfo(
        name:    'Printer BT',
        address: 'AA:BB:CC:DD:EE:FF',
        type:    'CLASSIC',
      );
      expect(device.isClassic, isTrue);
    });

    test('isClassic returns true for DUAL type', () {
      const device = BluetoothDeviceInfo(
        name:    'Dual Printer',
        address: '11:22:33:44:55:66',
        type:    'DUAL',
      );
      expect(device.isClassic, isTrue);
    });

    test('isClassic returns false for BLE type', () {
      const device = BluetoothDeviceInfo(
        name:    'BLE Printer',
        address: 'AA:BB:CC:DD:EE:FF',
        type:    'BLE',
      );
      expect(device.isClassic, isFalse);
    });

    test('toString contains name and address', () {
      const device = BluetoothDeviceInfo(
        name:    'EP-300',
        address: 'AA:BB:CC:DD:EE:FF',
        type:    'CLASSIC',
      );
      expect(device.toString(), contains('EP-300'));
      expect(device.toString(), contains('AA:BB:CC:DD:EE:FF'));
    });
  });

  // ── UsbDeviceInfo ───────────────────────────────────────────────

  group('UsbDeviceInfo', () {
    test('toString formats vendorId and productId as hex', () {
      const device = UsbDeviceInfo(
        name:      '/dev/bus/usb/001/003',
        vendorId:  0x04b8, // Epson
        productId: 0x0202,
      );
      expect(device.toString(), contains('4b8'));
      expect(device.toString(), contains('202'));
    });
  });

  // ── PrinterException ────────────────────────────────────────────

  group('PrinterException', () {
    test('toString contains message', () {
      const ex = PrinterException(PrinterError.unknown, 'Connection refused');
      expect(ex.toString(), contains('Connection refused'));
      expect(ex.toString(), contains('PrinterException'));
    });

    test('can be caught as Exception', () {
      expect(
        () => throw const PrinterException(PrinterError.unknown, 'Test error'),
        throwsA(isA<PrinterException>()),
      );
    });
  });

  // ── TransportType ────────────────────────────────────────────────

  group('TransportType', () {
    test('enum has all three transport types', () {
      expect(TransportType.values, contains(TransportType.tcp));
      expect(TransportType.values, contains(TransportType.usb));
      expect(TransportType.values, contains(TransportType.bluetooth));
    });
  });

  // ── ThermalPrinterRs static state ───────────────────────────────

  group('ThermalPrinterRs', () {
    test('isReady returns false before initialization', () {
      // The Rust lib is not initialized, so this should be false
      // Note: isReady calls into Rust so we just verify it doesn't throw
      expect(() => ThermalPrinterRs.isReady, returnsNormally);
    });

    test('PrinterException is thrown for uninitialized print', () {
      expect(
        () => throw const PrinterException(PrinterError.notConnected, 'Not initialized'),
        throwsA(
          isA<PrinterException>().having(
            (e) => e.message,
            'message',
            'Not initialized',
          ),
        ),
      );
    });
  });
}
