import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
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
    console.log(JSON.stringify(state.pages ?? [state.releases]));
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
  fs.writeFileSync(process.env.FAKE_GH_STATE, JSON.stringify(state));
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
  await writeFile(path.join(directory, "state.json"), JSON.stringify(state));
  await writeFile(path.join(directory, "calls.jsonl"), "");
  await writeFile(path.join(directory, "CHANGELOG.md"), "## [0.4.0]\n\nRelease fixture.\n");
  await symlink(path.join(root, "scripts"), path.join(directory, "scripts"));
  return {
    directory,
    env: {
      ...process.env,
      PATH: `${directory}${path.delimiter}${process.env.PATH}`,
      FAKE_GH_STATE: path.join(directory, "state.json"),
      FAKE_GH_CALLS: path.join(directory, "calls.jsonl"),
      GITHUB_REPOSITORY: "example/project",
      GITHUB_SHA: sha,
      GITHUB_OUTPUT: path.join(directory, "output.txt"),
      RELEASE_TAG: tag,
      RELEASE_VERSION: "0.4.0",
    },
  };
}

async function stepRun(workflow, name) {
  const text = await readFile(path.join(root, ".github/workflows", workflow), "utf8");
  const start = text.indexOf(`      - name: ${name}\n`);
  assert.ok(start >= 0);
  const end = text.indexOf("\n      - ", start + 1);
  const step = text.slice(start, end < 0 ? undefined : end);
  return step.split("        run: |\n")[1].split("\n")
    .filter(line => line.startsWith("          ")).map(line => line.slice(10)).join("\n");
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
