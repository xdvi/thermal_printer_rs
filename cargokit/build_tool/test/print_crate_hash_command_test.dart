import 'dart:io';

import 'package:build_tool/src/crate_hash.dart';
import 'package:path/path.dart' as p;
import 'package:test/test.dart';

void main() {
  test('print-crate-hash prints the consumer crate hash', () async {
    final fixtureDir = p.join(
      Directory.current.path,
      'test',
      'fixtures',
      'mini_crate',
    );
    final expected = CrateHash.compute(fixtureDir);

    final result = await Process.run(
      'dart',
      [
        'run',
        'bin/build_tool.dart',
        'print-crate-hash',
        '--manifest-dir',
        fixtureDir
      ],
      workingDirectory: Directory.current.path,
    );

    expect(result.exitCode, 0);
    expect((result.stdout as String).trim(), expected);
  });
}
