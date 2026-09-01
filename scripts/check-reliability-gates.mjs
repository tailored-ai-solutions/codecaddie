import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";

const read = (relative) => readFile(new URL(`../${relative}`, import.meta.url), "utf8");
const workflowDirectory = new URL("../.github/workflows/", import.meta.url);
const workflows = await Promise.all(
  (await readdir(workflowDirectory))
    .filter((name) => /\.ya?ml$/.test(name))
    .map((name) => readFile(new URL(name, workflowDirectory), "utf8")),
);
const policy = JSON.parse(await read("config/reliability-gates.json"));
const providerContract = JSON.parse(await read("protocol/provider-contract-v1.schema.json"));
const ci = await read(".github/workflows/ci.yml");
const release = await read(".github/workflows/release.yml");
const supportMatrix = await read("docs/SUPPORT-MATRIX.md");
const reliability = await read("docs/RELIABILITY-AND-PERFORMANCE.md");
const localReliability = await read("docs/LOCAL-RELIABILITY.md");
const localReliabilityPolicy = JSON.parse(await read("config/service-level-objectives.json"));
const localReliabilitySchema = JSON.parse(await read("protocol/local-reliability-event-v1.schema.json"));
const operationalAssurance = await read("docs/OPERATIONAL-ASSURANCE.md");
const assuranceIndex = JSON.parse(await read(".codecaddie/assurance.json"));
const faultMatrix = JSON.parse(await read("config/operational-fault-matrix.json"));

assert.equal(policy.schemaVersion, 1);
assert.ok(policy.quality.minimumRustLineCoveragePercent >= 50);
assert.ok(policy.quality.developerBootstrapMinutes > 0);
assert.ok(policy.quality.criticalVulnerabilitySlaHours <= 24);
assert.ok(policy.quality.dependencyUpdateCadenceDays <= 7);
assert.ok(policy.performance.firstSavedReportP95Seconds <= 600);
assert.ok(policy.performance.providerExecutionP95Seconds <= 480);
assert.ok(policy.performance.coreRequestP95Milliseconds <= 250);
assert.ok(policy.performance.minimumCoreRequestsPerMinute >= 120);
assert.ok(policy.performance.maximumRepositoryFiles >= 100_000);
assert.ok(policy.performance.maximumRepositoryBytes >= 2 * 1024 * 1024 * 1024);
assert.ok(policy.performance.maximumReportsPerWorkspace >= 1_000);

assert.match(ci, /cron: "17 8 \* \* \*"/);
assert.match(ci, /cargo llvm-cov/);
assert.match(ci, /--fail-under-lines/);
assert.match(ci, /performance_gate/);
assert.match(ci, /timeout-minutes: 15/);
for (const suite of policy.requiredSuites) {
  const declaredName = suite.startsWith("macOS native (")
    ? "name: macOS native (${{ matrix.architecture }})"
    : `name: ${suite}`;
  assert.ok(ci.includes(declaredName), `CI is missing required suite ${suite}`);
}

// `requiredChecks` reproduces the protected-main ruleset: every CI release
// suite plus the checks other workflows provide (the DCO sign-off gate).
assert.ok(Array.isArray(policy.requiredChecks) && policy.requiredChecks.length > 0);
assert.equal(new Set(policy.requiredChecks).size, policy.requiredChecks.length, "required checks must be unique");
for (const suite of policy.requiredSuites) {
  assert.ok(policy.requiredChecks.includes(suite), `required checks omit CI suite ${suite}`);
}
for (const check of policy.requiredChecks) {
  if (policy.requiredSuites.includes(check)) continue;
  assert.ok(
    workflows.some((workflow) => workflow.includes(`name: ${check}`)),
    `required check ${check} is not declared by any workflow job`,
  );
}
assert.ok(policy.requiredChecks.includes("DCO sign-off"), "the DCO sign-off gate must be a required check");

assert.match(release, /Verify exact-commit CI release gates/);
assert.match(release, /config\/reliability-gates\.json/);
assert.match(release, /actions: read/);

assert.equal(providerContract.properties.fallback.const, "forbidden");
for (const provider of ["codex", "claude", "grok"]) {
  assert.ok(providerContract.properties.provider.enum.includes(provider));
}
for (const phrase of [
  "macOS 15",
  "Windows Server 2025",
  "100,000 regular files",
  "XChaCha20-Poly1305"
]) {
  const haystack = phrase === "XChaCha20-Poly1305"
    ? await read("docs/BACKUP-AND-PORTABILITY.md")
    : supportMatrix;
  assert.ok(haystack.includes(phrase), `support contract is missing ${phrase}`);
}
assert.match(reliability, /24-hour remediation target/);
assert.match(reliability, /currently no exceptions/);
assert.equal(localReliabilityPolicy.schemaVersion, 1);
assert.equal(localReliabilityPolicy.localOnly, true);
assert.ok(localReliabilityPolicy.owner.length > 0);
assert.ok(localReliabilityPolicy.reviewCadence.length > 0);
assert.ok(localReliabilityPolicy.operations.some(({ operation }) => operation === "scan.run"));
assert.equal(localReliabilityPolicy.customerJourneys.length, 4);
assert.equal(assuranceIndex.schemaVersion, 1);
assert.ok(assuranceIndex.controls.length >= 5);
assert.equal(faultMatrix.localOnly, true);
assert.ok(faultMatrix.scenarios.length >= 5);
assert.equal(localReliabilitySchema.additionalProperties, false);
for (const forbidden of ["repositoryPath", "repositorySource", "attachmentContent", "goalText", "prompt", "freeText", "errorMessage"]) {
  assert.equal(localReliabilitySchema.properties[forbidden], undefined, `reliability schema must forbid ${forbidden}`);
}
assert.match(localReliability, /There is no remote telemetry endpoint/);
assert.match(localReliability, /correlation ID/);
assert.match(operationalAssurance, /index cannot support a criterion by itself/i);
assert.match(operationalAssurance, /provider timeout/);
assert.match(operationalAssurance, /disk exhaustion/);

console.log("reliability, performance, support, and provider contracts are synchronized");
