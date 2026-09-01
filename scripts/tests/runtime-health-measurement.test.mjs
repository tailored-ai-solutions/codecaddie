import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");

test("native crashes and provider-boundary failures have local executable measurement", async () => {
  const contractText = await read("config/runtime-health-measurement.json");
  const contract = JSON.parse(contractText);
  assert.equal(contract.schemaVersion, 1);
  assert.equal(contract.localOnly, true);
  assert.equal(contract.externalTransmission, false);
  assert.equal(contract.clientCrashMeasurement.event, "desktop_crash_detected");
  assert.equal(contract.clientCrashMeasurement.aggregate, "desktopCrashesDetected");
  assert.equal(contract.clientCrashMeasurement.recordSchema, "protocol/local-reliability-event-v1.schema.json");
  assert.equal(contract.clientCrashMeasurement.markerBodyRead, false);
  assert.equal(contract.clientCrashMeasurement.markerPathRecorded, false);
  assert.deepEqual(contract.providerBridgeMeasurement.operations, [
    "scan.run",
    "goals.generate",
    "map.generate",
    "provider.*",
  ]);
  assert.deepEqual(contract.providerBridgeMeasurement.aggregates, [
    "providerOperationSamples",
    "providerOperationFailures",
    "providerAlertsRaised",
  ]);
  assert.equal(contract.localTraceMeasurement.event, "operation_completed");
  assert.equal(contract.localTraceMeasurement.legacyEvent, "trace_span_completed");
  assert.match(contract.localTraceMeasurement.representation, /prior-binary wire compatibility/);
  assert.equal(contract.localTraceMeasurement.traceIdentity, "correlationId");
  assert.equal(contract.localTraceMeasurement.aggregate, "traceSpansRecorded");
  assert.deepEqual(contract.privacy.surfaces, [
    "logs",
    "traces",
    "metrics",
    "alerts",
    "crash reports",
  ]);
  assert.equal(contract.privacy.schema, "protocol/local-reliability-event-v1.schema.json");
  assert.equal(contract.privacy.defaultFieldAdmission, "deny");
  assert.equal(contract.privacy.forbiddenData.includes("disallowed personal data"), true);

  const schema = JSON.parse(await read(contract.privacy.schema));
  assert.equal(schema.additionalProperties, false);
  assert.equal(schema.properties.kind.enum.includes("desktop_crash_detected"), true);

  const reliability = await read("crates/codecaddie-core/src/reliability.rs");
  const service = await read("crates/codecaddie-core/src/service.rs");
  const store = await read("crates/codecaddie-core/src/local_state/workspace_store.rs");
  const nativeModel = await read("apps/desktop/src/model.zig");
  assert.match(reliability, /pub fn is_provider_operation/);
  assert.match(service, /privacy_adversarial_provider_errors_receive_content_free_local_correlation_details/);
  assert.match(store, /provider_operation_failures/);
  assert.match(store, /privacy_adversarial_crash_markers_become_content_free_reliability_events/);
  const assurance = await read("crates/codecaddie-core/src/runtime_health_assurance.rs");
  assert.match(assurance, /privacy_adversarial_all_runtime_telemetry_surfaces_exclude_private_content/);
  assert.match(assurance, /person@example\.invalid/);
  assert.match(assurance, /sk-local-secret-canary/);
  assert.match(nativeModel, /provider-bridge failures/);

  for (const forbidden of ["repositorySource", "attachmentContent", "goalText", "freeText"]) {
    assert.equal(contractText.includes(forbidden), false);
  }
});
