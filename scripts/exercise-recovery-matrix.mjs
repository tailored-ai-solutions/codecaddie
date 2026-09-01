#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const matrix = JSON.parse(
  readFileSync(resolve(ROOT, "config/executable-recovery-matrix.json"), "utf8"),
);

assert.equal(matrix.schemaVersion, 1);
assert.equal(matrix.localOnly, true);
assert.ok(matrix.snapshotLifecycle.requiredOutcomes.length > 0);
assert.ok(matrix.snapshotLifecycle.tests.length > 0);
assert.ok(matrix.disasterRecovery.length > 0);
assert.ok(matrix.disasterRecovery.every(({ recoverable, tests }) => !recoverable || tests.length > 0));

const tests = new Set([
  ...matrix.snapshotLifecycle.tests,
  ...matrix.disasterRecovery.flatMap((item) => item.tests),
]);
for (const test of tests) {
  assert.match(test, /^[A-Za-z0-9_:]+$/);
  const result = spawnSync(
    "cargo",
    ["test", "-p", "codecaddie-core", "--locked", test, "--", "--exact"],
    { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
  );
  assert.equal(result.status, 0, result.stderr || `recovery test failed: ${test}`);
  assert.match(result.stdout, /1 passed/);
}

console.log(
  `executable recovery matrix passed: ${matrix.snapshotLifecycle.requiredOutcomes.length} snapshot outcomes; ${matrix.disasterRecovery.length} disaster cases; ${tests.size} focused tests`,
);
