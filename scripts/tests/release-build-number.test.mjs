import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { releaseBuildNumber } from "../release-build-number.mjs";

const exec = promisify(execFile);
const root = fileURLToPath(new URL("../..", import.meta.url));

test("release build number is calibrated above the previous 1999 range", async () => {
  const { stdout } = await exec("git", ["rev-list", "--first-parent", "--count", "HEAD"], {
    cwd: root,
  });
  const count = Number(stdout.trim());
  const build = await releaseBuildNumber("HEAD", root);
  assert.equal(build, 2000 + count);
  assert.ok(build > 1999);
});

test("release build number is stable for an exact commit", async () => {
  const { stdout } = await exec("git", ["rev-parse", "HEAD"], { cwd: root });
  assert.equal(
    await releaseBuildNumber(stdout.trim(), root),
    await releaseBuildNumber("HEAD", root),
  );
});
