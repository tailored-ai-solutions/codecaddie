# Architecture

CodeCaddie is a native desktop host plus a Rust core. The desktop owns user
interaction and launches one core process per request. The core owns state,
provider execution, evidence validation, exports, and updates.
`codecaddie-domain` contains deterministic domain rules shared by these paths.

## Analysis data flow

1. The core resolves an exact Git commit and materializes tracked blobs into a
   temporary, history-free snapshot. Symlinks become inert regular files.
2. The selected installed provider receives the snapshot, goals, and a strict
   JSON schema. CodeCaddie does not supply or store provider credentials.
3. The core accepts derived prose and repository-relative coordinates. It
   resolves every coordinate against the original commit, records blob IDs and
   hashes, and rejects source excerpts before creating a report.
4. The event projection signs and stores the report as readable local JSON. Source text never enters
   IPC, reports, or exports. The private desktop IPC may return the
   device-local checkout path; reports and exports contain
   repository-relative coordinates only.

Provider progress crosses IPC through static phase messages. Provider reasoning,
commands, queries, paths, stderr, and source-derived text are excluded.

## Goal analysis pipeline

A `scan.run` request moves through these stages, all in
`crates/codecaddie-core`:

1. **Approved goals.** `service::scan::apply_approved_goals` replaces the
   request's goals with the workspace's approved set, so a report is always
   bound to the goal versions the user approved.
2. **Freeze.** `repository::ProviderSnapshotWorkspace::snapshot_repository`
   resolves each repository to an exact commit and materializes a
   history-free snapshot under the size and file-count caps in
   `repository.rs`.
3. **Codebase map.** `service::scan::ensure_codebase_map` reuses the newest
   valid map recorded for the same frozen repository set
   (`WorkspaceProjection::latest_map_for`) or generates one with
   `analyzer::map_generate::generate_codebase_map`: an inventory digest, one
   survey call, chunked component deep-dives, then deterministic merge and
   validation in `analyzer::map_materialize::materialize_codebase_map`. A
   map failure degrades the scan to mapless with a warning; it never blocks
   the report. See [decision 0011](decisions/0011-codebase-map-and-goal-analysis.md).
4. **Goal batches.** `analyzer::scan::run_scan_with_map` writes the validated
   map into the snapshot workspace as `codecaddie-map.json`, splits goals
   into two-goal batches (`GOALS_PER_PROVIDER_BATCH`), and runs them
   concurrently under `ProviderRunner` with one bounded retry. Each batch
   prompt (`analyzer::analysis_contract::analysis_prompt`) carries the
   goals, the repository directory map, the product brief, and a bounded map
   digest, and is parameterized per provider by `provider_tool_text`.
5. **Materialize.** `analyzer::report_materialize::materialize_report` binds
   every citation to the frozen commit through `bind_evidence`, screens
   narrative fields for source text and credential markers with field-level
   redaction, caps architecture claims and recommendations, and validates
   component references against the seeding map.
6. **Record.** `LocalWorkspaceStore::record_report` appends a signed
   `ReportCompleted` event; the report integrity gate in
   [REPORT-INTEGRITY.md](REPORT-INTEGRITY.md) re-resolves every coordinate
   before the event is accepted.

The agent-session path (the plugin skill, `codecaddie mcp`, and the
`codecaddie-core agent` CLI) skips stages 3 and 4 — the external agent does
the reasoning — and shares stages 2, 5, and 6 through
`agent_gateway::AgentGateway` with a fail-closed narrative policy.

### Provider tool contracts

Each installed provider inspects the snapshot through its own read-only
tools. The analysis prompt names the matching tools and budget per provider.

| Provider | Snapshot access during a scan | Where configured |
| --- | --- | --- |
| Claude | `Read`, `Glob`, `Grep` allow-listed to the snapshot directory (`READ_ALLOWLIST`) | `provider/claude.rs` |
| Codex | CodeCaddie's bundled repository MCP server: `list_repository_files`, `search_repository`, `read_repository_file` | `provider/codex.rs`, `provider_repository_mcp.rs` |
| Grok | `list_dir`, `grep`, `read_file` under an enforced `--max-turns 24` | `provider/grok.rs` |

The bundled MCP server caps listings, search results, file bytes, and read
lines (`MAX_LIST_RESULTS`, `MAX_SEARCH_RESULTS`, `MAX_FILE_BYTES`,
`MAX_READ_LINES` in `provider_repository_mcp.rs`).

## State and projection

Local state is a readable append-only JSONL event log. Signed domain events rebuild
the workspace projection and enforce the signing device, epochs, immutable
goal versions, and report history. Device-local access records retain repository
paths; logged events retain repository IDs and immutable evidence coordinates.

Workspace state contains full goals, reports, actions, and coordinates. Managed
payload files are authenticated ciphertext; an owner-only 256-bit content-key
file in the same local data root relies on operating-system file permissions
for access control.

## Process and storage boundaries

- Zig and Rust exchange bounded, versioned JSON frames.
- Large desktop requests use one owner-only staging file, restricted to the
  operating-system temporary directory and deleted on every parse outcome.
- Provider output, evidence blobs, and update artifacts have explicit size
  limits.
- Developer and stable channels use separate application identities and state
  roots. Neither channel uses an operating-system credential store for local
  state.

## Release and update chain

Every protected-main commit receives a monotonic build and immutable GitHub
release. Xcode Cloud retains the non-exportable Developer ID key and returns
only a signed, notarized universal application archive. GitHub Actions creates
checksums, an SBOM, provenance, attestations, and a keyless Sigstore bundle over
the exact manifest bytes using a short-lived GitHub OIDC identity. No manifest
private key or cloud client secret exists.

The embedded verifier follows rotating Sigstore trust roots through TUF and
requires the pinned GitHub repository ID, protected `main` ref, canonical
release workflow, OIDC issuer, source commit, Fulcio chain, and Rekor inclusion
proof. The updater also checks version/build monotonicity, HTTPS URL, size,
SHA-256, architecture, Apple team and bundle identity, and the candidate app's
declared semantic version/build before replacement. Windows remains a
source-built preview until SignPath Foundation approves open-source signing.

See [Security model](SECURITY_MODEL.md), [Development](DEVELOPMENT.md), and
[Releasing](RELEASING.md).

- [Module map](MODULE-MAP.md) — where each module lives, its entry points, and
  tests.
- [Decision records](decisions/README.md) — dated records of the design
  decisions behind the current architecture.
