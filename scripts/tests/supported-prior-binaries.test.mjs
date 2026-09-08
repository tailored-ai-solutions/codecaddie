import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { releaseBuildNumber } from "../release-build-number.mjs";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const matrix = JSON.parse(await readFile(new URL("config/supported-upgrade-matrix.json", root), "utf8"));
const exercise = await readFile(new URL("scripts/exercise-supported-prior-binaries.mjs", root), "utf8");
const ci = await readFile(new URL(".github/workflows/ci.yml", root), "utf8");
const packageJson = JSON.parse(await readFile(new URL("package.json", root), "utf8"));

test("the first public signed release establishes the supported prior-build baseline", () => {
  assert.equal(matrix.schemaVersion, 2);
  assert.equal(matrix.firstPublicBaseline.status, "established");
  assert.equal(matrix.firstPublicBaseline.version, "0.4.0");
  assert.equal(matrix.firstPublicBaseline.build, 2015);
  assert.deepEqual(matrix.supportedPriorBuilds[0], {
    version: "0.4.0",
    build: 2015,
    sourceCommit: "2ee34e1515b76da58839c935ec1a29cb4c000df1",
    localStateFormat: "codecaddie-local-state-v3",
  });
  assert.match(exercise, /only the first public snapshot may have an empty prior-build matrix/);
});

test("established supported prior versions execute real binaries in both directions", () => {
  assert.match(exercise, /firstPublicBaseline\.status, "established"/);
  assert.match(exercise, /baseline\.sourceCommit/);
  assert.match(exercise, /git.*archive|"archive"/);
  assert.match(exercise, /requires a clean exact-commit checkout/);
  assert.match(exercise, /currentSource/);
  assert.match(exercise, /CODECADDIE_COMMIT_SHA/);
  assert.match(exercise, /exerciseInstalledCore\(output/);
  assert.match(exercise, /recordReport\(\s*priorBinary/);
  assert.match(exercise, /recordReport\(\s*currentBinary/);
  assert.match(exercise, /runCore\(priorBinary/);
  assert.match(exercise, /reportCount: 2/);
  assert.match(exercise, /assertDataRootExcludesSource/);
  assert.match(ci, /fetch-depth: 0/);
  assert.match(ci, /exercise-supported-prior-binaries\.mjs --require-clean/);
});

test("the clean-tree assertion is opt-in for CI and release checks, not the default gate", () => {
  assert.match(exercise, /if \(requireClean\) \{/);
  assert.equal(packageJson.scripts["compatibility:check"], "node scripts/exercise-supported-prior-binaries.mjs");
  assert.doesNotMatch(packageJson.scripts.check, /compatibility:check/);
  assert.match(packageJson.scripts["check:release"], /pnpm compatibility:check --require-clean/);
});

test("the candidate binary receives the canonical release build at the cargo boundary", async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), "codecaddie-prior-build-identity-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const captured = path.join(directory, "build.json");
  await writeFile(path.join(directory, "cargo"), `#!/usr/bin/env node
require("node:fs").writeFileSync(process.env.CAPTURE_BUILD, JSON.stringify({
  build: Number(process.env.CODECADDIE_BUILD_NUMBER),
  commit: process.env.CODECADDIE_COMMIT_SHA,
}));
process.exit(17);
`, { mode: 0o700 });
  const repository = fileURLToPath(root);
  const commit = spawnSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8" }).stdout.trim();
  const result = spawnSync(process.execPath, [fileURLToPath(new URL("scripts/exercise-supported-prior-binaries.mjs", root))], {
    cwd: repository,
    encoding: "utf8",
    env: { ...process.env, PATH: `${directory}${path.delimiter}${process.env.PATH}`, CAPTURE_BUILD: captured },
  });
  assert.notEqual(result.status, 0, "fixture deliberately stops before compiling");
  assert.deepEqual(JSON.parse(await readFile(captured, "utf8")), {
    build: await releaseBuildNumber(commit, repository),
    commit,
  });
});
