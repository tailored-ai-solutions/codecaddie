import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");

test("every sensitive transport is encrypted or a bounded same-user local exception", async () => {
  const contract = JSON.parse(await read("config/local-transport-protection.json"));
  const canaryMatrix = JSON.parse(await read("config/source-canary-matrix-v1.json"));
  const canaryTests = new Set(canaryMatrix.surfaces.flatMap(({ rustTests = [] }) => rustTests));
  assert.equal(contract.schemaVersion, 1);
  assert.equal(contract.remoteApplicationServer, false);
  assert.equal(contract.atRest.encrypted, true);
  assert.equal(contract.atRest.credentialManager, false);
  assert.equal(contract.inTransitRule.encryptionRequiredWhenSensitiveDataCrossesTrustBoundary, true);
  assert.deepEqual(contract.inTransitRule.sensitiveData, [
    "repository source",
    "attachment contents",
    "credentials",
    "disallowed personal data",
  ]);
  assert.deepEqual(contract.boundaries.map(({ id }) => id), [
    "desktop-core-stdio",
    "desktop-core-staged-request",
    "core-provider-stdio",
  ]);
  for (const boundary of contract.boundaries) {
    assert.equal(boundary.encryptionRequired, false);
    assert.equal(boundary.sensitiveDataInTransit, false);
    assert.match(boundary.transport, /same-user/);
    assert.ok(boundary.localOnlyException.length > 80);
    assert.ok(boundary.controls.length >= 4);
    assert.ok(boundary.sensitiveDataExclusionProof.length >= 2);
    for (const testName of boundary.sensitiveDataExclusionProof) {
      assert.ok(canaryTests.has(testName), `${boundary.id} lacks executable sensitive-data exclusion proof: ${testName}`);
    }
  }

  const coreMain = await read("crates/codecaddie-core/src/main.rs");
  assert.match(coreMain, /from_mode\(0o600\)/);
  assert.match(coreMain, /RequestFileGuard/);
  assert.match(coreMain, /remove_file/);
  assert.match(coreMain, /must contain exactly one frame/);
  const provider = await read("crates/codecaddie-core/src/provider/runner.rs");
  assert.match(provider, /stdout\(Stdio::piped\(\)\)/);
  assert.match(provider, /stderr\(Stdio::piped\(\)\)/);
  assert.match(provider, /kill_on_drop\(true\)/);
  const platform = await read("apps/desktop/src/platform.zig");
  assert.match(platform, /permission_command/);
  assert.match(platform, /permission_view/);
  assert.doesNotMatch(platform, /permission_(?:network|http|socket)/);
});
