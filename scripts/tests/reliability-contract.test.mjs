import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);

test("local reliability policy is owned, bounded, and never remote", async () => {
  const policy = JSON.parse(
    await readFile(new URL("config/service-level-objectives.json", root), "utf8"),
  );
  assert.equal(policy.schemaVersion, 1);
  assert.equal(policy.localOnly, true);
  assert.ok(policy.owner.length > 0);
  assert.match(policy.reviewCadence, /release|monthly/i);
  assert.ok(policy.operations.length >= 6);
  for (const operation of policy.operations) {
    assert.match(operation.operation, /^[a-z0-9._-]+$/);
    assert.ok(operation.availabilityPercent > 0);
    assert.ok(operation.latencyMilliseconds > 0);
    assert.ok(operation.alertAfterConsecutiveFailures > 0);
  }
  assert.equal(policy.customerJourneys.length, 4);
  for (const journey of policy.customerJourneys) {
    assert.match(journey.journey, /^[a-z0-9._-]+$/);
    assert.ok(journey.availabilityPercent > 0);
    assert.ok(journey.latencyMilliseconds > 0);
    assert.ok(journey.owner.length > 0);
    assert.ok(journey.alertCode.length > 0);
    assert.ok(journey.measuredBy.length > 0);
  }
  const serialized = JSON.stringify(policy);
  for (const forbidden of ["endpoint", "url", "token", "apiKey", "sourceText", "prompt"]) {
    assert.equal(serialized.includes(forbidden), false);
  }
});

test("reliability schema is an explicit content-free serialization allowlist", async () => {
  const schema = JSON.parse(
    await readFile(new URL("protocol/local-reliability-event-v1.schema.json", root), "utf8"),
  );
  assert.equal(schema.additionalProperties, false);
  assert.ok(schema.properties.kind.enum.includes("operation_completed"));
  assert.ok(schema.properties.kind.enum.includes("trace_span_completed"));
  const allowed = new Set(Object.keys(schema.properties));
  for (const forbidden of [
    "repositoryPath",
    "sourceText",
    "attachment",
    "prompt",
    "goalText",
    "message",
    "email",
    "secret",
  ]) {
    assert.equal(allowed.has(forbidden), false);
  }
});
