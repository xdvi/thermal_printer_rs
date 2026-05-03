import 'dart:io';

import 'package:build_tool/src/publish_precompiled_artifacts.dart';
import 'package:build_tool/src/target.dart';
import 'package:ed25519_edwards/ed25519_edwards.dart';
import 'package:github/github.dart';
import 'package:path/path.dart' as path;
import 'package:test/test.dart';

void main() {
  test('releaseTagForHash prefixes crate hash', () {
    expect(releaseTagForHash('abc123'), 'precompiled_abc123');
  });

  test('releaseTitleForVersion formats a versioned release title', () {
    expect(
      releaseTitleForVersion('1.2.3'),
      'Precompiled binaries v1.2.3',
    );
  });

  test('releaseTitleForHash falls back to the short crate hash', () {
    expect(
      releaseTitleForHash('0123456789abcdef'),
      'Precompiled binaries 01234567',
    );
  });

  test('desiredReleaseTitle prefers version and falls back to hash', () {
    expect(
      desiredReleaseTitle(packageVersion: '1.2.3', crateHash: '0123456789abcdef'),
      'Precompiled binaries v1.2.3',
    );
    expect(
      desiredReleaseTitle(packageVersion: null, crateHash: '0123456789abcdef'),
      'Precompiled binaries 01234567',
    );
  });

  test('releaseTitleNeedsUpdate detects mismatched existing titles', () {
    expect(
      releaseTitleNeedsUpdate(
        Release(name: 'Precompiled binaries 01234567'),
        'Precompiled binaries v1.2.3',
      ),
      isTrue,
    );
    expect(
      releaseTitleNeedsUpdate(
        Release(name: 'Precompiled binaries v1.2.3'),
        'Precompiled binaries v1.2.3',
      ),
      isFalse,
    );
  });

  test('expectedRemoteAssetsForTarget uses remote artifact names', () {
    final target = Target.forRustTriple('x86_64-pc-windows-msvc')!;

    expect(
      expectedRemoteAssetsForTarget(target, 'thermal_printer_rs'),
      [
        'x86_64-pc-windows-msvc_thermal_printer_rs.dll',
        'x86_64-pc-windows-msvc_thermal_printer_rs.dll.lib',
      ],
    );
  });

  test('collectLocalArtifacts reads unprefixed local artifact names', () {
    final tempDir = Directory.systemTemp.createTempSync(
      'publish-precompiled-artifacts-test',
    );
    addTearDown(() => tempDir.deleteSync(recursive: true));

    final target = Target.forRustTriple('x86_64-pc-windows-msvc')!;
    final targetDir = Directory(path.join(tempDir.path, target.rust))
      ..createSync(recursive: true);
    File(path.join(targetDir.path, 'thermal_printer_rs.dll'))
        .writeAsBytesSync([1]);
    File(path.join(targetDir.path, 'thermal_printer_rs.dll.lib'))
        .writeAsBytesSync([2]);

    final files = collectLocalArtifacts(
      artifactsDir: tempDir.path,
      target: target,
      libraryName: 'thermal_printer_rs',
    );

    expect(
      files.map((file) => path.basename(file.path)).toList(),
      ['thermal_printer_rs.dll', 'thermal_printer_rs.dll.lib'],
    );
  });

  test('collectLocalArtifacts throws when required file is missing', () {
    final tempDir = Directory.systemTemp.createTempSync(
      'publish-precompiled-artifacts-test',
    );
    addTearDown(() => tempDir.deleteSync(recursive: true));

    final target = Target.forRustTriple('aarch64-linux-android')!;
    Directory(path.join(tempDir.path, target.rust)).createSync(recursive: true);

    expect(
      () => collectLocalArtifacts(
        artifactsDir: tempDir.path,
        target: target,
        libraryName: 'thermal_printer_rs',
      ),
      throwsA(isA<Exception>()),
    );
  });

  test('createAssetsToUpload prefixes local basenames and skips existing', () {
    final tempDir = Directory.systemTemp.createTempSync(
      'publish-precompiled-artifacts-test',
    );
    addTearDown(() => tempDir.deleteSync(recursive: true));

    final target = Target.forRustTriple('aarch64-linux-android')!;
    final file = File(path.join(tempDir.path, 'libthermal_printer_rs.so'))
      ..writeAsBytesSync([1, 2, 3]);
    final release = Release(assets: [
      ReleaseAsset(name: 'aarch64-linux-android_libthermal_printer_rs.so'),
    ]);

    final assets = createAssetsToUpload(
      release: release,
      target: target,
      file: file,
      privateKey: generateKey().privateKey,
    );

    expect(
      assets.map((asset) => asset.name).toList(),
      ['aarch64-linux-android_libthermal_printer_rs.so.sig'],
    );
  });

  test('uploadReleaseAssetWithRetry retries transient failures', () async {
    var attempts = 0;

    await uploadReleaseAssetWithRetry(
      upload: () async {
        attempts++;
        if (attempts < 3) {
          throw Exception('transient failure');
        }
      },
      retryDelay: Duration.zero,
    );

    expect(attempts, 3);
  });
}
