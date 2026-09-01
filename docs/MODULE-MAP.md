# Module map

Where behavior lives: module, responsibility, entry points, and the tests that
pin it. Use it with pstack's `/how` and `/blast-radius` before editing, and
update it when code moves. `scripts/tests/agent-config.test.mjs` fails when a
path named here stops existing. `docs/ARCHITECTURE.md` explains how the
pieces fit; `docs/decisions/` explains why.

## Rust core: `crates/codecaddie-core`

| Module | Responsibility | Entry points | Tests |
| --- | --- | --- | --- |
| `crates/codecaddie-core/src/main.rs` | Process entry: health check, MCP modes, the one-shot agent CLI, staged request files, and the stdin frame loop | `main`, `respond`, `read_request_file` | `scripts/exercise-installed-core.mjs`, `protocol/fixtures/` |
| `crates/codecaddie-core/src/protocol.rs` | Envelope types and length-prefixed or NDJSON framing | `CoreRequest`, `CoreResponse`, `CoreEvent`, `read_frame`, `write_frame`, `write_json_line` | inline tests over `protocol/fixtures/` |
| `crates/codecaddie-core/src/service.rs`, `crates/codecaddie-core/src/service/` | Request dispatch; one handler module per domain (system, updates, settings, providers, context, workspace, reports, goals, actions, instrumentation, reliability, recommendations, scan, map) | `METHODS`, `DISPATCH`, `handle`, `handle_with_progress`, `streams_progress` | inline METHODS/DISPATCH parity and request-id echo; `crates/codecaddie-core/src/runtime_health_assurance.rs` |
| `crates/codecaddie-core/src/runtime_channel.rs` | Stable versus development channel and the single data-root resolver | `RuntimeChannel::detect`, `RuntimeChannel::data_root`, `native_panic_marker_path` | inline; `scripts/tests/native-runtime-config.test.mjs` |
| `crates/codecaddie-core/src/storage.rs` | Encrypted append-only JSONL event log per workspace | `LocalEventLog` (`append`, `load`, `restore_exact`, `raw_values`) | inline; `crates/codecaddie-core/src/operational_fault_assurance.rs` |
| `crates/codecaddie-core/src/at_rest.rs` | Authenticated encryption of local state; owner-only content key | `ContentCipher`, `LOCAL_KEY_FILE` | inline |
| `crates/codecaddie-core/src/persistence.rs` | Crash-safe write, replace, and sync primitives with fault injection | `write_private_atomic_new`, `write_private_replace`, `sync_parent`, `PersistenceFaultInjector` | inline; `pnpm recovery:check` via `scripts/exercise-recovery-matrix.mjs` |
| `crates/codecaddie-core/src/local_state/` | Device-local workspace state: `crates/codecaddie-core/src/local_state/workspace_store.rs` (CRUD over the event log, backup schedule, summaries), `crates/codecaddie-core/src/local_state/identity.rs` (device identity, project context), `crates/codecaddie-core/src/local_state/locks.rs` (cross-process write locks), `crates/codecaddie-core/src/local_state/heatmap.rs` (report history projection), `crates/codecaddie-core/src/local_state/portable_backup.rs` (passphrase-encrypted bundles) | `LocalWorkspaceStore::from_environment`, `LocalWorkspaceStore::record_report`, `ReportHistoryPage` | inline; `crates/codecaddie-core/src/decision_journey_assurance.rs`; `scripts/tests/disaster-recovery-policy.test.mjs` |
| `crates/codecaddie-core/src/repository.rs` | Local Git access, exact-commit resolution, disposable history-free snapshots | `LocalRepository`, `DisposableClone`, `ProviderSnapshotWorkspace` | inline plus `crates/codecaddie-core/src/repository_snapshot_lifecycle_assurance.rs` |
| `crates/codecaddie-core/src/analyzer.rs`, `crates/codecaddie-core/src/analyzer/` | Analysis pipeline: `crates/codecaddie-core/src/analyzer/scan.rs` (batches, retries), `crates/codecaddie-core/src/analyzer/analysis_contract.rs` (schemas and prompts embedded from `plugin/`), `crates/codecaddie-core/src/analyzer/report_materialize.rs` (evidence binding, redaction), `crates/codecaddie-core/src/analyzer/map_generate.rs` and `crates/codecaddie-core/src/analyzer/map_materialize.rs` (codebase map), `crates/codecaddie-core/src/analyzer/goal_drafts.rs` and `crates/codecaddie-core/src/analyzer/goal_catalog.rs` (goal generation, coverage families, repair), `crates/codecaddie-core/src/analyzer/product_profile.rs`, `crates/codecaddie-core/src/analyzer/assurance.rs` | `run_scan_with_map`, `materialize_report`, `generate_codebase_map`, `analysis_prompt`, `GOAL_TEMPLATES`, `COVERAGE_FAMILIES` | inline with `crates/codecaddie-core/src/analyzer/test_support.rs`; golden `plugin/skills/codecaddie-analysis/references/goal-template-catalog.md` |
| `crates/codecaddie-core/src/report_integrity.rs` | Fail-closed validation of every claim before a report is signed | `validate_report_for_persistence` | inline table-driven corruption tests; `scripts/exercise-saved-evidence-checkout.mjs` |
| `crates/codecaddie-core/src/provider/` | Provider CLI detection and bounded execution: `crates/codecaddie-core/src/provider/mod.rs` (detection), `crates/codecaddie-core/src/provider/runner.rs` (lifecycle), `crates/codecaddie-core/src/provider/stream.rs` (NDJSON), `crates/codecaddie-core/src/provider/contract.rs` (shared execution contract), `crates/codecaddie-core/src/provider/claude.rs`, `crates/codecaddie-core/src/provider/codex.rs`, `crates/codecaddie-core/src/provider/grok.rs` (adapters) | `ProviderKind`, `detect_all`, `ProviderRunner`, `contract_supported` | `crates/codecaddie-core/src/provider/contract_assurance.rs`; `protocol/provider-contract-v1.schema.json` |
| `crates/codecaddie-core/src/provider_repository_mcp.rs` | Snapshot-confined list, search, and read tools served to provider processes | `run`, `MAX_LIST_RESULTS`, `MAX_SEARCH_RESULTS`, `MAX_FILE_BYTES`, `MAX_READ_LINES` | inline |
| `crates/codecaddie-core/src/mcp.rs`, `crates/codecaddie-core/src/agent_gateway.rs`, `crates/codecaddie-core/src/agent_cli.rs` | Coding-agent surfaces: stdio MCP server, shared gateway logic, and the one-shot `agent` verbs | `mcp::run`, `AgentGateway`, `agent_cli::run` | inline; `scripts/exercise-installed-core.mjs` journey |
| `crates/codecaddie-core/src/context_documents.rs` | Product-context file inspection and bounded text extraction | `ContextFileReference` | inline |
| `crates/codecaddie-core/src/export.rs` | Metadata-only Word export | export entry used by `reports.export_word` and `agent export` | inline; `scripts/exercise-installed-core.mjs` |
| `crates/codecaddie-core/src/update.rs`, `crates/codecaddie-core/src/bin/codecaddie-updater.rs` | Sigstore-verified update check, download, staging, and the external replacement helper | `verify_release`, `SigstoreIdentityPolicy`, `current_version`, `current_build`, `UpdaterResultV1` | inline with `crates/codecaddie-core/src/testdata/`; `crates/codecaddie-core/tests/updater_result_privacy.rs`; `scripts/exercise-supported-prior-binaries.mjs` |
| `crates/codecaddie-core/src/reliability.rs`, `crates/codecaddie-core/src/runtime_controls.rs`, `crates/codecaddie-core/src/launch_at_login.rs` | Local reliability contracts, restart-safe containment controls, login item | policies loaded from `config/reliability-gates.json`, `config/runtime-feature-controls.json` | inline; `scripts/tests/reliability-contract.test.mjs`, `scripts/tests/runtime-feature-controls.test.mjs` |
| `crates/codecaddie-core/src/privacy_test_support.rs` | Test-only sentinels for the adversarial privacy gate | fixtures under `crates/codecaddie-core/tests/fixtures/adversarial/` | `pnpm privacy:check` |
| `crates/codecaddie-core/src/product_assurance.rs`, `crates/codecaddie-core/tests/performance_gate.rs` | Executable product-contract journeys and the latency, throughput, and capacity gate | test-only | `pnpm reliability:check` |
| `crates/codecaddie-core/rubrics/` | Vendored product rubrics embedded into prompts | `include_str!` in `crates/codecaddie-core/src/analyzer/analysis_contract.rs` | BLAKE3 pins in inline tests |

## Domain: `crates/codecaddie-domain`

| Module | Responsibility | Entry points | Tests |
| --- | --- | --- | --- |
| `crates/codecaddie-domain/src/model.rs` | Goals, criteria, verdicts, evidence references, reports, recommendations, device identity | `GoalVersion`, `Criterion`, `Verdict`, `EvidenceRef`, `Report`, `Recommendation` | inline; consumers above |
| `crates/codecaddie-domain/src/event.rs` | Signed domain events and content-free product and reliability records | `DomainEvent`, `EventEnvelope`, `ProductEventRecord`, `ReliabilityEventRecord` | inline; `protocol/local-product-events-v2.schema.json`, `protocol/local-reliability-event-v1.schema.json` |
| `crates/codecaddie-domain/src/projection.rs` | Replays events into workspace state with epoch and signature discipline | `WorkspaceProjection`, `ActionProjection`, `ProjectionError` | inline |
| `crates/codecaddie-domain/src/scoring.rs` | Deterministic verdict aggregation and coverage scores | `criterion_value`, `aggregate_goal`, `score_report` | inline |
| `crates/codecaddie-domain/src/map.rs` | Typed, evidence-bound codebase map | `CodebaseMap`, `Component`, `ComponentRelationship`, `DataFlow`, `EntryPoint`, `CodebaseMapDescriptor`, `component_id` | inline; `plugin/skills/codecaddie-analysis/references/codebase-map.schema.json` |

## Desktop host: `apps/desktop`

| Module | Responsibility | Entry points | Tests |
| --- | --- | --- | --- |
| `apps/desktop/src/main.zig` | App wiring, the message union, effects, and the `update` state machine | `update`, `Effects`, `initialModel` | `apps/desktop/src/tests.zig` |
| `apps/desktop/src/model.zig` | Application state and markup projections | `Model`, `Screen`, `ScanStatus`, `UpdateStatus` | `apps/desktop/src/tests.zig` |
| `apps/desktop/src/app.native` | Native SDK markup for every screen, dialog, and overlay | templates and handlers referenced from `apps/desktop/src/main.zig` | `pnpm exec native check apps/desktop --strict` |
| `apps/desktop/src/core_ipc.zig` | Length-prefixed request frames, staged request files, response bounds | frame builders used by `apps/desktop/src/main.zig` | `apps/desktop/src/tests.zig`, `scripts/tests/local-transport-protection.test.mjs` |
| `apps/desktop/src/resume_apply.zig` | Applies `workspace.recent` to the model on startup | resume entry used by `apps/desktop/src/main.zig` | `apps/desktop/src/tests.zig` |
| `apps/desktop/src/snippet_worker.zig` | On-device evidence snippet loading from local Git | worker entry used by `apps/desktop/src/main.zig` | `apps/desktop/src/tests.zig` |
| `apps/desktop/src/platform.zig` | Per-channel identity, URLs, and presentation configuration | constants used by `apps/desktop/src/main.zig` | `scripts/tests/native-runtime-config.test.mjs` |
| `apps/desktop/src/first_report_journey_assurance.zig` | The executable first-report journey through every state in `config/first-report-journey-v1.json` | test-only | `pnpm exec native test apps/desktop --yes` |
| `apps/desktop/build.zig`, `apps/desktop/app.zon` | Build graph and application manifest | build options | `pnpm native:build` |

## Plugin, protocol, and configuration

| Path | Responsibility | Consumers | Tests |
| --- | --- | --- | --- |
| `plugin/skills/codecaddie-analysis/SKILL.md`, `plugin/skills/codecaddie-analysis/references/` | The cross-agent analysis skill and the canonical schemas, rubric, and checklist | embedded by `crates/codecaddie-core/src/analyzer/analysis_contract.rs`; installed through `.claude-plugin/marketplace.json` | Rust inline schema tests; golden catalog |
| `plugin/.claude-plugin/plugin.json`, `plugin/.mcp.json`, `plugin/GROK-BOT-ROUTINE.md` | Plugin manifest, MCP launch, and the Grok Bot operating routine | Claude Code, Codex, Grok | `scripts/tests/ship-readiness.test.mjs` |
| `protocol/README.md`, `protocol/core.schema.json`, `protocol/fixtures/` | Method catalog, envelope schema, and the cross-language fixture corpus | both runtimes | Rust fixture walk; `apps/desktop/src/tests.zig` |
| `config/` | Executable policies: release trust and distribution, reliability gates, recovery matrices, runtime controls, journeys | Rust `include_str!` and Node scripts | `scripts/tests/` suites named after each policy |
| `testdata/golden/monolith/` | Synthetic repository fixture for demos and verification runs | `docs/GETTING-STARTED.md`, `.agents/skills/verify-codecaddie/SKILL.md` | none (input fixture) |
| `fixtures/journeys/synthetic-goals.json` | Synthetic approved goals for harness journeys | `scripts/lib/core-harness.mjs` | `scripts/exercise-installed-core.mjs` |

## Scripts

| Script | Responsibility | Test |
| --- | --- | --- |
| `scripts/lib/core-harness.mjs` | Shared framing, fixture repository, and core or agent runners for harness scripts | `scripts/tests/supported-prior-binaries.test.mjs` |
| `scripts/exercise-installed-core.mjs` | Health, ping, and the deterministic first-report journey (`pnpm verify:core`) | `scripts/tests/ship-readiness.test.mjs` |
| `scripts/assert-source-free.mjs` | Negative grep for the source canary and absolute paths over evidence | used by the verify skill and CI |
| `scripts/exercise-supported-prior-binaries.mjs` | Every supported prior binary through upgrade and rollback | `scripts/tests/supported-prior-binaries.test.mjs` |
| `scripts/exercise-recovery-matrix.mjs`, `scripts/exercise-saved-evidence-checkout.mjs` | Recovery matrix and evidence-after-checkout-switch journeys | `scripts/tests/executable-recovery-matrix.test.mjs`, `scripts/tests/saved-evidence-checkout.test.mjs` |
| `scripts/check-public-safety.sh` | Public-safety scan over tracked files | `scripts/tests/public-safety.test.mjs` |
| `scripts/check-reliability-gates.mjs`, `scripts/check-brand-assets.mjs`, `scripts/generate-brand-assets.mjs` | Reliability-gate policy and brand asset generation and drift checks | `scripts/tests/reliability-gates.test.mjs` |
| `scripts/version.mjs`, `scripts/release-build-number.mjs`, `scripts/release-manifest.mjs`, `scripts/verify-release-manifest.mjs`, `scripts/compare-release-identities.mjs`, `scripts/verify-reproducible-build.mjs`, `scripts/verify-public-root.mjs`, `scripts/check-protected-ref.mjs`, `scripts/fetch-xcode-cloud-artifact.mjs` | Release identity, manifest signing checks, reproducibility, public-root verification, protected-ref gating, Xcode Cloud artifact retrieval | `scripts/tests/version.test.mjs`, `scripts/tests/release-build-number.test.mjs`, `scripts/tests/release-manifest.test.mjs`, `scripts/tests/compare-release-identities.test.mjs`, `scripts/tests/reproducible-builds.test.mjs`, `scripts/tests/verify-public-root.test.mjs`, `scripts/tests/protected-ref.test.mjs`, `scripts/tests/xcode-cloud-artifact.test.mjs` |
| `scripts/install-local.mjs`, `scripts/install-local-macos.sh`, `scripts/install-local-windows.ps1`, `scripts/package-macos.sh`, `scripts/package-windows.ps1`, `scripts/generate-wix.mjs`, `scripts/assemble-macos-xcode.sh` | Developer installation and platform packaging | `scripts/tests/install-local.test.mjs`, `scripts/tests/generate-wix.test.mjs` |
| `scripts/check-native-credential-boundary.mjs`, `scripts/normalize-license-evidence.mjs`, `scripts/first-report-metric.mjs` | Native credential boundary, license evidence normalization, first-report metric | `scripts/tests/native-credential-boundary.test.mjs`, `scripts/tests/dependency-license-evidence.test.mjs`, `scripts/tests/product-metrics.test.mjs` |
| `scripts/dev-isolated.mjs` | Desktop against a per-worktree owner-only data root (`pnpm dev:isolated`) | `scripts/tests/agent-config.test.mjs` |
| `scripts/agents-setup.mjs` | Opt-in pstack skill links for Codex and Grok (`pnpm agents:setup`) | `scripts/tests/agents-setup.test.mjs` |

## Agent tooling

| Path | Responsibility | Test |
| --- | --- | --- |
| `AGENTS.md`, `CLAUDE.md`, `.cursor/rules/codecaddie-workflow.mdc` | The agent contract, the Claude Code import, and the Cursor always-on rule | `scripts/tests/agent-config.test.mjs` |
| `.agents/skills/verify-codecaddie/`, `.claude/skills/` | The runtime verification skill (canonical) and its Claude Code symlink | `scripts/tests/agent-config.test.mjs` |
| `.claude-plugin/marketplace.json`, `.claude/settings.json` | The codecaddie marketplace with pinned pstack entries, and the project plugin enablement | `scripts/tests/agent-config.test.mjs`; `claude plugin validate .` |
| `docs/decisions/` | Decision records and their index | `scripts/tests/agent-config.test.mjs` |
