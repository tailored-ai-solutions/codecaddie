import assert from "node:assert/strict";
import test from "node:test";

import { assertNoCredentialStoreEntrypoints } from "../check-native-credential-boundary.mjs";

test("native credential boundary accepts an application without credential APIs", () => {
  assert.doesNotThrow(() =>
    assertNoCredentialStoreEntrypoints(Buffer.from("CodeCaddie local owner-only state"), "_NSApplication\n"),
  );
});

test("native credential boundary rejects macOS Keychain entrypoints", () => {
  assert.throws(
    () => assertNoCredentialStoreEntrypoints(Buffer.from("client"), "_SecItemCopyMatching\n"),
    /SecItemCopyMatching/,
  );
});

test("native credential boundary rejects dynamically resolved Windows credentials", () => {
  assert.throws(
    () => assertNoCredentialStoreEntrypoints(Buffer.from("GetProcAddress\0CredWriteW\0")),
    /CredWriteW/,
  );
});
