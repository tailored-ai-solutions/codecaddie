#!/usr/bin/env node

import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const target = process.argv[2];
if (!target) throw new Error("usage: node scripts/normalize-license-evidence.mjs <file>");

const resolved = path.resolve(target);
const original = await readFile(resolved, "utf8");
const normalized = `${original
  .replaceAll("\r\n", "\n")
  .split("\n")
  .map((line) => line.trimEnd())
  .join("\n")
  .trimEnd()}\n`;

await writeFile(resolved, normalized, "utf8");
