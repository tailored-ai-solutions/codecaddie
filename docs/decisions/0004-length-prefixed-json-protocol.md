# 0004. Length-prefixed JSON frames between the desktop host and the core

- Status: Accepted
- Date: 2026-09-01

## Context

The Zig host and the Rust core are separate processes. The transport had to be
private (no port, no socket file), bounded, versioned, testable from any
language, and simple enough that a shell script or an agent can drive the
core directly.

## Decision

The host starts the core with private stdin/stdout pipes. Each request is a
4-byte unsigned big-endian length followed by one UTF-8 JSON object, at most
16 MiB; each response has the same shape. Logs go to stderr and never mix
with protocol output. The envelope carries `id`, `protocolVersion` (2),
`method`, `params`, and an optional `workspaceId` for scoped methods.
`service::METHODS` and `service::DISPATCH` list every method in the same
order; `protocol/README.md` documents each one and `protocol/fixtures/` holds
envelopes both runtimes validate. Methods that run a provider may set
`"stream": true` and then answer in NDJSON: progress events followed by one
terminal response. Requests beyond the host's spawn-stdin budget are staged
in a private `--request-file` the core deletes after reading.

## Consequences

Adding a method means adding a handler, a `METHODS` entry, a catalog row, and
a fixture; a test enforces the first two. Any harness can talk to the core
in a few lines, which is what `scripts/exercise-installed-core.mjs` and the
`verify-codecaddie` skill rely on.

## Evidence

- `crates/codecaddie-core/src/protocol.rs`: `CoreRequest`, `CoreResponse`, `CoreEvent`, `read_frame`, `write_frame`, `write_json_line`.
- `crates/codecaddie-core/src/service.rs`: `METHODS`, `DISPATCH`, `handle`, `handle_with_progress`; `crates/codecaddie-core/src/main.rs`: `respond`, `read_request_file`.
- `apps/desktop/src/core_ipc.zig`; `protocol/README.md`; `protocol/fixtures/`.
- `scripts/lib/core-harness.mjs`: `encodeFrame`, `decodeSingleFrame`.
