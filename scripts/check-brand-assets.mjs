#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function checkPng(relativePath, width, height) {
  const bytes = fs.readFileSync(path.join(root, relativePath));
  if (bytes.toString("ascii", 1, 4) !== "PNG") throw new Error(`${relativePath} is not a PNG`);
  if (bytes.readUInt32BE(16) !== width || bytes.readUInt32BE(20) !== height) {
    throw new Error(`${relativePath} must be ${width} by ${height}`);
  }
}

checkPng("apps/desktop/assets/icon.png", 1024, 1024);
checkPng("apps/desktop/assets/brand-mark.png", 256, 256);

const monogram = fs.readFileSync(
  path.join(root, "apps/desktop/assets/codecaddie-monogram.svg"),
  "utf8",
);
for (const color of ["#161B18", "#3FD59F"]) {
  if (!monogram.includes(color)) {
    throw new Error(`codecaddie-monogram.svg is missing Green Ink color ${color}`);
  }
}
if (!monogram.toLowerCase().includes("codecaddie")) {
  throw new Error("codecaddie-monogram.svg is missing the CodeCaddie identity string");
}
console.log("desktop brand assets are canonical and correctly sized");
