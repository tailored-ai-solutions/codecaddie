import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, rename, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../../", import.meta.url));
const tag = "v0.4.0+2001";
const sha = "a".repeat(40);
const release = (overrides = {}) => ({
  id: 42,
  tag_name: tag,
  target_commitish: sha,
  draft: true,
  prerelease: false,
  immutable: false,
  assets: [],
  ...overrides,
});

const fakeGh = `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
const state = JSON.parse(fs.readFileSync(process.env.FAKE_GH_STATE, "utf8"));
fs.appendFileSync(process.env.FAKE_GH_CALLS, JSON.stringify(args) + "\\n");
if (state.error) { console.error(state.error); process.exit(1); }
if (args[0] === "api") {
  const endpoint = args.find(value => value.startsWith("repos/"));
  if (endpoint.endsWith("/releases/latest")) {
    if (!state.latest) { console.error("gh: Not Found (HTTP 404)"); process.exit(1); }
    console.log(args.includes("--jq") ? state.latest : JSON.stringify({ tag_name: state.latest }));
  } else if (endpoint.endsWith("/releases?per_page=100")) {
    if (state.created && state.hiddenReads > 0) {
      state.hiddenReads -= 1;
      fs.writeFileSync(process.env.FAKE_GH_STATE, JSON.stringify(state));
      console.log("[[]]");
    } else {
      console.log(JSON.stringify(state.pages ?? [state.releases]));
    }
  } else if (endpoint.includes("/releases/tags/")) {
    const found = state.releases.find(value => value.tag_name === endpoint.split("/tags/")[1] && !value.draft);
    if (!found) { console.error("gh: Not Found (HTTP 404)"); process.exit(1); }
    console.log(JSON.stringify(found));
  } else { console.error("unexpected API call"); process.exit(1); }
} else if (args[0] === "release" && args[1] === "create") {
  if (state.releases.some(value => value.tag_name === args[2])) {
    console.error("release already exists"); process.exit(1);
  }
  state.releases.push({ id: 43, tag_name: args[2], target_commitish: args[args.indexOf("--target") + 1], draft: true, prerelease: false, immutable: false, assets: [] });
  state.created = true;
  Object.assign(state, state.afterCreate ?? {});
  fs.writeFileSync(process.env.FAKE_GH_STATE, JSON.stringify(state));
} else if (args[0] === "release" && args[1] === "download") {
  const name = args[args.indexOf("--pattern") + 1];
  if (typeof state.files?.[name] !== "string") { console.error("asset download failed"); process.exit(1); }
  fs.writeFileSync(require("node:path").join(args[args.indexOf("--dir") + 1], name), state.files[name]);
} else if (args[0] === "release" && args[1] === "edit") {
  const found = state.releases.find(value => value.tag_name === args[2]);
  if (!found || !found.draft) { console.error("cannot mutate a published release"); process.exit(1); }
  found.draft = false;
  found.immutable = true;
  if (args.includes("--latest")) state.latest = found.tag_name;
  fs.writeFileSync(process.env.FAKE_GH_STATE, JSON.stringify(state));
} else { console.error("unexpected gh call"); process.exit(1); }
`;

async function fixture(t, state) {
  const directory = await mkdtemp(path.join(tmpdir(), "codecaddie-release-lookup-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await writeFile(path.join(directory, "gh"), fakeGh, { mode: 0o700 });
  await writeFile(path.join(directory, "git"), '#!/usr/bin/env node\nprocess.stdout.write(process.env.GITHUB_SHA + "\\n");\n', { mode: 0o700 });
  await writeFile(path.join(directory, "state.json"), JSON.stringify(state));
  await writeFile(path.join(directory, "calls.jsonl"), "");
  await writeFile(path.join(directory, "sleeps.jsonl"), "");
  await writeFile(path.join(directory, "sleep"), '#!/usr/bin/env node\nrequire("node:fs").appendFileSync(process.env.FAKE_SLEEP_CALLS, JSON.stringify(process.argv.slice(2)) + "\\n");\n', { mode: 0o700 });
  await writeFile(path.join(directory, "CHANGELOG.md"), "## [0.4.0]\n\nRelease fixture.\n");
  await symlink(path.join(root, "scripts"), path.join(directory, "scripts"));
  return {
    directory,
    env: {
      ...process.env,
      PATH: `${directory}${path.delimiter}${process.env.PATH}`,
      FAKE_GH_STATE: path.join(directory, "state.json"),
      FAKE_GH_CALLS: path.join(directory, "calls.jsonl"),
      FAKE_SLEEP_CALLS: path.join(directory, "sleeps.jsonl"),
      GITHUB_REPOSITORY: "example/project",
      GITHUB_SHA: sha,
      GITHUB_OUTPUT: path.join(directory, "output.txt"),
      RELEASE_TAG: tag,
      RELEASE_VERSION: "0.4.0",
    },
  };
}

async function stepRun(workflow, name) {
  const lines = (await readFile(path.join(root, ".github/workflows", workflow), "utf8")).split("\n");
  const start = lines.findIndex(line => [`- name: ${name}`, `name: ${name}`].includes(line.trim()));
  assert.ok(start >= 0);
  const run = lines.findIndex((line, index) => index > start && line.trim() === "run: |");
  assert.ok(run > start);
  let end = run + 1;
  while (end < lines.length && (!lines[end].trim() || lines[end].startsWith("          "))) end += 1;
  return lines.slice(run + 1, end).map(line => line.slice(10)).join("\n");
}

for (const [name, initial, expectedDraft] of [
  ["creates and finds a new draft", [], true],
  ["resumes an existing draft without creating another", [release()], true],
  ["reuses a published immutable release with draft false", [release({ draft: false, immutable: true })], false],
]) {
  test(`publication ${name}`, async (t) => {
    const { directory, env } = await fixture(t, { releases: initial });
    const script = await stepRun("release.yml", "Create or resume the exact-SHA draft");
    const result = spawnSync("bash", ["-c", script], { cwd: directory, env, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await readFile(env.GITHUB_OUTPUT, "utf8"), `was_draft=${expectedDraft}\n`);
    const state = JSON.parse(await readFile(env.FAKE_GH_STATE, "utf8"));
    assert.equal(state.releases.length, 1);
    assert.equal(state.releases[0].target_commitish, sha);
  });
}

for (const [name, state] of [
  ["a draft for another commit", { releases: [release({ target_commitish: "b".repeat(40) })] }],
  ["a published mutable release", { releases: [release({ draft: false })] }],
  ["an API failure", { releases: [], error: "gh: Bad credentials (HTTP 401)" }],
  ["an API 404", { releases: [], error: "gh: Not Found (HTTP 404)" }],
  ["duplicate releases for the tag", { releases: [release(), release({ id: 43 })] }],
  ["duplicate asset names", { releases: [release({ assets: [{ id: 1, name: "manifest.json" }, { id: 2, name: "manifest.json" }] })] }],
]) {
  test(`publication fails closed for ${name}`, async (t) => {
    const { directory, env } = await fixture(t, state);
    const script = await stepRun("release.yml", "Create or resume the exact-SHA draft");
    const result = spawnSync("bash", ["-c", script], { cwd: directory, env, encoding: "utf8" });
    assert.notEqual(result.status, 0);
    const calls = await readFile(env.FAKE_GH_CALLS, "utf8");
    assert.doesNotMatch(calls, /"create"/);
  });
}

for (const draft of [true, false]) {
  test(`lookup finds draft=${draft} on a later page and preserves REST fields`, async (t) => {
    const expected = release({ draft, immutable: !draft });
    const { directory, env } = await fixture(t, { pages: [[release({ tag_name: "v0.3.0+1" })], [expected]] });
    const result = spawnSync(process.execPath, [path.join(root, "scripts/find-github-release.mjs"), "example/project", tag, "--required"], { cwd: directory, env, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), expected);
  });
}

test("only a successful complete list can report an optional release absent", async (t) => {
  const { directory, env } = await fixture(t, { releases: [] });
  const args = [path.join(root, "scripts/find-github-release.mjs"), "example/project", tag];
  const optional = spawnSync(process.execPath, args, { cwd: directory, env, encoding: "utf8" });
  assert.equal(optional.status, 0, optional.stderr);
  assert.equal(JSON.parse(optional.stdout), null);
  const required = spawnSync(process.execPath, [...args, "--required"], { cwd: directory, env, encoding: "utf8" });
  assert.notEqual(required.status, 0);
});

for (const [name, initial, latest, expectedStatus, expectedEdits] of [
  ["publishes a mutable draft once", release(), null, 0, 1],
  ["reuses an immutable published release", release({ draft: false, immutable: true }), tag, 0, 0],
  ["rejects a mutable published release", release({ draft: false }), tag, 1, 0],
]) {
  test(`stable reconciliation ${name}`, async (t) => {
    const { directory, env } = await fixture(t, { releases: [initial], latest });
    await writeFile(path.join(directory, "requested-expected-assets.txt"), "");
    env.PREVIOUS_LATEST_TAG = latest ?? "";
    env.PUBLISH_AS_LATEST = "1";
    const script = await stepRun("reconcile-stable-release.yml", "Publish once with the high-water decision in the immutable request");
    const result = spawnSync("bash", ["-c", script], { cwd: directory, env, encoding: "utf8" });
    assert.equal(result.status, expectedStatus, result.stderr);
    const calls = (await readFile(env.FAKE_GH_CALLS, "utf8")).trim().split("\n").map(line => JSON.parse(line));
    assert.equal(calls.filter(args => args[0] === "release" && args[1] === "edit").length, expectedEdits);
  });
}

for (const [name, state] of [
  ["missing pagination envelope", { pages: [release()] }],
  ["malformed page", { pages: [[release()], null] }],
  ["missing draft flag", { releases: [release({ draft: undefined })] }],
  ["nonboolean draft flag", { releases: [release({ draft: "false" })] }],
]) {
  test(`lookup rejects ${name}`, async (t) => {
    const { directory, env } = await fixture(t, state);
    const result = spawnSync(process.execPath, [path.join(root, "scripts/find-github-release.mjs"), "example/project", tag], { cwd: directory, env, encoding: "utf8" });
    assert.notEqual(result.status, 0);
    assert.equal(result.stdout, "");
  });
}

test("protected snapshot captures only fixed retry assets and the read-only signer reuses their bytes offline", async (t) => {
  const files = {
    "codecaddie-0.4.0.cdx.json": '{"bomFormat":"CycloneDX","specVersion":"1.6","serialNumber":"original"}\n',
    "manifest.json": "original manifest bytes\n",
    "manifest.sigstore.json": "original Sigstore bytes\n",
    "release-attestations.jsonl": "original attestation bytes\n",
    "CodeCaddie-macOS-universal.zip": "not part of the retry snapshot",
  };
  const metadata = release({ assets: Object.keys(files).map((name, index) => ({ id: index + 1, name })) });
  const { directory, env } = await fixture(t, { releases: [metadata], files });
  const snapshot = await stepRun("release.yml", "Snapshot the exact release and reusable proof bytes");
  const captured = spawnSync("bash", ["-c", snapshot], { cwd: directory, env, encoding: "utf8" });
  assert.equal(captured.status, 0, captured.stderr);
  const expected = Object.keys(files).filter(name => !name.endsWith(".zip"));
  assert.deepEqual((await readdir(path.join(directory, "release-state"))).sort(), [...expected, "release.json"].sort());
  for (const name of expected) assert.equal(await readFile(path.join(directory, "release-state", name), "utf8"), files[name]);
  assert.deepEqual(JSON.parse(await readFile(path.join(directory, "release-state/release.json"), "utf8")), metadata);

  await rename(path.join(directory, "release-state"), path.join(directory, "existing-release"));
  await mkdir(path.join(directory, "release-candidate"));
  await writeFile(env.FAKE_GH_STATE, JSON.stringify({ error: "network access forbidden in signer retry steps" }));
  const callsBefore = await readFile(env.FAKE_GH_CALLS, "utf8");
  for (const name of ["Reuse the exact published SBOM bytes on retry", "Reuse an existing release-local attestation bundle on retry"]) {
    const script = await stepRun("release.yml", name);
    const result = spawnSync("bash", ["-c", script], { cwd: directory, env, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
  }
  assert.equal(await readFile(path.join(directory, "release-candidate/codecaddie-0.4.0.cdx.json"), "utf8"), files["codecaddie-0.4.0.cdx.json"]);
  assert.equal(await readFile(path.join(directory, "release-candidate/release-attestations.jsonl"), "utf8"), files["release-attestations.jsonl"]);
  assert.equal(await readFile(env.GITHUB_OUTPUT, "utf8"), "reuse=true\n");
  assert.equal(await readFile(env.FAKE_GH_CALLS, "utf8"), callsBefore);
});

for (const [name, metadata, files] of [
  ["another commit", release({ target_commitish: "b".repeat(40) }), {}],
  ["a mutable published release", release({ draft: false }), {}],
  ["a listed asset that cannot be downloaded", release({ assets: [{ id: 1, name: "manifest.json" }] }), {}],
]) {
  test(`protected snapshot rejects ${name}`, async (t) => {
    const { directory, env } = await fixture(t, { releases: [metadata], files });
    const script = await stepRun("release.yml", "Snapshot the exact release and reusable proof bytes");
    const result = spawnSync("bash", ["-c", script], { cwd: directory, env, encoding: "utf8" });
    assert.notEqual(result.status, 0);
  });
}

for (const [name, hiddenReads, expectedStatus, expectedLookups, expectedSleeps] of [
  ["waits for the newly created draft to appear", 2, 0, 4, 2],
  ["stops when the newly created draft stays invisible", 20, 1, 13, 11],
]) {
  test(`publication ${name}`, async (t) => {
    const { directory, env } = await fixture(t, { releases: [], hiddenReads });
    const script = await stepRun("release.yml", "Create or resume the exact-SHA draft");
    const result = spawnSync("bash", ["-c", script], { cwd: directory, env, encoding: "utf8" });
    assert.equal(result.status, expectedStatus, result.stderr);
    const calls = (await readFile(env.FAKE_GH_CALLS, "utf8")).trim().split("\n").map(line => JSON.parse(line));
    assert.equal(calls.filter(args => args[0] === "release" && args[1] === "create").length, 1);
    assert.equal(calls.filter(args => args[0] === "api").length, expectedLookups);
    const sleeps = (await readFile(env.FAKE_SLEEP_CALLS, "utf8")).trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
    assert.deepEqual(sleeps, Array.from({ length: expectedSleeps }, () => ["5"]));
    if (expectedStatus === 0) {
      assert.equal(await readFile(env.GITHUB_OUTPUT, "utf8"), "was_draft=true\n");
    } else {
      assert.match(result.stderr, /Created draft did not become visible after 12 lookups/);
    }
  });
}

for (const [name, afterCreate] of [
  ["an authorization error", { error: "gh: Forbidden (HTTP 403)" }],
  ["an API 404", { error: "gh: Not Found (HTTP 404)" }],
  ["duplicate tags", { releases: [release(), release({ id: 43 })] }],
  ["malformed metadata", { releases: [release({ draft: "true" })] }],
  ["another commit", { releases: [release({ target_commitish: "b".repeat(40) })] }],
  ["a mutable published release", { releases: [release({ draft: false })] }],
]) {
  test(`publication does not retry ${name} after creation`, async (t) => {
    const { directory, env } = await fixture(t, { releases: [], afterCreate });
    const script = await stepRun("release.yml", "Create or resume the exact-SHA draft");
    const result = spawnSync("bash", ["-c", script], { cwd: directory, env, encoding: "utf8" });
    assert.notEqual(result.status, 0);
    const calls = (await readFile(env.FAKE_GH_CALLS, "utf8")).trim().split("\n").map(line => JSON.parse(line));
    assert.equal(calls.filter(args => args[0] === "release" && args[1] === "create").length, 1);
    assert.equal(calls.filter(args => args[0] === "api").length, 2);
    assert.equal(await readFile(env.FAKE_SLEEP_CALLS, "utf8"), "");
  });
}
