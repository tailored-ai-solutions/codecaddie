import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const incidentPolicy = JSON.parse(
  await readFile(new URL("config/incident-response.json", root), "utf8"),
);
const runbook = await readFile(new URL("docs/INCIDENT-RESPONSE.md", root), "utf8");
const index = await readFile(new URL("docs/incidents/README.md", root), "utf8");
const template = await readFile(new URL("docs/incidents/TEMPLATE.md", root), "utf8");
const release = await readFile(new URL(".github/workflows/release.yml", root), "utf8");
const reconcile = await readFile(
  new URL(".github/workflows/reconcile-stable-release.yml", root),
  "utf8",
);
const updater = await readFile(
  new URL("crates/codecaddie-core/src/bin/codecaddie-updater.rs", root),
  "utf8",
);

test("incident response contract owns severity, roles, safe diagnostics, and actions", () => {
  assert.equal(incidentPolicy.schemaVersion, 1);
  assert.deepEqual(
    incidentPolicy.severityLevels.map(({ id }) => id),
    ["SEV-1", "SEV-2", "SEV-3", "SEV-4"],
  );
  assert.deepEqual(incidentPolicy.roles, [
    "incident_commander",
    "technical_lead",
    "communications_owner",
    "scribe",
  ]);
  for (const field of [
    "correlation_id",
    "operation",
    "failure_category",
    "elapsed_milliseconds",
    "platform",
    "product_version",
    "build_number",
    "immutable_commit",
  ]) {
    assert.ok(incidentPolicy.diagnosticAllowlist.includes(field), `missing ${field}`);
  }
  for (const field of [
    "repository_source",
    "repository_path",
    "attachment_content",
    "goal_text",
    "prompt",
    "secret",
    "personal_data",
  ]) {
    assert.ok(incidentPolicy.diagnosticForbidden.includes(field), `missing ${field}`);
  }
  for (const phrase of [
    "## Severity and acknowledgement",
    "## Roles",
    "## Customer-safe diagnostics",
    "## Containment",
    "## Recovery verification",
    "## Learning and corrective actions",
    "exact-commit CI",
  ]) {
    assert.ok(runbook.includes(phrase), `runbook is missing ${phrase}`);
  }
  assert.match(index, /Open actions.*Recovery evidence/);
  assert.match(template, /Repository-verifiable completion evidence/);
});

test("incident policy routes release recovery to automatic GitHub workflows", () => {
  assert.equal(incidentPolicy.releaseControl, undefined);
  assert.equal(incidentPolicy.releaseWorkflow, ".github/workflows/release.yml");
  assert.equal(
    incidentPolicy.stableReconciliationWorkflow,
    ".github/workflows/reconcile-stable-release.yml",
  );
  assert.match(release, /push:\s*\n\s*branches: \[main\]/);
  assert.match(reconcile, /queue: max/);
  assert.match(reconcile, /Publish once with the high-water decision in the immutable request/);
  assert.doesNotMatch(`${release}\n${reconcile}`, /check-release-control|inputs\.mode|rollback-release/);
});

test("failed installation still rolls back locally without a release-channel downgrade", () => {
  assert.match(updater, /fn replace_application_transaction</);
  assert.match(updater, /failed candidate could not be quarantined before rollback/);
  assert.match(updater, /failed_upgrade_rolls_back/);
  assert.match(updater, /healthy_upgrade_preserves_state/);
  assert.match(
    updater,
    /supported_prior_version_upgrade_and_rollback_matrix_preserves_real_encrypted_workspace_state/,
  );
  assert.doesNotMatch(`${release}\n${reconcile}`, /deliberate_rollback|selected prior|prior stable/);
});

test("obsolete manual release-channel controls are absent", async () => {
  for (const path of [
    "config/release-control.json",
    "scripts/check-release-control.mjs",
    ".github/workflows/promote-release.yml",
    ".github/workflows/rollback-release.yml",
  ]) {
    await assert.rejects(access(new URL(path, root)));
  }
});
