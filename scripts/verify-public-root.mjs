#!/usr/bin/env node

import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const defaultRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PRIVATE_PATTERN_FILE_NAME = path.join("scripts", "private-patterns.local");

function git(root, args, options = {}) {
  return execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  }).trim();
}

/**
 * Reads the untracked private denylist: one extended regular expression per
 * line, blank lines and `#` comments ignored, trailing carriage returns
 * stripped so a CRLF-pasted secret cannot neuter every pattern.
 */
export function readPrivatePatterns(patternFile) {
  return readFileSync(patternFile, "utf8")
    .split("\n")
    .map((line, index) => ({ line: index + 1, pattern: line.replace(/\r$/, "") }))
    .filter(({ pattern }) => pattern.trim() !== "" && !pattern.startsWith("#"));
}

/**
 * Resolves which denylist applies: an explicit environment override, the
 * staging root's own untracked file, then the file beside this script so a
 * fresh staging directory still inherits the maintainer's denylist.
 */
export function resolvePrivatePatternFile(root, environment = process.env) {
  const explicit = environment.CODECADDIE_PRIVATE_PATTERN_FILE;
  if (explicit) {
    const resolved = path.resolve(explicit);
    assert.ok(existsSync(resolved), `CODECADDIE_PRIVATE_PATTERN_FILE does not exist: ${resolved}`);
    return resolved;
  }
  for (const candidate of [
    path.join(root, PRIVATE_PATTERN_FILE_NAME),
    path.join(defaultRoot, PRIVATE_PATTERN_FILE_NAME),
  ]) {
    if (existsSync(candidate)) return candidate;
  }
  return null;
}

/**
 * Every object path in the public object graph must stay clear of the private
 * denylist. `git grep` only sees blob contents, so a file or directory named
 * after a private identifier would otherwise pass; this scans the path of
 * every tree entry reachable from any ref, not only the checked-out HEAD.
 */
export function assertObjectPathsClearPrivatePatterns(root, patternFile) {
  // Commits print as a bare object ID; trees and blobs print `<id> <path>`.
  const objectPaths = git(root, ["rev-list", "--all", "--objects"])
    .split("\n")
    .map((line) => (line.includes(" ") ? line.slice(line.indexOf(" ") + 1) : ""))
    .filter(Boolean);
  const input = `${objectPaths.join("\n")}\n`;
  for (const { line, pattern } of readPrivatePatterns(patternFile)) {
    const match = spawnSync("grep", ["-E", "-i", "--", pattern], { input, encoding: "utf8" });
    assert.notEqual(match.status, 2, `private pattern file line ${line} is not a valid extended regular expression`);
    assert.equal(
      match.status,
      1,
      `public object path matched private pattern file line ${line}:\n${match.stdout.trim()}`,
    );
  }
  return objectPaths.length;
}

export function verifyPublicRoot(root = defaultRoot, expectedCommitCount = 1, options = {}) {
  assert.ok(Number.isSafeInteger(expectedCommitCount) && expectedCommitCount > 0);
  assert.equal(git(root, ["symbolic-ref", "--short", "HEAD"]), "main");

  const commits = git(root, ["rev-list", "--all"]).split("\n").filter(Boolean);
  assert.equal(commits.length, expectedCommitCount, "unexpected commit count in public object graph");
  const roots = git(root, ["rev-list", "--max-parents=0", "--all"])
    .split("\n")
    .filter(Boolean);
  assert.equal(roots.length, 1, "public repository must have exactly one parentless root");
  assert.equal(git(root, ["show", "-s", "--format=%P", roots[0]]), "");
  assert.equal(
    git(root, ["show", "-s", "--format=%s", roots[0]]),
    "Open source CodeCaddie snapshot",
    "public root commit subject is not canonical",
  );
  assert.equal(git(root, ["show", "-s", "--format=%an", roots[0]]), "Alex Go");
  assert.equal(
    git(root, ["show", "-s", "--format=%ae", roots[0]]),
    "138817+alexgo@users.noreply.github.com",
  );
  assert.equal(git(root, ["show", "-s", "--format=%cn", roots[0]]), "Alex Go");
  assert.equal(
    git(root, ["show", "-s", "--format=%ce", roots[0]]),
    "138817+alexgo@users.noreply.github.com",
  );
  assert.match(
    git(root, ["show", "-s", "--format=%B", roots[0]]),
    /(?:^|\n)Signed-off-by: Alex Go <138817\+alexgo@users\.noreply\.github\.com>$/,
    "public root commit is missing its canonical DCO sign-off",
  );

  const refs = git(root, ["for-each-ref", "--format=%(refname)"])
    .split("\n")
    .filter(Boolean);
  assert.deepEqual(refs, ["refs/heads/main"], "public staging repository contains an unexpected ref");
  assert.equal(git(root, ["tag", "--list"]), "", "public staging repository must begin without tags");

  const releaseTrust = JSON.parse(git(root, ["show", "HEAD:config/release-trust.json"]));
  assert.match(
    releaseTrust?.sigstore?.repositoryId ?? "",
    /^[1-9]\d*$/,
    "public root must pin the new public repository numeric ID in release trust",
  );

  const gitDirValue = git(root, ["rev-parse", "--git-dir"]);
  const gitDir = path.resolve(root, gitDirValue);
  const alternates = path.join(gitDir, "objects", "info", "alternates");
  assert.ok(
    !existsSync(alternates) || readFileSync(alternates, "utf8").trim() === "",
    "public staging repository must not borrow objects through alternates",
  );
  assert.equal(git(root, ["reflog", "show", "--all", "--format=%H"]), "", "public staging repository must not retain reflogs");
  assert.equal(
    git(root, ["fsck", "--unreachable", "--no-reflogs", "--no-progress"]),
    "",
    "public staging repository contains unreachable objects",
  );

  const tracked = git(root, ["ls-tree", "-r", "--name-only", "HEAD"])
    .split("\n")
    .filter(Boolean);
  assert.ok(!tracked.some((name) => name === "docs/self-improvement" || name.startsWith("docs/self-improvement/")));
  assert.ok(!tracked.some((name) => name.startsWith("audit-evidence/")));
  assert.ok(
    !tracked.includes(PRIVATE_PATTERN_FILE_NAME.replaceAll(path.sep, "/")),
    "the private pattern file must remain untracked",
  );
  if (expectedCommitCount === 1) {
    assert.ok(
      !tracked.some((name) => name.startsWith("docs/images/")),
      "the initial public snapshot must not carry the private development screenshots",
    );
  }

  const environment = options.environment ?? process.env;
  const patternFile = options.privatePatternFile === undefined
    ? resolvePrivatePatternFile(root, environment)
    : options.privatePatternFile;
  const requirePrivatePatterns = environment.CODECADDIE_REQUIRE_PRIVATE_PATTERNS ?? "0";
  assert.ok(
    requirePrivatePatterns === "0" || requirePrivatePatterns === "1",
    "CODECADDIE_REQUIRE_PRIVATE_PATTERNS must be 0 or 1",
  );
  assert.ok(
    requirePrivatePatterns === "0" || patternFile,
    "the required private pattern file is missing; point CODECADDIE_PRIVATE_PATTERN_FILE at the denylist",
  );
  let scannedObjectPaths = null;
  if (patternFile) {
    scannedObjectPaths = assertObjectPathsClearPrivatePatterns(root, patternFile);
  }
  return {
    rootCommit: roots[0],
    head: git(root, ["rev-parse", "HEAD"]),
    commitCount: commits.length,
    privatePatternFile: patternFile ?? null,
    scannedObjectPaths,
  };
}

const USAGE = "usage: verify-public-root.mjs [--root <path>] [--expected-commits <n>] [<n>]";

export function parseArguments(argv) {
  const options = { root: defaultRoot, expectedCommits: 1 };
  const positional = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const takeValue = (name) => {
      const value = argument.startsWith(`${name}=`) ? argument.slice(name.length + 1) : argv[++index];
      if (value === undefined || value === "") throw new Error(`${name} requires a value\n${USAGE}`);
      return value;
    };
    if (argument === "--root" || argument.startsWith("--root=")) {
      options.root = path.resolve(takeValue("--root"));
    } else if (argument === "--expected-commits" || argument.startsWith("--expected-commits=")) {
      options.expectedCommits = Number(takeValue("--expected-commits"));
    } else if (argument.startsWith("-")) {
      throw new Error(`unknown option ${argument}\n${USAGE}`);
    } else {
      positional.push(argument);
    }
  }
  if (positional.length > 1) throw new Error(`unexpected arguments: ${positional.join(" ")}\n${USAGE}`);
  if (positional.length === 1) options.expectedCommits = Number(positional[0]);
  if (!Number.isSafeInteger(options.expectedCommits) || options.expectedCommits <= 0) {
    throw new Error(`expected commit count must be a positive integer\n${USAGE}`);
  }
  return options;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { root, expectedCommits } = parseArguments(process.argv.slice(2));
  const result = verifyPublicRoot(root, expectedCommits);
  if (result.privatePatternFile) {
    console.log(`private denylist applied from ${result.privatePatternFile} to ${result.scannedObjectPaths} object paths`);
  } else {
    console.error("WARNING: private denylist not applied (scripts/private-patterns.local absent); object paths were not scanned");
  }
  console.log(`clean public repository verified: root ${result.rootCommit}; HEAD ${result.head}; commits ${result.commitCount}`);
}
