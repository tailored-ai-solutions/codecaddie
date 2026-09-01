# 0005. Encrypted append-only event log as the only durable state

- Status: Accepted
- Date: 2026-09-01

## Context

Reports must never be overwritten, goal versions must stay immutable, and
history must survive crashes and restarts. Local files also need protection
at rest without an operating-system credential manager, which CodeCaddie
deliberately does not use.

## Decision

Durable workspace state is a per-workspace JSONL event log under
`events-v2/` in the data root (`LocalEventLog` in `storage.rs`). Each line is
an authenticated XChaCha20-Poly1305 envelope (`ContentCipher` in
`at_rest.rs`, key material in the owner-only `local-content-key-v1` file)
around a device-signed `EventEnvelope`. `WorkspaceProjection` replays the
signed events to rebuild goals, reports, actions, and history, enforcing the
signing device, epochs, and immutable goal versions. Appends and replacements
use the crash-safe primitives in `persistence.rs`: private temporary file,
sync, rename, parent sync, and idempotent retry of an identical final record.
Device-local settings that must not enter the ledger (repository path,
project context, provider choice) live beside it in local state files.

## Consequences

Deleting a run is an event; nothing is rewritten. Portable backups use a
separate Argon2id-derived key so the content key never leaves the data root.
Fault-injection tests interrupt every write boundary and prove convergence.
The at-rest boundary protects a file separated from its root, not a machine
where an attacker already runs as the same user.

## Evidence

- `crates/codecaddie-core/src/storage.rs`: `LocalEventLog`; `crates/codecaddie-core/src/at_rest.rs`: `ContentCipher`, `LOCAL_KEY_FILE`.
- `crates/codecaddie-core/src/persistence.rs`: `write_private_atomic_new`, `write_private_replace`, `sync_parent`, `PersistenceFaultInjector`.
- `crates/codecaddie-domain/src/event.rs`: `DomainEvent`, `EventEnvelope`; `crates/codecaddie-domain/src/projection.rs`: `WorkspaceProjection`; `crates/codecaddie-core/src/local_state/portable_backup.rs`; `pnpm recovery:check`; `docs/SECURITY_MODEL.md`.
