import assert from "node:assert/strict";
import { lstat, readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (relative) => readFile(new URL(relative, root), "utf8");
const ci = await read(".github/workflows/ci.yml");
const faultAssurance = await read("crates/codecaddie-core/src/operational_fault_assurance.rs");

test("repository assurance index is bounded routing to real regular files", async () => {
  const indexText = await read(".codecaddie/assurance.json");
  const index = JSON.parse(indexText);
  const schema = JSON.parse(await read("protocol/repository-assurance-index-v1.schema.json"));
  assert.equal(index.schemaVersion, 1);
  assert.equal(schema.additionalProperties, false);
  assert.ok(index.controls.length > 0 && index.controls.length <= 64);
  const ids = new Set();
  for (const control of index.controls) {
    assert.match(control.id, /^[a-z0-9._-]{1,80}$/);
    assert.equal(ids.has(control.id), false);
    ids.add(control.id);
    assert.ok(control.topics.length > 0 && control.topics.length <= 12);
    assert.ok(control.artifacts.length > 0 && control.artifacts.length <= 12);
    for (const artifact of control.artifacts) {
      assert.equal(artifact.startsWith("/"), false);
      assert.equal(artifact.split("/").includes(".."), false);
      const metadata = await lstat(new URL(artifact, root));
      assert.equal(metadata.isFile(), true, `${artifact} must be a regular file`);
      assert.equal(metadata.isSymbolicLink(), false, `${artifact} must not be a link`);
    }
  }
  assert.ok(
    Buffer.byteLength(indexText) <= 64 * 1024,
    "assurance index must fit its ingestion boundary",
  );
  for (const required of [
    "supported-environments",
    "local-service-objectives",
    "local-runtime-health-measurement",
    "source-safe-failure-recovery",
    "operational-fault-injection",
    "restart-safe-runtime-and-release-recovery",
    "runtime-lifecycle-measurement",
    "cross-sink-privacy-proof",
    "disposable-snapshot-lifecycle",
    "dependency-maintenance",
    "required-check-enforcement",
    "prior-version-upgrade-rollback",
  ]) {
    assert.equal(ids.has(required), true, `missing assurance route ${required}`);
  }
  for (const forbidden of ["goalId", "desiredVerdict", "sourceExcerpt", "PRIVATE SOURCE SENTINEL"]) {
    assert.equal(indexText.includes(forbidden), false);
  }
});

test("operational fault matrix binds every required fault to a metric and alert", async () => {
  const matrixText = await read("config/operational-fault-matrix.json");
  const matrix = JSON.parse(matrixText);
  assert.equal(matrix.schemaVersion, 1);
  assert.equal(matrix.localOnly, true);
  assert.ok(matrix.owner.length > 0);
  const scenarios = new Map(matrix.scenarios.map((scenario) => [scenario.id, scenario]));
  for (const id of [
    "provider-timeout",
    "malformed-provider-response",
    "disk-exhaustion",
    "interrupted-write",
    "telemetry-outage",
  ]) {
    const scenario = scenarios.get(id);
    assert.ok(scenario, `missing fault scenario ${id}`);
    for (const field of ["productionSurface", "test", "expectedErrorCode", "expectedMetric", "expectedAlert"]) {
      assert.ok(scenario[field].length > 0, `${id} is missing ${field}`);
    }
    assert.ok(
      faultAssurance.includes(`fn ${scenario.test}(`),
      `${id} must route to the focused executable fault matrix`,
    );
    assert.ok(ci.includes(scenario.test), `${id} must run in CI`);
  }
  for (const forbidden of ["endpoint", "apiKey", "keychain", "credentialManager", "sourceText", "prompt"]) {
    assert.equal(matrixText.toLowerCase().includes(forbidden.toLowerCase()), false);
  }
});

test("service objectives cover owned customer journeys with local measurements", async () => {
  const policyText = await read("config/service-level-objectives.json");
  const policy = JSON.parse(policyText);
  const journeys = new Map(policy.customerJourneys.map((journey) => [journey.journey, journey]));
  for (const id of [
    "first_report_creation",
    "report_persistence",
    "provider_execution",
    "crash_free_desktop_sessions",
  ]) {
    const journey = journeys.get(id);
    assert.ok(journey, `missing customer journey SLO ${id}`);
    assert.ok(journey.availabilityPercent > 0);
    assert.ok(journey.latencyMilliseconds > 0);
    assert.ok(journey.owner.length > 0);
    assert.match(journey.alertCode, /^[a-z0-9._-]+$/);
    assert.ok(journey.measuredBy.length > 0);
  }
  assert.equal(policy.localOnly, true);
  for (const forbidden of ["endpoint", "url", "token", "apiKey", "keychain", "sourceText", "prompt"]) {
    assert.equal(policyText.toLowerCase().includes(forbidden.toLowerCase()), false);
  }
});
