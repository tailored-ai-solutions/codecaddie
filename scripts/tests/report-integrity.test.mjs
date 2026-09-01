import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

test("persisted report evidence is a closed immutable-coordinate allowlist", () => {
  const schema = JSON.parse(read("protocol/persisted-report-evidence-v1.schema.json"));
  assert.equal(schema.additionalProperties, false);
  assert.deepEqual(schema.required, [
    "repositoryId",
    "commitSha",
    "blobOid",
    "path",
    "startLine",
    "endLine",
    "contentHash",
    "kind",
  ]);
  assert.match(schema.properties.commitSha.pattern, /40/);
  assert.match(schema.properties.commitSha.pattern, /64/);
  assert.deepEqual(schema.properties.kind.enum, [
    "implementation",
    "test",
    "configuration",
    "documentation",
    "architecture",
  ]);
});

test("report signing and repeat history retain exact-commit evidence checks", () => {
  const store = read("crates/codecaddie-core/src/local_state/workspace_store.rs");
  const integrity = read("crates/codecaddie-core/src/report_integrity.rs");
  const history = read("crates/codecaddie-core/src/local_state/heatmap.rs");
  const desktop = read("apps/desktop/src/app.native");

  assert.match(store, /validate_report_for_persistence[\s\S]*DomainEvent::ReportCompleted/);
  assert.match(integrity, /verify_evidence/);
  assert.match(integrity, /supported and partial scorecard claims require immutable evidence/);
  assert.match(integrity, /prioritized actions require unique ids, ranks, goals, and immutable evidence/);
  assert.match(history, /previous_evidence/);
  assert.match(history, /field-level:/);
  assert.match(desktop, /Since prior/);
});
