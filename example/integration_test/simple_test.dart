import 'package:integration_test/integration_test.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:thermal_printer_rs/thermal_printer_rs.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  test('Check if printer service can be initialized', () async {
    expect(ThermalPrinterRs.isReady, false);
    await ThermalPrinterRs.initTcp(host: '127.0.0.1');
    expect(ThermalPrinterRs.isReady, true);
  });
}
