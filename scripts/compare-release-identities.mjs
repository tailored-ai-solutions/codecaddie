#!/usr/bin/env node

import { pathToFileURL } from "node:url";

function parseStableVersion(value) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(value ?? "");
  if (!match) throw new Error("stable release version must be X.Y.Z");
  const components = match.slice(1).map(Number);
  if (components.some((component) => !Number.isSafeInteger(component))) {
    throw new Error("stable release version components must be safe integers");
  }
  return components;
}

export function compareReleaseIdentities(left, right) {
  for (const identity of [left, right]) {
    if (!Number.isSafeInteger(identity.build) || identity.build < 1) {
      throw new Error("stable release build must be a positive safe integer");
    }
  }
  const leftVersion = parseStableVersion(left.version);
  const rightVersion = parseStableVersion(right.version);
  for (let index = 0; index < leftVersion.length; index += 1) {
    if (leftVersion[index] !== rightVersion[index]) {
      return Math.sign(leftVersion[index] - rightVersion[index]);
    }
  }
  return Math.sign(left.build - right.build);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [leftVersion, leftBuild, rightVersion, rightBuild] = process.argv.slice(2);
  const comparison = compareReleaseIdentities(
    { version: leftVersion, build: Number(leftBuild) },
    { version: rightVersion, build: Number(rightBuild) },
  );
  console.log(comparison);
}
