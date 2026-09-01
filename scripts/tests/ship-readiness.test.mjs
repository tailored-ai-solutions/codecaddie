import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { decodeSingleFrame, encodeFrame, exerciseInstalledJourney } from "../exercise-installed-core.mjs";

const root = new URL("../../", import.meta.url);
const policy = JSON.parse(await readFile(new URL("config/ship-readiness.json", root), "utf8"));
const assurance = JSON.parse(await readFile(new URL(".codecaddie/assurance.json", root), "utf8"));
const dependabot = await readFile(new URL(".github/dependabot.yml", root), "utf8");
const ci = await readFile(new URL(".github/workflows/ci.yml", root), "utf8");
const release = await readFile(new URL(".github/workflows/release.yml", root), "utf8");
const reconcile = await readFile(new URL(".github/workflows/reconcile-stable-release.yml", root), "utf8");
const installedJourney = await readFile(new URL("scripts/exercise-installed-core.mjs", root), "utf8");
const sources = await Promise.all([
  "crates/codecaddie-core/src/provider/runner.rs",
  "crates/codecaddie-core/src/provider/contract.rs",
  "crates/codecaddie-core/src/provider/contract_assurance.rs",
  "crates/codecaddie-core/src/provider/stream.rs",
  "crates/codecaddie-core/src/analyzer/scan.rs",
  "crates/codecaddie-core/src/repository.rs",
  "crates/codecaddie-core/src/repository_snapshot_lifecycle_assurance.rs",
  "crates/codecaddie-core/src/runtime_health_assurance.rs",
  "crates/codecaddie-core/src/persistence.rs",
  "crates/codecaddie-core/src/storage.rs",
  "crates/codecaddie-core/src/local_state/identity.rs",
].map((path) => readFile(new URL(path, root), "utf8")));
const source = sources.join("\n");

test("ship readiness binds every behavioral claim to an executable test", () => {
  assert.equal(policy.schemaVersion, 1);
  assert.equal(policy.localOnly, true);
  for (const section of ["providerContract", "snapshotLifecycle", "persistenceRecovery"]) {
    assert.ok(policy[section].executableTests.length >= 4);
    for (const testName of policy[section].executableTests) {
      assert.ok(source.includes(`fn ${testName}(`), `${testName} must exist in production-adjacent tests`);
      assert.ok(ci.includes(testName), `${testName} must run in the named ship-readiness CI suite`);
    }
  }
  const route = assurance.controls.find(({ id }) => id === "ship-readiness-assurance");
  assert.equal(route.artifacts[0], "config/ship-readiness.json");
  assert.match(ci, /name: Ship readiness assurance/);
});

test("weekly updates, vulnerability ownership, SLA, and release blocking stay enforceable", () => {
  assert.deepEqual(policy.dependencySecurity.ecosystems, ["cargo", "npm", "github-actions"]);
  assert.equal(policy.dependencySecurity.updateCadenceDays, 7);
  assert.equal(policy.dependencySecurity.criticalFindingSlaHours, 24);
  assert.equal(policy.dependencySecurity.releaseBlockOnCritical, true);
  for (const ecosystem of policy.dependencySecurity.ecosystems) {
    assert.match(dependabot, new RegExp(`package-ecosystem: ${ecosystem}`));
  }
  assert.equal((dependabot.match(/interval: weekly/g) ?? []).length, 3);
  for (const command of policy.dependencySecurity.scanCommands) {
    assert.ok(ci.includes(command), `${command} must be a protected release gate`);
  }
  assert.match(release, /Verify exact-commit CI release gates/);
  assert.match(release, /required suite did not pass/);
});

test("release health is keyless, immutable, automatic, and fix-forward", () => {
  assert.deepEqual(policy.releaseHealth.stages, [
    "keyless signed draft",
    "immutable GitHub publication",
    "forward-only Latest reconciliation",
  ]);
  assert.equal(policy.releaseHealth.releaseWorkflow, ".github/workflows/release.yml");
  assert.equal(
    policy.releaseHealth.reconciliationWorkflow,
    ".github/workflows/reconcile-stable-release.yml",
  );
  assert.match(policy.releaseHealth.rollbackStrategy, /corrective commit creates a newer build/i);
  assert.equal(policy.releaseHealth.manualAcceptance, undefined);
  assert.equal(policy.releaseHealth.rollbackWorkflow, undefined);
  assert.match(release, /--draft/);
  assert.match(release, /cosign sign-blob --yes/);
  assert.match(release, /attest-build-provenance/);
  assert.match(release, /anchore\/sbom-action@[a-f0-9]{40}/);
  assert.match(release, /uses: \.\/\.github\/workflows\/reconcile-stable-release\.yml/);

  assert.ok((reconcile.match(/node scripts\/verify-release-manifest\.mjs/g) ?? []).length >= 3);
  assert.match(reconcile, /CODECADDIE_REQUIRE_COMPLETE_RELEASE: "1"/);
  assert.match(reconcile, /gh attestation verify "requested-release\/\$artifact"/);
  assert.match(reconcile, /gh attestation verify "highest-release\/\$artifact"/);
  assert.match(reconcile, /--signer-workflow "\$GITHUB_REPOSITORY\/\.github\/workflows\/release\.yml"/);
  assert.match(reconcile, /--source-ref refs\/heads\/main/);
  assert.match(reconcile, /--source-digest "\$GITHUB_SHA"/);
  assert.match(reconcile, /--deny-self-hosted-runners/);
  assert.match(reconcile, /queue: max/);
  assert.match(reconcile, /node scripts\/compare-release-identities\.mjs/);
  assert.match(reconcile, /gh release edit "\$RELEASE_TAG" --draft=false --prerelease=false --latest/);
  assert.match(reconcile, /gh release edit "\$RELEASE_TAG" --draft=false --prerelease=false --latest=false/);
  assert.doesNotMatch(reconcile, /check-release-control|inputs\.mode|rollback-release|vercel blob/i);
});

test("installed core assurance uses one exact bounded protocol frame", () => {
  const fixture = { id: "fixture", protocolVersion: 2, method: "system.ping", params: {} };
  assert.deepEqual(decodeSingleFrame(encodeFrame(fixture)), fixture);
  assert.throws(() => decodeSingleFrame(Buffer.from([0, 0, 0, 8, 123])), /incomplete/);
  for (const suite of policy.crossPlatformJourney.requiredSuites) {
    const declared = suite.startsWith("macOS native")
      ? "name: macOS native (${{ matrix.architecture }})"
      : `name: ${suite}`;
    assert.ok(ci.includes(declared));
  }
  assert.equal((ci.match(/exercise-installed-core\.mjs/g) ?? []).length, 2);
  assert.equal(typeof exerciseInstalledJourney, "function");
  for (const outcome of ["analysis and provider failure tests", "saved report restart", "metadata-only export", "live installed scan cancellation with durable local cancellation record"]) {
    assert.ok(policy.crossPlatformJourney.requiredOutcomes.includes(outcome));
  }
  for (const proof of ["stream: true", "scan.progress", "operation_cancelled", "operationCancellations", "installed-cancelled-analysis"]) {
    assert.ok(installedJourney.includes(proof), `installed cancellation journey lacks ${proof}`);
  }
  assert.equal((ci.match(/--expected-commit "\$\(git rev-parse HEAD\)"/g) ?? []).length, 1);
  assert.equal((ci.match(/node scripts\/exercise-installed-core\.mjs --binary \$core --expected-commit \$commit/g) ?? []).length, 1);
  assert.doesNotMatch(ci, /git rev-parse --short=12 HEAD/);
  assert.ok(assurance.controls.find(({ id }) => id === "ship-readiness-assurance").artifacts.includes("apps/desktop/src/first_report_journey_assurance.zig"));
});

test("ship-readiness policy cannot become a data collection path", () => {
  const serialized = JSON.stringify(policy);
  for (const forbidden of ["sourceText", "attachmentContent", "goalText", "freeText", "apiKey", "token", "keychain", "credentialManager"]) {
    assert.equal(serialized.includes(forbidden), false);
  }
});
