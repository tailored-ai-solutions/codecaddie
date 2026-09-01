#!/usr/bin/env node

import { createHash } from "node:crypto";
import { copyFile, lstat, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

import { assertNoCredentialStoreEntrypoints } from "./check-native-credential-boundary.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));

export async function payloadManifest(files) {
  const entries = [];
  const descriptors = files
    .map((file) =>
      typeof file === "string" ? { path: file, absolute: path.join(root, file) } : file,
    )
    .sort((left, right) => left.path.localeCompare(right.path));
  for (const descriptor of descriptors) {
    const relative = descriptor.path;
    const absolute = descriptor.absolute;
    const metadata = await lstat(absolute);
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      throw new Error(`reproducibility input must be a regular file: ${relative}`);
    }
    const bytes = await readFile(absolute);
    entries.push({
      path: relative.split(path.sep).join("/"),
      bytes: bytes.length,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    });
  }
  return {
    files: entries,
    normalizedDigest: manifestDigest(entries),
  };
}

function manifestDigest(entries) {
  return createHash("sha256")
    .update(entries.map((entry) => `${entry.path}\0${entry.bytes}\0${entry.sha256}\n`).join(""))
    .digest("hex");
}

export function assertMatchingPayloads(first, second) {
  const identicalFiles = JSON.stringify(first.files) === JSON.stringify(second.files);
  if (first.normalizedDigest !== second.normalizedDigest || !identicalFiles) {
    const firstByPath = new Map(first.files.map((entry) => [entry.path, entry.sha256]));
    const changed = second.files
      .filter((entry) => {
        const firstEntry = first.files.find((candidate) => candidate.path === entry.path);
        return (
          firstByPath.get(entry.path) !== entry.sha256 ||
          firstEntry?.bytes !== entry.bytes
        );
      })
      .map((entry) => entry.path);
    throw new Error(`independent build payload changed: ${changed.join(", ") || "file set"}`);
  }
}

const NORMALIZATION_BY_PLATFORM = Object.freeze({
  macos: "unsigned-payload-v2-macos-native-strip-local-symbols",
  windows: "unsigned-payload-v2-windows-pe-build-identity",
});

function releasePayloadPaths(platform) {
  const extension = platform === "windows" ? ".exe" : "";
  return [
    `target/release/codecaddie-core${extension}`,
    `target/release/codecaddie-updater${extension}`,
    `apps/desktop/zig-out/bin/codecaddie${extension}`,
  ];
}

function validateEvidence(evidence, label) {
  if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) {
    throw new Error(`${label} evidence must be an object`);
  }
  const expectedKeys = [
    "architecture",
    "commit",
    "files",
    "normalization",
    "normalizedDigest",
    "platform",
    "schemaVersion",
  ];
  if (JSON.stringify(Object.keys(evidence).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error(`${label} evidence fields are invalid`);
  }
  if (evidence.schemaVersion !== 1) throw new Error(`${label} evidence schema is unsupported`);
  if (!/^[0-9a-f]{40}$/.test(evidence.commit ?? "")) {
    throw new Error(`${label} evidence commit is invalid`);
  }
  if (!Object.hasOwn(NORMALIZATION_BY_PLATFORM, evidence.platform)) {
    throw new Error(`${label} evidence platform is invalid`);
  }
  if (!["x64", "arm64"].includes(evidence.architecture)) {
    throw new Error(`${label} evidence architecture is invalid`);
  }
  if (evidence.normalization !== NORMALIZATION_BY_PLATFORM[evidence.platform]) {
    throw new Error(`${label} evidence normalization is invalid`);
  }
  if (!Array.isArray(evidence.files)) throw new Error(`${label} evidence files are invalid`);
  const expectedPaths = releasePayloadPaths(evidence.platform).sort((left, right) =>
    left.localeCompare(right),
  );
  const actualPaths = evidence.files.map((entry) => entry?.path);
  if (JSON.stringify(actualPaths) !== JSON.stringify(expectedPaths)) {
    throw new Error(`${label} evidence file manifest is invalid`);
  }
  for (const entry of evidence.files) {
    if (JSON.stringify(Object.keys(entry).sort()) !== JSON.stringify(["bytes", "path", "sha256"])) {
      throw new Error(`${label} evidence file fields are invalid: ${entry.path}`);
    }
    if (!Number.isSafeInteger(entry.bytes) || entry.bytes < 0) {
      throw new Error(`${label} evidence file size is invalid: ${entry.path}`);
    }
    if (!/^[0-9a-f]{64}$/.test(entry.sha256 ?? "")) {
      throw new Error(`${label} evidence file digest is invalid: ${entry.path}`);
    }
  }
  if (evidence.normalizedDigest !== manifestDigest(evidence.files)) {
    throw new Error(`${label} evidence normalized digest is invalid`);
  }
}

export function createCaptureEvidence({ commit, platform, architecture, payload }) {
  const evidence = {
    schemaVersion: 1,
    commit,
    platform,
    architecture,
    normalization: NORMALIZATION_BY_PLATFORM[platform],
    normalizedDigest: payload.normalizedDigest,
    files: payload.files,
  };
  validateEvidence(evidence, "captured");
  return evidence;
}

export function assertMatchingEvidence(first, second, expected) {
  validateEvidence(first, "primary");
  validateEvidence(second, "independent");
  for (const field of ["commit", "platform", "architecture"]) {
    if (first[field] !== expected[field] || second[field] !== expected[field]) {
      throw new Error(`${field} does not match the requested reproducibility context`);
    }
  }
  if (first.normalization !== second.normalization) {
    throw new Error("independent build normalization changed");
  }
  assertMatchingPayloads(first, second);
  return {
    schemaVersion: 1,
    commit: expected.commit,
    platform: expected.platform,
    architecture: expected.architecture,
    normalization: first.normalization,
    normalizedDigest: first.normalizedDigest,
    firstDigest: first.normalizedDigest,
    secondDigest: second.normalizedDigest,
    files: first.files,
  };
}

export function zeroMachOUuid(bytes) {
  const normalized = Buffer.from(bytes);
  if (normalized.length < 32 || normalized.readUInt32LE(0) !== 0xfeedfacf) {
    throw new Error("expected a 64-bit little-endian Mach-O executable");
  }
  const commandCount = normalized.readUInt32LE(16);
  let cursor = 32;
  for (let index = 0; index < commandCount; index += 1) {
    if (cursor + 8 > normalized.length) throw new Error("truncated Mach-O load commands");
    const command = normalized.readUInt32LE(cursor);
    const commandSize = normalized.readUInt32LE(cursor + 4);
    if (commandSize < 8 || cursor + commandSize > normalized.length) {
      throw new Error("invalid Mach-O load command size");
    }
    if (command === 0x1b) {
      if (commandSize < 24) throw new Error("invalid Mach-O UUID command");
      normalized.fill(0, cursor + 8, cursor + 24);
    }
    cursor += commandSize;
  }
  return normalized;
}

function peBounds(buffer, offset, size, label) {
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(size) || offset < 0 || size < 0 || offset + size > buffer.length) {
    throw new Error(`invalid PE ${label}`);
  }
}

/**
 * Returns the structural offsets needed to normalize documented PE/COFF
 * build identity without touching loadable code or application data.
 */
function peLayout(buffer) {
  peBounds(buffer, 0, 0x40, "DOS header");
  if (buffer.readUInt16LE(0) !== 0x5a4d) throw new Error("expected a PE executable");
  const peOffset = buffer.readUInt32LE(0x3c);
  peBounds(buffer, peOffset, 24, "signature and COFF header");
  if (buffer.readUInt32LE(peOffset) !== 0x00004550) throw new Error("expected a PE signature");
  const coffOffset = peOffset + 4;
  const machine = buffer.readUInt16LE(coffOffset);
  const sectionCount = buffer.readUInt16LE(coffOffset + 2);
  if (sectionCount === 0 || sectionCount > 96) throw new Error("invalid PE section count");
  const optionalSize = buffer.readUInt16LE(coffOffset + 16);
  const optionalOffset = coffOffset + 20;
  peBounds(buffer, optionalOffset, optionalSize, "optional header");
  const magic = buffer.readUInt16LE(optionalOffset);
  const dataDirectoryOffset = optionalOffset + (magic === 0x20b ? 112 : magic === 0x10b ? 96 : -1);
  const directoryCountOffset = optionalOffset + (magic === 0x20b ? 108 : magic === 0x10b ? 92 : -1);
  if (dataDirectoryOffset < optionalOffset || directoryCountOffset < optionalOffset) {
    throw new Error("expected a PE32 or PE32+ optional header");
  }
  peBounds(buffer, directoryCountOffset, 4, "data-directory count");
  const directoryCount = Math.min(buffer.readUInt32LE(directoryCountOffset), 16);
  const sectionTableOffset = optionalOffset + optionalSize;
  peBounds(buffer, sectionTableOffset, sectionCount * 40, "section table");
  const sections = [];
  for (let index = 0; index < sectionCount; index += 1) {
    const offset = sectionTableOffset + index * 40;
    sections.push({
      virtualSize: buffer.readUInt32LE(offset + 8),
      virtualAddress: buffer.readUInt32LE(offset + 12),
      rawSize: buffer.readUInt32LE(offset + 16),
      rawOffset: buffer.readUInt32LE(offset + 20),
    });
  }
  const sizeOfHeaders = buffer.readUInt32LE(optionalOffset + 60);
  const rvaToOffset = (rva, size = 1) => {
    if (rva < sizeOfHeaders) {
      peBounds(buffer, rva, size, "header-relative directory");
      return rva;
    }
    for (const section of sections) {
      const span = Math.max(section.virtualSize, section.rawSize);
      if (rva >= section.virtualAddress && rva - section.virtualAddress + size <= span) {
        const offset = section.rawOffset + (rva - section.virtualAddress);
        peBounds(buffer, offset, size, "section-relative directory");
        return offset;
      }
    }
    throw new Error("PE directory lies outside the image sections");
  };
  const directory = (index) => {
    if (index >= directoryCount || dataDirectoryOffset + (index + 1) * 8 > optionalOffset + optionalSize) {
      return null;
    }
    const offset = dataDirectoryOffset + index * 8;
    const rva = buffer.readUInt32LE(offset);
    const size = buffer.readUInt32LE(offset + 4);
    return rva === 0 || size === 0 ? null : { rva, size, offset: rvaToOffset(rva, size) };
  };
  return { machine, coffOffset, optionalOffset, directory, rvaToOffset };
}

export function assertPeArchitecture(bytes, architecture) {
  const expectedMachine = {
    x64: 0x8664,
    arm64: 0xaa64,
  }[architecture];
  if (expectedMachine === undefined) {
    throw new Error(`unsupported PE architecture: ${architecture}`);
  }
  const { machine } = peLayout(bytes);
  if (machine !== expectedMachine) {
    throw new Error(
      `PE machine 0x${machine.toString(16).padStart(4, "0")} does not match ${architecture}`,
    );
  }
}

function zeroResourceDirectoryTimestamps(buffer, resource) {
  const rootOffset = resource.offset;
  const visited = new Set();
  const visit = (relativeOffset, depth) => {
    if (depth > 16 || visited.size > 4096 || visited.has(relativeOffset)) return;
    visited.add(relativeOffset);
    const offset = rootOffset + relativeOffset;
    peBounds(buffer, offset, 16, "resource directory");
    buffer.fill(0, offset + 4, offset + 8);
    const entryCount = buffer.readUInt16LE(offset + 12) + buffer.readUInt16LE(offset + 14);
    peBounds(buffer, offset + 16, entryCount * 8, "resource entries");
    for (let index = 0; index < entryCount; index += 1) {
      const target = buffer.readUInt32LE(offset + 16 + index * 8 + 4);
      if ((target & 0x80000000) !== 0) visit(target & 0x7fffffff, depth + 1);
    }
  };
  visit(0, 0);
}

/**
 * Normalizes only PE/COFF fields that identify a build or its detached debug
 * symbols. The returned bytes are comparison-only; the packaged executable is
 * never rewritten.
 */
export function normalizePeBuildIdentity(bytes) {
  const normalized = Buffer.from(bytes);
  const { coffOffset, optionalOffset, directory, rvaToOffset } = peLayout(normalized);
  // COFF build timestamp and optional-header checksum are not executable
  // behavior. Deterministic linkers may derive the former from a content hash.
  normalized.fill(0, coffOffset + 4, coffOffset + 8);
  normalized.fill(0, optionalOffset + 64, optionalOffset + 68);

  const exportDirectory = directory(0);
  if (exportDirectory && exportDirectory.size >= 8) normalized.fill(0, exportDirectory.offset + 4, exportDirectory.offset + 8);

  const importDirectory = directory(1);
  if (importDirectory) {
    for (let offset = importDirectory.offset; offset + 20 <= importDirectory.offset + importDirectory.size; offset += 20) {
      const empty = normalized.subarray(offset, offset + 20).every((value) => value === 0);
      if (empty) break;
      normalized.fill(0, offset + 4, offset + 8);
    }
  }

  const resourceDirectory = directory(2);
  if (resourceDirectory) zeroResourceDirectoryTimestamps(normalized, resourceDirectory);

  const debugDirectory = directory(6);
  if (debugDirectory) {
    if (debugDirectory.size % 28 !== 0) throw new Error("invalid PE debug directory size");
    for (let offset = debugDirectory.offset; offset < debugDirectory.offset + debugDirectory.size; offset += 28) {
      normalized.fill(0, offset + 4, offset + 8);
      const dataSize = normalized.readUInt32LE(offset + 16);
      const dataRva = normalized.readUInt32LE(offset + 20);
      const dataPointer = normalized.readUInt32LE(offset + 24);
      if (dataSize === 0) continue;
      const dataOffset = dataPointer === 0 ? rvaToOffset(dataRva, dataSize) : dataPointer;
      peBounds(normalized, dataOffset, dataSize, "debug data");
      normalized.fill(0, dataOffset, dataOffset + dataSize);
    }
  }

  const loadConfigDirectory = directory(10);
  if (loadConfigDirectory && loadConfigDirectory.size >= 8) normalized.fill(0, loadConfigDirectory.offset + 4, loadConfigDirectory.offset + 8);

  const delayImportDirectory = directory(13);
  if (delayImportDirectory) {
    for (let offset = delayImportDirectory.offset; offset + 32 <= delayImportDirectory.offset + delayImportDirectory.size; offset += 32) {
      const empty = normalized.subarray(offset, offset + 32).every((value) => value === 0);
      if (empty) break;
      normalized.fill(0, offset + 28, offset + 32);
    }
  }
  return normalized;
}

function option(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) throw new Error(`${name} is required`);
  return process.argv[index + 1];
}

function optionalOption(name) {
  const index = process.argv.indexOf(name);
  return index < 0 ? undefined : process.argv[index + 1];
}

function run(command, args, environment = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    env: { ...process.env, ...environment },
    shell: process.platform === "win32",
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with status ${result.status}`);
}

async function normalizeMacNativeExecutable(source, destination) {
  await mkdir(path.dirname(destination), { recursive: true });
  await copyFile(source, destination);
  // Strip while LC_CODE_SIGNATURE still describes the link-edit segment. Some
  // Apple strip versions reject Zig's arm64 layout after codesign has already
  // compacted that segment. The final payload remains unsigned either way.
  run("strip", ["-S", "-x", destination]);
  run("codesign", ["--remove-signature", destination]);
  await writeFile(destination, zeroMachOUuid(await readFile(destination)));
}

async function normalizeWindowsExecutable(source, destination, architecture) {
  await mkdir(path.dirname(destination), { recursive: true });
  const bytes = await readFile(source);
  assertPeArchitecture(bytes, architecture);
  await writeFile(destination, normalizePeBuildIdentity(bytes));
}

function hostPlatform() {
  return process.platform === "win32"
    ? "windows"
    : process.platform === "darwin"
      ? "macos"
      : process.platform;
}

function assertRequestedTarget(platform, architecture, { requireHost = true } = {}) {
  if (!Object.hasOwn(NORMALIZATION_BY_PLATFORM, platform)) {
    throw new Error(`unsupported reproducibility platform: ${platform}`);
  }
  if (!["x64", "arm64"].includes(architecture)) {
    throw new Error(`unsupported reproducibility architecture: ${architecture}`);
  }
  if (requireHost && platform !== hostPlatform()) {
    throw new Error(`requested ${platform} comparison on ${hostPlatform()}`);
  }
  if (requireHost && architecture !== process.arch) {
    throw new Error(`requested ${architecture} comparison on ${process.arch}`);
  }
}

function repositoryCommit() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`git exited with status ${result.status}`);
  const commit = result.stdout.trim();
  if (!/^[0-9a-f]{40}$/.test(commit)) throw new Error("git returned an invalid commit");
  return commit;
}

function standardPayloadDescriptors(platform) {
  return releasePayloadPaths(platform).map((relative) => ({
    path: relative,
    absolute: path.join(root, relative),
  }));
}

async function normalizedBuiltPayload(platform, architecture, descriptors, workingRoot) {
  const files = descriptors.map((descriptor) => ({ ...descriptor }));
  const nativeExecutable = files[2].absolute;
  const nativeBytes = await readFile(nativeExecutable);
  let nativeImports = "";
  if (platform === "macos") {
    const nm = spawnSync("/usr/bin/nm", ["-u", nativeExecutable], { encoding: "utf8" });
    if (nm.error) throw nm.error;
    if (nm.status !== 0) throw new Error(`nm exited with status ${nm.status}`);
    nativeImports = nm.stdout;
  }
  assertNoCredentialStoreEntrypoints(nativeBytes, nativeImports);

  const normalizedRoot = path.join(workingRoot, "normalized");
  if (platform === "macos") {
    const normalizedNative = path.join(normalizedRoot, path.basename(nativeExecutable));
    await normalizeMacNativeExecutable(nativeExecutable, normalizedNative);
    files[2].absolute = normalizedNative;
  } else {
    for (const descriptor of files) {
      const normalized = path.join(normalizedRoot, path.basename(descriptor.absolute));
      await normalizeWindowsExecutable(descriptor.absolute, normalized, architecture);
      descriptor.absolute = normalized;
    }
  }
  return payloadManifest(files);
}

async function captureCurrentPayload(platform, architecture) {
  const workingRoot = path.join(
    root,
    "dist",
    `.reproducibility-capture-${platform}-${architecture}`,
  );
  await rm(workingRoot, { recursive: true, force: true });
  try {
    return await normalizedBuiltPayload(
      platform,
      architecture,
      standardPayloadDescriptors(platform),
      workingRoot,
    );
  } finally {
    await rm(workingRoot, { recursive: true, force: true });
  }
}

async function isolatedBuild(platform, architecture, workingRoot) {
  const paths = releasePayloadPaths(platform);
  const extension = platform === "windows" ? ".exe" : "";
  const isolatedTarget = path.join(workingRoot, "cargo-target");
  await rm(workingRoot, { recursive: true, force: true });
  await rm(path.join(root, paths[2]), { force: true });
  run(
    "cargo",
    ["build", "--release", "--package", "codecaddie-core", "--locked"],
    { CARGO_TARGET_DIR: isolatedTarget, CARGO_INCREMENTAL: "0" },
  );
  const pnpm = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
  const nativeBuildArguments = [
    "exec",
    "native",
    "build",
    "apps/desktop",
    "--yes",
    "-Dchannel=dev",
    "-Dtrace=off",
    "-Dstrip=true",
  ];
  if (platform === "windows") nativeBuildArguments.push("-Dcpu=baseline");
  run(
    pnpm,
    nativeBuildArguments,
    {
      ZIG_GLOBAL_CACHE_DIR: path.join(workingRoot, "zig-global-cache"),
      ZIG_LOCAL_CACHE_DIR: path.join(workingRoot, "zig-local-cache"),
    },
  );
  return normalizedBuiltPayload(
    platform,
    architecture,
    [
      {
        path: paths[0],
        absolute: path.join(isolatedTarget, "release", `codecaddie-core${extension}`),
      },
      {
        path: paths[1],
        absolute: path.join(isolatedTarget, "release", `codecaddie-updater${extension}`),
      },
      { path: paths[2], absolute: path.join(root, paths[2]) },
    ],
    workingRoot,
  );
}

async function readEvidence(filename, label) {
  const absolute = path.resolve(root, filename);
  const metadata = await lstat(absolute);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 64 * 1024) {
    throw new Error(`${label} evidence must be a small regular file`);
  }
  let evidence;
  try {
    evidence = JSON.parse(await readFile(absolute, "utf8"));
  } catch {
    throw new Error(`${label} evidence is not valid JSON`);
  }
  validateEvidence(evidence, label);
  return evidence;
}

async function writeEvidence(filename, evidence) {
  const output = path.resolve(root, filename);
  await mkdir(path.dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
}

async function main() {
  const mode = optionalOption("--mode") ?? "verify";
  const requestedPlatform = option("--platform");
  const requestedArchitecture = option("--architecture");

  if (mode === "compare") {
    assertRequestedTarget(requestedPlatform, requestedArchitecture, { requireHost: false });
    const expectedCommit = option("--expected-commit");
    const evidence = assertMatchingEvidence(
      await readEvidence(option("--first"), "primary"),
      await readEvidence(option("--second"), "independent"),
      {
        commit: expectedCommit,
        platform: requestedPlatform,
        architecture: requestedArchitecture,
      },
    );
    await writeEvidence(option("--output"), evidence);
    console.log(`verified independent payload ${evidence.normalizedDigest}`);
    return;
  }

  assertRequestedTarget(requestedPlatform, requestedArchitecture);
  if (mode === "capture") {
    const payload = await captureCurrentPayload(requestedPlatform, requestedArchitecture);
    const evidence = createCaptureEvidence({
      commit: repositoryCommit(),
      platform: requestedPlatform,
      architecture: requestedArchitecture,
      payload,
    });
    await writeEvidence(option("--output"), evidence);
    console.log(`captured payload ${evidence.normalizedDigest} (${payload.files.length} executables)`);
    return;
  }
  if (mode !== "verify") throw new Error(`unsupported reproducibility mode: ${mode}`);

  const workingRoot = path.join(
    root,
    "dist",
    `.reproducibility-build-${requestedPlatform}-${requestedArchitecture}`,
  );
  let first;
  let second;
  try {
    first = await isolatedBuild(requestedPlatform, requestedArchitecture, workingRoot);
    second = await isolatedBuild(requestedPlatform, requestedArchitecture, workingRoot);
  } finally {
    await rm(workingRoot, { recursive: true, force: true });
  }
  const expected = {
    commit: repositoryCommit(),
    platform: requestedPlatform,
    architecture: requestedArchitecture,
  };
  const evidence = assertMatchingEvidence(
    createCaptureEvidence({ ...expected, payload: first }),
    createCaptureEvidence({ ...expected, payload: second }),
    expected,
  );
  const output = path.join(
    root,
    "dist",
    "reproducibility",
    `${requestedPlatform}-${requestedArchitecture}.json`,
  );
  await writeEvidence(output, evidence);
  console.log(
    `reproducible payload ${evidence.normalizedDigest} (${evidence.files.length} executables)`,
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
