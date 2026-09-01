import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { summarizeFirstReportActivation } from "../first-report-metric.mjs";

const root = new URL("../../", import.meta.url);
const metrics = JSON.parse(await readFile(new URL("config/product-metrics.json", root), "utf8"));
const schema = JSON.parse(await readFile(new URL("protocol/local-product-events-v2.schema.json", root), "utf8"));
const activationSchema = JSON.parse(await readFile(new URL("protocol/first-report-activation-v1.schema.json", root), "utf8"));
const fixture = JSON.parse(await readFile(new URL("fixtures/product-metrics/first-report-activation-v1.json", root), "utf8"));
const journey = JSON.parse(await readFile(new URL("config/first-report-journey-v1.json", root), "utf8"));
const docs = await readFile(new URL("docs/LOCAL-PRODUCT-MEASUREMENT.md", root), "utf8");
const nativeTests = await readFile(new URL("apps/desktop/src/tests.zig", root), "utf8");

test("local activation metrics are versioned, bounded, and platform segmented", () => {
  assert.equal(metrics.schemaVersion, 1);
  assert.equal(metrics.eventSchema, "../protocol/local-product-events-v2.schema.json");
  assert.equal(metrics.storage.location, "signed-workspace-ledger");
  assert.equal(metrics.storage.transmission, "none");
  assert.equal(metrics.activation.metricId, "first-saved-report-within-ten-minutes-v1");
  assert.equal(metrics.activation.numerator.maximumElapsedMilliseconds, 600_000);
  assert.equal(metrics.activation.target.rate, 0.8);
  assert.deepEqual(metrics.activation.segmentBy, ["productVersion", "platformFamily", "cohort"]);
  assert.deepEqual(metrics.activation.requiredPlatformFamilies, ["macos", "windows"]);
  assert.equal(metrics.evidenceCompleteness.source, "saved-report-projection");
  assert.equal(metrics.evidenceCompleteness.denominator, "all-current-success-checks");
  assert.equal(metrics.evidenceCompleteness.target.rate, 1);
  assert.match(metrics.evidenceCompleteness.reviewCadence, /every-saved-report/);
  assert.equal(activationSchema.properties.activation.properties.numerator.properties.maximumElapsedMilliseconds.const, 600_000);
  assert.equal(activationSchema.properties.activation.properties.target.properties.rate.const, 0.8);
  assert.match(docs, /does not request a review rating or transmit this data/i);
});

test("first-report activation calculation is exact at the ten-minute boundary and segmented by macOS and Windows", () => {
  const summaries = summarizeFirstReportActivation(metrics, fixture.observations);
  assert.deepEqual(
    summaries.map(({ platformFamily, eligibleWorkspaces, qualifiedWorkspaces, successRate, targetMet }) => ({
      platformFamily,
      eligibleWorkspaces,
      qualifiedWorkspaces,
      successRate,
      targetMet,
    })),
    [
      { platformFamily: "macos", eligibleWorkspaces: 4, qualifiedWorkspaces: 2, successRate: 0.5, targetMet: false },
      { platformFamily: "windows", eligibleWorkspaces: 5, qualifiedWorkspaces: 4, successRate: 0.8, targetMet: true },
    ],
  );
  assert.throws(
    () => summarizeFirstReportActivation(metrics, [...fixture.observations, fixture.observations[0]]),
    /workspace appears more than once/,
  );
});

test("one native end-to-end matrix owns every first-report recovery state", () => {
  assert.equal(journey.schemaVersion, 1);
  assert.equal(journey.journeyId, "workspace-to-first-saved-report");
  assert.deepEqual(journey.states.map(({ id }) => id), [
    "repository-selected",
    "commit-frozen",
    "empty-report",
    "invalid-goals",
    "cancelled-analysis",
    "provider-failure",
    "retry-started",
    "saved-success",
  ]);
  assert.equal(journey.automatedTest.ciCommand, "pnpm native:check");
  assert.match(nativeTests, new RegExp(`test "${journey.automatedTest.name}"`));
});

test("event allowlist contains lifecycle evidence and excludes content fields", () => {
  assert.equal(schema.properties.schemaVersion.const, 2);
  assert.ok(schema.required.includes("workspaceId"));
  const kinds = schema.properties.kind.enum;
  for (const kind of [
    "workspace_created",
    "analysis_started",
    "scorecard_generated",
    "report_saved",
    "time_to_first_saved_report",
    "report_revisited",
    "evidence_opened",
    "repeat_analysis_started",
    "comparison_generated",
  ]) {
    assert.ok(kinds.includes(kind), `missing ${kind}`);
  }
  const serialized = JSON.stringify(schema);
  for (const forbidden of metrics.privacyDenylist) {
    assert.ok(!Object.hasOwn(schema.properties, forbidden), `schema admits ${forbidden}`);
  }
  assert.doesNotMatch(serialized, /repositorySource|attachmentContent|goalText|freeText/);
});
