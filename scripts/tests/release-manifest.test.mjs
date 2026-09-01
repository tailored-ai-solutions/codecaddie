import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  createReleaseManifest,
  parseReleaseArgs,
} from "../release-manifest.mjs";
import { verifyReleaseManifest } from "../verify-release-manifest.mjs";

const commit = "a".repeat(40);
const bundleMediaType = "application/vnd.dev.sigstore.bundle.v0.3+json";

async function writeConfiguration(directory, repository = "example/codecaddie") {
  const distributionPath = path.join(directory, "release-distribution.json");
  const trustPath = path.join(directory, "release-trust.json");
  await writeFile(distributionPath, `${JSON.stringify({
    schemaVersion: 2,
    githubRepository: repository,
    latestReleaseApiUrl: `https://api.github.com/repos/${repository}/releases/latest`,
    releaseDownloadBaseUrl: `https://github.com/${repository}/releases/download`,
    manifestAssetName: "manifest.json",
    manifestBundleAssetName: "manifest.sigstore.json",
    macosUniversalAssetName: "CodeCaddie-macOS-universal.zip",
  }, null, 2)}\n`);
  await writeFile(trustPath, `${JSON.stringify({
    schemaVersion: 2,
    sigstore: {
      bundleMediaType,
      oidcIssuer: "https://token.actions.githubusercontent.com",
      certificateIdentity: `https://github.com/${repository}/.github/workflows/release.yml@refs/heads/main`,
      repository,
      repositoryId: "123456789",
      workflowRef: "refs/heads/main",
      allowedTriggers: ["push", "workflow_dispatch"],
      tufMirror: "https://tuf-repo-cdn.sigstore.dev",
    },
  }, null, 2)}\n`);
  return {
    CODECADDIE_RELEASE_DISTRIBUTION_FILE: distributionPath,
    CODECADDIE_RELEASE_TRUST_FILE: trustPath,
    CODECADDIE_EXPECTED_REPOSITORY_ID: "123456789",
    CODECADDIE_EXPECTED_SOURCE_COMMIT: commit,
  };
}

async function releaseFixture(repository = "example/codecaddie") {
  const directory = await mkdtemp(path.join(os.tmpdir(), "codecaddie-keyless-manifest-"));
  const artifact = path.join(directory, "CodeCaddie-macOS-universal.zip");
  const output = path.join(directory, "release");
  const bundle = path.join(output, "manifest.sigstore.json");
  await writeFile(artifact, "signed and notarized universal application");
  const options = parseReleaseArgs([
    "--version", "0.4.0",
    "--build", "2001",
    "--output", output,
    "--repository", repository,
    "--source-commit", commit,
    "--release-base", `https://github.com/${repository}/releases/download/v0.4.0+2001`,
    "--release-notes", `https://github.com/${repository}/releases/tag/v0.4.0+2001`,
    "--published", "2026-08-30T12:00:00Z",
    "--artifact", `macos:arm64:zip:${artifact}`,
    "--artifact", `macos:x64:zip:${artifact}`,
  ]);
  const result = await createReleaseManifest(options);
  await writeFile(bundle, `${JSON.stringify({ mediaType: bundleMediaType })}\n`);
  return { artifact, bundle, directory, options, output, ...result };
}

test("release arguments require a build-qualified immutable source identity", () => {
  assert.throws(() => parseReleaseArgs(["--version", "0.4.0"]), /--build/);
  const common = [
    "--version", "0.4.0",
    "--build", "1",
    "--output", "release",
    "--repository", "example/codecaddie",
    "--release-base", "https://github.com/example/codecaddie/releases/download/v0.4.0+1",
    "--release-notes", "https://github.com/example/codecaddie/releases/tag/v0.4.0+1",
    "--artifact", "macos:arm64:zip:CodeCaddie.zip",
  ];
  assert.throws(() => parseReleaseArgs([...common, "--source-commit", "ABC"]), /lowercase 40-character/);
  assert.throws(
    () => parseReleaseArgs([...common, "--source-commit", commit, "--required", "sometimes"]),
    /required must be true or false/,
  );
});

test("one fixed universal asset produces the exact keyless ReleaseManifestV2 contract", async () => {
  const fixture = await releaseFixture();
  const artifactBytes = await readFile(fixture.artifact);
  const expectedHash = createHash("sha256").update(artifactBytes).digest("hex");
  assert.deepEqual(Object.keys(fixture.manifest), [
    "schemaVersion",
    "version",
    "build",
    "channel",
    "publishedAt",
    "minimumSupportedVersion",
    "required",
    "releaseNotesUrl",
    "sourceRepository",
    "sourceCommit",
    "artifacts",
  ]);
  assert.equal(fixture.manifest.schemaVersion, 2);
  assert.equal(fixture.manifest.minimumSupportedVersion, "0.4.0");
  assert.equal(fixture.manifest.sourceCommit, commit);
  assert.equal(fixture.manifest.required, false);
  assert.deepEqual(
    fixture.manifest.artifacts.map(({ platform, architecture, format }) => ({
      platform,
      architecture,
      format,
    })),
    [
      { platform: "macos", architecture: "arm64", format: "zip" },
      { platform: "macos", architecture: "x64", format: "zip" },
    ],
  );
  assert.equal(new Set(fixture.manifest.artifacts.map(({ url }) => url)).size, 1);
  assert.equal(fixture.manifest.artifacts[0].sha256, expectedHash);
  assert.equal(
    await readFile(path.join(fixture.output, "SHA256SUMS.txt"), "utf8"),
    `${expectedHash}  CodeCaddie-macOS-universal.zip\n`,
  );
});

test("manifest verification binds the Sigstore policy, source commit, tag, and artifact bytes", async () => {
  const fixture = await releaseFixture();
  const environment = await writeConfiguration(fixture.directory);
  let verifiedBundle = false;
  const verifier = async ({ bundle, manifest, policy, rawManifest }) => {
    assert.equal(bundle.mediaType, bundleMediaType);
    assert.equal(manifest.sourceCommit, commit);
    assert.equal(policy.repositoryId, "123456789");
    assert.deepEqual(JSON.parse(rawManifest), manifest);
    verifiedBundle = true;
  };
  const manifest = await verifyReleaseManifest(
    path.join(fixture.output, "manifest.json"),
    fixture.bundle,
    "v0.4.0+2001",
    { ...environment, CODECADDIE_REQUIRE_COMPLETE_RELEASE: "1" },
    fixture.directory,
    verifier,
  );
  assert.equal(manifest.build, 2001);
  assert.equal(verifiedBundle, true);

  await assert.rejects(
    verifyReleaseManifest(
      path.join(fixture.output, "manifest.json"),
      fixture.bundle,
      "v0.4.0+2002",
      environment,
      fixture.directory,
      verifier,
    ),
    /version or build does not match/,
  );
  await writeFile(fixture.artifact, "tampered application");
  await assert.rejects(
    verifyReleaseManifest(
      path.join(fixture.output, "manifest.json"),
      fixture.bundle,
      "v0.4.0+2001",
      environment,
      fixture.directory,
      verifier,
    ),
    /size or type|hash/,
  );
});

test("manifest verification rejects non-GitHub distribution and non-v0.3 bundles", async () => {
  const fixture = await releaseFixture();
  const environment = await writeConfiguration(fixture.directory);
  const manifestPath = path.join(fixture.output, "manifest.json");
  const altered = structuredClone(fixture.manifest);
  altered.artifacts[0].url = "https://downloads.example.test/CodeCaddie-macOS-universal.zip";
  await writeFile(manifestPath, `${JSON.stringify(altered, null, 2)}\n`);
  await assert.rejects(
    verifyReleaseManifest(
      manifestPath,
      fixture.bundle,
      "v0.4.0+2001",
      environment,
      undefined,
      async () => {},
    ),
    /outside the promoted repository/,
  );
  await writeFile(manifestPath, fixture.rawManifest);
  await writeFile(fixture.bundle, '{"mediaType":"application/example"}\n');
  await assert.rejects(
    verifyReleaseManifest(
      manifestPath,
      fixture.bundle,
      "v0.4.0+2001",
      environment,
      undefined,
      async () => {},
    ),
    /Sigstore bundle v0.3/,
  );
});

test("the checked-in trust policy accepts only the staging sentinel or a pinned numeric repository ID", async () => {
  const fixture = await releaseFixture("tailored-ai-solutions/codecaddie");
  const checkedInTrust = JSON.parse(
    await readFile(new URL("../../config/release-trust.json", import.meta.url), "utf8"),
  );
  const verify = () =>
    verifyReleaseManifest(
      path.join(fixture.output, "manifest.json"),
      fixture.bundle,
      "v0.4.0+2001",
      {},
      undefined,
      async () => {},
  );
  if (checkedInTrust.sigstore.repositoryId === "REPLACE_WITH_NEW_PUBLIC_REPOSITORY_ID") {
    await assert.rejects(
      verify(),
      /must be replaced with the new public repository numeric ID/,
    );
  } else {
    assert.match(checkedInTrust.sigstore.repositoryId, /^[1-9]\d*$/);
    await assert.doesNotReject(verify());
  }
});
