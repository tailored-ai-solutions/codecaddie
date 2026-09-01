import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { fileURLToPath } from "node:url";

import { parseArguments, verifyPublicRoot } from "../verify-public-root.mjs";

const verifierPath = fileURLToPath(new URL("../verify-public-root.mjs", import.meta.url));
const repositoryRoot = path.resolve(path.dirname(verifierPath), "..");

function git(root, args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
}

async function publicFixture(t, extraFiles = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "codecaddie-public-root-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  git(root, ["init", "--quiet", "--initial-branch=main"]);
  git(root, ["config", "core.logAllRefUpdates", "false"]);
  git(root, ["config", "user.name", "Alex Go"]);
  git(root, ["config", "user.email", "138817+alexgo@users.noreply.github.com"]);
  await mkdir(path.join(root, "config"), { recursive: true });
  await mkdir(path.join(root, "docs"), { recursive: true });
  await writeFile(path.join(root, "README.md"), "# CodeCaddie\n");
  await writeFile(
    path.join(root, "config", "release-trust.json"),
    `${JSON.stringify({ sigstore: { repositoryId: "123456789" } }, null, 2)}\n`,
  );
  for (const [relative, contents] of Object.entries(extraFiles)) {
    await mkdir(path.dirname(path.join(root, relative)), { recursive: true });
    await writeFile(path.join(root, relative), contents);
  }
  git(root, ["add", "README.md", "config/release-trust.json", ...Object.keys(extraFiles)]);
  git(root, ["commit", "--quiet", "--signoff", "-m", "Open source CodeCaddie snapshot"]);
  return root;
}

test("one independent parentless main commit passes the public-root audit", async (t) => {
  const root = await publicFixture(t);
  const result = verifyPublicRoot(root, 1);
  assert.equal(result.head, result.rootCommit);
  assert.equal(result.commitCount, 1);
});

test("an unexpected second commit fails the one-commit audit", async (t) => {
  const root = await publicFixture(t);
  await writeFile(path.join(root, "README.md"), "# CodeCaddie\n\nSecond commit.\n");
  git(root, ["add", "README.md"]);
  git(root, ["commit", "--quiet", "-m", "second"]);
  assert.throws(() => verifyPublicRoot(root, 1), /unexpected commit count/);
});

test("a self-improvement diary in the root tree fails the public audit", async (t) => {
  const root = await publicFixture(t);
  await mkdir(path.join(root, "docs", "self-improvement"), { recursive: true });
  await writeFile(path.join(root, "docs", "self-improvement", "JOURNAL.md"), "private\n");
  git(root, ["add", "."]);
  git(root, ["commit", "--quiet", "--amend", "--no-edit"]);
  assert.throws(() => verifyPublicRoot(root, 1));
});

test("the staging repository-ID sentinel fails the public-root audit", async (t) => {
  const root = await publicFixture(t);
  await writeFile(
    path.join(root, "config", "release-trust.json"),
    `${JSON.stringify({
      sigstore: { repositoryId: "REPLACE_WITH_NEW_PUBLIC_REPOSITORY_ID" },
    }, null, 2)}\n`,
  );
  git(root, ["add", "config/release-trust.json"]);
  git(root, ["commit", "--quiet", "--amend", "--no-edit"]);
  assert.throws(
    () => verifyPublicRoot(root, 1),
    /public root must pin the new public repository numeric ID/,
  );
});

test("command-line flags select the staging root and expected commit count", async (t) => {
  const root = await publicFixture(t);
  assert.deepEqual(parseArguments([]), { root: repositoryRoot, expectedCommits: 1 });
  assert.deepEqual(parseArguments(["2"]), { root: repositoryRoot, expectedCommits: 2 });
  assert.deepEqual(parseArguments(["--root", root, "--expected-commits", "3"]), {
    root,
    expectedCommits: 3,
  });
  assert.deepEqual(parseArguments([`--root=${root}`, "--expected-commits=4"]), {
    root,
    expectedCommits: 4,
  });
  assert.throws(() => parseArguments(["--root"]), /--root requires a value/);
  assert.throws(() => parseArguments(["--expected-commits", "0"]), /positive integer/);
  assert.throws(() => parseArguments(["--unknown"]), /unknown option --unknown/);
  assert.throws(() => parseArguments(["1", "2"]), /unexpected arguments/);

  const run = execFileSync(
    process.execPath,
    [verifierPath, "--root", root, "--expected-commits", "1"],
    { encoding: "utf8", env: { ...process.env, CODECADDIE_PRIVATE_PATTERN_FILE: "" } },
  );
  assert.match(run, /clean public repository verified: root [0-9a-f]{40}; HEAD [0-9a-f]{40}; commits 1/);
});

test("an object path that names a private identifier fails the public-root audit", async (t) => {
  const root = await publicFixture(t, {
    "testdata/synthetic-private-marker-2044/notes.txt": "the body names nothing private\n",
  });
  const patternFile = path.join(root, "..", `${path.basename(root)}-patterns.local`);
  t.after(() => rm(patternFile, { force: true }));
  await writeFile(patternFile, "# pasted with CRLF endings\r\nsynthetic-private-marker-[0-9]+\r\n");

  assert.throws(
    () => verifyPublicRoot(root, 1, { privatePatternFile: patternFile }),
    /public object path matched private pattern file line 2:\ntestdata\/synthetic-private-marker-2044\ntestdata\/synthetic-private-marker-2044\/notes\.txt/,
  );
  assert.throws(
    () => verifyPublicRoot(root, 1, { environment: { CODECADDIE_PRIVATE_PATTERN_FILE: patternFile } }),
    /public object path matched private pattern file line 2/,
  );
  const clean = verifyPublicRoot(root, 1, { privatePatternFile: null });
  assert.equal(clean.privatePatternFile, null);
  assert.equal(clean.scannedObjectPaths, null);
});

test("the object-path scan reports how many paths the denylist covered", async (t) => {
  const root = await publicFixture(t);
  const patternFile = path.join(root, "..", `${path.basename(root)}-patterns.local`);
  t.after(() => rm(patternFile, { force: true }));
  await writeFile(patternFile, "synthetic-private-marker-[0-9]+\n");
  const result = verifyPublicRoot(root, 1, { privatePatternFile: patternFile });
  assert.equal(result.privatePatternFile, patternFile);
  // README.md, config, config/release-trust.json
  assert.equal(result.scannedObjectPaths, 3);
  assert.throws(
    () => verifyPublicRoot(root, 1, { environment: { CODECADDIE_PRIVATE_PATTERN_FILE: `${patternFile}.missing` } }),
    /CODECADDIE_PRIVATE_PATTERN_FILE does not exist/,
  );
});

test("a required private denylist that is absent fails the public-root audit closed", async (t) => {
  const root = await publicFixture(t);
  assert.throws(
    () => verifyPublicRoot(root, 1, {
      privatePatternFile: null,
      environment: { CODECADDIE_REQUIRE_PRIVATE_PATTERNS: "1" },
    }),
    /required private pattern file is missing/,
  );
  assert.throws(
    () => verifyPublicRoot(root, 1, {
      privatePatternFile: null,
      environment: { CODECADDIE_REQUIRE_PRIVATE_PATTERNS: "maybe" },
    }),
    /must be 0 or 1/,
  );
});
