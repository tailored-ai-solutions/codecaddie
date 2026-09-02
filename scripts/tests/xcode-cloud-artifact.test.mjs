import assert from "node:assert/strict";
import { generateKeyPairSync, verify } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  createAppStoreConnectToken,
  formatGitHubOutput,
  selectExactCommitBuild,
  selectStapledArtifact,
} from "../fetch-xcode-cloud-artifact.mjs";

const commit = "0123456789abcdef0123456789abcdef01234567";
const fetcherSource = await readFile(new URL("../fetch-xcode-cloud-artifact.mjs", import.meta.url), "utf8");

test("exact-commit Xcode Cloud selection never falls back to a different revision", () => {
  const selected = selectExactCommitBuild([
    { id: "other", attributes: { number: 99, sourceCommit: { commitSha: "f".repeat(40) } } },
    { id: "older", attributes: { number: 7, sourceCommit: { commitSha: commit } } },
    { id: "newer", attributes: { number: 8, sourceCommit: { commitSha: commit } } },
  ], commit);
  assert.equal(selected.id, "newer");
  assert.equal(selectExactCommitBuild([], commit), null);
});

test("stapled artifact selection is singular and rejects unsafe metadata", () => {
  const artifact = {
    id: "artifact",
    attributes: {
      fileType: "STAPLED_NOTARIZED_ARCHIVE",
      fileName: "CodeCaddie.zip",
      fileSize: 42,
      downloadUrl: "https://example.invalid/CodeCaddie.zip",
    },
  };
  assert.equal(selectStapledArtifact([artifact]), artifact);
  assert.throws(() => selectStapledArtifact([]), /exactly one/);
  assert.throws(() => selectStapledArtifact([artifact, artifact]), /exactly one/);
  assert.throws(
    () => selectStapledArtifact([{ ...artifact, attributes: { ...artifact.attributes, fileName: "../escape.zip" } }]),
    /unsafe filename/,
  );
  for (const fileName of [".", "..", "CodeCaddie\nprovenance_path=escape.zip", "Code Caddie.zip", "CodeCaddie.dmg"]) {
    assert.throws(
      () => selectStapledArtifact([{ ...artifact, attributes: { ...artifact.attributes, fileName } }]),
      /unsafe filename/,
    );
  }
  assert.throws(
    () => selectStapledArtifact([{ ...artifact, attributes: { ...artifact.attributes, fileName: `${"a".repeat(197)}.zip` } }]),
    /unsafe filename/,
  );
  assert.throws(
    () => selectStapledArtifact([{ ...artifact, attributes: { ...artifact.attributes, downloadUrl: "http://example.invalid/app.zip" } }]),
    /no HTTPS download URL/,
  );
  assert.throws(
    () => selectStapledArtifact([{ ...artifact, attributes: { ...artifact.attributes, fileSize: 2 * 1024 * 1024 * 1024 + 1 } }]),
    /invalid size/,
  );
});

test("GitHub output serialization rejects command injection characters", () => {
  assert.equal(formatGitHubOutput("artifact_path", "/tmp/CodeCaddie.zip"), "artifact_path=/tmp/CodeCaddie.zip\n");
  assert.throws(() => formatGitHubOutput("artifact-path", "/tmp/app.zip"), /unsafe GitHub output name/);
  for (const value of ["/tmp/app.zip\nprovenance_path=escape", "/tmp/app.zip\rbad=1", "/tmp/app.zip\0bad"]) {
    assert.throws(() => formatGitHubOutput("artifact_path", value), /unsafe GitHub output value/);
  }
});

test("individual App Store Connect token uses sub user and one exact read-only scope", () => {
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const token = createAppStoreConnectToken({
    keyId: "ABCDEFGHIJ",
    privateKey,
    requestPath: "/v1/ciWorkflows/workflow/buildRuns?fields%5BciBuildRuns%5D=number&limit=200",
    now: Date.UTC(2026, 7, 30),
  });
  const [header, payload, signature] = token.split(".");
  assert.deepEqual(JSON.parse(Buffer.from(header, "base64url")), {
    alg: "ES256",
    kid: "ABCDEFGHIJ",
    typ: "JWT",
  });
  const claims = JSON.parse(Buffer.from(payload, "base64url"));
  assert.equal(claims.aud, "appstoreconnect-v1");
  assert.equal(claims.sub, "user");
  assert.equal(claims.iss, undefined);
  assert.deepEqual(claims.scope, [
    "GET /v1/ciWorkflows/workflow/buildRuns?fields%5BciBuildRuns%5D=number&limit=200",
  ]);
  assert.equal(claims.exp - claims.iat, 900);
  assert.ok(verify("sha256", Buffer.from(`${header}.${payload}`), {
    key: publicKey,
    dsaEncoding: "ieee-p1363",
  }, Buffer.from(signature, "base64url")));
});

test("artifact-reader tokens cannot be minted without one safe API request path", () => {
  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  for (const requestPath of [undefined, "", "/v2/apps", "https://example.invalid/v1/apps", "/v1/apps\nPOST /v1/apps"]) {
    assert.throws(
      () => createAppStoreConnectToken({ keyId: "ABCDEFGHIJ", privateKey, requestPath }),
      /safe v1 request path/,
    );
  }
});

test("artifact retrieval uses only Apple's ciArtifacts read endpoint", () => {
  assert.match(
    fetcherSource,
    /\/v1\/ciBuildActions\/\$\{encodeURIComponent\(action\.id\)\}\/artifacts\?fields\[ciArtifacts\]=fileType,fileName,fileSize,downloadUrl/,
  );
  assert.match(fetcherSource, /scope: \[`GET \$\{requestUrl\.pathname\}\$\{requestUrl\.search\}`\]/);
  assert.doesNotMatch(fetcherSource, /method:\s*["'`](?:POST|PUT|PATCH|DELETE)["'`]/);
  assert.doesNotMatch(fetcherSource, /APP_STORE_CONNECT_(?:ISSUER_ID|KEY_SCOPE)/);
});

test("a twelve-character individual key ID is accepted and a malformed one is rejected", () => {
  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  assert.ok(createAppStoreConnectToken({ keyId: "ABCDEFGHIJKL", privateKey, requestPath: "/v1/ciBuildRuns/1" }));
  assert.throws(
    () => createAppStoreConnectToken({ keyId: "abc", privateKey, requestPath: "/v1/ciBuildRuns/1" }),
    /invalid App Store Connect key ID/,
  );
});
