import 'dart:io';

import 'package:ed25519_edwards/ed25519_edwards.dart';
import 'package:github/github.dart';
import 'package:path/path.dart' as path;

import 'artifacts_provider.dart';
import 'cargo.dart';
import 'crate_hash.dart';
import 'precompile_binaries.dart';
import 'target.dart';

String releaseTagForHash(String crateHash) => 'precompiled_$crateHash';

String releaseTitleForVersion(String packageVersion) =>
    'Precompiled binaries v$packageVersion';

String releaseTitleForHash(String crateHash) =>
    'Precompiled binaries ${crateHash.substring(0, 8)}';

String desiredReleaseTitle({
  required String? packageVersion,
  required String crateHash,
}) {
  return packageVersion == null
      ? releaseTitleForHash(crateHash)
      : releaseTitleForVersion(packageVersion);
}

bool releaseTitleNeedsUpdate(Release release, String desiredTitle) {
  return release.name != desiredTitle;
}

List<String> expectedLocalArtifactsForTarget(Target target, String libraryName) {
  return getArtifactNames(
    target: target,
    libraryName: libraryName,
    remote: true,
  );
}

List<String> expectedRemoteAssetsForTarget(Target target, String libraryName) {
  return expectedLocalArtifactsForTarget(target, libraryName)
      .map((name) => PrecompileBinaries.fileName(target, name))
      .toList();
}

List<File> collectLocalArtifacts({
  required String artifactsDir,
  required Target target,
  required String libraryName,
}) {
  return expectedLocalArtifactsForTarget(target, libraryName).map((assetName) {
    final file = File(path.join(artifactsDir, target.rust, assetName));
    if (!file.existsSync()) {
      throw Exception('Missing artifact: ${file.path}');
    }
    return file;
  }).toList();
}

bool releaseHasAsset(Release release, String assetName) {
  return (release.assets ?? []).any((asset) => asset.name == assetName);
}

List<CreateReleaseAsset> createAssetsToUpload({
  required Release release,
  required Target target,
  required File file,
  required PrivateKey privateKey,
}) {
  final assetData = file.readAsBytesSync();
  final localName = path.basename(file.path);
  final remoteName = PrecompileBinaries.fileName(target, localName);
  final signatureName = PrecompileBinaries.signatureFileName(target, localName);
  final assets = <CreateReleaseAsset>[];

  if (!releaseHasAsset(release, remoteName)) {
    assets.add(CreateReleaseAsset(
      name: remoteName,
      contentType: 'application/octet-stream',
      assetData: assetData,
    ));
  }

  if (!releaseHasAsset(release, signatureName)) {
    assets.add(CreateReleaseAsset(
      name: signatureName,
      contentType: 'application/octet-stream',
      assetData: sign(privateKey, assetData),
    ));
  }

  return assets;
}

Future<void> uploadReleaseAssetWithRetry({
  required Future<void> Function() upload,
  Duration retryDelay = const Duration(seconds: 2),
}) async {
  var retryCount = 0;
  while (true) {
    try {
      await upload();
      break;
    } on Exception {
      if (retryCount == 10) {
        rethrow;
      }
      ++retryCount;
      await Future.delayed(retryDelay);
    }
  }
}

class PublishPrecompiledArtifacts {
  PublishPrecompiledArtifacts({
    required this.privateKey,
    required this.githubToken,
    required this.repositorySlug,
    required this.manifestDir,
    required this.artifactsDir,
    required this.targets,
  });

  final PrivateKey privateKey;
  final String githubToken;
  final RepositorySlug repositorySlug;
  final String manifestDir;
  final String artifactsDir;
  final List<Target> targets;

  Future<void> run() async {
    final crateInfo = CrateInfo.load(manifestDir);
    final crateHash = CrateHash.compute(manifestDir);
    final github = GitHub(auth: Authentication.withToken(githubToken));
    final repo = github.repositories;
    final release = await _getOrCreateRelease(
      repo: repo,
      tagName: releaseTagForHash(crateHash),
      packageName: crateInfo.packageName,
      packageVersion: crateInfo.packageVersion,
      crateHash: crateHash,
    );

    for (final target in targets) {
      final artifacts = collectLocalArtifacts(
        artifactsDir: artifactsDir,
        target: target,
        libraryName: crateInfo.packageName,
      );

      for (final file in artifacts) {
        final assets = createAssetsToUpload(
          release: release,
          target: target,
          file: file,
          privateKey: privateKey,
        );

        for (final asset in assets) {
          await uploadReleaseAssetWithRetry(
            upload: () => repo.uploadReleaseAssets(release, [asset]),
          );
        }
      }
    }
  }

  Future<Release> _getOrCreateRelease({
    required RepositoriesService repo,
    required String tagName,
    required String packageName,
    required String? packageVersion,
    required String crateHash,
  }) async {
    final releaseTitle = desiredReleaseTitle(
      packageVersion: packageVersion,
      crateHash: crateHash,
    );

    try {
      final release = await repo.getReleaseByTagName(repositorySlug, tagName);
      if (!releaseTitleNeedsUpdate(release, releaseTitle)) {
        return release;
      }
      return repo.editRelease(
        repositorySlug,
        release,
        name: releaseTitle,
      );
    } on ReleaseNotFound {
      return repo.createRelease(
        repositorySlug,
        CreateRelease.from(
          tagName: tagName,
          name: releaseTitle,
          targetCommitish: null,
          isDraft: false,
          isPrerelease: false,
          body: 'Precompiled binaries for crate $packageName, '
              'crate hash $crateHash.',
        ),
      );
    }
  }
}
