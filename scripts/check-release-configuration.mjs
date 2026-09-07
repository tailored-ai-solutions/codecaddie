#!/usr/bin/env node

import { createPrivateKey } from "node:crypto";
import { pathToFileURL } from "node:url";

// Offline only: configuration validity cannot establish Apple API authorization,
// workflow readiness, notarization, or publication. Never return input values or
// crypto parser errors: they can contain credential material.
export function checkReleaseConfiguration(environment) {
  const failures = [];
  for (const [name, pattern, description] of [
    ["CODECADDIE_APPLE_TEAM_ID", /^[A-Z0-9]{10}$/, "a ten-character Apple team ID"],
    ["CODECADDIE_XCODE_CLOUD_WORKFLOW_ID", /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i, "an Xcode Cloud workflow UUID"],
    ["APP_STORE_CONNECT_KEY_ID", /^[A-Z0-9]{10,12}$/, "the individual App Store Connect key ID"],
  ]) {
    if (!environment[name]) failures.push(`Missing variable ${name}; set ${description} in release-apple.`);
    else if (!pattern.test(environment[name])) failures.push(`Invalid variable ${name}; expected ${description} in release-apple.`);
  }

  const name = "APP_STORE_CONNECT_PRIVATE_KEY_BASE64";
  const encoded = environment[name];
  // The validator's sole credential is removed before parsing or reporting.
  delete environment[name];
  if (!encoded) {
    failures.push(`Missing secret ${name}; add the base64-encoded individual API .p8 key to release-apple.`);
  } else {
    let bytes;
    try {
      // Bound input and reject the invalid characters Node's decoder ignores.
      if (encoded.length > 16_384) throw new Error();
      const compact = encoded.replace(/\s/g, "");
      if (!compact || compact.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(compact)) throw new Error();
      bytes = Buffer.from(compact, "base64");
      if (bytes.toString("base64") !== compact) throw new Error();
      const key = createPrivateKey(bytes);
      if (key.asymmetricKeyType !== "ec" || key.asymmetricKeyDetails?.namedCurve !== "prime256v1") throw new Error();
    } catch {
      failures.push(`Invalid secret ${name}; replace it with the base64-encoded, unencrypted P-256 individual API .p8 key in release-apple.`);
    } finally {
      bytes?.fill(0);
    }
  }
  return failures;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const failures = checkReleaseConfiguration(process.env);
  for (const failure of failures) console.error(`::error::${failure} See docs/RELEASING.md#release-configuration-preflight.`);
  if (failures.length) process.exitCode = 1;
  else console.log("Release configuration passed offline validation. Apple access and exact-commit archive verification remain required.");
}
