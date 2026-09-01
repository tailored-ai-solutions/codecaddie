#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import opentype from "opentype.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const assetRoot = path.join(root, "apps/desktop/assets");
const monogramSource = path.join(assetRoot, "codecaddie-monogram.svg");
const iconOutput = path.join(assetRoot, "icon.png");
const brandMarkOutput = path.join(assetRoot, "brand-mark.png");

// Green Ink palette.
const recordInk = "#161B18";
const phosphorGreen = "#3FD59F";

const fontBytes = fs.readFileSync(path.join(root, "node_modules/@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-600-normal.woff"));
const font = opentype.parse(fontBytes.buffer.slice(fontBytes.byteOffset, fontBytes.byteOffset + fontBytes.byteLength));

function runSips(args) {
  const result = spawnSync("sips", args, { stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function assertPngSize(file, expectedWidth, expectedHeight) {
  const bytes = fs.readFileSync(file);
  if (bytes.readUInt32BE(16) !== expectedWidth || bytes.readUInt32BE(20) !== expectedHeight) {
    throw new Error(`${file} must be ${expectedWidth} by ${expectedHeight}`);
  }
}

// Monogram seal: phosphor-green cc on record ink with an engraved keyline.
// 436px keeps the same ink footprint as the previous 500px Baumans mark
// (Plex Mono's cc is wider per em, so the size comes down slightly).
const monogramFontSize = 436;
const initial = font.getPath("cc", 0, 0, monogramFontSize);
const bounds = initial.getBoundingBox();
const ccWidth = bounds.x2 - bounds.x1;
const ccHeight = bounds.y2 - bounds.y1;
const monogram = font.getPath("cc", (1024 - ccWidth) / 2 - bounds.x1, (1024 - ccHeight) / 2 - bounds.y1, monogramFontSize);
fs.writeFileSync(monogramSource, `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" role="img" aria-labelledby="title description">
  <title id="title">CodeCaddie</title>
  <description id="description">Lowercase IBM Plex Mono cc seal in phosphor green on record ink with an engraved keyline</description>
  <rect width="1024" height="1024" rx="228" fill="${recordInk}"/>
  <rect x="56" y="56" width="912" height="912" rx="172" fill="none" stroke="${phosphorGreen}" stroke-opacity="0.45" stroke-width="12"/>
  <path fill="${phosphorGreen}" d="${monogram.toPathData(2)}"/>
</svg>\n`);

if (process.platform !== "darwin") throw new Error("brand raster generation currently requires macOS sips");

// Desktop icon: 1024px raster of the monogram seal.
runSips(["-s", "format", "png", monogramSource, "--out", iconOutput]);
assertPngSize(iconOutput, 1024, 1024);
console.log(`generated ${iconOutput}`);

// Brand mark: 256px resample of the icon.
runSips(["-z", "256", "256", iconOutput, "--out", brandMarkOutput]);
assertPngSize(brandMarkOutput, 256, 256);
console.log(`generated ${brandMarkOutput}`);
