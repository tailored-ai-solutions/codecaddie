import assert from "node:assert/strict";
import test from "node:test";
import { compareReleaseIdentities } from "../compare-release-identities.mjs";

test("stable release identities advance by SemVer then build", () => {
  assert.equal(
    compareReleaseIdentities({ version: "0.3.0", build: 1032 }, { version: "0.3.0", build: 1031 }),
    1,
  );
  assert.equal(
    compareReleaseIdentities({ version: "0.3.0", build: 1031 }, { version: "0.3.0", build: 1031 }),
    0,
  );
  assert.equal(
    compareReleaseIdentities({ version: "0.4.0", build: 1 }, { version: "0.3.99", build: 65535 }),
    1,
  );
  assert.equal(
    compareReleaseIdentities({ version: "0.2.9", build: 65535 }, { version: "0.3.0", build: 1 }),
    -1,
  );
  assert.throws(
    () => compareReleaseIdentities({ version: "0.3.0", build: 0 }, { version: "0.3.0", build: 1 }),
    /positive safe integer/,
  );
  assert.throws(
    () => compareReleaseIdentities({ version: "01.3.0", build: 1 }, { version: "0.3.0", build: 1 }),
    /must be X.Y.Z/,
  );
  assert.throws(
    () => compareReleaseIdentities({ version: "2.0.0", build: 0 }, { version: "1.0.0", build: 1 }),
    /positive safe integer/,
  );
});
