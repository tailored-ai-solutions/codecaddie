#!/usr/bin/env node
import { execFileSync } from "node:child_process";

const [repository, tag, required, ...extra] = process.argv.slice(2);
try {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? "") ||
      !/^v\d+\.\d+\.\d+(?:-rc\.\d+)?\+[1-9]\d*$/.test(tag ?? "") ||
      (required !== undefined && required !== "--required") || extra.length) {
    throw new Error("usage: find-github-release.mjs OWNER/REPO TAG [--required]");
  }

  // The tag endpoint omits drafts. A successful authenticated list must cover
  // every page before absence can authorize creating a release.
  const pages = JSON.parse(execFileSync("gh", [
    "api", "--paginate", "--slurp", `repos/${repository}/releases?per_page=100`,
  ], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] }));
  if (!Array.isArray(pages) || !pages.length || pages.some(page => !Array.isArray(page))) {
    throw new Error("GitHub returned an invalid paginated release list");
  }
  const releases = pages.flat();
  if (releases.some(release => !release || typeof release.tag_name !== "string")) {
    throw new Error("GitHub returned an invalid release entry");
  }
  const matches = releases.filter(release => release.tag_name === tag);
  if (matches.length > 1) throw new Error(`Multiple GitHub releases use ${tag}`);
  const release = matches[0] ?? null;
  if (release) {
    if (!Number.isSafeInteger(release.id) || release.id <= 0 ||
        typeof release.target_commitish !== "string" ||
        [release.draft, release.prerelease, release.immutable].some(value => typeof value !== "boolean") ||
        !Array.isArray(release.assets) || release.assets.some(asset => !asset || typeof asset.name !== "string")) {
      throw new Error(`GitHub returned invalid release metadata for ${tag}`);
    }
    const names = release.assets.map(asset => asset.name);
    if (new Set(names).size !== names.length) throw new Error(`Duplicate GitHub release asset names for ${tag}`);
  } else if (required) {
    throw new Error(`GitHub release ${tag} was not found in the authenticated release list`);
  }
  process.stdout.write(`${JSON.stringify(release)}\n`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
