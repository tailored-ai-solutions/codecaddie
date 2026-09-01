import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const safetyScript = await readFile(
  new URL("scripts/check-public-safety.sh", root),
  "utf8",
);
const releaseRunbook = await readFile(new URL("docs/RELEASING.md", root), "utf8");
const releaseWorkflow = await readFile(
  new URL(".github/workflows/release.yml", root),
  "utf8",
);
const architecture = await readFile(new URL("docs/ARCHITECTURE.md", root), "utf8");
const development = await readFile(new URL("docs/DEVELOPMENT.md", root), "utf8");

function git(directory, args) {
  const result = spawnSync("git", args, { cwd: directory, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
}

async function makeRepository(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "codecaddie-public-safety-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(path.join(directory, "scripts"), { recursive: true });
  await writeFile(path.join(directory, "scripts", "check-public-safety.sh"), safetyScript);
  git(directory, ["init", "--quiet"]);
  return directory;
}

test("public safety rejects a tracked App Store Connect private key", async (t) => {
  const directory = await makeRepository(t);
  await writeFile(path.join(directory, "AuthKey_Example.p8"), "fixture-not-a-real-key\n");
  git(directory, ["add", "."]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /AuthKey_Example\.p8/);
  assert.match(result.stderr, /forbidden credential-shaped filename/);
});

test("public safety permits a tracked public signing certificate", async (t) => {
  const directory = await makeRepository(t);
  await writeFile(path.join(directory, "release-signing.cer"), "fixture-public-certificate\n");
  git(directory, ["add", "."]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /tracked public files pass the local safety scan/);
});

test("public safety rejects tracked exportable signing containers", async (t) => {
  const directory = await makeRepository(t);
  await writeFile(path.join(directory, "mac-signing.p12"), "fixture-not-a-real-key\n");
  await writeFile(path.join(directory, "windows-signing.pfx"), "fixture-not-a-real-key\n");
  git(directory, ["add", "."]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /mac-signing\.p12/);
  assert.match(result.stderr, /windows-signing\.pfx/);
  assert.match(result.stderr, /forbidden credential-shaped filename/);
});

test("public safety applies site-specific patterns without tracking the identifiers", async (t) => {
  const directory = await makeRepository(t);
  await writeFile(path.join(directory, "public.txt"), "synthetic-private-marker-7391\n");
  await writeFile(
    path.join(directory, "scripts", "private-patterns.local"),
    "synthetic-private-marker-[0-9]+\n",
  );
  git(directory, ["add", "public.txt", "scripts/check-public-safety.sh"]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /public\.txt/);
  assert.match(result.stderr, /private pattern file line 1/);
});

test("trusted public-safety runs fail closed without the private denylist", async (t) => {
  const directory = await makeRepository(t);
  git(directory, ["add", "scripts/check-public-safety.sh"]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
    env: { ...process.env, CODECADDIE_REQUIRE_PRIVATE_PATTERNS: "1" },
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /required private pattern file is missing or empty/);
});

test("public safety rejects a tracked path that names a private identifier", async (t) => {
  const directory = await makeRepository(t);
  const fixtureDirectory = path.join(directory, "testdata", "synthetic-private-marker-2044");
  await mkdir(fixtureDirectory, { recursive: true });
  await writeFile(path.join(fixtureDirectory, "notes.txt"), "the body names nothing private\n");
  await writeFile(
    path.join(directory, "scripts", "private-patterns.local"),
    "synthetic-private-marker-[0-9]+\n",
  );
  git(directory, ["add", "testdata", "scripts/check-public-safety.sh"]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /testdata\/synthetic-private-marker-2044\/notes\.txt/);
  assert.match(result.stderr, /tracked file name against private pattern file line 1/);
});

test("public safety strips carriage returns so CRLF private patterns still apply", async (t) => {
  const directory = await makeRepository(t);
  await writeFile(path.join(directory, "public.txt"), "synthetic-private-marker-7391\n");
  await writeFile(
    path.join(directory, "scripts", "private-patterns.local"),
    "# pasted from a CRLF editor\r\nsynthetic-private-marker-[0-9]+\r\n",
  );
  git(directory, ["add", "public.txt", "scripts/check-public-safety.sh"]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /public\.txt/);
  assert.match(result.stderr, /private pattern file line 2/);
});

test("untrusted public-safety runs warn visibly when the private denylist is absent", async (t) => {
  const directory = await makeRepository(t);
  await writeFile(path.join(directory, "public.txt"), "nothing private here\n");
  git(directory, ["add", "public.txt", "scripts/check-public-safety.sh"]);

  const result = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
    env: { ...process.env, CODECADDIE_REQUIRE_PRIVATE_PATTERNS: "0" },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /tracked public files pass the local safety scan/);
  assert.match(
    result.stderr,
    /WARNING: private denylist not applied \(scripts\/private-patterns\.local absent\)/,
  );

  await writeFile(path.join(directory, "scripts", "private-patterns.local"), "# no patterns yet\n");
  const applied = spawnSync("bash", ["scripts/check-public-safety.sh"], {
    cwd: directory,
    encoding: "utf8",
  });
  assert.equal(applied.status, 0, applied.stderr);
  assert.doesNotMatch(applied.stderr, /private denylist not applied/);
});

test("release runbook enforces the GitHub-only keyless macOS release contract", () => {
  const normalizedReleaseRunbook = releaseRunbook.replace(/\s+/g, " ");
  for (const phrase of [
    "one parentless root commit",
    "`0.4.0+2001`",
    "`releaseBuildEpoch` at `2000`",
    "first-parent commit count",
    "squash-only merges",
    "default to read-only",
    "first-root/manual-dispatch exception",
    "GitHub Releases is the only permanent binary and update distribution system",
    "Xcode Cloud retains the non-exportable Developer ID credential",
    "`org.codecaddie.desktop`",
    "company-domain individual App Store Connect user",
    "CodeCaddie-only app access",
    "Generate Individual API Keys",
    "cannot generate or download another user's individual private key",
    "team keys cannot be limited to CodeCaddie",
    "`release-apple`",
    "`APP_STORE_CONNECT_PRIVATE_KEY_BASE64`",
    "15-minute JWT",
    "Only the manifest job receives `id-token: write`",
    "`CODECADDIE_PRIVATE_PATTERNS`",
    "`manifest.sigstore.json`",
    "Fulcio certificate",
    "Rekor inclusion",
    "Sigstore TUF mirror",
    "`queue: max`",
    "maximum `(SemVer, build)`",
    "verified high-water mark",
    "A product rollback is fix-forward",
    "SignPath Foundation",
    "Never approve a paid upgrade automatically",
  ]) {
    assert.ok(
      normalizedReleaseRunbook.includes(phrase),
      `release runbook is missing ${phrase}`,
    );
  }

  assert.match(
    releaseRunbook,
    /Release automation never uses personal Apple account credentials, browser\s+session data, a login credential store, or any exportable code-signing\s+credential\./,
  );
  assert.match(releaseRunbook, /artifact-reader key is not a code-signing\s+key/);
  assert.match(releaseRunbook, /JWT uses `sub: user` and never `iss`/);
  assert.match(releaseRunbook, /`scope` authorizes only\s+that exact `GET \/v1\/\.\.\.` request/);
  assert.match(releaseRunbook, /no reports\s+access, no Certificates, Identifiers & Profiles permission, and no signing\s+permission/);
  assert.match(releaseRunbook, /downloads every asset and verifies bytes, checksums, manifest policy,\s+Sigstore identity, and attestations again/);
  assert.match(releaseRunbook, /does not create a second keyless signature/);
  assert.match(releaseRunbook, /high-water decision and `make_latest` value belong to\s+the single serialized draft-publication boundary/);
  assert.match(releaseRunbook, /Rerunning build 2001 after build 2002 therefore verifies 2001 without changing\s+Latest/);

  for (const asset of [
    "CodeCaddie-macOS-universal.zip",
    "manifest.json",
    "manifest.sigstore.json",
    "SHA256SUMS.txt",
    "codecaddie-0.4.0.cdx.json",
    "release-attestations.jsonl",
    "xcode-cloud-provenance.json",
    "RUST-DEPENDENCY-LICENSES.md",
    "dependency-license-exceptions.json",
  ]) {
    assert.ok(releaseRunbook.includes(`\`${asset}\``), `release runbook is missing asset ${asset}`);
  }

  // The release chain is keyless and GitHub-only. These shapes describe what
  // must stay absent without naming any particular retired vendor or design:
  // hosted object storage in the distribution path, cloud key-management or
  // hardware-security-module services, exportable signing containers, personal
  // Apple credentials, fixed long-lived signing keys, manual promotion or
  // rollback paths, and detached signature files beside the keyless bundle.
  const forbidden = [
    /\bblob storage\b|\b(?:BLOB|STORAGE|BUCKET)_[A-Z0-9_]*(?:TOKEN|KEY|SECRET)\b/i,
    /releases\/stable\//i,
    /\bazure\b|\bAZURE_[A-Z0-9_]+\b|vercel blob/i,
    /\bcloud (?:kms|hsm)\b|\bkey vault\b|\bmanaged hsm\b|\b(?:EC|RSA)-HSM\b/i,
    /\bpkcs ?#?12\b|\bp12\b|\bpfx\b/i,
    /exportable (?:code[- ])?signing (?:key|credential|certificate)/i,
    /Account Holder (?:credential|API key)/i,
    /APP_STORE_CONNECT_(?:ISSUER_ID|KEY_SCOPE)/,
    /Apple ID password/i,
    /browser cookie/i,
    /\b[A-Z][A-Z0-9]*_(?:CLIENT_SECRET|TENANT_ID|SUBSCRIPTION_ID|ACCESS_KEY_ID|SECRET_ACCESS_KEY)\b/,
    /\b[A-Z0-9_]*RELEASE_(?:PRIVATE|SIGNING)_KEY\b/,
    /long-lived .*signing key/i,
    /(?:promote|promotion|rollback)-release\.ya?ml/i,
    /manual (?:promotion|rollback)/i,
    /stable pointer/i,
    /manifest\.(?:sig|asc|minisig)(?:`|\s|$)/im,
  ];
  for (const pattern of forbidden) {
    assert.doesNotMatch(releaseRunbook, pattern);
    assert.doesNotMatch(releaseWorkflow, pattern);
  }

  assert.equal(
    (releaseWorkflow.match(/id-token:\s*write/g) ?? []).length,
    1,
    "only the manifest job may request an OIDC identity",
  );
  const manifestJob = releaseWorkflow.indexOf("\n  manifest:");
  const publishJob = releaseWorkflow.indexOf("\n  publish-release:");
  assert.ok(manifestJob >= 0 && publishJob > manifestJob);
  assert.match(releaseWorkflow.slice(manifestJob, publishJob), /id-token:\s*write/);
  assert.doesNotMatch(releaseWorkflow, /pull_request_target/);

  assert.doesNotMatch(architecture, /Ed25519-signed update manifest/);
  assert.doesNotMatch(development, /signed over their exact bytes with Ed25519/);
});
