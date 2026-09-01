import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const policy = JSON.parse(await readFile(new URL("../../config/reliability-gates.json", import.meta.url), "utf8"));
const ci = await readFile(new URL("../../.github/workflows/ci.yml", import.meta.url), "utf8");
const release = await readFile(new URL("../../.github/workflows/release.yml", import.meta.url), "utf8");
const dependabot = await readFile(new URL("../../.github/dependabot.yml", import.meta.url), "utf8");
const protectedRef = await readFile(new URL("../check-protected-ref.mjs", import.meta.url), "utf8");
const dco = await readFile(new URL("../../.github/workflows/dco.yml", import.meta.url), "utf8");

test("the release policy names every exact required CI suite", () => {
  assert.equal(new Set(policy.requiredSuites).size, policy.requiredSuites.length);
  for (const suite of policy.requiredSuites) {
    const declaredName = suite.startsWith("macOS native (")
      ? "name: macOS native (${{ matrix.architecture }})"
      : `name: ${suite}`;
    assert.ok(ci.includes(declaredName));
  }
  assert.match(release, /requiredSuites/);
  assert.match(release, /conclusion == "success"/);
});

test("the required-check list reproduces the protected-main ruleset exactly", () => {
  assert.equal(new Set(policy.requiredChecks).size, policy.requiredChecks.length);
  assert.equal(policy.requiredChecks.length, policy.requiredSuites.length + 1);
  for (const suite of policy.requiredSuites) {
    assert.ok(policy.requiredChecks.includes(suite), `required checks omit ${suite}`);
  }
  const extras = policy.requiredChecks.filter((check) => !policy.requiredSuites.includes(check));
  assert.deepEqual(extras, ["DCO sign-off"]);
  assert.match(dco, /name: DCO sign-off/);
  assert.match(dco, /Signed-off-by/);
  assert.doesNotMatch(dco, /pull_request_target/);
});

test("daily dependency scans fail closed before release with a named owner and SLA", () => {
  assert.match(ci, /on:\s*\n\s*push:\s*\n\s*branches: \[main\]\s*\n\s*pull_request:/);
  assert.match(ci, /cron: "17 8 \* \* \*"/);
  assert.match(ci, /pnpm audit --audit-level high/);
  assert.match(ci, /cargo audit/);
  assert.equal(policy.owners.security, "CodeCaddie security owner");
  assert.ok(policy.quality.criticalVulnerabilitySlaHours <= 24);
  assert.ok(policy.requiredSuites.includes("Policy, version, and installers"));
  assert.ok(policy.requiredSuites.includes("Rust"));
  assert.match(release, /head_sha=\$GITHUB_SHA/);
  assert.match(release, /conclusion == "success"/);
  assert.match(release, /required suite did not pass/);
});

test("coverage capacity and clean bootstrap remain release gates", () => {
  assert.match(ci, /cargo llvm-cov/);
  assert.match(ci, /performance_gate/);
  assert.match(ci, /repeatable_first_report_load_stays_within_versioned_p95_budget/);
  assert.match(ci, /repeatable_provider_execution_load_stays_within_versioned_p95_budget/);
  assert.match(ci, /Clean developer bootstrap/);
  assert.match(ci, /timeout-minutes: 15/);
});

test("dependency updates and protected-main requirements are checked in", () => {
  for (const ecosystem of ["cargo", "npm", "github-actions"]) {
    assert.match(dependabot, new RegExp(`package-ecosystem: ${ecosystem}`));
  }
  assert.equal((dependabot.match(/interval: weekly/g) ?? []).length, 3);
  assert.match(ci, /CODECADDIE_REF_PROTECTED/);
  assert.match(ci, /Protected main release gates/);
  assert.match(protectedRef, /GitHub must report that main is protected/);
  assert.match(release, /head_sha=\$GITHUB_SHA/);
  assert.match(release, /required suite did not pass/);
});

test("CI admits branch changes once and bounds superseded or automated work", () => {
  const concurrency = ci.slice(ci.indexOf("concurrency:"), ci.indexOf("jobs:"));
  assert.match(ci, /push:\s*\n\s*branches: \[main\]/);
  assert.match(ci, /\n  pull_request:\s*\n/);
  assert.match(concurrency, /github\.event\.pull_request\.user\.login == 'dependabot\[bot\]'/);
  assert.match(concurrency, /format\('\{0\}-dependabot-\{1\}-\{2\}', github\.workflow, github\.base_ref, github\.event\.pull_request\.number\)/);
  assert.match(concurrency, /format\('\{0\}-pr-\{1\}', github\.workflow, github\.event\.pull_request\.number\)/);
  assert.match(concurrency, /format\('\{0\}-main-\{1\}', github\.workflow, github\.run_id\)/);
  assert.match(
    concurrency,
    /cancel-in-progress: \$\{\{ github\.ref != 'refs\/heads\/main' && !\(github\.event_name == 'pull_request' && github\.event\.pull_request\.user\.login == 'dependabot\[bot\]'\) \}\}/,
  );
  assert.doesNotMatch(concurrency, /queue: max/);
});

test("Dependabot cannot create or automatically rebase an unbounded version-update backlog", () => {
  const entries = dependabot.split(/\n  - package-ecosystem: /).slice(1);
  assert.equal(entries.length, 3);
  for (const entry of entries) {
    assert.match(entry, /open-pull-requests-limit: 1/);
    assert.match(entry, /rebase-strategy: disabled/);
    assert.match(entry, /applies-to: version-updates/);
    assert.match(entry, /patterns:\s*\n\s*- "\*"/);
  }
});

test("required macOS PR coverage remains on both supported architectures", () => {
  const macos = ci.slice(ci.indexOf("  macos-native:"), ci.indexOf("  windows-native-primary:"));
  assert.match(macos, /runner: macos-15-intel\s*\n\s*architecture: x64/);
  assert.match(macos, /runner: macos-15\s*\n\s*architecture: arm64/);
  assert.doesNotMatch(macos, /^    if:/m);
  assert.ok(policy.requiredSuites.includes("macOS native (x64)"));
  assert.ok(policy.requiredSuites.includes("macOS native (arm64)"));
});

test("the release runbook lists the required checks in the same order as the policy", async () => {
  const { readFile } = await import("node:fs/promises");
  const runbook = await readFile(new URL("../../docs/RELEASING.md", import.meta.url), "utf8");
  const lines = runbook.split("\n");
  const start = lines.findIndex((line) => line.includes("eleven status checks"));
  assert.ok(start >= 0, "runbook no longer introduces the required status checks");
  const listed = [];
  let began = false;
  for (const line of lines.slice(start + 1)) {
    const match = /^\d+\. `([^`]+)`$/.exec(line);
    if (match) {
      began = true;
      listed.push(match[1]);
    } else if (began) {
      break;
    }
  }
  assert.deepEqual(listed, policy.requiredChecks, "docs/RELEASING.md required-check list drifted from config/reliability-gates.json");
});
