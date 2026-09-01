#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const semanticVersion = /^\d+\.\d+\.\d+(?:-rc\.\d+)?$/;
const sourceCommit = /^[a-f0-9]{40}$/;
const exactManifestKeys = [
  "artifacts",
  "build",
  "channel",
  "minimumSupportedVersion",
  "publishedAt",
  "releaseNotesUrl",
  "required",
  "schemaVersion",
  "sourceCommit",
  "sourceRepository",
  "version",
].sort();

function exactKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  if (JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...expected].sort())) {
    throw new Error(`${label} has unexpected fields`);
  }
}

async function readConfiguration(environment) {
  const distributionPath = environment.CODECADDIE_RELEASE_DISTRIBUTION_FILE
    ? path.resolve(environment.CODECADDIE_RELEASE_DISTRIBUTION_FILE)
    : new URL("../config/release-distribution.json", import.meta.url);
  const trustPath = environment.CODECADDIE_RELEASE_TRUST_FILE
    ? path.resolve(environment.CODECADDIE_RELEASE_TRUST_FILE)
    : new URL("../config/release-trust.json", import.meta.url);
  const distribution = JSON.parse(await readFile(distributionPath, "utf8"));
  const trust = JSON.parse(await readFile(trustPath, "utf8"));
  exactKeys(distribution, [
    "githubRepository",
    "latestReleaseApiUrl",
    "macosUniversalAssetName",
    "manifestAssetName",
    "manifestBundleAssetName",
    "releaseDownloadBaseUrl",
    "schemaVersion",
  ], "release distribution configuration");
  if (distribution.schemaVersion !== 2) {
    throw new Error("release distribution configuration schema is unsupported");
  }
  const expectedLatestApi = `https://api.github.com/repos/${distribution.githubRepository}/releases/latest`;
  const expectedDownloadBase = `https://github.com/${distribution.githubRepository}/releases/download`;
  if (
    distribution.latestReleaseApiUrl !== expectedLatestApi
    || distribution.releaseDownloadBaseUrl !== expectedDownloadBase
    || distribution.manifestAssetName !== "manifest.json"
    || distribution.manifestBundleAssetName !== "manifest.sigstore.json"
    || distribution.macosUniversalAssetName !== "CodeCaddie-macOS-universal.zip"
  ) {
    throw new Error("release distribution configuration is not canonical");
  }
  exactKeys(trust, ["schemaVersion", "sigstore"], "release trust configuration");
  if (trust.schemaVersion !== 2) throw new Error("release trust configuration schema is unsupported");
  exactKeys(trust.sigstore, [
    "allowedTriggers",
    "bundleMediaType",
    "certificateIdentity",
    "oidcIssuer",
    "repository",
    "repositoryId",
    "tufMirror",
    "workflowRef",
  ], "Sigstore trust policy");
  const policy = trust.sigstore;
  if (
    policy.bundleMediaType !== "application/vnd.dev.sigstore.bundle.v0.3+json"
    || policy.oidcIssuer !== "https://token.actions.githubusercontent.com"
    || policy.repository !== distribution.githubRepository
    || policy.certificateIdentity
      !== `https://github.com/${policy.repository}/.github/workflows/release.yml@refs/heads/main`
    || policy.workflowRef !== "refs/heads/main"
    || policy.tufMirror !== "https://tuf-repo-cdn.sigstore.dev"
  ) {
    throw new Error("Sigstore trust policy is not canonical");
  }
  if (!/^[1-9]\d*$/.test(policy.repositoryId)) {
    throw new Error("Sigstore repositoryId must be replaced with the new public repository numeric ID");
  }
  if (
    !Array.isArray(policy.allowedTriggers)
    || policy.allowedTriggers.length === 0
    || new Set(policy.allowedTriggers).size !== policy.allowedTriggers.length
    || policy.allowedTriggers.some((trigger) => !["push", "workflow_dispatch"].includes(trigger))
  ) {
    throw new Error("Sigstore allowed triggers are invalid");
  }
  return { distribution, policy };
}

function defaultSigstoreVerifier({ bundlePath, manifestPath, manifest, policy }) {
  const common = [
    "verify-blob",
    "--bundle",
    bundlePath,
    "--certificate-identity",
    policy.certificateIdentity,
    "--certificate-oidc-issuer",
    policy.oidcIssuer,
    "--certificate-github-workflow-repository",
    policy.repository,
    "--certificate-github-workflow-ref",
    policy.workflowRef,
    "--certificate-github-workflow-sha",
    manifest.sourceCommit,
  ];
  let lastError;
  for (const trigger of policy.allowedTriggers) {
    try {
      execFileSync(
        "cosign",
        [...common, "--certificate-github-workflow-trigger", trigger, manifestPath],
        { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 120_000 },
      );
      return;
    } catch (error) {
      lastError = error;
    }
  }
  const detail = lastError?.stderr?.toString().trim();
  throw new Error(`Sigstore bundle verification failed${detail ? `: ${detail}` : ""}`);
}

function requiredReleaseArtifacts(universalAssetName) {
  return new Map([
    ["macos-arm64-zip", universalAssetName],
    ["macos-x64-zip", universalAssetName],
  ]);
}

export async function verifyReleaseManifest(
  manifestPath,
  bundlePath,
  expectedTag,
  environment = process.env,
  artifactDirectory,
  verifySigstore = defaultSigstoreVerifier,
) {
  const resolvedManifestPath = path.resolve(manifestPath);
  const resolvedBundlePath = path.resolve(bundlePath);
  for (const [file, label] of [
    [resolvedManifestPath, "release manifest"],
    [resolvedBundlePath, "Sigstore bundle"],
  ]) {
    const details = await lstat(file);
    if (!details.isFile() || details.isSymbolicLink()) throw new Error(`${label} must be a regular file`);
  }
  const rawManifest = await readFile(resolvedManifestPath);
  const manifest = JSON.parse(rawManifest.toString("utf8"));
  const bundle = JSON.parse(await readFile(resolvedBundlePath, "utf8"));
  const { distribution, policy } = await readConfiguration(environment);
  exactKeys(manifest, exactManifestKeys, "release manifest");
  if (manifest.schemaVersion !== 2) throw new Error("unsupported release manifest schema");
  if (bundle.mediaType !== policy.bundleMediaType) {
    throw new Error("release signature is not a Sigstore bundle v0.3");
  }
  if (!semanticVersion.test(manifest.version)) throw new Error("release manifest version is invalid");
  if (!Number.isSafeInteger(manifest.build) || manifest.build < 1) {
    throw new Error("release manifest build is invalid");
  }
  const expectedChannel = manifest.version.includes("-rc.") ? "beta" : "stable";
  if (manifest.channel !== expectedChannel) throw new Error("release manifest channel is invalid");
  if (!semanticVersion.test(manifest.minimumSupportedVersion)) {
    throw new Error("release manifest minimum supported version is invalid");
  }
  if (typeof manifest.required !== "boolean") throw new Error("release manifest required flag is invalid");
  if (
    typeof manifest.publishedAt !== "string"
    || !Number.isFinite(Date.parse(manifest.publishedAt))
  ) {
    throw new Error("release manifest published timestamp is invalid");
  }
  if (manifest.sourceRepository !== distribution.githubRepository || !sourceCommit.test(manifest.sourceCommit)) {
    throw new Error("release manifest source identity is invalid");
  }
  const expectedSourceCommit = environment.CODECADDIE_EXPECTED_SOURCE_COMMIT;
  if (expectedSourceCommit !== undefined && manifest.sourceCommit !== expectedSourceCommit) {
    throw new Error("release manifest source commit does not match the expected commit");
  }
  const expectedRepositoryId = environment.CODECADDIE_EXPECTED_REPOSITORY_ID;
  if (expectedRepositoryId !== undefined && policy.repositoryId !== expectedRepositoryId) {
    throw new Error("Sigstore repository ID does not match the executing repository");
  }
  const tagMatch = expectedTag.match(/^v(\d+\.\d+\.\d+(?:-rc\.\d+)?)\+([1-9]\d*)$/);
  if (!tagMatch) throw new Error("release tag is not a build-qualified version identity");
  if (tagMatch[1] !== manifest.version || Number(tagMatch[2]) !== manifest.build) {
    throw new Error("release manifest version or build does not match the promoted tag");
  }
  const expectedNotes = `https://github.com/${distribution.githubRepository}/releases/tag/${expectedTag}`;
  if (manifest.releaseNotesUrl !== expectedNotes) {
    throw new Error("release notes URL does not match the promoted tag");
  }
  await verifySigstore({
    bundle,
    bundlePath: resolvedBundlePath,
    manifest,
    manifestPath: resolvedManifestPath,
    policy,
    rawManifest,
  });

  const githubPrefix = `/${distribution.githubRepository}/releases/download/${expectedTag}/`;
  const identities = new Set();
  const artifactNames = new Map();
  if (!Array.isArray(manifest.artifacts) || manifest.artifacts.length === 0) {
    throw new Error("release manifest contains no artifacts");
  }
  for (const artifact of manifest.artifacts) {
    exactKeys(artifact, ["architecture", "format", "platform", "sha256", "size", "url"], "release artifact");
    const identity = `${artifact.platform}-${artifact.architecture}-${artifact.format}`;
    if (identities.has(identity)) throw new Error(`duplicate release artifact ${identity}`);
    identities.add(identity);
    const url = new URL(artifact.url);
    if (
      url.protocol !== "https:"
      || url.hostname !== "github.com"
      || url.port
      || url.username
      || url.password
      || url.search
      || url.hash
      || !url.pathname.startsWith(githubPrefix)
    ) {
      throw new Error(`release artifact URL is outside the promoted repository and tag: ${url}`);
    }
    if (
      !Number.isSafeInteger(artifact.size)
      || artifact.size <= 0
      || !/^[a-f0-9]{64}$/.test(artifact.sha256)
    ) {
      throw new Error(`release artifact metadata is invalid for ${identity}`);
    }
    const fileName = decodeURIComponent(url.pathname.split("/").at(-1));
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,200}$/.test(fileName)) {
      throw new Error(`release artifact filename is invalid for ${identity}`);
    }
    artifactNames.set(identity, fileName);
    if (artifactDirectory) {
      const artifactPath = path.join(path.resolve(artifactDirectory), fileName);
      const details = await lstat(artifactPath);
      if (!details.isFile() || details.isSymbolicLink() || details.size !== artifact.size) {
        throw new Error(`downloaded release artifact size or type is invalid for ${identity}`);
      }
      const digest = createHash("sha256");
      for await (const chunk of createReadStream(artifactPath)) digest.update(chunk);
      if (digest.digest("hex") !== artifact.sha256) {
        throw new Error(`downloaded release artifact hash is invalid for ${identity}`);
      }
    }
  }
  if (environment.CODECADDIE_REQUIRE_COMPLETE_RELEASE === "1") {
    const required = requiredReleaseArtifacts(distribution.macosUniversalAssetName);
    if (artifactNames.size !== required.size) {
      throw new Error("release manifest does not contain the complete macOS artifact inventory");
    }
    for (const [identity, fileName] of required) {
      if (artifactNames.get(identity) !== fileName) {
        throw new Error(`release manifest is missing the required artifact ${identity}:${fileName}`);
      }
    }
  }
  return manifest;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [manifestPath, bundlePath, expectedTag, artifactDirectory] = process.argv.slice(2);
  if (!manifestPath || !bundlePath || !expectedTag) {
    throw new Error(
      "usage: verify-release-manifest.mjs manifest.json manifest.sigstore.json vX.Y.Z+BUILD [artifact-directory]",
    );
  }
  const manifest = await verifyReleaseManifest(
    manifestPath,
    bundlePath,
    expectedTag,
    process.env,
    artifactDirectory,
  );
  console.log(`verified keyless release manifest for v${manifest.version}+${manifest.build}`);
}
