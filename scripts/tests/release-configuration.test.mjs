import assert from "node:assert/strict";
import { generateKeyPairSync } from "node:crypto";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { checkReleaseConfiguration } from "../check-release-configuration.mjs";

function encodedKey(curve = "prime256v1") {
  return Buffer.from(generateKeyPairSync("ec", { namedCurve: curve }).privateKey.export({
    type: "pkcs8", format: "pem",
  })).toString("base64");
}

function validEnvironment() {
  return {
    CODECADDIE_APPLE_TEAM_ID: "A".repeat(10),
    CODECADDIE_XCODE_CLOUD_WORKFLOW_ID: "00000000-0000-4000-8000-000000000001",
    APP_STORE_CONNECT_KEY_ID: "B".repeat(12),
    APP_STORE_CONNECT_PRIVATE_KEY_BASE64: encodedKey(),
  };
}

test("preflight aggregates missing configuration with safe environment-specific instructions", () => {
  const failures = checkReleaseConfiguration({});
  assert.equal(failures.length, 4);
  for (const failure of failures) {
    assert.match(failure, /Missing (variable|secret)/);
    assert.match(failure, /release-apple/);
  }
});

test("valid P-256 private key passes and the credential is removed from the environment", () => {
  const environment = validEnvironment();
  environment.APP_STORE_CONNECT_PRIVATE_KEY_BASE64 = environment.APP_STORE_CONNECT_PRIVATE_KEY_BASE64.match(/.{1,64}/g).join("\n");
  assert.deepEqual(checkReleaseConfiguration(environment), []);
  assert.equal(environment.APP_STORE_CONNECT_PRIVATE_KEY_BASE64, undefined);
});

test("malformed variables produce field names but never their values", () => {
  const environment = validEnvironment();
  for (const name of ["CODECADDIE_APPLE_TEAM_ID", "CODECADDIE_XCODE_CLOUD_WORKFLOW_ID", "APP_STORE_CONNECT_KEY_ID"]) {
    environment[name] = "do-not-echo-this::error::\n";
  }
  const failures = checkReleaseConfiguration(environment);
  assert.equal(failures.length, 3);
  assert.doesNotMatch(failures.join("\n"), /do-not-echo-this|::error::/);
});

test("malformed encoding, oversized input, non-key content, and wrong curves fail closed", () => {
  const key = encodedKey();
  for (const input of [" \n ", "!" + key, key + "===", "A".repeat(16_385), Buffer.from("not-a-private-key").toString("base64"), encodedKey("secp384r1")]) {
    const environment = { ...validEnvironment(), APP_STORE_CONNECT_PRIVATE_KEY_BASE64: input };
    const failures = checkReleaseConfiguration(environment);
    assert.equal(failures.length, 1);
    assert.match(failures[0], /Invalid secret APP_STORE_CONNECT_PRIVATE_KEY_BASE64/);
    assert.equal(environment.APP_STORE_CONNECT_PRIVATE_KEY_BASE64, undefined);
  }
});

test("CLI returns failure without leaking malformed credential or parser diagnostics", () => {
  const marker = "not-a-key-private-marker";
  const result = spawnSync(process.execPath, [new URL("../check-release-configuration.mjs", import.meta.url).pathname], {
    env: { ...validEnvironment(), APP_STORE_CONNECT_PRIVATE_KEY_BASE64: Buffer.from(marker).toString("base64") }, encoding: "utf8",
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /::error::Invalid secret/);
  assert.match(result.stderr, /docs\/RELEASING.md#release-configuration-preflight/);
  assert.doesNotMatch(result.stderr + result.stdout, /not-a-key-private-marker|DECODER|at checkReleaseConfiguration/);
});
