import assert from "node:assert/strict";
import test from "node:test";
import { windowsInstallerVersion } from "../generate-wix.mjs";

test("same-SemVer releases map monotonically to MSI versions", () => {
  assert.equal(windowsInstallerVersion("0.3.0-rc.1", "1031"), "0.3.1031");
  assert.equal(windowsInstallerVersion("0.3.0", "1032"), "0.3.1032");
  assert.equal(windowsInstallerVersion("0.3.0", "1033"), "0.3.1033");
  assert.equal(windowsInstallerVersion("0.3.1", "1034"), "0.3.1034");
  assert.equal(
    windowsInstallerVersion("0.3.99", "1034"),
    windowsInstallerVersion("0.3.0", "1034"),
    "the dedicated release build, not the SemVer patch, must drive same-line MSI upgrades",
  );
  assert.equal(windowsInstallerVersion("0.4.0", "1"), "0.4.1");
  assert.throws(() => windowsInstallerVersion("0.3.0", "0"), /represented/);
  assert.throws(() => windowsInstallerVersion("0.3.0", "65536"), /represented/);
  assert.throws(() => windowsInstallerVersion("256.0.0", "1"), /represented/);
  assert.throws(() => windowsInstallerVersion("0.256.0", "1"), /represented/);
  assert.throws(() => windowsInstallerVersion("0.3.0", "1.5"), /represented/);
});
