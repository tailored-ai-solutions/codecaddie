import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");

test("snapshot and disaster recovery cases are one executable release matrix", async () => {
  const matrix = JSON.parse(await read("config/executable-recovery-matrix.json"));
  const disaster = JSON.parse(await read("config/disaster-recovery.json"));
  const ship = JSON.parse(await read("config/ship-readiness.json"));
  const runner = await read("scripts/exercise-recovery-matrix.mjs");
  const packageJson = JSON.parse(await read("package.json"));
  const ci = await read(".github/workflows/ci.yml");

  assert.equal(matrix.schemaVersion, 1);
  assert.equal(matrix.localOnly, true);
  assert.deepEqual(matrix.snapshotLifecycle.requiredOutcomes, ship.snapshotLifecycle.requiredOutcomes);
  assert.deepEqual(matrix.disasterRecovery.map((item) => item.case), disaster.failureDrills);
  assert.ok(matrix.disasterRecovery.every((item) => item.recoverable && item.tests.length > 0));
  assert.match(runner, /cargo/);
  assert.match(runner, /--exact/);
  assert.equal(packageJson.scripts["recovery:check"], "node scripts/exercise-recovery-matrix.mjs");
  assert.match(packageJson.scripts.check, /pnpm recovery:check/);
  assert.match(packageJson.scripts["check:release"], /pnpm recovery:check/);
  assert.match(ci, /pnpm recovery:check/);

  const allTests = [
    ...matrix.snapshotLifecycle.tests,
    ...matrix.disasterRecovery.flatMap((item) => item.tests),
  ];
  assert.ok(allTests.some((name) => name.includes("snapshot_lifecycle_matrix")));
  assert.ok(allTests.some((name) => name.includes("portable_backup_authenticates")));
  assert.ok(allTests.some((name) => name.includes("storage_capacity_failures")));
  assert.ok(allTests.some((name) => name.includes("migration")));
});
