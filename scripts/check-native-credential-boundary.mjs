#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const forbiddenEntrypoints = [
  "SecItemAdd",
  "SecItemCopyMatching",
  "SecItemDelete",
  "SecItemUpdate",
  "SecKeychain",
  "CredReadW",
  "CredWriteW",
  "CredDeleteW",
];

export function assertNoCredentialStoreEntrypoints(bytes, importedSymbols = "") {
  const searchable = [
    importedSymbols,
    bytes.toString("latin1"),
    bytes.length % 2 === 0 ? bytes.toString("utf16le") : "",
  ].join("\n");
  const found = forbiddenEntrypoints.filter((entrypoint) => searchable.includes(entrypoint));
  if (found.length > 0) {
    throw new Error(`native client contains operating-system credential-store entrypoints: ${found.join(", ")}`);
  }
}

function requiredOption(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) throw new Error(`${name} is required`);
  return process.argv[index + 1];
}

async function main() {
  const binary = path.resolve(requiredOption("--binary"));
  const platform = requiredOption("--platform");
  if (platform !== "macos" && platform !== "windows") {
    throw new Error("--platform must be macos or windows");
  }
  let importedSymbols = "";
  if (platform === "macos") {
    const nm = spawnSync("/usr/bin/nm", ["-u", binary], { encoding: "utf8" });
    if (nm.error) throw nm.error;
    if (nm.status !== 0) throw new Error(`nm exited with status ${nm.status}`);
    importedSymbols = nm.stdout;
  }
  const bytes = await readFile(binary);
  assertNoCredentialStoreEntrypoints(bytes, importedSymbols);
  console.log(`native credential boundary passed (${platform}, ${bytes.length} bytes)`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
