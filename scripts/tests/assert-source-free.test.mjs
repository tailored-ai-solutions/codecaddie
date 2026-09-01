import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { DEFAULT_CANARIES, assertSourceFree, parseArguments } from "../assert-source-free.mjs";

const root = new URL("../../", import.meta.url);
const script = fileURLToPath(new URL("scripts/assert-source-free.mjs", root));
const rustSentinels = await readFile(
  new URL("crates/codecaddie-core/src/privacy_test_support.rs", root),
  "utf8",
);
const installedJourney = await readFile(new URL("scripts/exercise-installed-core.mjs", root), "utf8");

async function scratch(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "codecaddie-source-free-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

test("the default canaries mirror the Rust privacy sentinels and the journey canary", () => {
  for (const sentinel of rustSentinels.matchAll(/_SENTINEL: &str = "([A-Z_0-9]+)"/g)) {
    assert.ok(DEFAULT_CANARIES.includes(sentinel[1]), `${sentinel[1]} must be a default canary`);
  }
  assert.ok(installedJourney.includes(`"${DEFAULT_CANARIES[2]} must never enter CodeCaddie output"`));
  assert.equal(DEFAULT_CANARIES.length, 3);
});

test("a directory without canaries or the fixture path passes and reports coverage", async (t) => {
  const directory = await scratch(t);
  const fixture = path.join(directory, "elsewhere", "repository");
  await mkdir(path.join(directory, "data", "nested"), { recursive: true });
  await writeFile(path.join(directory, "data", "report.json"), '{"verdict":"supported"}\n');
  await writeFile(path.join(directory, "data", "nested", "export.bin"), Buffer.from([0x50, 0x4b, 0, 1]));
  const result = assertSourceFree({ directory: path.join(directory, "data"), fixtureRepository: fixture });
  assert.equal(result.scannedFiles, 2);
  assert.ok(result.forbiddenMarkers >= 4);
});

test("a canary or the fixture repository's absolute path fails and names the file", async (t) => {
  const directory = await scratch(t);
  const fixture = path.join(directory, "repository");
  await mkdir(path.join(directory, "data"), { recursive: true });
  await writeFile(path.join(directory, "data", "leak.txt"), `prefix ${DEFAULT_CANARIES[0]} suffix\n`);
  assert.throws(
    () => assertSourceFree({ directory: path.join(directory, "data"), fixtureRepository: fixture }),
    /leak\.txt: contains canary "REPOSITORY_PRIVATE_SENTINEL_7DB9562A"/,
  );

  await writeFile(path.join(directory, "data", "leak.txt"), `saw ${fixture}/journey.txt\n`);
  assert.throws(
    () => assertSourceFree({ directory: path.join(directory, "data"), fixtureRepository: fixture }),
    /leak\.txt: contains fixture repository path/,
  );

  await writeFile(path.join(directory, "data", "leak.txt"), "custom marker here\n");
  assert.throws(
    () => assertSourceFree({ directory: path.join(directory, "data"), canaries: ["custom marker"] }),
    /contains canary "custom marker"/,
  );
});

test("a fixture repository nested under the scanned directory is skipped, symlinks are ignored", async (t) => {
  const directory = await scratch(t);
  const fixture = path.join(directory, "repository");
  await mkdir(fixture, { recursive: true });
  await mkdir(path.join(directory, "data"), { recursive: true });
  await writeFile(path.join(fixture, "journey.txt"), `${DEFAULT_CANARIES[2]} must never enter CodeCaddie output\n`);
  await writeFile(path.join(directory, "data", "state.json"), "{}\n");
  await symlink(fixture, path.join(directory, "data", "link-to-repository"));
  const result = assertSourceFree({ directory, fixtureRepository: fixture });
  assert.equal(result.scannedFiles, 1);
  assert.throws(() => assertSourceFree({ directory }), /journey\.txt: contains canary/);
});

test("the command line scans a directory and exits non-zero on a leak", async (t) => {
  const directory = await scratch(t);
  await mkdir(path.join(directory, "data"), { recursive: true });
  await writeFile(path.join(directory, "data", "clean.txt"), "nothing here\n");
  assert.deepEqual(parseArguments(["--directory", "d", "--fixture-repo", "r", "--canary", "x"]), {
    directory: "d",
    fixtureRepository: "r",
    canaries: [...DEFAULT_CANARIES, "x"],
  });
  assert.throws(() => parseArguments([]), /usage/);
  assert.throws(() => parseArguments(["--bogus"]), /unexpected argument --bogus/);

  const passed = execFileSync(
    process.execPath,
    [script, "--directory", path.join(directory, "data"), "--fixture-repo", path.join(directory, "repository")],
    { encoding: "utf8" },
  );
  assert.match(passed, /source-free: scanned 1 files under/);

  await writeFile(path.join(directory, "data", "clean.txt"), `${DEFAULT_CANARIES[1]}\n`);
  const failed = spawnSync(process.execPath, [script, "--directory", path.join(directory, "data")], {
    encoding: "utf8",
  });
  assert.notEqual(failed.status, 0);
  assert.match(failed.stderr, /private source escaped into/);
});
