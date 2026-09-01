import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  SYNTHETIC_GOALS_PATH,
  createFixtureRepository,
  decodeSingleFrame,
  encodeFrame,
  loadSyntheticGoals,
  runGit,
} from "../lib/core-harness.mjs";
import {
  SECTIONS,
  exerciseInstalledCore,
  healthPattern,
  parseOnly,
} from "../exercise-installed-core.mjs";

const root = new URL("../../", import.meta.url);
const installedCoreScript = fileURLToPath(new URL("scripts/exercise-installed-core.mjs", root));
const priorBinariesScript = await readFile(
  new URL("scripts/exercise-supported-prior-binaries.mjs", root),
  "utf8",
);
const packageJson = JSON.parse(await readFile(new URL("package.json", root), "utf8"));
const ci = await readFile(new URL(".github/workflows/ci.yml", root), "utf8");
const unixOnly = { skip: process.platform === "win32" ? "fake executables need a POSIX shebang" : false };

test("the synthetic journey goals are one shared, source-free fixture", () => {
  assert.equal(path.relative(fileURLToPath(root), SYNTHETIC_GOALS_PATH), path.join("fixtures", "journeys", "synthetic-goals.json"));
  const goals = loadSyntheticGoals();
  assert.equal(goals.length, 3);
  assert.deepEqual(goals.map(({ position }) => position), [1, 2, 3]);
  assert.deepEqual(
    goals.map(({ rubricDimensions }) => rubricDimensions),
    [["Business & product"], ["Architecture & platform"], ["Operations & reliability"]],
  );
  for (const goal of goals) {
    assert.deepEqual(
      Object.keys(goal).sort(),
      ["businessOutcome", "criteria", "goalId", "position", "priority", "rubricDimensions", "title"],
    );
    assert.match(goal.goalId, /^synthetic-[a-z-]+$/);
    assert.ok(goal.criteria.length >= 2);
    assert.ok(goal.priority >= 1 && goal.priority <= 5);
  }
  const serialized = JSON.stringify(goals).toLowerCase();
  for (const forbidden of ["sourcetext", "attachmentcontent", "goaltext", "prompt", "/users/", "/home/"]) {
    assert.equal(serialized.includes(forbidden), false, `synthetic goals must not carry ${forbidden}`);
  }
  goals[0].title = "mutated";
  assert.notEqual(loadSyntheticGoals()[0].title, "mutated", "each load must return a fresh copy");
});

test("both executable journeys consume the shared harness and fixture", () => {
  assert.match(priorBinariesScript, /from "\.\/lib\/core-harness\.mjs"/);
  assert.match(priorBinariesScript, /loadSyntheticGoals\(\)/);
  assert.doesNotMatch(priorBinariesScript, /function goalDefinitions\(/);
  assert.match(priorBinariesScript, /requireClean/);
  assert.match(priorBinariesScript, /process\.argv\.includes\("--require-clean"\)/);
  assert.match(ci, /node scripts\/exercise-supported-prior-binaries\.mjs --require-clean/);
  assert.equal(
    packageJson.scripts["verify:core"],
    "node scripts/exercise-installed-core.mjs --binary target/debug/codecaddie-core --dev --only ping,journey --json --keep",
  );
});

test("protocol frames round-trip and reject malformed input", () => {
  const fixture = { id: "fixture", protocolVersion: 2, method: "system.ping", params: {} };
  assert.deepEqual(decodeSingleFrame(encodeFrame(fixture)), fixture);
  assert.throws(() => decodeSingleFrame(Buffer.from([0, 0, 0, 8, 123])), /incomplete/);
  assert.throws(() => decodeSingleFrame(Buffer.from([0, 0])), /truncated/);
});

test("section selection accepts only the known sections", () => {
  assert.deepEqual([...parseOnly(undefined)], SECTIONS);
  assert.deepEqual([...parseOnly("health,ping")], ["health", "ping"]);
  assert.deepEqual([...parseOnly(" journey ")], ["journey"]);
  assert.throws(() => parseOnly("health,export"), /--only accepts health, ping, journey; received export/);
  assert.throws(() => parseOnly(","), /at least one section/);
});

test("development builds are admitted only with --dev", () => {
  const developmentLine = "CodeCaddie 0.4.0+0 development\n";
  const releaseLine = "CodeCaddie 0.4.0+2001 0123456789abcdef0123456789abcdef01234567\n";
  assert.match(developmentLine, healthPattern(true));
  assert.doesNotMatch(developmentLine, healthPattern(false));
  assert.match(releaseLine, healthPattern(true));
  assert.match(releaseLine, healthPattern(false));
  assert.doesNotMatch("CodeCaddie 0.4.0+0 development extra\n", healthPattern(true));
});

test("fixture repositories are one commit holding only the caller's canary", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "codecaddie-harness-fixture-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const repository = path.join(directory, "repository");
  const commit = createFixtureRepository(repository, {
    fileName: "fixture.txt",
    contents: "SYNTHETIC CANARY\n",
    userName: "Harness test",
    userEmail: "harness@invalid.example",
    message: "harness fixture",
  });
  assert.match(commit, /^[0-9a-f]{40}$/);
  assert.equal(runGit(repository, ["rev-list", "--count", "HEAD"]), "1");
  assert.equal(runGit(repository, ["ls-files"]), "fixture.txt");
});

async function fakeCore(t, { build = 0, commit = "development", channel = "dev" } = {}) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "codecaddie-fake-core-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const binary = path.join(directory, "codecaddie-core");
  await writeFile(binary, `#!/usr/bin/env node
const chunks = [];
if (process.argv[2] === "--health-check") {
  process.stdout.write("CodeCaddie 0.4.0+${build} ${commit}\\n");
} else {
  process.stdin.on("data", (chunk) => chunks.push(chunk));
  process.stdin.on("end", () => {
    const bytes = Buffer.concat(chunks);
    const request = JSON.parse(bytes.subarray(4, 4 + bytes.readUInt32BE(0)).toString("utf8"));
    const response = {
      id: request.id,
      ok: true,
      result: {
        protocolVersion: 2,
        service: "codecaddie-core",
        build: { version: "0.4.0", build: ${build}, commit: ${JSON.stringify(commit)}, channel: ${JSON.stringify(channel)} },
      },
    };
    const payload = Buffer.from(JSON.stringify(response));
    const frame = Buffer.alloc(4 + payload.length);
    frame.writeUInt32BE(payload.length, 0);
    payload.copy(frame, 4);
    process.stdout.write(frame);
  });
}
`);
  await chmod(binary, 0o755);
  return binary;
}

test("--dev --only health,ping --json emits one machine-readable summary", unixOnly, async (t) => {
  const binary = await fakeCore(t);
  const stdout = execFileSync(
    process.execPath,
    [installedCoreScript, "--binary", binary, "--dev", "--only", "health,ping", "--json"],
    { encoding: "utf8" },
  );
  const lines = stdout.trim().split("\n");
  assert.equal(lines.length, 1, "JSON mode must emit exactly one stdout line");
  const summary = JSON.parse(lines[0]);
  assert.deepEqual(Object.keys(summary), ["health", "ping", "journey"]);
  assert.equal(summary.health, "CodeCaddie 0.4.0+0 development");
  assert.equal(summary.ping.result.build.channel, "dev");
  assert.equal(summary.journey, null);

  const healthOnly = JSON.parse(execFileSync(
    process.execPath,
    [installedCoreScript, "--binary", binary, "--dev", "--only", "health", "--json"],
    { encoding: "utf8" },
  ));
  assert.equal(healthOnly.ping, null);
  assert.equal(healthOnly.journey, null);

  const direct = exerciseInstalledCore(binary, {}, { dev: true, only: new Set(["ping"]) });
  assert.equal(direct.health, null);
  assert.equal(direct.response.result.service, "codecaddie-core");
});

test("without --dev a development build fails the health check", unixOnly, async (t) => {
  const binary = await fakeCore(t);
  const result = spawnSync(
    process.execPath,
    [installedCoreScript, "--binary", binary, "--only", "health"],
    { encoding: "utf8" },
  );
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /CodeCaddie 0\.4\.0\+0 development/);

  const release = await fakeCore(t, {
    build: 2001,
    commit: "0123456789abcdef0123456789abcdef01234567",
    channel: "stable",
  });
  const passed = spawnSync(
    process.execPath,
    [
      installedCoreScript,
      "--binary", release,
      "--only", "health,ping",
      "--expected-commit", "0123456789abcdef0123456789abcdef01234567",
      "--expected-build", "2001",
    ],
    { encoding: "utf8" },
  );
  assert.equal(passed.status, 0, passed.stderr);
  assert.match(passed.stdout, /installed core sections passed \(health, ping\)/);
});
