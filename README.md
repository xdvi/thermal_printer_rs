# thermal_printer_rs

Cross-platform ESC/POS thermal printer library for Flutter, powered by Rust via `flutter_rust_bridge`.

## Platform Support

| Platform | TCP/IP | USB | Bluetooth Classic (SPP) | BLE |
|---|---|---|---|---|
| **Linux**   | ✅ | ✅ | — | ✅ |
| **Windows** | ✅ | ✅ (WinUSB) | — | ✅ |
| **macOS**   | ✅ | ✅ | — | ✅ |
| **Android** | ✅ | ✅ (JNI) | ✅ (JNI) | ✅ |
| **iOS**     | ✅ | ❌ (MFi) | ❌ (MFi) | ✅ (CoreBluetooth) |

## Quick Start

```dart
import 'package:thermal_printer_rs/thermal_printer_rs.dart';
```

### TCP/IP (all platforms)

```dart
await ThermalPrinterRs.initTcp(host: '192.168.1.100');
await ThermalPrinterRs.printText('Hello World');
await ThermalPrinterRs.printReceipt(
  title: 'SALE TICKET',
  lines: [('Americano Coffee', '\$45.00'), ('Sandwich', '\$89.00')],
  total: '\$134.00',
  qrData: 'https://mystore.com/invoice/001',
);
await ThermalPrinterRs.disconnect();
```

### Android — Bluetooth Classic (SPP)

Most low-cost thermal printers use BT Classic, not BLE.

```dart
// List paired devices
final devices = await AndroidBluetoothTransport.listPairedDevices();
final printers = devices.where((d) => d.isClassic).toList();

// Connect and print
final transport = await ThermalPrinterRs.initAndroidBluetooth(
  address: printers.first.address,
);
// Build ESC/POS buffer and send
await transport.write(escPosBuffer);
await transport.disconnect();
```

### Android — USB

```dart
final devices = await AndroidUsbTransport.listDevices();
final transport = await ThermalPrinterRs.initAndroidUsb(
  vendorId:  devices.first.vendorId,
  productId: devices.first.productId,
);
await transport.write(escPosBuffer);
await transport.disconnect();
```

### iOS — BLE (CoreBluetooth)

```dart
// Scan for nearby BLE printers
final devices = await IosBleTransport.scan(timeoutMs: 6000);
final printer = devices.firstWhere((d) => d.name.contains('Printer'));

// Connect and print
final transport = await ThermalPrinterRs.initIosBle(printer.uuid);
await transport.write(escPosBuffer);
await transport.disconnect();
```

### Background queue (non-blocking)

```dart
// Returns immediately — job is processed in the Rust background worker
await ThermalPrinterRs.enqueueText('Background print job');
await ThermalPrinterRs.enqueueReceipt(
  title: 'ASYNC TICKET',
  lines: [('Item A', '\$10.00')],
  total: '\$10.00',
);
```

## Setup

### Android — `AndroidManifest.xml`

```xml
<uses-permission android:name="android.permission.BLUETOOTH" />
<uses-permission android:name="android.permission.BLUETOOTH_ADMIN" />
<uses-permission android:name="android.permission.BLUETOOTH_CONNECT" android:minSdkVersion="31" />
<uses-permission android:name="android.permission.BLUETOOTH_SCAN" android:minSdkVersion="31" />
<uses-feature android:name="android.hardware.usb.host" android:required="false" />
```

### iOS — `Info.plist`

```xml
<key>NSBluetoothAlwaysUsageDescription</key>
<string>Required to connect to BLE thermal printers.</string>
```

### Linux — USB udev rule

```bash
# /etc/udev/rules.d/99-thermal-printer.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="04b8", MODE="0666", GROUP="plugdev"
```

### Windows — USB

Install the WinUSB driver for your printer using [Zadig](https://zadig.akeo.ie/).

## Architecture

```
Flutter (Dart)
    ├── ThermalPrinterRs         — high-level public API
    ├── AndroidBluetoothTransport — BT Classic via MethodChannel → Kotlin
    ├── AndroidUsbTransport       — USB via MethodChannel → Kotlin
    └── IosBleTransport           — BLE via MethodChannel → Swift (CoreBluetooth)

Rust (flutter_rust_bridge)
    ├── PrintService              — orchestrates ESC/POS + transport
    ├── EscposAdapter             — builds ESC/POS buffers (escpos crate)
    ├── PrintWorker               — async background job queue (mpsc)
    └── Transport (trait)
            ├── TcpTransport      — TCP/IP (all platforms)
            ├── UsbTransport      — USB Desktop (rusb/libusb)
            ├── BleTransport      — BLE Desktop+Mobile (btleplug)
            └── MockTransport     — in-memory (testing)
```

## Running Tests

```bash
# Rust unit + integration tests
cd rust && cargo test --features tcp,codes_2d,barcodes

# Dart unit tests
flutter test

# TCP integration with virtual printer (requires Docker)
./docker/run_docker_tests.sh
```

## License

This project is licensed under the GNU Affero General Public License v3.0 (`AGPL-3.0`).

You may use, study, modify, and distribute this package, including for commercial use, under the terms of the AGPL-3.0. If you want to use this package in a closed-source or proprietary application, or otherwise do not want to comply with the AGPL obligations, you must obtain a commercial license.

For commercial licensing inquiries, contact `xdvii@icloud.com`.
