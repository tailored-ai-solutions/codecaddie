#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const semanticVersion = /^\d+\.\d+\.\d+(?:-rc\.\d+)?$/;
const sourceCommit = /^[a-f0-9]{40}$/;
const repositoryName = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

export function parseReleaseArgs(args) {
  const options = { artifacts: [], channel: "stable", required: "false" };
  for (let index = 0; index < args.length; index += 1) {
    const name = args[index];
    if (name === "--artifact") {
      const value = args[++index];
      if (!value) throw new Error("--artifact requires a value");
      options.artifacts.push(value);
      continue;
    }
    if (!name?.startsWith("--")) throw new Error(`unknown argument ${name}`);
    const value = args[++index];
    if (value === undefined) throw new Error(`${name} requires a value`);
    options[name.slice(2)] = value;
  }
  for (const required of [
    "version",
    "build",
    "output",
    "release-base",
    "release-notes",
    "repository",
    "source-commit",
  ]) {
    if (!options[required]) throw new Error(`--${required} is required`);
  }
  if (!semanticVersion.test(options.version)) {
    throw new Error("version must be a stable or rc semantic version");
  }
  const expectedChannel = options.version.includes("-rc.") ? "beta" : "stable";
  if (options.channel !== expectedChannel) {
    throw new Error(`${options.version} must use the ${expectedChannel} channel`);
  }
  options.minimum ??= "0.4.0";
  if (!semanticVersion.test(options.minimum)) {
    throw new Error("minimum version must be a stable or rc semantic version");
  }
  if (!/^[1-9]\d*$/.test(options.build)) throw new Error("build must be a positive integer");
  if (!sourceCommit.test(options["source-commit"])) {
    throw new Error("source commit must be a lowercase 40-character Git SHA");
  }
  if (!repositoryName.test(options.repository)) {
    throw new Error("repository must be an owner/name GitHub repository");
  }
  if (!["true", "false"].includes(options.required)) {
    throw new Error("required must be true or false");
  }
  if (!options.artifacts.length) throw new Error("at least one --artifact is required");
  const releaseBase = new URL(options["release-base"]);
  const releaseNotes = new URL(options["release-notes"]);
  if (releaseBase.protocol !== "https:" || releaseNotes.protocol !== "https:") {
    throw new Error("release URLs must use HTTPS");
  }
  return options;
}

function parseArtifact(value) {
  const [platform, architecture, format, ...fileParts] = value.split(":");
  const file = fileParts.join(":");
  if (!platform || !architecture || !format || !file) {
    throw new Error("artifact must be platform:architecture:format:path");
  }
  return { platform, architecture, format, file: path.resolve(file) };
}

export async function createReleaseManifest(options) {
  const output = path.resolve(options.output);
  await mkdir(output, { recursive: true });
  const releaseBase = options["release-base"].replace(/\/$/, "");
  const identities = new Set();
  const artifacts = [];
  for (const descriptor of options.artifacts.map(parseArtifact)) {
    const identity = `${descriptor.platform}:${descriptor.architecture}:${descriptor.format}`;
    if (identities.has(identity)) throw new Error(`duplicate release artifact ${identity}`);
    identities.add(identity);
    const bytes = await readFile(descriptor.file);
    const details = await stat(descriptor.file);
    if (!details.isFile()) throw new Error(`release artifact is not a regular file: ${descriptor.file}`);
    artifacts.push({
      platform: descriptor.platform,
      architecture: descriptor.architecture,
      format: descriptor.format,
      url: `${releaseBase}/${encodeURIComponent(path.basename(descriptor.file))}`,
      size: details.size,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    });
  }
  artifacts.sort((left, right) =>
    `${left.platform}:${left.architecture}:${left.format}`.localeCompare(
      `${right.platform}:${right.architecture}:${right.format}`,
    ),
  );
  const publishedAt = options.published ?? new Date().toISOString();
  if (
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?(?:Z|[+-]\d{2}:\d{2})$/.test(publishedAt)
    || !Number.isFinite(Date.parse(publishedAt))
  ) {
    throw new Error("published timestamp must be an ISO-8601 instant");
  }
  const manifest = {
    schemaVersion: 2,
    version: options.version,
    build: Number(options.build),
    channel: options.channel,
    publishedAt,
    minimumSupportedVersion: options.minimum,
    required: options.required === "true",
    releaseNotesUrl: options["release-notes"],
    sourceRepository: options.repository,
    sourceCommit: options["source-commit"],
    artifacts,
  };
  const rawManifest = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  const checksumByName = new Map();
  for (const artifact of artifacts) {
    const name = decodeURIComponent(artifact.url.split("/").at(-1));
    const existing = checksumByName.get(name);
    if (existing && existing !== artifact.sha256) {
      throw new Error(`release artifact filename has conflicting contents: ${name}`);
    }
    checksumByName.set(name, artifact.sha256);
  }
  const checksums = [...checksumByName]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, digest]) => `${digest}  ${name}`)
    .join("\n");
  await writeFile(path.join(output, "manifest.json"), rawManifest);
  await writeFile(path.join(output, "SHA256SUMS.txt"), `${checksums}\n`, "utf8");
  return { manifest, rawManifest };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const options = parseReleaseArgs(process.argv.slice(2));
  await createReleaseManifest(options);
  console.log(path.resolve(options.output));
}
