import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);

async function writeFixture(root, relative, contents) {
  const target = path.join(root, relative);
  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, contents);
}

test("version updates keep RC and stable Cargo metadata locked", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "codecaddie-version-"));
  await mkdir(path.join(root, "scripts"), { recursive: true });
  await copyFile(
    path.join(repositoryRoot, "scripts/version.mjs"),
    path.join(root, "scripts/version.mjs"),
  );
  // The fixture Cargo workspace uses edition 2024, and the temporary
  // directory sits outside the repository, so rustup would otherwise fall
  // back to the machine's default toolchain. Copying the repository pin
  // keeps `cargo metadata` on the same toolchain the workspace requires.
  await copyFile(
    path.join(repositoryRoot, "rust-toolchain.toml"),
    path.join(root, "rust-toolchain.toml"),
  );
  await writeFixture(root, "package.json", '{"name":"fixture","version":"0.3.0"}\n');
  await writeFixture(
    root,
    "config/release-version.json",
    '{"schemaVersion":1,"releaseBuildEpoch":2000,"msiVersionPrefix":"0.3","msiBuildNumberEpoch":0}\n',
  );
  await writeFixture(
    root,
    "Cargo.toml",
    '[workspace]\nmembers = ["crates/core", "crates/domain"]\nresolver = "2"\n\n[workspace.package]\nversion = "0.3.0"\nedition = "2024"\n',
  );
  await writeFixture(
    root,
    "Cargo.lock",
    'version = 4\n\n[[package]]\nname = "codecaddie-core"\nversion = "0.3.0"\n\n[[package]]\nname = "codecaddie-domain"\nversion = "0.3.0"\n',
  );
  await writeFixture(
    root,
    "crates/core/Cargo.toml",
    '[package]\nname = "codecaddie-core"\nversion.workspace = true\nedition.workspace = true\n',
  );
  await writeFixture(root, "crates/core/src/lib.rs", "pub fn core() {}\n");
  await writeFixture(
    root,
    "crates/domain/Cargo.toml",
    '[package]\nname = "codecaddie-domain"\nversion.workspace = true\nedition.workspace = true\n',
  );
  await writeFixture(root, "crates/domain/src/lib.rs", "pub fn domain() {}\n");
  await writeFixture(root, "apps/desktop/app.zon", '.version = "0.3.0",\n');
  await writeFixture(root, "apps/desktop/build.zig.zon", '.version = "0.3.0",\n');
  await writeFixture(root, "scripts/package-macos.sh", 'VERSION="0.3.0"\n');
  await writeFixture(root, "scripts/package-windows.ps1", '[string]$Version = "0.3.0",\n');
  await writeFixture(
    root,
    "xcode/CodeCaddie/Info.plist",
    "<plist><dict><key>CFBundleShortVersionString</key><string>0.3.0</string></dict></plist>\n",
  );
  await writeFixture(
    root,
    "xcode/CodeCaddie.xcodeproj/project.pbxproj",
    "MARKETING_VERSION = 0.3.0;\nMARKETING_VERSION = 0.3.0;\n",
  );
  await writeFixture(
    root,
    ".github/workflows/release.yml",
    "inputs:\n  version:\n    default: 0.3.0\n",
  );
  for (const version of ["0.3.0-rc.1", "0.3.1", "0.4.0"]) {
    execFileSync(process.execPath, ["scripts/version.mjs", "set", version], {
      cwd: root,
      stdio: "pipe",
    });
    execFileSync(process.execPath, ["scripts/version.mjs", "check"], {
      cwd: root,
      stdio: "pipe",
    });
    execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], {
      cwd: root,
      stdio: "pipe",
    });
    const lock = await readFile(path.join(root, "Cargo.lock"), "utf8");
    assert.match(lock, new RegExp(`name = "codecaddie-core"\\nversion = "${version.replaceAll(".", "\\.")}"`));
    assert.match(lock, new RegExp(`name = "codecaddie-domain"\\nversion = "${version.replaceAll(".", "\\.")}"`));
    const plist = await readFile(path.join(root, "xcode/CodeCaddie/Info.plist"), "utf8");
    assert.match(plist, new RegExp(`<string>${version.replaceAll(".", "\\.")}</string>`));
    const project = await readFile(
      path.join(root, "xcode/CodeCaddie.xcodeproj/project.pbxproj"),
      "utf8",
    );
    assert.equal((project.match(new RegExp(`MARKETING_VERSION = ${version.replaceAll(".", "\\.")};`, "g")) ?? []).length, 2);
    assert.match(
      await readFile(path.join(root, "scripts/package-macos.sh"), "utf8"),
      new RegExp(`VERSION="${version.replaceAll(".", "\\.")}"`),
    );
    assert.match(
      await readFile(path.join(root, "scripts/package-windows.ps1"), "utf8"),
      new RegExp(`\\$Version = "${version.replaceAll(".", "\\.")}"`),
    );
    const releaseVersion = JSON.parse(
      await readFile(path.join(root, "config/release-version.json"), "utf8"),
    );
    assert.equal(releaseVersion.msiVersionPrefix, version.match(/^(\d+\.\d+)\./)[1]);
  }
});
