import 'package:build_tool/src/cargo.dart';
import 'package:test/test.dart';

void main() {
  test('CrateInfo.parseManifest parses direct package version string', () {
    final crateInfo = CrateInfo.parseManifest('''
[package]
name = "thermal_printer_rs"
version = "0.4.2"
''');

    expect(crateInfo.packageName, 'thermal_printer_rs');
    expect(crateInfo.packageVersion, '0.4.2');
  });

  test(
    'CrateInfo.parseManifest leaves workspace-inherited package version unresolved',
    () {
      final crateInfo = CrateInfo.parseManifest('''
[package]
name = "thermal_printer_rs"
version.workspace = true
''');

      expect(crateInfo.packageName, 'thermal_printer_rs');
      expect(crateInfo.packageVersion, isNull);
    },
  );
}
