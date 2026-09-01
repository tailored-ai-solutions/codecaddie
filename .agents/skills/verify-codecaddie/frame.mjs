#!/usr/bin/env node
// Sends one length-prefixed JSON request to codecaddie-core and prints the
// decoded response. Shares the framing code with the CI harness so the skill
// and CI can never disagree about the wire format.
//
//   node .agents/skills/verify-codecaddie/frame.mjs <method> [params-json]
//        [--workspace <id>] [--binary <path>] [--id <request-id>]
//
// Exit status: 0 when the core answered ok, 1 when it answered with an error
// object, 2 when the process failed or the invocation was invalid.
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { decodeSingleFrame, encodeFrame } from "../../../scripts/exercise-installed-core.mjs";

const MAX_FRAME_BYTES = 16 * 1024 * 1024;
const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

function fail(message) {
  process.stderr.write(`frame.mjs: ${message}\n`);
  process.exit(2);
}

function parse(argv) {
  const options = { method: undefined, params: {}, workspaceId: undefined, binary: undefined, id: undefined };
  const positional = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const next = () => {
      index += 1;
      if (index >= argv.length) fail(`${argument} requires a value`);
      return argv[index];
    };
    if (argument === "--workspace") options.workspaceId = next();
    else if (argument === "--binary") options.binary = next();
    else if (argument === "--id") options.id = next();
    else if (argument.startsWith("--")) fail(`unknown flag ${argument}`);
    else positional.push(argument);
  }
  if (positional.length === 0) fail("usage: frame.mjs <method> [params-json] [--workspace <id>] [--binary <path>] [--id <request-id>]");
  if (positional.length > 2) fail("too many positional arguments");
  options.method = positional[0];
  if (positional[1] !== undefined) {
    try {
      options.params = JSON.parse(positional[1]);
    } catch (error) {
      fail(`params must be a JSON object: ${error.message}`);
    }
    if (options.params === null || typeof options.params !== "object" || Array.isArray(options.params)) {
      fail("params must be a JSON object");
    }
  }
  return options;
}

const options = parse(process.argv.slice(2));
if (!process.env.CODECADDIE_DATA_DIR) {
  fail("CODECADDIE_DATA_DIR is not set; export a fresh temporary data root before driving the core");
}
const binary = options.binary ?? resolve(repositoryRoot, "target/debug/codecaddie-core");
if (!existsSync(binary)) fail(`core binary not found at ${binary}; run cargo build --workspace --locked`);

const request = {
  id: options.id ?? `verify-${options.method}-${Date.now()}`,
  protocolVersion: 2,
  method: options.method,
  params: options.params,
};
if (options.workspaceId) request.workspaceId = options.workspaceId;

const result = spawnSync(binary, [], { input: encodeFrame(request), maxBuffer: MAX_FRAME_BYTES + 4 });
if (result.error) fail(`could not start the core: ${result.error.message}`);
if (result.status !== 0) {
  process.stderr.write(result.stderr?.toString() ?? "");
  fail(`core exited with status ${result.status}`);
}
let response;
try {
  response = decodeSingleFrame(result.stdout);
} catch (error) {
  process.stderr.write(result.stderr?.toString() ?? "");
  fail(`could not decode the response frame: ${error.message}`);
}
process.stdout.write(`${JSON.stringify(response, null, 2)}\n`);
process.exit(response.ok === true ? 0 : 1);
