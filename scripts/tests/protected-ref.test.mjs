import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(new URL("../check-protected-ref.mjs", import.meta.url));

function run(overrides) {
  return spawnSync(process.execPath, [script], {
    encoding: "utf8",
    env: {
      ...process.env,
      CODECADDIE_EVENT_NAME: "push",
      CODECADDIE_REF_NAME: "main",
      CODECADDIE_BASE_REF: "",
      CODECADDIE_REF_PROTECTED: "true",
      ...overrides,
    },
  });
}

test("protected direct main runs satisfy the branch gate", () => {
  const result = run({});
  assert.equal(result.status, 0, result.stderr);
});

test("an unprotected direct main run fails closed", () => {
  const result = run({ CODECADDIE_REF_PROTECTED: "false" });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /GitHub must report that main is protected/);
});

test("pull request checks are scoped to main without administration access", () => {
  const result = run({
    CODECADDIE_EVENT_NAME: "pull_request",
    CODECADDIE_REF_NAME: "123/merge",
    CODECADDIE_BASE_REF: "main",
    CODECADDIE_REF_PROTECTED: "false",
  });
  assert.equal(result.status, 0, result.stderr);
});
