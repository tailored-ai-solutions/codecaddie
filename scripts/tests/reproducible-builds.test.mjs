import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  assertPeArchitecture,
  assertMatchingEvidence,
  assertMatchingPayloads,
  createCaptureEvidence,
  normalizePeBuildIdentity,
  zeroMachOUuid,
} from "../verify-reproducible-build.mjs";

const root = new URL("../../", import.meta.url);
const read = (relative) => readFile(new URL(relative, root), "utf8");

test("normalized payload comparison accepts identical bytes and rejects drift", () => {
  const first = {
    normalizedDigest: "a".repeat(64),
    files: [{ path: "bin/app", bytes: 3, sha256: "b".repeat(64) }],
  };
  assert.doesNotThrow(() => assertMatchingPayloads(first, structuredClone(first)));
  assert.throws(
    () =>
      assertMatchingPayloads(first, {
        normalizedDigest: "c".repeat(64),
        files: [{ path: "bin/app", bytes: 3, sha256: "d".repeat(64) }],
      }),
    /independent build payload changed: bin\/app/,
  );
  assert.throws(
    () =>
      assertMatchingPayloads(first, {
        normalizedDigest: first.normalizedDigest,
        files: [{ path: "bin/app", bytes: 4, sha256: "b".repeat(64) }],
      }),
    /independent build payload changed: bin\/app/,
  );
});

function windowsPayload(seed = "a") {
  const files = [
    "target/release/codecaddie-core.exe",
    "target/release/codecaddie-updater.exe",
    "apps/desktop/zig-out/bin/codecaddie.exe",
  ]
    .sort((left, right) => left.localeCompare(right))
    .map((path, index) => ({
      path,
      bytes: index + 1,
      sha256: String.fromCharCode(seed.charCodeAt(0) + index).repeat(64),
    }));
  return {
    files,
    normalizedDigest: createHash("sha256")
      .update(files.map((entry) => `${entry.path}\0${entry.bytes}\0${entry.sha256}\n`).join(""))
      .digest("hex"),
  };
}

test("cross-job evidence comparison binds commit, target, and exact file manifest", () => {
  const expected = {
    commit: "1".repeat(40),
    platform: "windows",
    architecture: "x64",
  };
  const primary = createCaptureEvidence({ ...expected, payload: windowsPayload() });
  const independent = structuredClone(primary);
  const result = assertMatchingEvidence(primary, independent, expected);
  assert.equal(result.firstDigest, primary.normalizedDigest);
  assert.equal(result.secondDigest, independent.normalizedDigest);
  assert.deepEqual(result.files, primary.files);

  assert.throws(
    () => assertMatchingEvidence(primary, independent, { ...expected, commit: "2".repeat(40) }),
    /commit does not match/,
  );
  const changed = createCaptureEvidence({ ...expected, payload: windowsPayload("d") });
  assert.throws(
    () => assertMatchingEvidence(primary, changed, expected),
    /independent build payload changed/,
  );
  const sourceBearing = structuredClone(independent);
  sourceBearing.files[0].sourceText = "must never enter retained evidence";
  assert.throws(
    () => assertMatchingEvidence(primary, sourceBearing, expected),
    /evidence file fields are invalid/,
  );
});

test("Mach-O normalization removes only the linker UUID payload", () => {
  const binary = Buffer.alloc(56, 0xaa);
  binary.writeUInt32LE(0xfeedfacf, 0);
  binary.writeUInt32LE(1, 16);
  binary.writeUInt32LE(24, 20);
  binary.writeUInt32LE(0x1b, 32);
  binary.writeUInt32LE(24, 36);
  const normalized = zeroMachOUuid(binary);
  assert.deepEqual(normalized.subarray(40, 56), Buffer.alloc(16));
  assert.deepEqual(normalized.subarray(0, 40), binary.subarray(0, 40));
  assert.throws(() => zeroMachOUuid(Buffer.alloc(32)), /Mach-O/);
});

function peFixture({
  timestamp,
  checksum,
  debugTimestamp,
  debugByte,
  runtimeByte,
  machine = 0x8664,
}) {
  const binary = Buffer.alloc(0x400);
  binary.writeUInt16LE(0x5a4d, 0);
  binary.writeUInt32LE(0x80, 0x3c);
  binary.writeUInt32LE(0x00004550, 0x80);
  const coff = 0x84;
  binary.writeUInt16LE(machine, coff);
  binary.writeUInt16LE(1, coff + 2);
  binary.writeUInt32LE(timestamp, coff + 4);
  binary.writeUInt16LE(0xf0, coff + 16);
  const optional = coff + 20;
  binary.writeUInt16LE(0x20b, optional);
  binary.writeUInt32LE(0x400, optional + 60);
  binary.writeUInt32LE(checksum, optional + 64);
  binary.writeUInt32LE(16, optional + 108);
  const directories = optional + 112;
  binary.writeUInt32LE(0x1000, directories + 6 * 8);
  binary.writeUInt32LE(28, directories + 6 * 8 + 4);
  const section = optional + 0xf0;
  binary.write(".rdata", section, "ascii");
  binary.writeUInt32LE(0x200, section + 8);
  binary.writeUInt32LE(0x1000, section + 12);
  binary.writeUInt32LE(0x200, section + 16);
  binary.writeUInt32LE(0x200, section + 20);
  const debug = 0x200;
  binary.writeUInt32LE(debugTimestamp, debug + 4);
  binary.writeUInt32LE(2, debug + 12);
  binary.writeUInt32LE(32, debug + 16);
  binary.writeUInt32LE(0x1020, debug + 20);
  binary.writeUInt32LE(0x220, debug + 24);
  binary.fill(debugByte, 0x220, 0x240);
  binary[0x260] = runtimeByte;
  return binary;
}

test("PE normalization removes build identity but preserves runtime payload", () => {
  const first = peFixture({
    timestamp: 1,
    checksum: 2,
    debugTimestamp: 3,
    debugByte: 4,
    runtimeByte: 0x5a,
  });
  const second = peFixture({
    timestamp: 11,
    checksum: 12,
    debugTimestamp: 13,
    debugByte: 14,
    runtimeByte: 0x5a,
  });
  const normalized = normalizePeBuildIdentity(first);
  assert.deepEqual(normalized, normalizePeBuildIdentity(second));
  assert.equal(normalized[0x260], 0x5a);
  assert.equal(first.readUInt32LE(0x88), 1, "input bytes must remain untouched");
  assert.throws(() => normalizePeBuildIdentity(Buffer.alloc(64)), /PE executable/);
});

test("PE normalization still rejects loadable-code drift", () => {
  const first = peFixture({ timestamp: 1, checksum: 2, debugTimestamp: 3, debugByte: 4, runtimeByte: 5 });
  const second = peFixture({ timestamp: 1, checksum: 2, debugTimestamp: 3, debugByte: 4, runtimeByte: 6 });
  assert.notDeepEqual(normalizePeBuildIdentity(first), normalizePeBuildIdentity(second));
});

test("PE capture rejects a machine type that does not match the requested architecture", () => {
  const x64 = peFixture({ timestamp: 1, checksum: 2, debugTimestamp: 3, debugByte: 4, runtimeByte: 5 });
  const arm64 = peFixture({
    timestamp: 1,
    checksum: 2,
    debugTimestamp: 3,
    debugByte: 4,
    runtimeByte: 5,
    machine: 0xaa64,
  });
  assert.doesNotThrow(() => assertPeArchitecture(x64, "x64"));
  assert.doesNotThrow(() => assertPeArchitecture(arm64, "arm64"));
  assert.throws(() => assertPeArchitecture(arm64, "x64"), /does not match x64/);
  assert.throws(() => assertPeArchitecture(x64, "arm64"), /does not match arm64/);
});

test("cross-platform CI independently rebuilds and retains comparison evidence", async () => {
  const workflow = await read(".github/workflows/ci.yml");
  const release = await read(".github/workflows/release.yml");
  const cargo = await read("Cargo.toml");
  assert.equal((workflow.match(/verify-reproducible-build\.mjs/g) || []).length, 4);
  for (const required of [
    "Independently reproduce macOS build payload",
    "Capture primary Windows build payload",
    "Capture independent Windows build payload",
    "Compare exact-commit Windows payloads",
    "reproducibility-macos-${{ matrix.architecture }}",
    "reproducibility-windows-x64",
  ]) {
    assert.ok(workflow.includes(required), `missing reproducibility gate: ${required}`);
  }
  assert.ok(release.includes("Verify exact-commit CI release gates"));
  assert.ok(release.includes("config/reliability-gates.json"));
  assert.match(cargo, /\[profile\.release\]\nlto = "off"\ncodegen-units = 1\nstrip = "symbols"/);
  const invocations = workflow.match(
    /uses: dtolnay\/rust-toolchain@[0-9a-f]{40}(?:\n {8}with:\n(?: {10}[^\n]+\n)*)?/g,
  ) ?? [];
  assert.ok(invocations.length > 0, "expected Rust toolchain actions in CI");
  for (const invocation of invocations) {
    assert.match(invocation, /toolchain: 1\.95\.0/, "Rust action must match rust-toolchain.toml");
  }
  assert.doesNotMatch(release, /dtolnay\/rust-toolchain/);
  assert.match(release, /Import exact-commit notarized universal macOS application/);
  assert.match(release, /xcode-cloud-provenance\.json/);
});

test("Windows reproducibility captures one app build on each isolated runner and compares metadata", async () => {
  const workflow = await read(".github/workflows/ci.yml");
  const script = await read("scripts/verify-reproducible-build.mjs");
  const windowsPackager = await read("scripts/package-windows.ps1");
  const desktopBuild = await read("apps/desktop/build.zig");
  const primary = workflow.slice(
    workflow.indexOf("  windows-native-primary:"),
    workflow.indexOf("  windows-native-independent:"),
  );
  const independent = workflow.slice(
    workflow.indexOf("  windows-native-independent:"),
    workflow.indexOf("  windows-native:"),
  );
  const comparison = workflow.slice(workflow.indexOf("  windows-native:"));

  for (const build of [primary, independent]) {
    assert.match(build, /ref: \$\{\{ github\.sha \}\}/);
    assert.match(build, /fetch-depth: 0/);
    assert.match(build, /use-cache: false/);
    assert.match(build, /CODECADDIE_COMMIT_SHA=\$commit/);
    assert.match(build, /CODECADDIE_BUILD_NUMBER=\$\(git rev-list --count HEAD\)/);
    assert.equal(
      (build.match(/zig build --summary all -Doptimize=ReleaseFast -Dchannel=dev -Dtrace=off -Dstrip=true -Dcpu=baseline -j1/g) || [])
        .length,
      1,
    );
    assert.equal(
      (build.match(/zig build test --summary all -Dchannel=dev -Dcpu=baseline -j1/g) || []).length,
      1,
      "each isolated runner must serialize the baseline-CPU cache warm-up",
    );
    assert.match(build, /name: Disable 8\.3 short-name creation so embedded source paths are deterministic/);
    assert.match(build, /fsutil 8dot3name set C: 1\n\s+fsutil 8dot3name set D: 1/);
    assert.ok(
      build.indexOf("fsutil 8dot3name set") < build.indexOf("actions/checkout@"),
      "8.3 short-name creation must be disabled before any checkout, toolchain, or dependency step",
    );
    assert.match(build, /name: Isolate the Windows Zig warm-up cache/);
    assert.match(build, /ZIG_LOCAL_CACHE_DIR=\$warmLocal/);
    assert.match(build, /name: Reset the (?:primary|independent) release-local Zig graph/);
    assert.match(build, /Remove-Item -Recurse -Force "apps\\desktop\\zig-out"/);
    assert.match(build, /ZIG_LOCAL_CACHE_DIR=\$releaseLocal/);
    assert.ok(
      build.indexOf("zig build test") < build.indexOf("release-local Zig graph") &&
        build.indexOf("release-local Zig graph") < build.indexOf("native build graph"),
      "each build must warm globally and then link through a fresh local graph",
    );
    assert.match(build, /--mode capture/);
    assert.match(build, /path: dist\/reproducibility\/windows-x64-(?:primary|independent)\.json/);
  }
  assert.equal((primary.match(/package-windows\.ps1/g) || []).length, 1);
  assert.match(primary, /package-windows\.ps1 -Version 0\.3\.0-dev -Channel dev -UseExistingBuild/);
  assert.equal(
    (primary.match(/cargo build --release --package codecaddie-core --locked/g) || []).length,
    1,
  );
  assert.doesNotMatch(primary, /pnpm exec native build/);
  assert.doesNotMatch(windowsPackager, /pnpm exec native (?:test|build)/);
  assert.equal(
    (windowsPackager.match(/& \$Zig build test --summary all[^\n]+-Dcpu=baseline -j1/g) || []).length,
    1,
  );
  assert.equal(
    (windowsPackager.match(/& \$Zig build --summary all -Doptimize=ReleaseFast[^\n]+-Dstrip=true[^\n]+-Dcpu=baseline -j1/g) || [])
      .length,
    1,
  );
  assert.match(desktopBuild, /b\.option\(bool, "strip"/);
  assert.match(desktopBuild, /artifacts\.exe\.root_module\.strip = strip_release/);
  assert.match(desktopBuild, /if \(strip_release\) \{/);
  assert.match(desktopBuild, /artifacts\.install\.pdb_dir = null/);
  assert.match(desktopBuild, /artifacts\.install\.emitted_pdb = null/);
  assert.ok(
    primary.indexOf("Capture primary Windows build payload") <
      primary.indexOf("Package captured exact-commit Windows build"),
    "both isolated jobs must capture immediately after the equivalent release link",
  );
  assert.match(windowsPackager, /if \(-not \$UseExistingBuild\)/);
  assert.match(windowsPackager, /UseExistingBuild is restricted to dev packaging on CI/);
  assert.match(windowsPackager, /existing Windows build is missing/);
  assert.match(windowsPackager, /Native SDK failed to package the Windows application/);
  assert.match(windowsPackager, /Get-FileHash -Algorithm SHA256/);
  assert.match(windowsPackager, /Native SDK packaging modified an existing Windows build/);
  assert.match(primary, /Exercise exact-commit installed journey/);
  assert.ok(
    primary.indexOf("Capture primary Windows build payload") <
      primary.indexOf("cargo test --workspace --locked") &&
      primary.indexOf("cargo test --workspace --locked") <
        primary.indexOf("native check apps/desktop --strict"),
    "primary-only gates must not perturb the captured release graph",
  );
  assert.doesNotMatch(independent, /pnpm exec native build/);
  assert.doesNotMatch(independent, /cargo test --workspace|package-windows/);

  assert.match(comparison, /name: Windows native \(x64\)/);
  assert.match(comparison, /needs: \[windows-native-primary, windows-native-independent\]/);
  assert.match(comparison, /if: \$\{\{ always\(\) \}\}/);
  assert.match(
    comparison,
    /PRIMARY_RESULT: \$\{\{ needs\.windows-native-primary\.result \}\}[\s\S]*INDEPENDENT_RESULT" = success/,
  );
  assert.match(
    comparison,
    /INDEPENDENT_RESULT: \$\{\{ needs\.windows-native-independent\.result \}\}[\s\S]*PRIMARY_RESULT" = success/,
  );
  assert.equal(
    (comparison.match(/actions\/download-artifact@[0-9a-f]{40}/g) || [])
      .length,
    2,
  );
  for (const required of [
    "--mode compare",
    '--expected-commit "${{ github.sha }}"',
    "--first dist/reproducibility/primary/windows-x64-primary.json",
    "--second dist/reproducibility/independent/windows-x64-independent.json",
    "path: dist/reproducibility/windows-x64.json",
  ]) {
    assert.ok(comparison.includes(required), `missing final Windows comparison binding: ${required}`);
  }
  assert.doesNotMatch(workflow, /native build[^\n]*-j1/);

  const capture = script.slice(
    script.indexOf("async function captureCurrentPayload"),
    script.indexOf("async function isolatedBuild"),
  );
  assert.doesNotMatch(capture, /run\(|cargo|pnpm|native.{0,10}build/i);
  const isolated = script.slice(script.indexOf("async function isolatedBuild"));
  assert.match(isolated, /"-Dstrip=true"/);
  assert.match(isolated, /if \(platform === "windows"\) nativeBuildArguments\.push\("-Dcpu=baseline"\)/);
});

test("reproducibility evidence contains metadata only", async () => {
  const script = await read("scripts/verify-reproducible-build.mjs");
  for (const required of [
    'process.platform === "darwin"',
    '"macos"',
    "firstDigest",
    "secondDigest",
    "unsigned-payload-v2-macos-native-strip-local-symbols",
    "unsigned-payload-v2-windows-pe-build-identity",
    "sha256",
  ]) {
    assert.ok(script.includes(required));
  }
  for (const forbidden of ["sourceText", "sourceExcerpt", "keychain", "credentialManager"]) {
    assert.equal(script.toLowerCase().includes(forbidden.toLowerCase()), false);
  }
});

test("macOS strips the intact link-edit segment before removing its signature", async () => {
  const script = await read("scripts/verify-reproducible-build.mjs");
  const normalizer = script.slice(
    script.indexOf("async function normalizeMacNativeExecutable"),
    script.indexOf("async function main"),
  );
  const strip = normalizer.indexOf('run("strip"');
  const removeSignature = normalizer.indexOf('run("codesign", ["--remove-signature"');
  const uuid = normalizer.indexOf("zeroMachOUuid");
  assert.ok(strip >= 0, "missing symbol normalization");
  assert.ok(strip < removeSignature, "strip must see the intact LC_CODE_SIGNATURE layout");
  assert.ok(removeSignature < uuid, "UUID normalization must operate on the unsigned payload");
});
