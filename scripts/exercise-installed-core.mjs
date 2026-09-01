#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { once } from "node:events";
import { pathToFileURL } from "node:url";
import {
  createFixtureRepository,
  decodeSingleFrame,
  encodeFrame,
  loadSyntheticGoals,
  runAgent,
  runCore,
} from "./lib/core-harness.mjs";

export { decodeSingleFrame, encodeFrame };

export const SECTIONS = ["health", "ping", "journey"];

/** Parses `--only health,ping,journey`; the default selects every section. */
export function parseOnly(value) {
  if (value === undefined) return new Set(SECTIONS);
  const selected = new Set();
  for (const part of String(value).split(",").map((entry) => entry.trim()).filter(Boolean)) {
    assert.ok(SECTIONS.includes(part), `--only accepts ${SECTIONS.join(", ")}; received ${part}`);
    selected.add(part);
  }
  assert.ok(selected.size > 0, "--only requires at least one section");
  return selected;
}

/**
 * Release builds carry a real build number and a commit hash. Development
 * builds (`cargo build` without CODECADDIE_BUILD_NUMBER / CODECADDIE_COMMIT_SHA)
 * report build 0 and the literal commit `development`, which `--dev` admits.
 */
export function healthPattern(dev = false) {
  return dev
    ? /^CodeCaddie \d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?\+\d+ (?:[0-9a-f]{12,40}|development)\s*$/
    : /^CodeCaddie \d+\.\d+\.\d+\+\d+ [0-9a-f]{12,40}\s*$/;
}

export function checkInstalledHealth(binary, expected = {}, { dev = false } = {}) {
  const health = spawnSync(binary, ["--health-check"], { encoding: "utf8" });
  assert.equal(health.status, 0, health.stderr || "installed core health check failed");
  assert.match(health.stdout, healthPattern(dev));
  if (expected.commit) {
    assert.match(expected.commit, /^[0-9a-f]{40}$/, "expected commit must be a full immutable SHA");
    assert.ok(health.stdout.includes(` ${expected.commit.slice(0, 12)}`));
  }
  if (expected.build) assert.ok(health.stdout.includes(`+${expected.build} `));
  return health.stdout.trim();
}

export function checkInstalledPing(binary, expected = {}) {
  const request = {
    id: "installed-core-assurance",
    protocolVersion: 2,
    method: "system.ping",
    params: {},
  };
  const ping = spawnSync(binary, [], { input: encodeFrame(request) });
  assert.equal(ping.status, 0, ping.stderr?.toString() || "installed core ping failed");
  const response = decodeSingleFrame(ping.stdout);
  assert.deepEqual(Object.keys(response).sort(), ["id", "ok", "result"]);
  assert.equal(response.id, request.id);
  assert.equal(response.ok, true);
  assert.deepEqual(Object.keys(response.result).sort(), ["build", "protocolVersion", "service"]);
  assert.equal(response.result.protocolVersion, 2);
  assert.equal(response.result.service, "codecaddie-core");
  assert.deepEqual(Object.keys(response.result.build).sort(), ["build", "channel", "commit", "version"]);
  assert.match(response.result.build.version, /^\d+\.\d+\.\d+$/);
  assert.match(response.result.build.channel, /^(dev|beta|stable)$/);
  if (expected.commit) assert.equal(response.result.build.commit, expected.commit);
  if (expected.build) assert.equal(String(response.result.build.build), expected.build);
  const serialized = JSON.stringify(response).toLowerCase();
  for (const forbidden of ["source", "attachment", "goaltext", "prompt", "credential", "keychain"]) {
    assert.equal(serialized.includes(forbidden), false);
  }
  return response;
}

/**
 * Checks the executable's health line and its `system.ping` frame. `only`
 * selects a subset; a skipped section reports `null`.
 */
export function exerciseInstalledCore(binary, expected = {}, { dev = false, only } = {}) {
  const selected = only ?? new Set(["health", "ping"]);
  const health = selected.has("health") ? checkInstalledHealth(binary, expected, { dev }) : null;
  const response = selected.has("ping") ? checkInstalledPing(binary, expected) : null;
  return { health, response };
}

async function cancelLiveCoreAnalysis(binary, request, environment) {
  const child = spawn(binary, [], { env: environment, stdio: ["pipe", "pipe", "pipe"] });
  let pending = "";
  let stderr = "";
  let progressSeen = false;
  let terminationRequested = false;
  const observed = [];
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  child.stdout.on("data", (chunk) => {
    pending += chunk;
    const lines = pending.split("\n");
    pending = lines.pop();
    for (const line of lines) {
      if (!line.trim()) continue;
      const event = JSON.parse(line);
      observed.push({ topic: event.topic, ok: event.ok, code: event.error?.code, message: event.error?.message });
      if (event.topic === "scan.progress" && !progressSeen) {
        progressSeen = true;
        terminationRequested = child.kill();
      }
    }
  });
  await once(child, "spawn");
  const exited = once(child, "close");
  child.stdin.end(encodeFrame(request));
  const timeout = setTimeout(() => child.kill(), 10_000);
  const [code, signal] = await exited;
  clearTimeout(timeout);
  assert.equal(
    progressSeen,
    true,
    `installed analysis emitted no progress before exit (code ${code}, signal ${signal}, events ${JSON.stringify(observed)}, stdout ${JSON.stringify(pending.slice(0, 500))}): ${stderr}`,
  );
  assert.equal(terminationRequested, true, "installed analysis could not be terminated after progress");
  assert.ok(signal || code !== 0, "installed analysis cancellation must terminate the process");
}

/**
 * Exercises the installed executable through one cross-platform first-report
 * journey. Every process gets the same owner-only temporary data root, so a
 * fresh process proves restart recovery without touching the runner account's
 * real CodeCaddie data. The repository canary is never copied into a request,
 * report, response, or export. With `keep`, the temporary root survives so a
 * caller can inspect the data root and export; otherwise it is removed.
 */
export async function exerciseInstalledJourney(binary, { keep = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "codecaddie-installed-journey-"));
  const data = join(root, "data");
  const repository = join(root, "repository");
  const sourceCanary = "PRIVATE SOURCE CANARY must never enter CodeCaddie output";
  mkdirSync(data, { recursive: true });
  const commit = createFixtureRepository(repository, {
    fileName: "journey.txt",
    contents: `${sourceCanary}\n`,
    userEmail: "installed-journey@invalid.example",
    userName: "Installed Journey",
    message: "installed journey fixture",
  });
  const environment = { ...process.env, CODECADDIE_DATA_DIR: data };

  try {
    const created = runCore(binary, {
      id: "installed-journey-create",
      protocolVersion: 2,
      method: "workspace.create",
      params: {
        name: "Installed journey",
        repositoryDisplayName: "installed-journey",
        repositoryPath: repository,
        productBrief: "A local tool turns a frozen repository review into a durable decision artifact.",
        context: {},
      },
    }, environment);
    assert.equal(created.ok, true);
    const workspaceId = created.result.workspaceId;

    const goalDefinitions = loadSyntheticGoals();
    const goals = goalDefinitions.map((params, index) => {
      const approved = runCore(binary, {
        id: `installed-journey-goal-${index + 1}`,
        protocolVersion: 2,
        workspaceId,
        method: "goals.approve",
        params,
      }, environment);
      assert.equal(approved.ok, true);
      return approved.result.goalVersion;
    });
    const goal = goals[0];

    const status = runAgent(binary, ["status", "--workspace", workspaceId], environment);
    const begun = runAgent(binary, [
      "begin-analysis",
      "--repo", `attached-repository@${commit}`,
      "--workspace", workspaceId,
    ], environment);
    assert.equal(begun.repositories[0].commitSha, commit);
    const payloadPath = join(status.exchange.inbox, "installed-analysis.json");
    writeFileSync(payloadPath, JSON.stringify({
      providerVersion: "installed-journey 1.0",
      assessments: [{
        goalVersionId: goal.id,
        summary: "The installed exact-commit journey completed.",
        criteria: [{
          criterionId: goal.criteria[0].id,
          verdict: "supported",
          rationale: "The packaged executable persisted the frozen-commit assessment.",
          confidence: 1,
          evidence: [{
            repositoryId: "attached-repository",
            path: "journey.txt",
            startLine: 1,
            endLine: 1,
            kind: "test",
          }],
        }],
      }],
      architecture: [],
      recommendations: [],
    }));
    const submitted = runAgent(binary, [
      "submit-analysis",
      "--session", begun.analysisSessionId,
      "--file", payloadPath,
    ], environment);
    assert.equal(submitted.recorded, true);
    assert.equal(submitted.coverage, 1);
    assert.equal(existsSync(payloadPath), false);

    // A new installed-core process must recover the exact saved report.
    const reopened = runCore(binary, {
      id: "installed-journey-reopen",
      protocolVersion: 2,
      method: "workspace.open",
      params: { workspaceId },
    }, environment);
    assert.equal(reopened.ok, true);
    assert.equal(reopened.result.workspace.latestReport.id, submitted.reportId);
    assert.equal(reopened.result.workspace.latestReport.repositories[0].commitSha, commit);
    assert.equal(reopened.result.workspace.latestReport.assessments[0].criteria[0].verdict, "supported");
    assert.equal(JSON.stringify(reopened).includes(sourceCanary), false);

    const exportPath = join(status.exchange.outbox, "installed-journey.docx");
    const exported = runAgent(binary, ["export", "--kind", "word", "--out", exportPath], environment);
    assert.equal(exported.path, exportPath);
    const wordBytes = readFileSync(exportPath);
    assert.equal(wordBytes.subarray(0, 2).toString("ascii"), "PK");
    assert.equal(wordBytes.includes(Buffer.from(sourceCanary)), false);

    // The installed failure boundary must return a stable, source-safe code.
    const missingProviderEnvironment = { ...environment, PATH: join(root, "missing-provider-bin") };
    mkdirSync(missingProviderEnvironment.PATH);
    const failed = runCore(binary, {
      id: "installed-journey-provider-failure",
      protocolVersion: 2,
      method: "scan.run",
      params: {
        reportId: "installed-provider-failure",
        repositories: [{ repositoryId: "attached-repository", repositoryPath: repository, commit }],
        provider: "codex",
        goals,
        productBrief: "A local tool turns a frozen repository review into a durable decision artifact.",
      },
    }, missingProviderEnvironment);
    assert.equal(failed.ok, false);
    assert.equal(failed.error.code, "scan_failed");
    assert.equal(JSON.stringify(failed).includes(sourceCanary), false);

    // Cancel a valid live analysis only after the installed core reports real
    // scan progress. A fresh process then records the same source-free event as
    // the native Cancel action and proves restart recovery did not overwrite
    // the last committed report.
    await cancelLiveCoreAnalysis(binary, {
      id: "installed-journey-cancel",
      protocolVersion: 2,
      workspaceId,
      method: "scan.run",
      params: {
        stream: true,
        reportId: "installed-cancelled-analysis",
        repositories: [{ repositoryId: "attached-repository", repositoryPath: repository, commit }],
        provider: "codex",
        goals,
        productBrief: "A local tool turns a frozen repository review into a durable decision artifact.",
      },
    }, environment);
    const cancellation = runCore(binary, {
      id: "installed-journey-cancellation-record",
      protocolVersion: 2,
      workspaceId,
      method: "reliability.record",
      params: {
        kind: "operation_cancelled",
        operation: "scan.run",
        sessionId: "installed-journey",
      },
    }, environment);
    assert.equal(cancellation.ok, true);
    const recoveredAfterCancellation = runCore(binary, {
      id: "installed-journey-recover-cancellation",
      protocolVersion: 2,
      method: "workspace.open",
      params: { workspaceId },
    }, environment);
    assert.equal(recoveredAfterCancellation.ok, true);
    assert.equal(recoveredAfterCancellation.result.workspace.latestReport.id, submitted.reportId);
    assert.ok(recoveredAfterCancellation.result.workspace.reliability.operationCancellations >= 1);
    assert.equal(JSON.stringify(recoveredAfterCancellation).includes("installed-cancelled-analysis"), false);
    assert.equal(JSON.stringify(recoveredAfterCancellation).includes(sourceCanary), false);

    return {
      workspaceId,
      reportId: submitted.reportId,
      commit,
      exportPath,
      dataRoot: data,
      repository,
      journeyRoot: root,
    };
  } finally {
    if (keep) {
      console.error(`kept installed journey data root: ${data} (repository fixture ${repository})`);
    } else {
      rmSync(root, { recursive: true, force: true });
    }
  }
}

function option(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? undefined : process.argv[index + 1];
}

function flag(name) {
  return process.argv.includes(name);
}

const USAGE = "usage: exercise-installed-core.mjs --binary <path> [--expected-commit <sha>] [--expected-build <number>] [--dev] [--only health,ping,journey] [--json] [--keep]";

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const binary = option("--binary");
  if (!binary) throw new Error(USAGE);
  const only = parseOnly(option("--only"));
  const dev = flag("--dev");
  const json = flag("--json");
  const keep = flag("--keep");
  const expected = { commit: option("--expected-commit"), build: option("--expected-build") };
  const core = exerciseInstalledCore(binary, expected, { dev, only });
  const journey = only.has("journey") ? await exerciseInstalledJourney(binary, { keep }) : null;
  const summary = {
    health: core.health,
    ping: core.response,
    journey: journey && {
      workspaceId: journey.workspaceId,
      reportId: journey.reportId,
      commit: journey.commit,
      exportPath: journey.exportPath,
      ...(keep ? { dataRoot: journey.dataRoot, repository: journey.repository } : {}),
    },
  };
  if (json) {
    process.stdout.write(`${JSON.stringify(summary)}\n`);
  } else {
    const parts = [];
    if (core.health) parts.push(core.health);
    if (core.response) parts.push(`ping ${core.response.result.service} protocol ${core.response.result.protocolVersion}`);
    if (journey) parts.push(`report ${journey.reportId}`);
    console.log(`installed core sections passed (${[...only].join(", ")}): ${parts.join("; ")}`);
  }
}
