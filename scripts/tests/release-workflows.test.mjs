import assert from "node:assert/strict";
import { access, readdir, readFile, stat } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const release = await readFile(new URL(".github/workflows/release.yml", root), "utf8");
const reconcile = await readFile(
  new URL(".github/workflows/reconcile-stable-release.yml", root),
  "utf8",
);
const ci = await readFile(new URL(".github/workflows/ci.yml", root), "utf8");
const trust = JSON.parse(await readFile(new URL("config/release-trust.json", root), "utf8"));
const distribution = JSON.parse(
  await readFile(new URL("config/release-distribution.json", root), "utf8"),
);
const verifier = await readFile(new URL("scripts/verify-release-manifest.mjs", root), "utf8");
const generator = await readFile(new URL("scripts/release-manifest.mjs", root), "utf8");
const comparison = await readFile(
  new URL("scripts/compare-release-identities.mjs", root),
  "utf8",
);

function namedJob(workflow, id) {
  const lines = workflow.split("\n");
  const start = lines.findIndex((line) => line === `  ${id}:`);
  assert.notEqual(start, -1, `workflow is missing job: ${id}`);
  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    if (/^  [A-Za-z0-9_-]+:\s*$/.test(lines[index])) {
      end = index;
      break;
    }
  }
  return lines.slice(start, end).join("\n");
}

function namedStep(workflow, name) {
  const lines = workflow.split("\n");
  const start = lines.findIndex((line) => line.trim() === `- name: ${name}`);
  assert.notEqual(start, -1, `workflow is missing step: ${name}`);
  const indentation = lines[start].match(/^\s*/)[0].length;
  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    if (lines[index].trim() && lines[index].match(/^\s*/)[0].length <= indentation) {
      end = index;
      break;
    }
  }
  return lines.slice(start, end).join("\n");
}

test("every canonical main push derives a unique build-qualified release", () => {
  assert.match(release, /push:\s*\n\s*branches: \[main\]/);
  assert.match(release, /workflow_dispatch:/);
  assert.match(release, /github\.repository == 'tailored-ai-solutions\/codecaddie'/);
  assert.match(release, /github\.ref == 'refs\/heads\/main'/);
  assert.match(release, /build="\$\(node scripts\/release-build-number\.mjs "\$GITHUB_SHA"\)"/);
  assert.match(release, /tag="v\$version\+\$build"/);
  assert.match(release, /Verify exact-commit CI release gates/);
  assert.match(release, /config\/reliability-gates\.json/);
  assert.match(release, /required suite did not pass/);
  assert.match(release, /uses: \.\/\.github\/workflows\/reconcile-stable-release\.yml/);
});

test("the one-time build 2001 cutover may use exact-SHA manual CI but later builds cannot", () => {
  assert.match(ci, /workflow_dispatch:/);
  const gate = namedStep(release, "Verify exact-commit CI release gates");
  assert.match(gate, /head_sha=\$GITHUB_SHA/);
  assert.match(gate, /\.head_branch == "main"/);
  assert.match(gate, /\.event == "push"/);
  assert.match(gate, /\$build == "2001" and \.event == "workflow_dispatch"/);
  assert.equal((gate.match(/\$build == "2001"/g) ?? []).length, 2);
});

test("trusted CI and release runs require the private identifier denylist", () => {
  const prepare = namedJob(release, "prepare");
  assert.match(ci, /if: github\.event_name != 'pull_request'/);
  assert.match(ci, /secrets\.CODECADDIE_PRIVATE_PATTERNS/);
  assert.match(ci, /CODECADDIE_REQUIRE_PRIVATE_PATTERNS=1/);
  assert.match(prepare, /secrets\.CODECADDIE_PRIVATE_PATTERNS/);
  assert.match(prepare, /CODECADDIE_REQUIRE_PRIVATE_PATTERNS=1/);
  assert.doesNotMatch(release.slice(release.indexOf("\n  sbom:")), /CODECADDIE_PRIVATE_PATTERNS/);
  assert.doesNotMatch(`${ci}\n${release}`, /pull_request_target/);
});

test("same-commit retries do not cancel and stable designation uses one global lease", () => {
  const releaseConcurrency = release.slice(
    release.indexOf("concurrency:"),
    release.indexOf("\nenv:"),
  );
  assert.match(releaseConcurrency, /group: \$\{\{ github\.workflow \}\}-\$\{\{ github\.sha \}\}/);
  assert.match(releaseConcurrency, /queue: max/);
  assert.match(releaseConcurrency, /cancel-in-progress: false/);
  assert.match(reconcile, /group: codecaddie-stable-publication/);
  assert.match(reconcile, /queue: max/);
  assert.match(reconcile, /cancel-in-progress: false/);
  assert.match(reconcile, /Latest is selected in that\s*\n# same publication request/);
});

test("release publication is macOS-only while Windows remains a CI platform", () => {
  assert.match(ci, /runs-on: windows-2025/);
  assert.match(ci, /Windows primary test, build, and package/);
  assert.doesNotMatch(release, /windows-latest|CodeCaddie-Windows|windows:x64|\.msi\b/);
  assert.doesNotMatch(reconcile, /windows-latest|CodeCaddie-Windows|windows:x64|\.msi\b/);
  assert.match(release, /CodeCaddie-macOS-universal\.zip/);
  assert.match(release, /macos:arm64:zip:release-candidate\/CodeCaddie-macOS-universal\.zip/);
  assert.match(release, /macos:x64:zip:release-candidate\/CodeCaddie-macOS-universal\.zip/);
  for (const proof of [
    "STAPLED_NOTARIZED_ARCHIVE",
    "lipo -archs",
    "arm64",
    "x86_64",
    "codesign --verify",
    "stapler validate",
    "spctl --assess",
  ]) {
    assert.ok(release.includes(proof), `macOS release proof is missing ${proof}`);
  }
});

test("Apple import credential is isolated from signing and publication jobs", () => {
  const macos = namedJob(release, "macos");
  const manifest = namedJob(release, "manifest");
  const publish = namedJob(release, "publish-release");
  assert.match(macos, /environment:\s*\n\s*name: release-apple/);
  assert.match(macos, /secrets\.APP_STORE_CONNECT_PRIVATE_KEY_BASE64/);
  assert.match(macos, /vars\.APP_STORE_CONNECT_KEY_ID/);
  assert.doesNotMatch(macos, /APP_STORE_CONNECT_(?:ISSUER_ID|KEY_SCOPE)/);
  for (const job of [manifest, publish, reconcile]) {
    assert.doesNotMatch(job, /APP_STORE_CONNECT|release-apple/);
  }
  assert.doesNotMatch(release, /APPLE_CERTIFICATE_P12|APPLE_NOTARY_PRIVATE_KEY/);
});

test("only the manifest job receives OIDC and it creates both keyless proof systems", () => {
  const manifest = namedJob(release, "manifest");
  const publish = namedJob(release, "publish-release");
  assert.equal((release.match(/id-token: write/g) ?? []).length, 1);
  assert.match(manifest, /id-token: write/);
  assert.match(manifest, /attestations: write/);
  assert.match(manifest, /sigstore\/cosign-installer@[a-f0-9]{40}/);
  assert.match(manifest, /cosign sign-blob --yes/);
  assert.match(manifest, /--bundle release-candidate\/manifest\.sigstore\.json/);
  assert.match(manifest, /actions\/attest-build-provenance@[a-f0-9]{40}/);
  assert.match(manifest, /Reuse an existing release-local attestation bundle on retry/);
  assert.match(manifest, /release-attestations\.jsonl/);
  assert.doesNotMatch(publish, /id-token: write|attest-build-provenance/);
  assert.match(publish, /attestations: read/);
});

test("manifest verification pins GitHub workflow identity and source commit", () => {
  assert.match(verifier, /verify-blob/);
  for (const option of [
    "--certificate-identity",
    "--certificate-oidc-issuer",
    "--certificate-github-workflow-repository",
    "--certificate-github-workflow-ref",
    "--certificate-github-workflow-sha",
    "--certificate-github-workflow-trigger",
  ]) {
    assert.ok(verifier.includes(option), `Sigstore verifier is missing ${option}`);
  }
  assert.match(verifier, /manifest\.sourceCommit/);
  assert.match(verifier, /sourceCommit = \/\^\[a-f0-9\]\{40\}\$\//);
  for (const forbidden of [
    /AZURE_/,
    /azure\/login/,
    /artifact-signing-action/,
    /BLOB_READ_WRITE_TOKEN/,
    /vercel blob/,
    /CODECADDIE_RELEASE_PRIVATE_KEY/,
    /release-public-key\.json/,
    /manifest\.sig(?:\s|$)/,
  ]) {
    assert.doesNotMatch(`${release}\n${reconcile}`, forbidden);
  }
});

test("trust is either the fail-closed staging sentinel or the canonical numeric repository ID", () => {
  assert.equal(trust.schemaVersion, 2);
  assert.deepEqual(Object.keys(trust.sigstore).sort(), [
    "allowedTriggers",
    "bundleMediaType",
    "certificateIdentity",
    "oidcIssuer",
    "repository",
    "repositoryId",
    "tufMirror",
    "workflowRef",
  ]);
  assert.equal(trust.sigstore.repository, "tailored-ai-solutions/codecaddie");
  assert.equal(
    trust.sigstore.certificateIdentity,
    "https://github.com/tailored-ai-solutions/codecaddie/.github/workflows/release.yml@refs/heads/main",
  );
  assert.equal(trust.sigstore.oidcIssuer, "https://token.actions.githubusercontent.com");
  assert.equal(
    trust.sigstore.bundleMediaType,
    "application/vnd.dev.sigstore.bundle.v0.3+json",
  );
  assert.match(
    trust.sigstore.repositoryId,
    /^(?:REPLACE_WITH_NEW_PUBLIC_REPOSITORY_ID|[1-9]\d*)$/,
  );
  for (const workflow of [release, reconcile]) {
    assert.match(workflow, /\[\[ "\$configured_repository_id" =~ \^\[1-9\]\[0-9\]\*\$ \]\]/);
    assert.match(workflow, /test "\$configured_repository_id" = "\$ACTUAL_REPOSITORY_ID"/);
  }
});

test("distribution is GitHub Releases only and starts at the 0.4.0 trust boundary", () => {
  assert.deepEqual(distribution, {
    schemaVersion: 2,
    githubRepository: "tailored-ai-solutions/codecaddie",
    latestReleaseApiUrl:
      "https://api.github.com/repos/tailored-ai-solutions/codecaddie/releases/latest",
    releaseDownloadBaseUrl:
      "https://github.com/tailored-ai-solutions/codecaddie/releases/download",
    manifestAssetName: "manifest.json",
    manifestBundleAssetName: "manifest.sigstore.json",
    macosUniversalAssetName: "CodeCaddie-macOS-universal.zip",
  });
  assert.doesNotMatch(JSON.stringify(distribution), /vercel|blob/i);
  assert.match(generator, /options\.minimum \?\?= "0\.4\.0"/);
  assert.match(generator, /schemaVersion: 2/);
  assert.match(generator, /sourceRepository: options\.repository/);
  assert.match(generator, /sourceCommit: options\["source-commit"\]/);
  assert.doesNotMatch(generator, /keyId|trustPolicy|privateKey/);
});

test("release inventory includes CycloneDX, keyless bundle, and attestations", () => {
  assert.match(release, /anchore\/sbom-action@[0-9a-f]{40}/);
  assert.match(release, /format: cyclonedx-json/);
  assert.match(release, /codecaddie-\$\{\{ needs\.prepare\.outputs\.version \}\}\.cdx\.json/);
  const inventory = namedStep(release, "Verify exact candidate inventory");
  for (const name of [
    "CodeCaddie-macOS-universal.zip",
    "RUST-DEPENDENCY-LICENSES.md",
    "SHA256SUMS.txt",
    "codecaddie-$RELEASE_VERSION.cdx.json",
    "dependency-license-exceptions.json",
    "manifest.json",
    "manifest.sigstore.json",
    "release-attestations.jsonl",
    "xcode-cloud-provenance.json",
  ]) {
    assert.ok(inventory.includes(name), `candidate inventory is missing ${name}`);
  }
});

test("every third-party action is pinned to exactly one commit SHA across all workflows", async () => {
  const directory = new URL(".github/workflows/", root);
  const pins = new Map();
  for (const name of (await readdir(directory)).filter((entry) => entry.endsWith(".yml")).sort()) {
    const workflow = await readFile(new URL(name, directory), "utf8");
    for (const [, action, ref] of workflow.matchAll(/^\s+(?:- )?uses:\s*([^@\s]+)@(\S+)\s*$/gm)) {
      if (action.startsWith("./")) continue;
      assert.match(ref, /^[0-9a-f]{40}$/, `${name}: ${action} must be pinned to a full commit SHA`);
      const seen = pins.get(action) ?? new Set();
      seen.add(ref);
      pins.set(action, seen);
    }
  }
  assert.ok(pins.size >= 8, "expected the workflows to use third-party actions");
  for (const [action, refs] of pins) {
    assert.equal(refs.size, 1, `${action} is pinned to different SHAs: ${[...refs].join(", ")}`);
  }
});

test("publisher creates a complete draft and publishes only beta directly", () => {
  // The repository setting is admin-only and unreadable by the workflow token,
  // so immutability is asserted on the release object itself, before and after publication.
  assert.doesNotMatch(release, /immutable-releases/);
  assert.doesNotMatch(reconcile, /immutable-releases/);
  assert.match(release, /jq -er \.immutable candidate-release\.json/);
  assert.match(release, /isImmutable/);
  assert.match(reconcile, /jq -er \.immutable requested-release-before-publication\.json/);
  assert.match(release, /gh release create "\$RELEASE_TAG"/);
  assert.match(release, /--draft/);
  assert.match(release, /--target "\$GITHUB_SHA"/);
  assert.match(release, /cmp release-candidate\/manifest\.json existing-signature\/manifest\.json/);
  assert.match(release, /gh release upload "\$RELEASE_TAG" "release-candidate\/\$name"/);
  assert.match(release, /gh release edit "\$RELEASE_TAG" --draft=false --prerelease --latest=false/);
  assert.doesNotMatch(release, /gh release edit "\$RELEASE_TAG" --draft=false --prerelease=false/);
});

test("reconciliation scans all published stable tags and decides before immutable publication", () => {
  assert.match(reconcile, /workflow_call:/);
  assert.match(reconcile, /gh api --paginate --slurp "repos\/\$GITHUB_REPOSITORY\/releases\?per_page=100"/);
  assert.match(reconcile, /node scripts\/compare-release-identities\.mjs/);
  assert.match(comparison, /parseStableVersion/);
  assert.match(comparison, /left\.build - right\.build/);
  assert.match(reconcile, /if \[\[ "\$comparison" -gt 0 \]\]; then/);
  assert.match(reconcile, /if \[\[ "\$comparison" -lt 0 \]\]; then\s*\n\s*publish_as_latest=0/);
  assert.match(reconcile, /gh release edit "\$RELEASE_TAG" --draft=false --prerelease=false --latest/);
  assert.match(reconcile, /gh release edit "\$RELEASE_TAG" --draft=false --prerelease=false --latest=false/);
  assert.match(reconcile, /test "\$observed_latest" = "\$PREVIOUS_LATEST_TAG"/);
  assert.equal((reconcile.match(/gh release edit/g) ?? []).length, 2);
  assert.doesNotMatch(reconcile, /gh release edit "\$(?:highest|previous|selected)/i);
  assert.doesNotMatch(reconcile, /inputs\.mode|rollback|stableManifestUrl|stableSignatureUrl/);
});

test("reconciliation verifies the trigger, published high-water, and public bytes", () => {
  assert.match(reconcile, /Download and independently verify the triggering stable draft/);
  assert.match(reconcile, /Verify the published stable high-water mark/);
  assert.doesNotMatch(reconcile, /release-control|check-release-control/);
  assert.match(reconcile, /gh attestation verify "requested-release\/\$artifact"/);
  assert.match(reconcile, /gh attestation verify "highest-release\/\$artifact"/);
  assert.match(reconcile, /--source-digest "\$highest_sha"/);
  assert.match(reconcile, /Verify public bytes, tag identity, and immutable postconditions/);
  assert.match(
    reconcile,
    /https:\/\/github\.com\/\$GITHUB_REPOSITORY\/releases\/download\/\$RELEASE_TAG\/\$name/,
  );
  const decision = reconcile.indexOf("Verify the published stable high-water mark");
  const mutation = reconcile.indexOf(
    "Publish once with the high-water decision in the immutable request",
  );
  assert.ok(decision < mutation);
});

test("obsolete manual promotion, rollback, and fixed-key scripts are absent", async () => {
  for (const path of [
    ".github/workflows/promote-release.yml",
    ".github/workflows/rollback-release.yml",
    "config/release-control.json",
    "scripts/check-release-control.mjs",
    "scripts/release-key.mjs",
  ]) {
    await assert.rejects(access(new URL(path, root)));
  }
});

test("release command-line scripts remain executable", async () => {
  for (const path of [
    "scripts/release-build-number.mjs",
    "scripts/release-manifest.mjs",
    "scripts/verify-release-manifest.mjs",
    "scripts/compare-release-identities.mjs",
    "scripts/verify-public-root.mjs",
  ]) {
    const details = await stat(new URL(path, root));
    assert.notEqual(details.mode & 0o111, 0, `${path} must be executable`);
  }
});
