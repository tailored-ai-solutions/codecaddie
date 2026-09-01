import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");
const matrix = JSON.parse(read("config/source-canary-matrix-v1.json"));

test("source-canary matrix binds every required privacy surface to executable tests", () => {
  assert.equal(matrix.schemaVersion, 1);
  assert.equal(
    matrix.privacyGate,
    "cargo test --workspace --locked privacy_adversarial -- --nocapture",
  );
  assert.deepEqual(
    matrix.surfaces.map(({ id }) => id).sort(),
    [...matrix.requiredSurfaces].sort(),
  );
  assert.equal(new Set(matrix.requiredSurfaces).size, matrix.requiredSurfaces.length);

  for (const fixture of matrix.fixtures) {
    assert.match(read(fixture), /(?:REPOSITORY|ATTACHMENT)_PRIVATE_SENTINEL_/);
  }

  for (const surface of matrix.surfaces) {
    assert.match(surface.id, /^[a-z0-9-]+$/);
    assert.ok(surface.assertion.length >= 60);
    assert.ok(surface.rustTests.length >= 1);
    const sourceFiles = surface.sourceFiles ?? [surface.sourceFile];
    assert.ok(sourceFiles.length >= 1);
    const source = sourceFiles.map(read).join("\n");
    for (const rustTest of surface.rustTests) {
      assert.match(rustTest, /^privacy_adversarial_/);
      assert.ok(
        source.includes(`${rustTest}(`),
        `${surface.id} is missing executable test ${rustTest}`,
      );
    }
  }
});

test("the named privacy CI job runs the exact canary matrix and Rust filter", () => {
  const workflow = read(".github/workflows/ci.yml");
  assert.match(workflow, /Adversarial privacy and prompt-injection gate/);
  assert.match(
    workflow,
    /adversarial-privacy:[\s\S]*?node-version: 24\n\s+package-manager-cache: false/,
  );
  assert.match(workflow, /node --test scripts\/tests\/source-canary-matrix\.test\.mjs/);
  assert.ok(workflow.includes(`run: ${matrix.privacyGate}`));
});
