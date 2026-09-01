#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packagePath = path.join(root, "package.json");
const releaseVersionPath = path.join(root, "config/release-version.json");
const canonical = JSON.parse(fs.readFileSync(packagePath, "utf8")).version;
const semver = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

const files = [
  { path: "Cargo.toml", pattern: /(?<=\[workspace\.package\][\s\S]*?version = ")[^"]+(?=")/ },
  { path: "Cargo.lock", pattern: /(?<=name = "codecaddie-core"\nversion = ")[^"]+(?=")/ },
  { path: "Cargo.lock", pattern: /(?<=name = "codecaddie-domain"\nversion = ")[^"]+(?=")/ },
  { path: "apps/desktop/app.zon", pattern: /(?<=\.version = ")[^"]+(?=")/ },
  { path: "apps/desktop/build.zig.zon", pattern: /(?<=\.version = ")[^"]+(?=")/ },
  { path: "scripts/package-macos.sh", pattern: /(?<=^VERSION=")[^"]+(?="$)/m },
  { path: "scripts/package-windows.ps1", pattern: /(?<=\[string\]\$Version = ")[^"]+(?=")/ },
  {
    path: "xcode/CodeCaddie/Info.plist",
    pattern: /(?<=<key>CFBundleShortVersionString<\/key>\s*<string>)[^<]+(?=<\/string>)/g,
  },
  {
    path: "xcode/CodeCaddie.xcodeproj/project.pbxproj",
    pattern: /(?<=MARKETING_VERSION = )[^;]+(?=;)/g,
  },
];

function values() {
  return files.map((file) => {
    const text = fs.readFileSync(path.join(root, file.path), "utf8");
    const matches = file.pattern.global
      ? [...text.matchAll(file.pattern)].map((match) => match[0])
      : [text.match(file.pattern)?.[0]].filter(Boolean);
    if (!matches.length) throw new Error(`could not find version in ${file.path}`);
    return { ...file, values: matches, text };
  });
}

function check() {
  if (!semver.test(canonical)) throw new Error(`package.json has invalid SemVer: ${canonical}`);
  const releaseVersion = JSON.parse(fs.readFileSync(releaseVersionPath, "utf8"));
  const prefix = canonical.match(/^(\d+\.\d+)\./)?.[1];
  if (
    releaseVersion.schemaVersion !== 1
    || !Number.isSafeInteger(releaseVersion.releaseBuildEpoch)
    || releaseVersion.releaseBuildEpoch < 2000
    || releaseVersion.msiVersionPrefix !== prefix
    || !Number.isSafeInteger(releaseVersion.msiBuildNumberEpoch)
    || releaseVersion.msiBuildNumberEpoch < 0
  ) {
    throw new Error(
      "config/release-version.json must contain a release build epoch of at least 2000, the current major/minor prefix, and a nonnegative MSI build-number epoch",
    );
  }
  const drift = values().flatMap((entry) =>
    entry.values
      .filter((value) => value !== canonical)
      .map((value) => ({ path: entry.path, value })),
  );
  if (drift.length) {
    for (const entry of drift) console.error(`${entry.path}: ${entry.value} (expected ${canonical})`);
    process.exitCode = 1;
    return;
  }
  console.log(`version ${canonical} is synchronized`);
}

function setVersion(version) {
  if (!semver.test(version ?? "")) throw new Error("usage: pnpm version:set -- X.Y.Z[-prerelease]");
  const prefix = version.match(/^(\d+\.\d+)\./)?.[1];
  if (!prefix) throw new Error("version must include major, minor, and patch components");
  const packageJson = JSON.parse(fs.readFileSync(packagePath, "utf8"));
  packageJson.version = version;
  fs.writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);
  const releaseVersion = JSON.parse(fs.readFileSync(releaseVersionPath, "utf8"));
  releaseVersion.msiVersionPrefix = prefix;
  fs.writeFileSync(releaseVersionPath, `${JSON.stringify(releaseVersion, null, 2)}\n`);
  for (const entry of values()) {
    const target = path.join(root, entry.path);
    const current = fs.readFileSync(target, "utf8");
    fs.writeFileSync(target, current.replace(entry.pattern, version));
  }
  console.log(`set CodeCaddie version to ${version}`);
}

const [command, version] = process.argv.slice(2);
try {
  if (command === "check") check();
  else if (command === "set") setVersion(version);
  else throw new Error("usage: node scripts/version.mjs check|set [version]");
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
