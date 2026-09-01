#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

export const supportedFlags = new Set([
  "--no-build",
  "--no-launch",
  "--uninstall",
  "--help",
]);

export function parseArgs(args) {
  args = args.filter((arg) => arg !== "--");
  for (const arg of args) {
    if (!supportedFlags.has(arg) && arg !== "--destination") {
      if (args[args.indexOf(arg) - 1] !== "--destination") {
        throw new Error(`unknown option: ${arg}`);
      }
    }
  }
  const destinationIndex = args.indexOf("--destination");
  if (destinationIndex !== -1 && !args[destinationIndex + 1]) {
    throw new Error("--destination requires an absolute path");
  }
  return args;
}

export function commandFor(platform, root, args) {
  if (platform === "darwin") {
    return { command: "bash", args: [path.join(root, "scripts/install-local-macos.sh"), ...args] };
  }
  if (platform === "win32") {
    const translated = args.flatMap((arg, index) => {
      if (index > 0 && args[index - 1] === "--destination") return [arg];
      return [{
        "--no-build": "-NoBuild",
        "--no-launch": "-NoLaunch",
        "--uninstall": "-Uninstall",
        "--destination": "-Destination",
        "--help": "-Help",
      }[arg] ?? arg];
    });
    return {
      command: "powershell.exe",
      args: ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(root, "scripts/install-local-windows.ps1"), ...translated],
    };
  }
  throw new Error(`local installation is supported on macOS and Windows, not ${platform}`);
}

export function main(args = process.argv.slice(2), platform = process.platform) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const parsed = parseArgs(args);
  const invocation = commandFor(platform, root, parsed);
  const result = spawnSync(invocation.command, invocation.args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    process.exitCode = main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}
