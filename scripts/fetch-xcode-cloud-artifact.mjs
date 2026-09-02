#!/usr/bin/env node

import { createHash, createPrivateKey, sign } from "node:crypto";
import { mkdir, open, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { Readable } from "node:stream";
import { fileURLToPath } from "node:url";

const API_ORIGIN = "https://api.appstoreconnect.apple.com";
const STAPLED_ARTIFACT_TYPE = "STAPLED_NOTARIZED_ARCHIVE";
const MAX_ARTIFACT_BYTES = 2 * 1024 * 1024 * 1024;

function base64Url(value) {
  return Buffer.from(value).toString("base64url");
}

export function createAppStoreConnectToken({
  keyId,
  privateKey,
  requestPath,
  now = Date.now(),
}) {
  // Team keys carry ten-character IDs; individual keys (ApiKey_<id>.p8) carry
  // twelve. Only individual keys are accepted by this pipeline, but the shape
  // check stays permissive to both so a key ID is never rejected for its length.
  if (!/^[A-Z0-9]{10,12}$/.test(keyId)) throw new Error("invalid App Store Connect key ID");
  if (
    typeof requestPath !== "string"
    || !requestPath.startsWith("/v1/")
    || /[\r\n\0]/.test(requestPath)
  ) {
    throw new Error("App Store Connect token requires one safe v1 request path");
  }
  const requestUrl = new URL(requestPath, API_ORIGIN);
  if (requestUrl.origin !== API_ORIGIN || requestUrl.hash) {
    throw new Error("App Store Connect token request path is outside the API origin");
  }
  const issuedAt = Math.floor(now / 1000) - 5;
  const header = base64Url(JSON.stringify({ alg: "ES256", kid: keyId, typ: "JWT" }));
  const claims = {
    sub: "user",
    iat: issuedAt,
    exp: issuedAt + 15 * 60,
    aud: "appstoreconnect-v1",
    scope: [`GET ${requestUrl.pathname}${requestUrl.search}`],
  };
  const payload = base64Url(JSON.stringify(claims));
  const unsigned = `${header}.${payload}`;
  const signature = sign("sha256", Buffer.from(unsigned), {
    key: privateKey,
    dsaEncoding: "ieee-p1363",
  });
  return `${unsigned}.${signature.toString("base64url")}`;
}

export function selectExactCommitBuild(runs, commit) {
  if (!Array.isArray(runs)) throw new Error("Xcode Cloud build list is invalid");
  const matches = runs.filter((run) => run?.attributes?.sourceCommit?.commitSha === commit);
  matches.sort((left, right) => Number(right.attributes?.number ?? 0) - Number(left.attributes?.number ?? 0));
  return matches[0] ?? null;
}

export function selectStapledArtifact(artifacts) {
  if (!Array.isArray(artifacts)) throw new Error("Xcode Cloud artifact list is invalid");
  const matches = artifacts.filter(
    (artifact) => artifact?.attributes?.fileType === STAPLED_ARTIFACT_TYPE,
  );
  if (matches.length !== 1) {
    throw new Error(`expected exactly one ${STAPLED_ARTIFACT_TYPE}, found ${matches.length}`);
  }
  const artifact = matches[0];
  const { downloadUrl, fileName, fileSize } = artifact.attributes ?? {};
  if (typeof downloadUrl !== "string" || !downloadUrl.startsWith("https://")) {
    throw new Error("stapled Xcode Cloud artifact has no HTTPS download URL");
  }
  if (
    typeof fileName !== "string"
    || fileName === "."
    || fileName === ".."
    || fileName.length > 200
    || !/^[A-Za-z0-9][A-Za-z0-9._-]*\.zip$/.test(fileName)
    || path.basename(fileName) !== fileName
  ) {
    throw new Error("stapled Xcode Cloud artifact has an unsafe filename");
  }
  if (!Number.isSafeInteger(fileSize) || fileSize <= 0 || fileSize > MAX_ARTIFACT_BYTES) {
    throw new Error("stapled Xcode Cloud artifact has an invalid size");
  }
  return artifact;
}

export function formatGitHubOutput(name, value) {
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
    throw new Error("unsafe GitHub output name");
  }
  if (typeof value !== "string" || /[\r\n\0]/.test(value)) {
    throw new Error(`unsafe GitHub output value for ${name}`);
  }
  return `${name}=${value}\n`;
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value === undefined) {
      throw new Error("arguments must be --name value pairs");
    }
    options[flag.slice(2)] = value;
  }
  const workflowId = options["workflow-id"];
  const commit = options.commit;
  const outputDirectory = options["output-dir"];
  const timeoutSeconds = Number(options["timeout-seconds"] ?? 7200);
  const pollSeconds = Number(options["poll-seconds"] ?? 30);
  if (!/^[0-9a-f-]{16,}$/i.test(workflowId ?? "")) throw new Error("invalid Xcode Cloud workflow ID");
  if (!/^[0-9a-f]{40}$/.test(commit ?? "")) throw new Error("commit must be a lowercase full SHA-1");
  if (!outputDirectory) throw new Error("--output-dir is required");
  if (!Number.isFinite(timeoutSeconds) || timeoutSeconds < 1 || timeoutSeconds > 21600) {
    throw new Error("timeout must be between 1 and 21600 seconds");
  }
  if (!Number.isFinite(pollSeconds) || pollSeconds < 1 || pollSeconds > 300) {
    throw new Error("poll interval must be between 1 and 300 seconds");
  }
  return {
    workflowId,
    commit,
    outputDirectory: path.resolve(outputDirectory),
    timeoutMilliseconds: timeoutSeconds * 1000,
    pollMilliseconds: pollSeconds * 1000,
  };
}

async function sleep(milliseconds) {
  await new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function fetchWithHttpsRedirects(url, fetchImpl, maximumRedirects = 5) {
  let current = new URL(url);
  for (let redirects = 0; redirects <= maximumRedirects; redirects += 1) {
    if (current.protocol !== "https:") {
      throw new Error(`Xcode Cloud artifact redirect used forbidden protocol ${current.protocol}`);
    }
    const response = await fetchImpl(current, {
      redirect: "manual",
      signal: AbortSignal.timeout(30 * 60 * 1000),
    });
    if (![301, 302, 303, 307, 308].includes(response.status)) return response;
    const location = response.headers.get("location");
    if (!location) throw new Error("Xcode Cloud artifact redirect has no location");
    current = new URL(location, current);
  }
  throw new Error(`Xcode Cloud artifact exceeded ${maximumRedirects} HTTPS redirects`);
}

async function downloadFile(url, destination, expectedSize, fetchImpl) {
  const temporary = `${destination}.partial`;
  await rm(temporary, { force: true });
  const response = await fetchWithHttpsRedirects(url, fetchImpl);
  if (!response.ok || !response.body) {
    throw new Error(`Xcode Cloud artifact download failed with HTTP ${response.status}`);
  }
  const digest = createHash("sha256");
  let size = 0;
  const stream = Readable.fromWeb(response.body);
  let file;
  try {
    file = await open(temporary, "wx", 0o600);
    for await (const chunk of stream) {
      size += chunk.length;
      if (size > expectedSize) {
        throw new Error(`Xcode Cloud artifact exceeded its declared ${expectedSize}-byte size`);
      }
      digest.update(chunk);
      let offset = 0;
      while (offset < chunk.length) {
        const { bytesWritten } = await file.write(chunk, offset, chunk.length - offset);
        if (bytesWritten <= 0) throw new Error("Xcode Cloud artifact write made no progress");
        offset += bytesWritten;
      }
    }
    await file.sync();
  } catch (error) {
    await rm(temporary, { force: true });
    throw error;
  } finally {
    await file?.close();
  }
  if (size !== expectedSize) {
    await rm(temporary, { force: true });
    throw new Error(`Xcode Cloud artifact size mismatch: expected ${expectedSize}, received ${size}`);
  }
  await rename(temporary, destination);
  return { size, sha256: digest.digest("hex") };
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const keyId = process.env.APP_STORE_CONNECT_KEY_ID;
  const encodedPrivateKey = process.env.APP_STORE_CONNECT_PRIVATE_KEY_BASE64;
  if (!encodedPrivateKey) throw new Error("APP_STORE_CONNECT_PRIVATE_KEY_BASE64 is required");
  const privateKeyBytes = Buffer.from(encodedPrivateKey, "base64");
  let privateKey;
  try {
    privateKey = createPrivateKey(privateKeyBytes);
  } finally {
    privateKeyBytes.fill(0);
    delete process.env.APP_STORE_CONNECT_PRIVATE_KEY_BASE64;
  }

  const apiFetch = async (pathname) => {
    const token = createAppStoreConnectToken({ keyId, privateKey, requestPath: pathname });
    const response = await fetch(new URL(pathname, API_ORIGIN), {
      headers: { Authorization: `Bearer ${token}`, Accept: "application/json" },
      redirect: "error",
      signal: AbortSignal.timeout(60_000),
    });
    if (!response.ok) {
      throw new Error(`App Store Connect request failed with HTTP ${response.status}`);
    }
    return response.json();
  };

  const deadline = Date.now() + options.timeoutMilliseconds;
  let buildRun;
  while (Date.now() < deadline) {
    const query = new URLSearchParams({
      "fields[ciBuildRuns]": "number,createdDate,finishedDate,sourceCommit,executionProgress,completionStatus",
      limit: "200",
      sort: "-number",
    });
    const response = await apiFetch(
      `/v1/ciWorkflows/${encodeURIComponent(options.workflowId)}/buildRuns?${query}`,
    );
    buildRun = selectExactCommitBuild(response.data, options.commit);
    if (!buildRun) {
      console.log(`waiting for Xcode Cloud to start exact commit ${options.commit}`);
      await sleep(options.pollMilliseconds);
      continue;
    }
    const { executionProgress, completionStatus } = buildRun.attributes ?? {};
    if (executionProgress !== "COMPLETE") {
      console.log(`waiting for Xcode Cloud build ${buildRun.id} (${executionProgress ?? "unknown"})`);
      await sleep(options.pollMilliseconds);
      continue;
    }
    if (completionStatus !== "SUCCEEDED") {
      throw new Error(
        `Xcode Cloud build ${buildRun.id} for ${options.commit} completed as ${completionStatus ?? "unknown"}`,
      );
    }
    break;
  }
  if (!buildRun || buildRun.attributes?.completionStatus !== "SUCCEEDED") {
    throw new Error(`timed out waiting for successful Xcode Cloud build of ${options.commit}`);
  }

  let selectedAction;
  let selectedArtifact;
  while (Date.now() < deadline) {
    const actionsResponse = await apiFetch(
      `/v1/ciBuildRuns/${encodeURIComponent(buildRun.id)}/actions?fields[ciBuildActions]=name,actionType,executionProgress,completionStatus&limit=200`,
    );
    const archiveActions = (actionsResponse.data ?? []).filter(
      (action) => action?.attributes?.actionType === "ARCHIVE"
        && action?.attributes?.executionProgress === "COMPLETE"
        && action?.attributes?.completionStatus === "SUCCEEDED",
    );
    const stapled = [];
    for (const action of archiveActions) {
      const artifactsResponse = await apiFetch(
        `/v1/ciBuildActions/${encodeURIComponent(action.id)}/artifacts?fields[ciArtifacts]=fileType,fileName,fileSize,downloadUrl&limit=200`,
      );
      for (const artifact of artifactsResponse.data ?? []) {
        if (artifact?.attributes?.fileType === STAPLED_ARTIFACT_TYPE) {
          stapled.push({ action, artifact });
        }
      }
    }
    if (stapled.length > 1) {
      throw new Error(`Xcode Cloud build ${buildRun.id} produced multiple stapled notarized archives`);
    }
    if (stapled.length === 1) {
      selectedAction = stapled[0].action;
      selectedArtifact = selectStapledArtifact([stapled[0].artifact]);
      break;
    }
    console.log(`waiting for stapled notarized archive from Xcode Cloud build ${buildRun.id}`);
    await sleep(options.pollMilliseconds);
  }
  if (!selectedArtifact) throw new Error("timed out waiting for stapled notarized archive");

  await mkdir(options.outputDirectory, { recursive: true, mode: 0o700 });
  const artifactPath = path.join(options.outputDirectory, selectedArtifact.attributes.fileName);
  const downloaded = await downloadFile(
    selectedArtifact.attributes.downloadUrl,
    artifactPath,
    selectedArtifact.attributes.fileSize,
    fetch,
  );
  const provenance = {
    schemaVersion: 1,
    workflowId: options.workflowId,
    buildRunId: buildRun.id,
    buildRunNumber: buildRun.attributes.number,
    sourceCommit: buildRun.attributes.sourceCommit.commitSha,
    actionId: selectedAction.id,
    artifactId: selectedArtifact.id,
    fileType: selectedArtifact.attributes.fileType,
    fileName: selectedArtifact.attributes.fileName,
    fileSize: downloaded.size,
    sha256: downloaded.sha256,
  };
  const provenancePath = path.join(options.outputDirectory, "xcode-cloud-provenance.json");
  await writeFile(provenancePath, `${JSON.stringify(provenance, null, 2)}\n`, { mode: 0o600 });

  if (process.env.GITHUB_OUTPUT) {
    await writeFile(
      process.env.GITHUB_OUTPUT,
      formatGitHubOutput("artifact_path", artifactPath)
        + formatGitHubOutput("provenance_path", provenancePath),
      { flag: "a" },
    );
  }
  console.log(JSON.stringify(provenance));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
