import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:thermal_printer_rs/thermal_printer_rs.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('Plugin initialization test', (WidgetTester tester) async {
    // Check initial state
    expect(ThermalPrinterRs.isReady, false);

    // Initialize with a mock TCP config (won't connect yet)
    await ThermalPrinterRs.initTcp(host: '127.0.0.1');

    // Should be ready now
    expect(ThermalPrinterRs.isReady, true);
  });
}
