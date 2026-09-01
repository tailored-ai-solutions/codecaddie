import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");
const json = async (path) => JSON.parse(await read(path));

test("retention deletion consent minimization and exceptions are versioned and release blocking", async () => {
  const governance = await json("config/data-governance.json");
  const register = await json(governance.exceptionPolicy.register);
  assert.equal(governance.consent.externalTransmission, "forbidden");
  assert.equal(governance.consent.freeTextCollection, "forbidden");
  assert.equal(governance.retention.lifetime, "workspace lifetime");
  assert.match(governance.deletion.scope, /events, reports, maps, measurements, diagnostics, backups, and owner-only local content key/);
  assert.equal(governance.deletion.remoteDeletionRequired, false);
  assert.equal(governance.minimization.forbidden.includes("repository source"), true);
  assert.deepEqual(governance.exceptions, []);
  assert.equal(register.owner, governance.owner);
  assert.deepEqual(register.requiredFields, governance.exceptionPolicy.requiredFields);
  assert.equal(register.expiredExceptionsBlockRelease, true);
  assert.deepEqual(register.exceptions, []);
});

test("every serialized sink is closed allowlisted and canary gated before field admission", async () => {
  const admission = await json("config/serialized-field-admission-v1.json");
  const matrix = await json("config/source-canary-matrix-v1.json");
  assert.equal(admission.default, "deny");
  assert.match(admission.changeRule, /new serialized field is rejected/i);
  for (const item of admission.schemas) {
    const schema = await json(item.path);
    assert.equal(schema.additionalProperties, false, `${item.path} must reject unknown fields`);
    assert.deepEqual(Object.keys(schema.properties), item.allowedFields, `${item.path} changed without field admission`);
  }
  assert.deepEqual(admission.sinks.map(({ id }) => id), [
    "word-and-recovery-exports",
    "local-runtime-telemetry",
    "reports-ipc-prompts-and-product-events",
    "updater-result-mailbox-and-startup-ipc",
  ]);
  const declaredTests = new Set(matrix.surfaces.flatMap((surface) => surface.rustTests ?? []));
  for (const sink of admission.sinks) {
    assert.ok(sink.allowlist.length > 0);
    assert.ok(sink.implementation.length > 0);
    for (const testName of sink.tests) assert.ok(declaredTests.has(testName), `${sink.id} is missing canary test ${testName}`);
  }

  const updaterSchema = await json("protocol/updater-result-v1.schema.json");
  assert.deepEqual(updaterSchema.required, ["schemaVersion", "status", "code"]);
  assert.equal(updaterSchema.properties.schemaVersion.const, 1);
  assert.equal(updaterSchema.properties.status.const, "failed");
  assert.deepEqual(updaterSchema.properties.code.enum, [
    "installFailed",
    "reopenFailed",
    "restartRequired",
    "manualRepairRequired",
    "resultUnreadable",
  ]);
  const updaterSink = admission.sinks.find(({ id }) => id === "updater-result-mailbox-and-startup-ipc");
  assert.match(updaterSink.allowlist, /no paths/);
  assert.match(updaterSink.allowlist, /source text/);
  assert.match(updaterSink.allowlist, /secrets/);
  assert.match(updaterSink.allowlist, /raw installer or OS errors/);
});

test("least privilege local transport encryption and signed audits form one executable contract", async () => {
  const transport = await json("config/local-transport-protection.json");
  const audit = await json("config/security-audit-controls.json");
  assert.equal(transport.remoteApplicationServer, false);
  assert.equal(transport.atRest.encrypted, true);
  assert.equal(transport.atRest.credentialManager, false);
  assert.match(transport.atRest.keyStorage, /owner-only local data-root key/);
  assert.equal(transport.boundaries.length, 3);
  for (const boundary of transport.boundaries) {
    assert.equal(boundary.encryptionRequired, false);
    assert.match(boundary.localOnlyException, /No network hop|transient file|opens no application network transport/);
    assert.ok(boundary.controls.length >= 4);
  }
  assert.equal(audit.localOnly, true);
  assert.ok(audit.controls.length >= 5);
  for (const control of audit.controls) {
    assert.ok(control.leastPrivilege.length > 0);
    assert.match(control.auditRecord, /signed/i);
    assert.ok(control.operationCodes.length > 0);
  }
});

test("the named CI privacy gate runs both field admission and all adversarial sinks", async () => {
  const ci = await read(".github/workflows/ci.yml");
  assert.match(ci, /Validate retention deletion exceptions and serialized-field admission/);
  assert.match(ci, /node --test scripts\/tests\/privacy-contract-assurance\.test\.mjs/);
  assert.match(ci, /cargo test --workspace --locked privacy_adversarial -- --nocapture/);
});
