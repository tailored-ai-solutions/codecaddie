#!/usr/bin/env node
// Opt-in, idempotent pstack setup for agents that read ~/.agents/skills
// (Codex and Grok Build). It sparse-clones cursor/plugins at the pinned
// commit into ~/.agents/pstack-src and symlinks every pstack skill into
// ~/.agents/skills. It never runs from install or CI, refuses to overwrite
// anything that is not a symlink, and with --dry-run touches neither the
// network nor the filesystem.
//
//   pnpm agents:setup --codex [--grok] [--dry-run]
import { spawnSync } from "node:child_process";
import { lstatSync, mkdirSync, readdirSync, readlinkSync, symlinkSync, unlinkSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

/** The upstream pstack pin. Keep in step with `pstack-upstream` in
 * `.claude-plugin/marketplace.json`; `scripts/tests/agent-config.test.mjs`
 * asserts they match. Bump procedure: docs/decisions/0009. */
export const PSTACK_PIN = Object.freeze({
  url: "https://github.com/cursor/plugins.git",
  path: "pstack",
  sha: "23a56e2dac2efd54788056db8eced26e371d7b5e",
});

/** Skill directories under `pstack/skills` at the pinned commit. Used only to
 * print a --dry-run plan before the source exists locally; a real run lists
 * the checked-out directory instead. */
export const KNOWN_PSTACK_SKILLS = Object.freeze([
  "architect",
  "arena",
  "automate-me",
  "blast-radius",
  "bro",
  "create-verification-skill",
  "figure-it-out",
  "how",
  "interrogate",
  "maintain-verification-skill",
  "make-bot-ui",
  "no-comments",
  "poteto-mode",
  "principle-boundary-discipline",
  "principle-build-the-lever",
  "principle-encode-lessons-in-structure",
  "principle-exhaust-the-design-space",
  "principle-experience-first",
  "principle-fix-root-causes",
  "principle-foundational-thinking",
  "principle-guard-the-context-window",
  "principle-laziness-protocol",
  "principle-make-operations-idempotent",
  "principle-migrate-callers-then-delete-legacy-apis",
  "principle-minimize-reader-load",
  "principle-model-the-domain",
  "principle-never-block-on-the-human",
  "principle-outcome-oriented-execution",
  "principle-prove-it-works",
  "principle-redesign-from-first-principles",
  "principle-separate-before-serializing-shared-state",
  "principle-sequence-verifiable-units",
  "principle-subtract-before-you-add",
  "principle-type-system-discipline",
  "recall",
  "reflect",
  "setup-pstack",
  "show-me-your-work",
  "swarm",
  "tdd",
  "teach",
  "technical-writing",
  "typescript-best-practices",
  "unslop",
  "why",
]);

export const USAGE = "usage: agents-setup.mjs (--codex | --grok)... [--dry-run]";

export function parseArguments(argv) {
  const options = { codex: false, grok: false, dryRun: false, help: false };
  for (const argument of argv) {
    switch (argument) {
      case "--codex":
        options.codex = true;
        break;
      case "--grok":
        options.grok = true;
        break;
      case "--dry-run":
        options.dryRun = true;
        break;
      case "--help":
      case "-h":
        options.help = true;
        break;
      default:
        throw new Error(`unknown argument ${argument}\n${USAGE}`);
    }
  }
  if (!options.help && !options.codex && !options.grok) {
    throw new Error(`choose at least one target\n${USAGE}`);
  }
  return options;
}

export function defaultLayout(home = homedir()) {
  return {
    home,
    sourceDir: join(home, ".agents", "pstack-src"),
    skillsDir: join(home, ".agents", "skills"),
  };
}

/** Plans the source checkout from observed state. `headSha` is the commit
 * currently checked out in `sourceDir`, or null when there is no clone. */
export function planSource({ sourceDir, headSha, sparse }) {
  if (headSha === null) {
    return [
      { kind: "clone", sourceDir, url: PSTACK_PIN.url },
      { kind: "sparse-checkout", sourceDir, path: PSTACK_PIN.path },
      { kind: "checkout", sourceDir, sha: PSTACK_PIN.sha },
    ];
  }
  const steps = [];
  if (!sparse) steps.push({ kind: "sparse-checkout", sourceDir, path: PSTACK_PIN.path });
  if (headSha !== PSTACK_PIN.sha) {
    steps.push({ kind: "fetch", sourceDir, sha: PSTACK_PIN.sha });
    steps.push({ kind: "checkout", sourceDir, sha: PSTACK_PIN.sha });
  }
  if (steps.length === 0) steps.push({ kind: "up-to-date", sourceDir, sha: PSTACK_PIN.sha });
  return steps;
}

/**
 * Plans one symlink per skill. `inspect(path)` returns null when nothing
 * exists there, `{ kind: "symlink", target }`, or `{ kind: "directory" | "file" }`.
 * Existing symlinks are skipped when correct and relinked otherwise; anything
 * that is not a symlink is a conflict the caller must refuse to overwrite.
 */
export function planLinks({ sourceDir, skillsDir, skillNames, inspect }) {
  const skillsRoot = join(sourceDir, PSTACK_PIN.path, "skills");
  const actions = [];
  for (const name of [...new Set(skillNames)].sort()) {
    const target = join(skillsRoot, name);
    const linkPath = join(skillsDir, name);
    const existing = inspect(linkPath);
    if (existing === null) {
      actions.push({ kind: "link", name, linkPath, target });
    } else if (existing.kind === "symlink") {
      const resolved = resolve(skillsDir, existing.target);
      if (resolved === target) actions.push({ kind: "skip", name, linkPath, target });
      else actions.push({ kind: "relink", name, linkPath, target, previous: existing.target });
    } else {
      actions.push({ kind: "conflict", name, linkPath, target, previous: existing.kind });
    }
  }
  return actions;
}

export function describe(step) {
  switch (step.kind) {
    case "clone":
      return `clone ${step.url} (blobless, no checkout) into ${step.sourceDir}`;
    case "sparse-checkout":
      return `sparse-checkout ${step.path}/ in ${step.sourceDir}`;
    case "fetch":
      return `fetch ${step.sha} in ${step.sourceDir}`;
    case "checkout":
      return `checkout ${step.sha} (detached) in ${step.sourceDir}`;
    case "up-to-date":
      return `source already at ${step.sha}`;
    case "link":
      return `link ${step.linkPath} -> ${step.target}`;
    case "relink":
      return `relink ${step.linkPath} -> ${step.target} (was ${step.previous})`;
    case "skip":
      return `skip ${step.linkPath} (already linked)`;
    case "conflict":
      return `conflict ${step.linkPath} is a ${step.previous}, not a symlink; move it aside and rerun`;
    default:
      return JSON.stringify(step);
  }
}

function inspectPath(path) {
  let stat;
  try {
    stat = lstatSync(path);
  } catch {
    return null;
  }
  if (stat.isSymbolicLink()) return { kind: "symlink", target: readlinkSync(path) };
  if (stat.isDirectory()) return { kind: "directory" };
  return { kind: "file" };
}

function git(sourceDir, args, { allowFailure = false } = {}) {
  const result = spawnSync("git", ["-C", sourceDir, ...args], { encoding: "utf8" });
  if (result.status !== 0 && !allowFailure) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  }
  return result;
}

function observeSource(sourceDir) {
  if (inspectPath(join(sourceDir, ".git")) === null) return { headSha: null, sparse: false };
  const head = git(sourceDir, ["rev-parse", "HEAD"], { allowFailure: true });
  const headSha = head.status === 0 ? head.stdout.trim() : null;
  const list = git(sourceDir, ["sparse-checkout", "list"], { allowFailure: true });
  const sparse = list.status === 0 && list.stdout.split("\n").map((line) => line.trim()).includes(PSTACK_PIN.path);
  return { headSha, sparse };
}

function listSkills(sourceDir) {
  const skillsRoot = join(sourceDir, PSTACK_PIN.path, "skills");
  if (inspectPath(skillsRoot) === null) return null;
  return readdirSync(skillsRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name);
}

function executeSource(step) {
  switch (step.kind) {
    case "clone": {
      const parent = resolve(step.sourceDir, "..");
      mkdirSync(parent, { recursive: true });
      const result = spawnSync(
        "git",
        ["clone", "--quiet", "--filter=blob:none", "--no-checkout", step.url, step.sourceDir],
        { encoding: "utf8" },
      );
      if (result.status !== 0) throw new Error(`git clone failed: ${result.stderr.trim()}`);
      return;
    }
    case "sparse-checkout":
      git(step.sourceDir, ["sparse-checkout", "set", step.path]);
      return;
    case "fetch":
      git(step.sourceDir, ["fetch", "--quiet", "origin", step.sha]);
      return;
    case "checkout":
      git(step.sourceDir, ["checkout", "--quiet", "--detach", step.sha]);
      return;
    case "up-to-date":
      return;
    default:
      throw new Error(`unknown source step ${step.kind}`);
  }
}

function executeLink(action) {
  if (action.kind === "skip") return;
  if (action.kind === "relink") unlinkSync(action.linkPath);
  symlinkSync(action.target, action.linkPath, process.platform === "win32" ? "junction" : "dir");
}

function hints(options) {
  const lines = [];
  if (options.codex) {
    lines.push("Codex reads ~/.agents/skills; start work with $poteto-mode.");
  }
  if (options.grok) {
    lines.push(
      "Grok Build prefers the plugin route: grok plugin marketplace add tailored-ai-solutions/codecaddie && grok plugin install pstack --trust",
      "  (fallback: grok plugin marketplace add EnzoTironi/zoen-skills && grok plugin install pstack --trust).",
      "Grok Bot: grokbot://app/v1/plugin/add?id=9717366",
    );
  }
  return lines;
}

export function main(argv, { layout = defaultLayout(), log = console.log } = {}) {
  let options;
  try {
    options = parseArguments(argv);
  } catch (error) {
    console.error(error.message);
    return 2;
  }
  if (options.help) {
    log(USAGE);
    return 0;
  }
  const { sourceDir, skillsDir } = layout;
  const sourceSteps = planSource({ sourceDir, ...observeSource(sourceDir) });
  const skillNames = listSkills(sourceDir) ?? KNOWN_PSTACK_SKILLS;
  const links = planLinks({ sourceDir, skillsDir, skillNames, inspect: inspectPath });
  const conflicts = links.filter((action) => action.kind === "conflict");

  log(`pstack source: ${sourceDir} (pinned ${PSTACK_PIN.sha})`);
  log(`skills directory: ${skillsDir}`);
  log(options.dryRun ? "plan (dry run, nothing executed):" : "plan:");
  for (const step of sourceSteps) log(`  ${describe(step)}`);
  for (const action of links) log(`  ${describe(action)}`);
  if (options.dryRun) {
    for (const line of hints(options)) log(line);
    return 0;
  }
  if (conflicts.length > 0) {
    console.error(`refusing to continue: ${conflicts.length} path(s) in ${skillsDir} are not symlinks`);
    return 1;
  }

  for (const step of sourceSteps) executeSource(step);
  const actual = listSkills(sourceDir);
  if (actual === null || actual.length === 0) {
    console.error(`no skills found under ${join(sourceDir, PSTACK_PIN.path, "skills")} after checkout`);
    return 1;
  }
  mkdirSync(skillsDir, { recursive: true });
  const finalLinks = planLinks({ sourceDir, skillsDir, skillNames: actual, inspect: inspectPath });
  const lateConflicts = finalLinks.filter((action) => action.kind === "conflict");
  if (lateConflicts.length > 0) {
    console.error(`refusing to continue: ${lateConflicts.length} path(s) in ${skillsDir} are not symlinks`);
    return 1;
  }
  for (const action of finalLinks) executeLink(action);
  const counts = finalLinks.reduce((sum, action) => ({ ...sum, [action.kind]: (sum[action.kind] ?? 0) + 1 }), {});
  log(`done: ${JSON.stringify(counts)}`);
  for (const line of hints(options)) log(line);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exit(main(process.argv.slice(2)));
}
