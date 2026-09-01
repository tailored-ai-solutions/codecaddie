import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const matrix = JSON.parse(await readFile(new URL("config/supported-upgrade-matrix.json", root), "utf8"));
const exercise = await readFile(new URL("scripts/exercise-supported-prior-binaries.mjs", root), "utf8");
const ci = await readFile(new URL(".github/workflows/ci.yml", root), "utf8");
const packageJson = JSON.parse(await readFile(new URL("package.json", root), "utf8"));

test("the first public snapshot explicitly permits an empty prior-build baseline", () => {
  assert.equal(matrix.schemaVersion, 2);
  assert.equal(matrix.firstPublicBaseline.status, "pending");
  assert.equal(matrix.firstPublicBaseline.version, "0.4.0");
  assert.equal(matrix.firstPublicBaseline.build, 2001);
  assert.deepEqual(matrix.supportedPriorBuilds, []);
  assert.match(exercise, /only the first public snapshot may have an empty prior-build matrix/);
  assert.match(exercise, /first public build 2001 has no prior public baseline/);
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
