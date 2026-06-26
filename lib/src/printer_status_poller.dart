import 'dart:typed_data';
import 'printer_enums.dart';

/// Helper class to query printer hardware status using ESC/POS commands.
class PrinterStatusPoller {
  static DateTime? _lastPoll;
  static const _minPollInterval = Duration(milliseconds: 250);

  /// Queries the printer by sending DLE EOT commands and reading the response.
  /// Requires a [write] closure to send bytes and a [read] closure to fetch responses.
  ///
  /// Enforces a minimum 250 ms gap between polls to avoid hammering the device.
  static Future<HardwareStatus> checkStatus({
    required Future<void> Function(List<int>) write,
    required Future<Uint8List> Function(int bytesToRead) read,
  }) async {
    if (_lastPoll != null) {
      final elapsed = DateTime.now().difference(_lastPoll!);
      if (elapsed < _minPollInterval) {
        await Future<void>.delayed(_minPollInterval - elapsed);
      }
    }
    _lastPoll = DateTime.now();
    bool isPaperOut = false;
    bool isCoverOpen = false;
    bool isDrawerOpen = false;
    PrinterEvent? event;

    try {
      // Query 1: Paper sensor status (DLE EOT 4)
      await write([0x10, 0x04, 0x04]);
      final paperBytes = await read(1);
      if (paperBytes.isNotEmpty) {
        final byte = paperBytes[0];
        // Bits 5 and 6 high = Paper out
        isPaperOut = (byte & 0x60) == 0x60;
      }

      // Query 2: Offline status (DLE EOT 2)
      await write([0x10, 0x04, 0x02]);
      final offlineBytes = await read(1);
      if (offlineBytes.isNotEmpty) {
        final byte = offlineBytes[0];
        // Bit 2 high = Cover open
        isCoverOpen = (byte & 0x04) == 0x04;
      }

      // Query 3: General status (DLE EOT 1)
      await write([0x10, 0x04, 0x01]);
      final generalBytes = await read(1);
      if (generalBytes.isNotEmpty) {
        final byte = generalBytes[0];
        // Bit 2 low = Drawer open (depends on drawer wiring, usually 0 is open)
        isDrawerOpen = (byte & 0x04) == 0x00;
      }

      if (isCoverOpen) {
        event = PrinterEvent.lidOpen;
      } else if (isPaperOut) {
        event = PrinterEvent.paperEnd;
      }

      return HardwareStatus(
        event: event,
        isPaperOut: isPaperOut,
        isCoverOpen: isCoverOpen,
        isDrawerOpen: isDrawerOpen,
      );
    } catch (e) {
      // Timeout or connection error
      return const HardwareStatus(event: PrinterEvent.unknown);
    }
  }
}
