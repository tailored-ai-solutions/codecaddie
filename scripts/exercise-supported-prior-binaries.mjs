#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { exerciseInstalledCore } from "./exercise-installed-core.mjs";
import {
  DEFAULT_MAX_BUFFER as MAX_BUFFER,
  createFixtureRepository,
  loadSyntheticGoals,
  runAgent,
  runCore,
  runGit as git,
} from "./lib/core-harness.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const EXECUTABLE_NAME = process.platform === "win32" ? "codecaddie-core.exe" : "codecaddie-core";

function command(executable, args, options = {}) {
  const result = spawnSync(executable, args, { encoding: "utf8", maxBuffer: MAX_BUFFER, ...options });
  assert.equal(result.status, 0, result.stderr || `${executable} ${args.join(" ")} failed`);
  return result.stdout?.trim() ?? "";
}

function buildBinary({ source, commit, build, output, target }) {
  command(
    "cargo",
    [
      "build",
      "--release",
      "--locked",
      "--manifest-path",
      join(source, "Cargo.toml"),
      "--package",
      "codecaddie-core",
      "--bin",
      "codecaddie-core",
    ],
    {
      cwd: source,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: target,
        CODECADDIE_COMMIT_SHA: commit,
        CODECADDIE_BUILD_NUMBER: String(build),
      },
      stdio: ["ignore", "inherit", "inherit"],
    },
  );
  mkdirSync(dirname(output), { recursive: true });
  copyFileSync(join(target, "release", EXECUTABLE_NAME), output);
  if (process.platform !== "win32") chmodSync(output, 0o700);
  exerciseInstalledCore(output, { commit, build: String(build) });
  return output;
}

function extractCommit(commit, destination) {
  mkdirSync(destination, { recursive: true });
  const archive = spawnSync("git", ["-C", ROOT, "archive", "--format=tar", commit], {
    maxBuffer: MAX_BUFFER,
  });
  assert.equal(archive.status, 0, archive.stderr?.toString() || `could not archive ${commit}`);
  const extracted = spawnSync("tar", ["-xf", "-", "-C", destination], {
    input: archive.stdout,
    maxBuffer: MAX_BUFFER,
  });
  assert.equal(extracted.status, 0, extracted.stderr?.toString() || `could not extract ${commit}`);
}

function recordReport(binary, environment, workspaceId, repository, commit, goal, suffix) {
  const status = runAgent(binary, ["status", "--workspace", workspaceId], environment);
  const begun = runAgent(
    binary,
    ["begin-analysis", "--repo", `attached-repository@${commit}`, "--workspace", workspaceId],
    environment,
  );
  const payloadPath = join(status.exchange.inbox, `compatibility-${suffix}.json`);
  writeFileSync(payloadPath, JSON.stringify({
    providerVersion: `compatibility-${suffix}`,
    assessments: [{
      goalVersionId: goal.id,
      summary: "The executable compatibility journey retained the frozen decision.",
      criteria: [{
        criterionId: goal.criteria[0].id,
        verdict: "supported",
        rationale: "The saved coordinate resolves at the immutable repository commit.",
        confidence: 1,
        evidence: [{
          repositoryId: "attached-repository",
          path: "evidence.txt",
          startLine: 1,
          endLine: 1,
          kind: "test",
        }],
      }],
    }],
    architecture: [],
    recommendations: [],
  }));
  const submitted = runAgent(
    binary,
    ["submit-analysis", "--session", begun.analysisSessionId, "--file", payloadPath],
    environment,
  );
  assert.equal(submitted.recorded, true);
  assert.equal(existsSync(payloadPath), false);
  return submitted.reportId;
}

function assertProjection(response, { workspaceId, reportId, reportCount, commit, sentinel }) {
  assert.equal(response.ok, true, JSON.stringify(response.error ?? {}));
  const workspace = response.result.workspace;
  assert.equal(workspace.workspaceId, workspaceId);
  assert.equal(workspace.name, "Supported prior-version journey");
  assert.equal(workspace.approvedGoals[0].title, sentinel);
  assert.equal(workspace.latestReport.id, reportId);
  assert.equal(workspace.latestReport.repositories[0].commitSha, commit);
  assert.equal(workspace.latestReport.assessments[0].criteria[0].evidence[0].path, "evidence.txt");
  assert.equal(workspace.reportHeatmap.length, reportCount);
}

function assertDataRootExcludesSource(dataRoot, sourceCanary) {
  const pending = [dataRoot];
  while (pending.length) {
    const current = pending.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile()) assert.equal(readFileSync(path).includes(Buffer.from(sourceCanary)), false);
    }
  }
}

function exercisePriorBinary({ priorBinary, currentBinary, prior, root }) {
  const journeyRoot = join(root, `journey-${prior.build}`);
  const dataRoot = join(journeyRoot, "data");
  const repository = join(journeyRoot, "repository");
  const sourceCanary = `PRIVATE PRIOR VERSION SOURCE CANARY ${prior.build}`;
  const sentinel = `Prior build ${prior.version}+${prior.build} encrypted decision sentinel`;
  mkdirSync(dataRoot, { recursive: true });
  const commit = createFixtureRepository(repository, {
    fileName: "evidence.txt",
    contents: `${sourceCanary}\n`,
    userEmail: "compatibility@invalid.example",
    userName: "Compatibility journey",
    message: "compatibility fixture",
  });
  const environment = { ...process.env, CODECADDIE_DATA_DIR: dataRoot };

  const created = runCore(priorBinary, {
    id: `prior-${prior.build}-create`,
    protocolVersion: 2,
    method: "workspace.create",
    params: {
      name: "Supported prior-version journey",
      repositoryDisplayName: "compatibility-repository",
      repositoryPath: repository,
      productBrief: "A local release preserves frozen decision history through update and reversal.",
      context: {},
    },
  }, environment);
  assert.equal(created.ok, true);
  const workspaceId = created.result.workspaceId;
  const definitions = loadSyntheticGoals();
  definitions[0].title = sentinel;
  const goals = definitions.map((params, index) => {
    const approved = runCore(priorBinary, {
      id: `prior-${prior.build}-goal-${index + 1}`,
      protocolVersion: 2,
      workspaceId,
      method: "goals.approve",
      params,
    }, environment);
    assert.equal(approved.ok, true);
    return approved.result.goalVersion;
  });
  assert.equal(runCore(priorBinary, {
    id: `prior-${prior.build}-provider`,
    protocolVersion: 2,
    method: "settings.provider.set",
    params: { provider: "codex" },
  }, environment).ok, true);

  const priorReport = recordReport(
    priorBinary,
    environment,
    workspaceId,
    repository,
    commit,
    goals[0],
    `prior-${prior.build}`,
  );
  assertProjection(runCore(currentBinary, {
    id: `current-open-prior-${prior.build}`,
    protocolVersion: 2,
    method: "workspace.open",
    params: { workspaceId },
  }, environment), { workspaceId, reportId: priorReport, reportCount: 1, commit, sentinel });
  assert.equal(runCore(currentBinary, {
    id: `current-provider-${prior.build}`,
    protocolVersion: 2,
    method: "settings.provider.get",
    params: {},
  }, environment).result.provider, "codex");

  const currentReport = recordReport(
    currentBinary,
    environment,
    workspaceId,
    repository,
    commit,
    goals[0],
    `current-from-${prior.build}`,
  );
  assertProjection(runCore(priorBinary, {
    id: `rollback-open-${prior.build}`,
    protocolVersion: 2,
    method: "workspace.open",
    params: { workspaceId },
  }, environment), { workspaceId, reportId: currentReport, reportCount: 2, commit, sentinel });
  assert.equal(runCore(priorBinary, {
    id: `rollback-provider-${prior.build}`,
    protocolVersion: 2,
    method: "settings.provider.get",
    params: {},
  }, environment).result.provider, "codex");
  assertDataRootExcludesSource(dataRoot, sourceCanary);
}

/**
 * Builds the current commit and every supported prior build from exact source
 * archives, then drives each prior binary through upgrade and rollback against
 * the current one. `requireClean` (CI's `--require-clean`) additionally
 * refuses a dirty checkout so the exercised source is exactly the commit under
 * test; local runs may keep uncommitted work in the tree because the archive
 * of HEAD, not the working tree, is what gets built.
 */
export function exerciseSupportedPriorVersions({
  matrixPath = join(ROOT, "config/supported-upgrade-matrix.json"),
  requireClean = false,
} = {}) {
  const matrix = JSON.parse(readFileSync(matrixPath, "utf8"));
  assert.equal(matrix.schemaVersion, 2);
  assert.match(matrix.currentVersion, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/);
  assert.equal(matrix.versionIdentity, "semantic-version-plus-build");
  assert.ok(Array.isArray(matrix.supportedPriorBuilds));
  assert.deepEqual(
    Object.keys(matrix.firstPublicBaseline).sort(),
    ["build", "reason", "status", "version"],
  );
  assert.equal(matrix.firstPublicBaseline.version, "0.4.0");
  assert.equal(matrix.firstPublicBaseline.build, 2001);
  assert.ok(matrix.firstPublicBaseline.reason.length >= 40);
  if (matrix.firstPublicBaseline.status === "pending") {
    assert.equal(matrix.currentVersion, matrix.firstPublicBaseline.version);
    assert.deepEqual(
      matrix.supportedPriorBuilds,
      [],
      "only the first public snapshot may have an empty prior-build matrix",
    );
    return { currentCommit: null, builds: [], pendingFirstPublicBaseline: true };
  }
  assert.equal(matrix.firstPublicBaseline.status, "established");
  assert.ok(matrix.supportedPriorBuilds.length > 0);
  const baseline = matrix.supportedPriorBuilds[0];
  assert.equal(baseline.version, matrix.firstPublicBaseline.version);
  assert.equal(baseline.build, matrix.firstPublicBaseline.build);
  assert.match(baseline.sourceCommit, /^[0-9a-f]{40}$/);
  const root = mkdtempSync(join(tmpdir(), "codecaddie-prior-binaries-"));
  const targetRoot = join(ROOT, "target", "supported-prior-binaries");
  try {
    if (requireClean) {
      assert.equal(
        git(ROOT, ["status", "--porcelain"]),
        "",
        "supported prior-version verification requires a clean exact-commit checkout",
      );
    }
    const currentCommit = git(ROOT, ["rev-parse", "HEAD"]);
    const currentBuild = Number(git(ROOT, ["rev-list", "--count", "HEAD"]));
    const currentSource = join(root, "sources", `current-${currentBuild}`);
    extractCommit(currentCommit, currentSource);
    const currentBinary = buildBinary({
      source: currentSource,
      commit: currentCommit,
      build: currentBuild,
      output: join(root, "binaries", `current-${currentBuild}`, EXECUTABLE_NAME),
      target: join(targetRoot, currentCommit),
    });
    for (const prior of matrix.supportedPriorBuilds) {
      assert.match(prior.sourceCommit, /^[0-9a-f]{40}$/);
      const source = join(root, "sources", String(prior.build));
      extractCommit(prior.sourceCommit, source);
      const priorBinary = buildBinary({
        source,
        commit: prior.sourceCommit,
        build: prior.build,
        output: join(root, "binaries", String(prior.build), EXECUTABLE_NAME),
        target: join(targetRoot, prior.sourceCommit),
      });
      exercisePriorBinary({ priorBinary, currentBinary, prior, root });
    }
    return { currentCommit, builds: matrix.supportedPriorBuilds.map(({ build }) => build) };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const unknown = process.argv.slice(2).filter((argument) => argument !== "--require-clean");
  if (unknown.length > 0) {
    throw new Error(`usage: exercise-supported-prior-binaries.mjs [--require-clean] (unexpected: ${unknown.join(" ")})`);
  }
  const result = exerciseSupportedPriorVersions({ requireClean: process.argv.includes("--require-clean") });
  if (result.pendingFirstPublicBaseline) {
    console.log("supported prior-version binaries passed: first public build 2001 has no prior public baseline");
  } else {
    console.log(`supported prior-version binaries passed: current ${result.currentCommit}; builds ${result.builds.join(", ")}`);
  }
}
