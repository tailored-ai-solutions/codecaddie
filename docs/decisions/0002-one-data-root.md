# 0002. One data root selected by CODECADDIE_DATA_DIR

- Status: Accepted
- Date: 2026-09-01

## Context

The desktop launches one core process per request, and the agent CLI, the
MCP server, the updater helper, and every test also start their own core
processes. All of them must find the same state without a running service.
Developer and stable builds must never share state, and tests must never
touch a developer's real workspaces.

## Decision

`RuntimeChannel::data_root` in `crates/codecaddie-core/src/runtime_channel.rs`
is the only resolver. `CODECADDIE_DATA_DIR`, when set, is used verbatim.
Otherwise the per-platform default for the detected channel applies:
`CodeCaddie` or `CodeCaddie Dev` under the platform application-support
directory, and an XDG path on Linux. Everything durable lives under that root:
the event log (`events-v2/`), device-local state, the content key, the
agent-exchange directories, staged updates, and the Sigstore trust cache. No
operating-system credential store is used. There is no second storage
system; portable backups and recovery exports are explicit, user-directed
copies, not stores.

## Consequences

Tests and agent runs set `CODECADDIE_DATA_DIR` to a fresh owner-only
temporary directory; `scripts/dev-isolated.mjs` derives one per worktree so
parallel workers cannot collide. Any feature that needs persistence extends
the store under the existing root rather than adding a path of its own.

## Evidence

- `crates/codecaddie-core/src/runtime_channel.rs`: `RuntimeChannel::detect`, `RuntimeChannel::data_root`.
- `crates/codecaddie-core/src/local_state/workspace_store.rs`: `LocalWorkspaceStore::from_environment`.
- `docs/PLATFORMS.md` "Data locations"; `docs/DEVELOPMENT.md` "State and update safety".
- `scripts/dev-isolated.mjs`: `deriveDataDir`.
