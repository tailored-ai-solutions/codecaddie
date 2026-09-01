import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");

test("report save and export decision is explicit and fail closed", async () => {
  const policy = JSON.parse(await read("config/report-evidence-policy.json"));
  assert.equal(policy.schemaVersion, 1);
  assert.ok(policy.owner.length > 0);
  assert.match(policy.reviewCadence, /monthly|incident/i);
  assert.ok(policy.saveDecision.required.some((item) => /every approved success check/.test(item)));
  assert.ok(policy.saveDecision.required.some((item) => /unsupported or unverified/.test(item)));
  assert.ok(policy.saveDecision.block.some((item) => /without bindable immutable evidence/.test(item)));
  assert.equal(policy.exportDecision.requiresPersistedValidatedReport, true);
  assert.equal(policy.exportDecision.allowUnverifiedWhenExplicitlyLabeled, true);
  assert.equal(policy.exportDecision.sourceTextAllowed, false);
  assert.match(policy.executableAssurance.actionReferenceRejectionMatrix, /prioritized_action_reference_rejection_matrix_is_complete/);
  assert.match(
    policy.executableAssurance.nonOverwritingComparisonJourney,
    /repeat_analysis_retains_both_commits_and_emits_review_and_comparison_lifecycle_events/,
  );
  assert.match(policy.executableAssurance.workingTreeSwitchUiProjection, /switching_the_working_tree_cannot_change_saved_or_displayed_evidence/);
});

test("local data governance has explicit consent deletion minimization and no exceptions", async () => {
  const policyText = await read("config/data-governance.json");
  const policy = JSON.parse(policyText);
  assert.equal(policy.schemaVersion, 1);
  assert.equal(policy.consent.externalTransmission, "forbidden");
  assert.equal(policy.consent.freeTextCollection, "forbidden");
  assert.match(policy.consent.basis, /explicitly creates|explicit local/i);
  assert.match(policy.consent.revoke, /Delete/);
  assert.equal(policy.retention.lifetime, "workspace lifetime");
  assert.equal(policy.retention.aggregateExportOnly, true);
  assert.deepEqual(policy.minimization.disallowedPersonalDataTaxonomy, [
    "names and aliases",
    "email addresses and phone numbers",
    "postal addresses and precise location",
    "government and financial account identifiers",
    "authentication secrets",
    "biometric health and demographic attributes",
    "free-form user or provider content",
  ]);
  assert.deepEqual(policy.exceptions, []);
  for (const forbidden of ["endpoint", "apiKey", "keychain", "credentialManager"]) {
    assert.equal(policyText.toLowerCase().includes(forbidden.toLowerCase()), false);
  }
});

test("serialized measurement fields fail closed until their privacy contract changes", async () => {
  const expected = new Map([
    ["protocol/local-product-events-v2.schema.json", ["schemaVersion", "kind", "workspaceId", "sessionId", "productVersion", "platform", "cohort", "reportId", "elapsedMilliseconds"]],
    ["protocol/local-reliability-event-v1.schema.json", ["schemaVersion", "kind", "correlationId", "sessionId", "operation", "outcome", "errorCategory", "errorCode", "retryable", "elapsedMilliseconds", "alertCode", "productVersion", "platform"]],
    ["protocol/persisted-report-evidence-v1.schema.json", ["repositoryId", "commitSha", "blobOid", "path", "startLine", "endLine", "contentHash", "kind"]],
    ["protocol/updater-result-v1.schema.json", ["schemaVersion", "status", "code"]],
  ]);
  const governance = JSON.parse(await read("config/data-governance.json"));
  assert.deepEqual(governance.minimization.allowlistedSchemas, [...expected.keys()]);
  for (const [path, fields] of expected) {
    const schema = JSON.parse(await read(path));
    assert.equal(schema.additionalProperties, false, `${path} must stay closed`);
    assert.deepEqual(Object.keys(schema.properties), fields, `${path} added or removed a field without governance review`);
  }
  const matrix = JSON.parse(await read("config/source-canary-matrix-v1.json"));
  for (const surface of ["reports", "desktop-ipc", "progress-diagnostics-and-logs", "word-export", "recovery-export", "coding-prompt", "product-analytics", "native-crash-marker", "provider-runtime-errors", "local-trace-spans", "updater-result-mailbox-and-startup-ipc"]) {
    assert.ok(matrix.requiredSurfaces.includes(surface), `privacy gate is missing ${surface}`);
  }
});

test("security-relevant actions have least-privilege and signed local audit mappings", async () => {
  const controls = JSON.parse(await read("config/security-audit-controls.json"));
  assert.equal(controls.schemaVersion, 1);
  assert.equal(controls.localOnly, true);
  assert.ok(controls.owner.length > 0);
  assert.match(controls.transportBoundary.desktopCore, /device-local process pipes/);
  assert.match(controls.transportBoundary.updates, /HTTPS/);
  assert.match(controls.transportBoundary.auditStorage, /owner-only local content key/);
  assert.match(controls.executableTest, /security_relevant_operations/);
  assert.ok(controls.controls.length >= 5);
  const operations = new Set();
  for (const control of controls.controls) {
    assert.ok(control.action.length > 0);
    assert.ok(control.leastPrivilege.length > 0);
    assert.match(control.auditRecord, /signed|immutable/i);
    assert.ok(control.operationCodes.length > 0);
    for (const operation of control.operationCodes) {
      assert.match(operation, /^[a-z][a-z0-9_.]+$/);
      assert.equal(operations.has(operation), false, `duplicate audit operation ${operation}`);
      operations.add(operation);
    }
  }
});
