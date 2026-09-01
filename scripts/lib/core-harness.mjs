/**
 * Shared harness for driving a built `codecaddie-core` executable through its
 * length-prefixed JSON protocol and its `agent` command line. The installed
 * core journey, the supported prior-binary matrix, and the agent verification
 * skill all build on these helpers.
 *
 * Nothing here reads or writes the real CodeCaddie data root: every helper
 * takes an explicit environment, and callers set `CODECADDIE_DATA_DIR` to a
 * fresh temporary directory before spawning the core.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const MAX_FRAME_BYTES = 16 * 1024 * 1024;
export const DEFAULT_MAX_BUFFER = 64 * 1024 * 1024;
export const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const SYNTHETIC_GOALS_PATH = join(REPOSITORY_ROOT, "fixtures", "journeys", "synthetic-goals.json");

/** Encodes one request as a 4-byte big-endian length prefix plus UTF-8 JSON. */
export function encodeFrame(value) {
  const payload = Buffer.from(JSON.stringify(value));
  assert.ok(payload.length <= MAX_FRAME_BYTES, "request exceeds the core frame limit");
  const frame = Buffer.allocUnsafe(4 + payload.length);
  frame.writeUInt32BE(payload.length, 0);
  payload.copy(frame, 4);
  return frame;
}

/** Decodes exactly one response frame and rejects truncated or extra bytes. */
export function decodeSingleFrame(bytes) {
  assert.ok(bytes.length >= 4, "installed core returned a truncated frame header");
  const length = bytes.readUInt32BE(0);
  assert.ok(length <= MAX_FRAME_BYTES, "installed core returned an oversized frame");
  assert.equal(bytes.length, 4 + length, "installed core returned an incomplete or extra frame");
  return JSON.parse(bytes.subarray(4).toString("utf8"));
}

/** Sends one framed request to a fresh core process and returns its response. */
export function runCore(binary, request, environment, { maxBuffer = DEFAULT_MAX_BUFFER } = {}) {
  const result = spawnSync(binary, [], {
    env: environment,
    input: encodeFrame(request),
    maxBuffer,
  });
  assert.equal(result.status, 0, result.stderr?.toString() || `core failed for ${request.method}`);
  const response = decodeSingleFrame(result.stdout);
  assert.equal(response.id, request.id);
  return response;
}

/** Runs one `codecaddie-core agent <command>` invocation and parses its JSON. */
export function runAgent(binary, args, environment, expectedStatus = 0) {
  const result = spawnSync(binary, ["agent", ...args], {
    env: environment,
    encoding: "utf8",
    maxBuffer: DEFAULT_MAX_BUFFER,
  });
  assert.equal(result.status, expectedStatus, result.stderr || `agent command failed: ${args[0]}`);
  const response = JSON.parse(result.stdout);
  assert.equal(response.ok, expectedStatus === 0);
  return response;
}

/** Runs git inside a repository and returns trimmed stdout. */
export function runGit(repository, args) {
  const result = spawnSync("git", ["-C", repository, ...args], {
    encoding: "utf8",
    maxBuffer: DEFAULT_MAX_BUFFER,
  });
  assert.equal(result.status, 0, result.stderr || `git ${args.join(" ")} failed`);
  return result.stdout.trim();
}

/**
 * Creates a one-file, one-commit fixture repository whose only content is the
 * caller's canary text, and returns the commit SHA. Journeys assert that the
 * canary never reaches a response, report, export, or the data root.
 */
export function createFixtureRepository(directory, {
  fileName,
  contents,
  userName,
  userEmail,
  message,
}) {
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, fileName), contents);
  runGit(directory, ["init", "--quiet"]);
  runGit(directory, ["config", "user.email", userEmail]);
  runGit(directory, ["config", "user.name", userName]);
  runGit(directory, ["add", fileName]);
  runGit(directory, ["commit", "--quiet", "-m", message]);
  return runGit(directory, ["rev-parse", "HEAD"]);
}

/**
 * Loads the three synthetic approved goals shared by every executable
 * journey. Each call returns a fresh copy so callers can override titles
 * (for example with an encrypted-state sentinel) without leaking between runs.
 */
export function loadSyntheticGoals(fixturePath = SYNTHETIC_GOALS_PATH) {
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8"));
  assert.equal(fixture.schemaVersion, 1, "unsupported synthetic goal fixture schema");
  assert.ok(Array.isArray(fixture.goals) && fixture.goals.length === 3, "expected three synthetic goals");
  return structuredClone(fixture.goals);
}
