# Decision records

Architecture decision records (ADRs) explain why CodeCaddie is built the way
it is. The public repository history starts from one snapshot commit (see
0008), so `git log` cannot answer "why"; this directory can. pstack's `/why`
and `/how` skills read it, and so should a reviewer before questioning a
boundary.

## Rules

- One decision per file, numbered in the order it is recorded:
  `NNNN-short-title.md`. Numbers never change. A superseded record stays in
  place and points forward to its replacement.
- Copy `TEMPLATE.md`. Keep records short and grounded: cite repository paths,
  symbol names, tests, and public documents. Never cite private history,
  tickets, customers, or people.
- The names allowed here are the same as everywhere else in the tree:
  CodeCaddie, ThoughtfulBits, Tailored AI Solutions, the maintainer's public
  handle, and public vendors. `scripts/check-public-safety.sh` scans tracked
  files, so stage a new record with `git add` before running it; the private
  denylist applies to its contents and its file name.
- Protocol, storage, cryptography, and distribution changes need a record;
  the pull request template asks for it. Add or update the record in the same
  pull request as the change.
- `scripts/tests/agent-config.test.mjs` checks that every record is indexed
  here and every index row has a file.

## Index

| Number | Title | Status | Date |
| --- | --- | --- | --- |
| [0001](0001-source-never-crosses-ipc-reports-or-exports.md) | Repository source never crosses IPC, reports, or exports | Accepted | 2026-09-01 |
| [0002](0002-one-data-root.md) | One data root selected by CODECADDIE_DATA_DIR | Accepted | 2026-09-01 |
| [0003](0003-evidence-is-immutable-coordinates.md) | Evidence is immutable Git coordinates, validated fail-closed | Accepted | 2026-09-01 |
| [0004](0004-length-prefixed-json-protocol.md) | Length-prefixed JSON frames between the desktop host and the core | Accepted | 2026-09-01 |
| [0005](0005-encrypted-append-only-event-log.md) | Encrypted append-only event log as the only durable state | Accepted | 2026-09-01 |
| [0006](0006-keyless-sigstore-release-chain.md) | Keyless Sigstore release and update chain | Accepted | 2026-09-01 |
| [0007](0007-analysis-contract-embedded-from-plugin.md) | The analysis contract is embedded from the plugin at compile time | Accepted | 2026-09-01 |
| [0008](0008-public-repository-re-rooted.md) | Public repository re-rooted on a single snapshot commit | Accepted | 2026-08-30 |
| [0009](0009-pstack-adoption-and-pin-procedure.md) | pstack adoption and pin procedure | Accepted | 2026-09-01 |
| [0010](0010-provider-clis-are-the-only-model-access.md) | Installed provider CLIs are the only model access | Accepted | 2026-09-01 |
| [0011](0011-codebase-map-and-goal-analysis.md) | Codebase map and goal analysis pipeline | Accepted | 2026-08-23 |
| [0012](0012-random-material-comes-only-from-the-os-generator.md) | Random key, nonce, and salt material comes only from the fallible operating-system generator | Accepted | 2026-09-01 |
