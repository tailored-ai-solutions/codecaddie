# 0001. Repository source never crosses IPC, reports, or exports

- Status: Accepted
- Date: 2026-09-01

## Context

CodeCaddie is local-first, and its reports and Word exports are meant to be
shared with people who must not receive the code. The selected provider CLI
is a separate data processor that reads a disposable snapshot under its own
terms; CodeCaddie itself must never become a second copy of the source.
Provider output is untrusted text and could quote code verbatim.

## Decision

The core accepts only derived prose and repository-relative coordinates from
a provider. Before a report is signed,
`report_integrity::validate_report_for_persistence` re-resolves every
coordinate against the frozen commit and rejects excerpts, and
`analyzer::report_materialize` screens narrative fields for source and
credential markers with field-level redaction. The private desktop IPC may
carry the device-local repository path so the app can reopen a workspace;
reports and exports carry repository-relative coordinates only. Snippets the
desktop shows are read on the device by `apps/desktop/src/snippet_worker.zig`
from local Git, never from the report. Provider stderr, queries, and absolute
snapshot paths are never forwarded as progress.

## Consequences

Every fixture that feeds the core carries a canary string and every surface
asserts its absence, so adding a response, event, log, or export means adding
it to the source-canary matrix. This is the first invariant in `AGENTS.md`
and why `scripts/check-public-safety.sh` rejects serialized source fields.

## Evidence

- `crates/codecaddie-core/src/report_integrity.rs`: `validate_report_for_persistence`.
- `crates/codecaddie-core/src/analyzer/report_materialize.rs`, `crates/codecaddie-core/src/analyzer/map_materialize.rs`: marker screening.
- `crates/codecaddie-core/src/privacy_test_support.rs`, `config/source-canary-matrix-v1.json`, `pnpm privacy:check`; `scripts/exercise-installed-core.mjs` (`PRIVATE SOURCE CANARY`); `docs/SECURITY_MODEL.md` "Data retained by CodeCaddie".
