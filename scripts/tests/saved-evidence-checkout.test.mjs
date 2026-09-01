import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");

test("saved evidence checkout pinning is an exact executable release gate", async () => {
  const policy = JSON.parse(await read("config/report-evidence-policy.json"));
  const runner = await read("scripts/exercise-saved-evidence-checkout.mjs");
  const journey = await read("crates/codecaddie-core/src/decision_journey_assurance.rs");
  const packageJson = JSON.parse(await read("package.json"));
  const ci = await read(".github/workflows/ci.yml");
  const exactTest = policy.executableAssurance.workingTreeSwitchUiProjection;

  assert.equal(
    exactTest,
    "decision_journey_assurance::switching_the_working_tree_cannot_change_saved_or_displayed_evidence",
  );
  assert.match(journey, /assert!\(proof\.checkout_changed\)/);
  assert.match(journey, /assert!\(proof\.report_commit_preserved\)/);
  assert.match(journey, /assert!\(proof\.displayed_reference_preserved\)/);
  assert.match(journey, /assert!\(proof\.original_blob_reopened\)/);
  assert.match(runner, /--exact/);
  assert.equal(
    packageJson.scripts["evidence:check"],
    "node scripts/exercise-saved-evidence-checkout.mjs",
  );
  assert.match(packageJson.scripts.check, /pnpm evidence:check/);
  assert.match(packageJson.scripts["check:release"], /pnpm evidence:check/);
  assert.match(ci, /node scripts\/exercise-saved-evidence-checkout\.mjs/);
});
