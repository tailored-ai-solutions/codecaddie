import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  KNOWN_PSTACK_SKILLS,
  PSTACK_PIN,
  main,
  parseArguments,
  planLinks,
  planSource,
} from "../agents-setup.mjs";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const script = path.join(repositoryRoot, "scripts/agents-setup.mjs");

test("the upstream pin is a full commit sha with a bundled skill list", () => {
  assert.match(PSTACK_PIN.sha, /^[0-9a-f]{40}$/);
  assert.equal(PSTACK_PIN.url, "https://github.com/cursor/plugins.git");
  assert.equal(PSTACK_PIN.path, "pstack");
  assert.ok(KNOWN_PSTACK_SKILLS.length > 20);
  assert.deepEqual([...KNOWN_PSTACK_SKILLS], [...new Set(KNOWN_PSTACK_SKILLS)].sort());
  assert.ok(KNOWN_PSTACK_SKILLS.includes("poteto-mode"));
  assert.ok(KNOWN_PSTACK_SKILLS.includes("principle-prove-it-works"));
});

test("arguments require a target and reject unknown flags", () => {
  assert.throws(() => parseArguments([]), /choose at least one target/);
  assert.throws(() => parseArguments(["--codex", "--nope"]), /unknown argument --nope/);
  assert.deepEqual(parseArguments(["--codex"]), { codex: true, grok: false, dryRun: false, help: false });
  assert.deepEqual(parseArguments(["--grok", "--dry-run"]), { codex: false, grok: true, dryRun: true, help: false });
  assert.equal(parseArguments(["--help"]).help, true);
});

test("the source plan clones once, then only fetches when the pin moved", () => {
  const sourceDir = "/Users/example/.agents/pstack-src";
  assert.deepEqual(
    planSource({ sourceDir, headSha: null, sparse: false }).map((step) => step.kind),
    ["clone", "sparse-checkout", "checkout"],
  );
  assert.deepEqual(
    planSource({ sourceDir, headSha: PSTACK_PIN.sha, sparse: true }).map((step) => step.kind),
    ["up-to-date"],
  );
  assert.deepEqual(
    planSource({ sourceDir, headSha: "0".repeat(40), sparse: true }).map((step) => step.kind),
    ["fetch", "checkout"],
  );
  assert.deepEqual(
    planSource({ sourceDir, headSha: PSTACK_PIN.sha, sparse: false }).map((step) => step.kind),
    ["sparse-checkout"],
  );
});

test("the link planner is idempotent and never plans over a non-symlink", () => {
  const sourceDir = "/Users/example/.agents/pstack-src";
  const skillsDir = "/Users/example/.agents/skills";
  const target = (name) => path.join(sourceDir, "pstack", "skills", name);
  const existing = {
    [path.join(skillsDir, "how")]: { kind: "symlink", target: target("how") },
    [path.join(skillsDir, "why")]: { kind: "symlink", target: "/Users/example/elsewhere/why" },
    [path.join(skillsDir, "tdd")]: { kind: "directory" },
  };
  const plan = planLinks({
    sourceDir,
    skillsDir,
    skillNames: ["why", "how", "tdd", "swarm", "how"],
    inspect: (candidate) => existing[candidate] ?? null,
  });
  assert.deepEqual(
    plan.map((action) => [action.name, action.kind]),
    [["how", "skip"], ["swarm", "link"], ["tdd", "conflict"], ["why", "relink"]],
  );
  assert.equal(plan.find((action) => action.name === "swarm").target, target("swarm"));
  assert.equal(plan.find((action) => action.name === "why").previous, "/Users/example/elsewhere/why");
  assert.equal(plan.find((action) => action.name === "tdd").previous, "directory");
});

test("a dry run prints the full plan and touches nothing", async (t) => {
  const home = await mkdtemp(path.join(os.tmpdir(), "codecaddie-agents-setup-"));
  t.after(() => rm(home, { recursive: true, force: true }));
  const result = spawnSync(process.execPath, [script, "--codex", "--grok", "--dry-run"], {
    encoding: "utf8",
    env: { ...process.env, HOME: home, USERPROFILE: home },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /dry run, nothing executed/);
  assert.match(result.stdout, /clone https:\/\/github\.com\/cursor\/plugins\.git/);
  assert.match(result.stdout, new RegExp(`checkout ${PSTACK_PIN.sha}`));
  const linkLines = result.stdout.split("\n").filter((line) => /^ {2}link /.test(line));
  assert.equal(linkLines.length, KNOWN_PSTACK_SKILLS.length);
  assert.match(result.stdout, /\$poteto-mode/);
  assert.match(result.stdout, /grok plugin install pstack --trust/);
  assert.deepEqual(await readdir(home), [], "a dry run must not create anything under the home directory");
});

test("main refuses to run without a target and reports usage", () => {
  const lines = [];
  assert.equal(main([], { log: (line) => lines.push(line) }), 2);
  assert.equal(main(["--help"], { log: (line) => lines.push(line) }), 0);
  assert.match(lines.join("\n"), /usage: agents-setup\.mjs/);
});
