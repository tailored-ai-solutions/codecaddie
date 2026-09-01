import assert from "node:assert/strict";

const eventName = process.env.CODECADDIE_EVENT_NAME ?? "";
const refName = process.env.CODECADDIE_REF_NAME ?? "";
const baseRef = process.env.CODECADDIE_BASE_REF ?? "";
const refProtected = process.env.CODECADDIE_REF_PROTECTED ?? "";

if (eventName === "pull_request") {
  assert.equal(baseRef, "main", "the protected-main gate must target a pull request into main");
} else {
  assert.equal(refName, "main", "the protected-main gate must run on main");
  assert.equal(
    refProtected,
    "true",
    "GitHub must report that main is protected for push, schedule, and manual runs",
  );
}

console.log("this workflow run targets protected main");
