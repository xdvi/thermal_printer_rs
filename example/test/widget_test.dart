import 'package:flutter_test/flutter_test.dart';
import 'package:thermal_printer_rs_example/main.dart';

void main() {
  testWidgets('Verify Example App UI', (WidgetTester tester) async {
    // Build our app and trigger a frame.
    await tester.pumpWidget(const PrinterExampleApp());

    // Verify that the title is present in the AppBar
    expect(find.text('thermal_printer_rs'), findsOneWidget);
    
    // Verify that the TCP Configuration section is present
    expect(find.text('TCP Configuration'), findsOneWidget);
  });
}
