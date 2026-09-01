import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const packageJson = JSON.parse(await readFile(new URL("package.json", root), "utf8"));
const macosPackage = await readFile(new URL("scripts/package-macos.sh", root), "utf8");
const macosXcodeAssembler = await readFile(new URL("scripts/assemble-macos-xcode.sh", root), "utf8");
const releaseWorkflow = await readFile(new URL(".github/workflows/release.yml", root), "utf8");
const windowsPackage = await readFile(new URL("scripts/package-windows.ps1", root), "utf8");
const workspaceCargo = await readFile(new URL("Cargo.toml", root), "utf8");
const coreCargo = await readFile(new URL("crates/codecaddie-core/Cargo.toml", root), "utf8");
const cargoLock = await readFile(new URL("Cargo.lock", root), "utf8");
const atRest = await readFile(new URL("crates/codecaddie-core/src/at_rest.rs", root), "utf8");
const nativeManifest = await readFile(new URL("apps/desktop/app.zon", root), "utf8");
const nativeSdkPatch = await readFile(new URL("patches/@native-sdk__cli@0.10.1.patch", root), "utf8");

function assertInOrder(source, fragments, label) {
  let cursor = -1;
  for (const fragment of fragments) {
    const index = source.indexOf(fragment, cursor + 1);
    assert.notEqual(index, -1, `${label} is missing ${fragment}`);
    assert.ok(index > cursor, `${label} has ${fragment} out of order`);
    cursor = index;
  }
}

test("interactive native development disables per-frame runtime tracing", () => {
  assert.match(packageJson.scripts["native:dev"], /(?:^|\s)-Dtrace=off(?:\s|$)/);
});

test("distributed native builds disable per-frame runtime tracing", () => {
  assert.match(packageJson.scripts["native:build"], /(?:^|\s)-Dtrace=off(?:\s|$)/);
  assert.match(macosPackage, /native build apps\/desktop --yes[^\n]*-Dtrace=off/);
  assert.match(windowsPackage, /build --summary all -Doptimize=ReleaseFast[^\n]*-Dtrace=off/);
});

test("distributed native builds omit detached debug information", () => {
  assert.match(packageJson.scripts["native:build"], /(?:^|\s)-Dstrip=true(?:\s|$)/);
  assert.match(macosPackage, /native build apps\/desktop --yes[^\n]*-Dstrip=true/);
  assert.match(windowsPackage, /build --summary all -Doptimize=ReleaseFast[^\n]*-Dstrip=true/);
});

test("Windows packaging pins and serializes the exact Zig release toolchain", () => {
  assert.match(windowsPackage, /\$RequiredVersion = "0\.16\.0"/);
  assert.match(windowsPackage, /\$ActualVersion -eq \$RequiredVersion/);
  assert.match(windowsPackage, /build test --summary all[^\n]*-Dcpu=baseline -j1/);
  assert.match(windowsPackage, /build --summary all -Doptimize=ReleaseFast[^\n]*-Dcpu=baseline -j1/);
});

test("local packages embed the full immutable source commit", () => {
  assert.match(macosPackage, /git -C "\$ROOT_DIR" rev-parse HEAD/);
  assert.doesNotMatch(macosPackage, /rev-parse --short/);
  assert.match(windowsPackage, /git rev-parse HEAD/);
  assert.doesNotMatch(windowsPackage, /rev-parse --short/);
});

test("macOS helper signatures keep stable application identities", () => {
  assert.match(macosPackage, /codecaddie-core\) executable_identifier="\$BUNDLE_ID\.core"/);
  assert.match(macosPackage, /--identifier "\$executable_identifier"/);
});

test("macOS distribution uses a universal-app ZIP rather than a disk image", () => {
  assert.match(macosPackage, /stable and beta releases are archived, signed, and notarized by Xcode Cloud/);
  assert.doesNotMatch(macosPackage, /hdiutil|\.dmg|notarytool/);
  assert.match(macosXcodeAssembler, /aarch64-macos:arm64 x86_64-macos:x86_64/);
  assert.match(releaseWorkflow, /CodeCaddie-macOS-universal\.zip/);
  assert.doesNotMatch(releaseWorkflow, /\.dmg|hdiutil/);
});

test("macOS release import validates Apple's stapled ticket and Gatekeeper acceptance", () => {
  assertInOrder(releaseWorkflow, [
    'fileType == "STAPLED_NOTARIZED_ARCHIVE"',
    '/usr/bin/codesign --verify --deep --strict --verbose=2 "$app_path"',
    '/usr/bin/xcrun stapler validate "$app_path"',
    '/usr/sbin/spctl --assess --verbose=4 --type execute "$app_path"',
  ], "Xcode Cloud notarization acceptance checks");
});

test("distributed packages embed the stable public repository identity for keyless verification", () => {
  assert.match(macosXcodeAssembler, /\.sigstore\?\.repositoryId/);
  assert.match(macosXcodeAssembler, /CODECADDIE_GITHUB_REPOSITORY_ID="\$GITHUB_REPOSITORY_ID"/);
  assert.match(macosPackage, /stable packaging requires CODECADDIE_GITHUB_REPOSITORY_ID/);
  assert.match(windowsPackage, /CODECADDIE_GITHUB_REPOSITORY_ID must be a positive numeric repository ID/);
  for (const source of [macosXcodeAssembler, macosPackage, windowsPackage]) {
    assert.doesNotMatch(source, /CODECADDIE_RELEASE_(?:KEY|ALGORITHM|PRIVATE|PUBLIC)/);
  }
});

test("runtime encryption uses an owner-only local key without credential-manager dependencies", () => {
  assert.doesNotMatch(workspaceCargo, /^keyring\s*=/m);
  assert.doesNotMatch(coreCargo, /^keyring\.workspace\s*=/m);
  for (const packageName of [
    "keyring",
    "apple-native-keyring-store",
    "windows-native-keyring-store",
    "secret-service",
  ]) {
    assert.doesNotMatch(cargoLock, new RegExp(`^name = "${packageName}"$`, "m"), `runtime lockfile contains ${packageName}`);
  }
  assert.doesNotMatch(atRest, /keyring::|KEYRING_SERVICE|from_system_credential/);
  assert.match(atRest, /LOCAL_KEY_FILE: &str = "local-content-key-v1"/);
  assert.match(atRest, /Permissions::from_mode\(0o600\)/);
  assert.doesNotMatch(nativeManifest, /[".]credentials["\s,}]/);
  assert.match(nativeSdkPatch, /NATIVE_SDK_CREDENTIALS=0/);
  // Native SDK 0.10.1's application-identity lookup uses Security.framework
  // even when credential APIs are disabled. Preserve that upstream link while
  // the compile-time define keeps actual credential entrypoints unavailable.
  assert.doesNotMatch(nativeSdkPatch, /if \(credentials_capability\) app_mod\.linkFramework\("Security"/);
});
