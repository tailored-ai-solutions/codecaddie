#!/usr/bin/env node
/**
 * Negative grep for private source. Given a directory (a data root, an
 * evidence directory, an export folder) and the fixture repository a journey
 * analyzed, this fails if any file under the directory contains one of the
 * private source canary strings or the fixture repository's absolute path.
 *
 * The canary constants mirror `crates/codecaddie-core/src/privacy_test_support.rs`
 * and the installed-core journey; a test keeps them in sync.
 */
import assert from "node:assert/strict";
import { lstatSync, readdirSync, readFileSync, realpathSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

export const DEFAULT_CANARIES = Object.freeze([
  "REPOSITORY_PRIVATE_SENTINEL_7DB9562A",
  "ATTACHMENT_PRIVATE_SENTINEL_4F128CDE",
  "PRIVATE SOURCE CANARY",
]);

function pathVariants(target) {
  const absolute = path.resolve(target);
  const variants = new Set([absolute]);
  try {
    variants.add(realpathSync(absolute));
  } catch {
    // A fixture that no longer exists still names a path that must not leak.
  }
  return [...variants];
}

/** Maps a human-readable label to the exact bytes that must not appear. */
export function collectForbiddenBytes({ fixtureRepository, canaries = DEFAULT_CANARIES } = {}) {
  const forbidden = new Map();
  for (const canary of canaries) {
    assert.ok(typeof canary === "string" && canary.length > 0, "canary strings must be non-empty");
    forbidden.set(`canary ${JSON.stringify(canary)}`, Buffer.from(canary));
  }
  if (fixtureRepository) {
    for (const variant of pathVariants(fixtureRepository)) {
      forbidden.set(`fixture repository path ${variant}`, Buffer.from(variant));
    }
  }
  return forbidden;
}

function isWithin(candidate, roots) {
  return roots.some((root) => candidate === root || candidate.startsWith(`${root}${path.sep}`));
}

/** Yields every regular file under `directory`, skipping symlinks and `skipRoots`. */
export function* walkFiles(directory, { skipRoots = [] } = {}) {
  const pending = [directory];
  while (pending.length > 0) {
    const current = pending.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const entryPath = path.join(current, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        if (isWithin(entryPath, skipRoots) || isWithin(realpathSync(entryPath), skipRoots)) continue;
        pending.push(entryPath);
      } else if (entry.isFile()) {
        yield entryPath;
      }
    }
  }
}

/**
 * Scans every file under `directory`. When the fixture repository itself lives
 * under the directory (a journey root holding both `data/` and `repository/`),
 * the repository is skipped because it legitimately contains the canary.
 */
export function assertSourceFree({ directory, fixtureRepository, canaries = DEFAULT_CANARIES }) {
  assert.ok(directory, "a directory to scan is required");
  const scanRoot = path.resolve(directory);
  assert.ok(lstatSync(scanRoot).isDirectory(), `${scanRoot} is not a directory`);
  const forbidden = collectForbiddenBytes({ fixtureRepository, canaries });
  const skipRoots = fixtureRepository ? pathVariants(fixtureRepository) : [];
  const violations = [];
  let scannedFiles = 0;
  for (const file of walkFiles(scanRoot, { skipRoots })) {
    scannedFiles += 1;
    const bytes = readFileSync(file);
    for (const [label, marker] of forbidden) {
      if (bytes.includes(marker)) violations.push({ file, label });
    }
  }
  if (violations.length > 0) {
    const detail = violations.map(({ file, label }) => `${file}: contains ${label}`).join("\n");
    throw new assert.AssertionError({
      message: `private source escaped into ${scanRoot}:\n${detail}`,
      actual: violations.length,
      expected: 0,
      operator: "assertSourceFree",
    });
  }
  return { directory: scanRoot, scannedFiles, forbiddenMarkers: forbidden.size };
}

const USAGE = "usage: assert-source-free.mjs --directory <path> [--fixture-repo <path>] [--canary <text>]...";

export function parseArguments(argv) {
  const options = { directory: undefined, fixtureRepository: undefined, canaries: [...DEFAULT_CANARIES] };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const value = argv[index + 1];
    if (argument === "--directory" && value) {
      options.directory = value;
      index += 1;
    } else if (argument === "--fixture-repo" && value) {
      options.fixtureRepository = value;
      index += 1;
    } else if (argument === "--canary" && value) {
      options.canaries.push(value);
      index += 1;
    } else {
      throw new Error(`unexpected argument ${argument}\n${USAGE}`);
    }
  }
  if (!options.directory) throw new Error(USAGE);
  return options;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const options = parseArguments(process.argv.slice(2));
  const result = assertSourceFree(options);
  console.log(
    `source-free: scanned ${result.scannedFiles} files under ${result.directory} against ${result.forbiddenMarkers} markers`,
  );
}
