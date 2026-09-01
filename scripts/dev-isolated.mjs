#!/usr/bin/env node
// Launches the desktop against an owner-only data root derived from the
// worktree path, so parallel workers (pstack /swarm, one Git worktree each)
// never share state and never write development data into the checkout.
//
//   pnpm dev:isolated              # prints the data root, then runs native:dev
//   node scripts/dev-isolated.mjs --print-only
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

/** The data root for a worktree: `<tmp>/codecaddie-dev/<sha256(worktree)[0..16]>`.
 * Deterministic per worktree path, never inside the checkout, and free of the
 * path itself so nothing personal leaks through the directory name. */
export function deriveDataDir(worktreePath, tmp = tmpdir()) {
  const key = createHash("sha256").update(resolve(worktreePath)).digest("hex").slice(0, 16);
  return join(resolve(tmp), "codecaddie-dev", key);
}

export function worktreeRoot() {
  return resolve(dirname(fileURLToPath(import.meta.url)), "..");
}

export function prepareDataDir(directory) {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  chmodSync(directory, 0o700);
  return directory;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const root = worktreeRoot();
  const dataDir = prepareDataDir(deriveDataDir(root));
  console.log(`CODECADDIE_DATA_DIR=${dataDir}`);
  if (process.argv.includes("--print-only")) process.exit(0);
  const child = spawn("pnpm", ["native:dev"], {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, CODECADDIE_DATA_DIR: dataDir },
  });
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.on(signal, () => child.kill(signal));
  }
  child.on("exit", (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
  child.on("error", (error) => {
    console.error(`could not start pnpm native:dev: ${error.message}`);
    process.exit(1);
  });
}
