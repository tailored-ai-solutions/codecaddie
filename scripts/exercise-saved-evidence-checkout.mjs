#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const policy = JSON.parse(
  readFileSync(resolve(ROOT, "config/report-evidence-policy.json"), "utf8"),
);
const test = policy.executableAssurance.workingTreeSwitchUiProjection;
assert.equal(
  test,
  "decision_journey_assurance::switching_the_working_tree_cannot_change_saved_or_displayed_evidence",
);

const result = spawnSync(
  "cargo",
  ["test", "-p", "codecaddie-core", "--locked", test, "--", "--exact"],
  { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
);
assert.equal(result.status, 0, result.stderr || "saved-evidence checkout gate failed");
assert.match(result.stdout, /1 passed/);
console.log(`saved-evidence checkout gate passed: ${test}`);
