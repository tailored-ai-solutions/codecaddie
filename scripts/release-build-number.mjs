#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export async function releaseBuildNumber(commit = "HEAD", repositoryRoot = root) {
  const config = JSON.parse(
    await readFile(path.join(repositoryRoot, "config/release-version.json"), "utf8"),
  );
  if (
    config.schemaVersion !== 1
    || !Number.isSafeInteger(config.releaseBuildEpoch)
    || config.releaseBuildEpoch < 2000
  ) {
    throw new Error("releaseBuildEpoch must be an integer of at least 2000");
  }
  const shallow = execFileSync("git", ["rev-parse", "--is-shallow-repository"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  }).trim();
  if (shallow !== "false") {
    throw new Error("release build numbers require a complete Git history");
  }
  const exactCommit = execFileSync("git", ["rev-parse", "--verify", `${commit}^{commit}`], {
    cwd: repositoryRoot,
    encoding: "utf8",
  }).trim();
  const countText = execFileSync(
    "git",
    ["rev-list", "--first-parent", "--count", exactCommit],
    { cwd: repositoryRoot, encoding: "utf8" },
  ).trim();
  if (!/^[1-9][0-9]*$/.test(countText)) throw new Error("invalid first-parent commit count");
  const value = config.releaseBuildEpoch + Number(countText);
  if (!Number.isSafeInteger(value)) throw new Error("release build number is outside the safe range");
  return value;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    console.log(await releaseBuildNumber(process.argv[2] ?? "HEAD"));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
