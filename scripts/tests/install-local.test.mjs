import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { commandFor, parseArgs } from "../install-local.mjs";

test("accepts the documented local install flags", () => {
  assert.deepEqual(parseArgs(["--", "--no-launch"]), ["--no-launch"]);
  assert.deepEqual(parseArgs(["--no-build", "--no-launch", "--uninstall"]), [
    "--no-build",
    "--no-launch",
    "--uninstall",
  ]);
  assert.deepEqual(parseArgs(["--destination", "/Applications"]), ["--destination", "/Applications"]);
});

test("rejects unknown or incomplete options", () => {
  assert.throws(() => parseArgs(["--delete-data"]), /unknown option/);
  assert.throws(() => parseArgs(["--destination"]), /requires an absolute path/);
});

test("dispatches to native platform installers", () => {
  const root = path.resolve("/repo");
  assert.equal(commandFor("darwin", root, []).command, "bash");
  const windows = commandFor("win32", root, ["--no-launch"]);
  assert.equal(windows.command, "powershell.exe");
  assert.ok(windows.args.includes("-NoLaunch"));
  assert.throws(() => commandFor("linux", root, []), /macOS and Windows/);
});
