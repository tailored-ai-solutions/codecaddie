import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);

test("runtime containment defaults are closed owned restart-safe and no-Keychain", async () => {
  const policyText = await readFile(new URL("config/runtime-feature-controls.json", root), "utf8");
  const policy = JSON.parse(policyText);
  const implementation = await readFile(new URL("crates/codecaddie-core/src/runtime_controls.rs", root), "utf8");
  assert.equal(policy.schemaVersion, 1);
  assert.ok(policy.owner.length > 0);
  assert.deepEqual(policy.features, {
    providerExecution: "enabled",
    portableBackupImport: "enabled",
    reportExport: "enabled",
    recommendationPromptCopy: "enabled",
  });
  assert.match(implementation, /runtime-feature-controls-v1\.json/);
  assert.match(implementation, /read_encrypted_migrating/);
  assert.match(implementation, /write_encrypted_replace/);
  assert.match(implementation, /workspace\.recent/);
  for (const forbidden of ["Keychain", "Credential Manager", "Secret Service", "security-framework", "keyring"] ) {
    assert.equal(implementation.includes(forbidden), false);
  }
});
