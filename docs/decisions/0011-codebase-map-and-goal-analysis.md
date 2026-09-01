# 0011. Codebase map and goal analysis pipeline

- Status: Accepted (implemented in 0.4.0)
- Date: 2026-08-23
- Scope: `crates/codecaddie-core/src/analyzer`, `crates/codecaddie-domain`,
  the `scan.run`, `map.generate`, and `map.get` protocol methods, the agent
  session tools, and the desktop report and architecture screens

This record closes the design that turned goal analysis from a single bounded
pass with no memory into a two-stage pipeline: generate a persistent,
evidence-bound codebase map from the frozen snapshot, then evaluate goals
seeded with that map and show, per goal, how the architecture supports it.
Parts I and II describe the pipeline as it stood when the decision was taken.
Part III is the target architecture that was built. Part IV is the roadmap
that was followed. The current-state overview lives in
[ARCHITECTURE.md](../ARCHITECTURE.md); system invariants (privacy, signing,
storage) stay with that document and [SECURITY_MODEL.md](../SECURITY_MODEL.md).

Citations name a path and a symbol rather than a line number so they survive
edits. Every symbol named here existed at the time this record was closed.

## Part I — Context at the time: how analysis worked

### I.1 System shape

- The Zig desktop host launched one Rust core process per request over
  private stdin/stdout JSON frames (`docs/ARCHITECTURE.md`,
  `protocol/README.md`). No core state survived between requests except what
  was on disk.
- All durable state was an append-only JSONL event log of signed envelopes,
  one file per workspace under the `events-v2` directory of the data root
  (`crates/codecaddie-core/src/storage.rs`, `LocalEventLog`), rebuilt into a
  `WorkspaceProjection` on every load
  (`crates/codecaddie-domain/src/projection.rs`, `WorkspaceProjection::rebuild`).
- The data root resolved from `CODECADDIE_DATA_DIR` or the per-platform
  default (`crates/codecaddie-core/src/runtime_channel.rs`,
  `RuntimeChannel::data_root`). Non-event siblings already lived beside the
  log in the same root: `local-state-v2.json`, `agent-sessions/`,
  `agent-exchange/`, pointer files, and `locks-v1/`.

### I.2 The goal model

- A goal was an immutable, content-addressed `GoalVersion`
  (`crates/codecaddie-domain/src/model.rs`): a stable `goal_id`, a versioned
  `id` derived from a BLAKE3 hash of the version material, title, business
  outcome, priority 1–5, portfolio position, one or more success criteria,
  and rubric dimensions whose first entry named one of three groups —
  "Business & product", "Architecture & platform", "Operations & reliability"
  (`crates/codecaddie-core/src/analyzer/goal_drafts.rs`).
- Goals were created two ways: provider generation (`goals.generate`,
  `crates/codecaddie-core/src/analyzer/goal_drafts.rs`, 6–9 goals with at most
  two revision rounds plus a deterministic repair pipeline) and user authoring
  in the desktop. Both persisted through `goals.replace` and
  `LocalWorkspaceStore::replace_goals`
  (`crates/codecaddie-core/src/local_state/workspace_store.rs`). Approval
  emitted domain events; the approved set was what analysis ran against.
- A report was bound to the goal set that produced it: the projection rejected
  a `ReportCompleted` whose goal-version set differed from the currently
  approved set (`crates/codecaddie-domain/src/projection.rs`,
  `WorkspaceProjection::apply`, the `GoalSetReplaced` and `ReportCompleted`
  arms).

### I.3 The analysis pipeline, stage by stage

```
scan.run {reportId, repositories, provider, goals: [], stream: true}
│
├─ 1  approved goals replace the request's goals
│       service/scan.rs · apply_approved_goals
│
├─ 2  freeze: resolve the exact commit → disposable, history-free snapshot
│       git ls-tree -r / cat-file --batch → read-only tempdir, no .git
│       repository.rs · LocalRepository::disposable_clone
│       limits: MAX_BLOB_BYTES, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_FILES
│       no parsing: no AST, symbols, index, or embeddings
│
├─ 3  batch: goals split two per provider call, bounded concurrency
│       analyzer/scan.rs · GOALS_PER_PROVIDER_BATCH,
│       MAX_CONCURRENT_PROVIDER_BATCHES, ProviderRunner timeout
│
├─ 4  provider call per batch → RawAnalysis JSON
│       analyzer/analysis_contract.rs · analysis_prompt, RawAnalysis
│       context: the batch's goals + repository directory map + product brief
│       — no file contents, no summaries, no map; pure agentic retrieval
│
├─ 5  merge: concatenate batches → sort by affected-goal priority →
│       adjacent-only dedup → truncate architecture and recommendations
│       analyzer/scan.rs · run_scan
│
├─ 6  materialize: bind every citation to the frozen commit, screen, score
│       analyzer/report_materialize.rs · materialize_report, bind_evidence
│
└─ 7  record: DomainEvent::ReportCompleted → signed log → projection
        local_state/workspace_store.rs · LocalWorkspaceStore::record_report
```

**Two paths, one validator.** The native scan was one of two entry points. The
other was an agent session: the plugin skill
(`plugin/skills/codecaddie-analysis/SKILL.md`) or the one-shot CLI drove
`begin_analysis` and `submit_analysis` through the MCP server
(`crates/codecaddie-core/src/mcp.rs`,
`crates/codecaddie-core/src/agent_gateway.rs`, `AgentGateway`), where the
external agent did the reasoning and the core only froze commits and
validated the submission. Both paths shared one wire contract —
`plugin/skills/codecaddie-analysis/references/analysis.schema.json`,
embedded with `include_str!` as `ANALYSIS_SCHEMA` in
`crates/codecaddie-core/src/analyzer/analysis_contract.rs` — and one
materializer, differing only in source-narrative policy: scans used `Redact`,
agent sessions failed closed with `Reject`
(`crates/codecaddie-core/src/analyzer/report_materialize.rs`,
`SourceNarrativePolicy`).

**Provider tool contracts differed per CLI** (this mattered in Part II):

| Provider | Repository access during a scan | Where configured |
|---|---|---|
| Claude | `Read`, `Glob`, `Grep` allow-listed to the snapshot | `crates/codecaddie-core/src/provider/claude.rs`, `READ_ALLOWLIST` |
| Codex | CodeCaddie's own MCP server: `list_repository_files`, `search_repository`, `read_repository_file` | `crates/codecaddie-core/src/provider/codex.rs` |
| Grok | `list_dir`, `grep`, `read_file`, `--max-turns 24` | `crates/codecaddie-core/src/provider/grok.rs` |

The bundled MCP server (`crates/codecaddie-core/src/provider_repository_mcp.rs`)
offered literal case-insensitive substring search capped at
`MAX_SEARCH_RESULTS` results walked in sorted path order, and file reads
capped at `MAX_READ_LINES` lines.

### I.4 The analysis prompt

The prompt was a single format string built by `analysis_prompt` in
`crates/codecaddie-core/src/analyzer/analysis_contract.rs`. Its load-bearing
instructions restricted inspection to the Codex MCP tool names, imposed a
tool-call budget of roughly a dozen calls with a fixed stopping turn, told the
model to return `unsupported` after two failed targeted searches, capped each
criterion at three citations, architecture at five claims, and
recommendations at five items, and forbade source excerpts. The prompt carried
the batch's goals, a `REPOSITORY DIRECTORY MAP` (repositoryId → directory
pairs), and the untrusted product brief. Nothing else.

### I.5 Evidence binding and the verdict model

Providers submitted coordinates only — `{repositoryId, path, startLine,
endLine, kind}` (`RawEvidence` in `analysis_contract.rs`). The core bound each
citation against the original frozen commit into an `EvidenceRef`
(`crates/codecaddie-domain/src/model.rs`): `git rev-parse <commit>:<path>`
yielded the blob OID, `git cat-file -p` yielded the text, and the cited slice
was BLAKE3-hashed into `content_hash`
(`crates/codecaddie-core/src/repository.rs`, `LocalRepository::evidence`).
Spans over `MAX_EVIDENCE_LINES` (80) were rejected. A citation that failed to
bind forced its criterion to `Unverified` with confidence 0.0
(`report_materialize.rs`, `bind_evidence`).

Before a materialized report became a signed `ReportCompleted` event, the core
applied a second persistence-boundary gate documented in
[REPORT-INTEGRITY.md](../REPORT-INTEGRITY.md). It re-resolved every claim and
ranked action from local Git and rejected a missing full commit, blob, line
range, or hash without returning source, keeping provider validation and
ledger acceptance as separate defenses.

Verdicts rolled up in three layers:

1. Per criterion: `Verdict` — `Supported | Partial | Unsupported | Unverified`
   (`crates/codecaddie-domain/src/model.rs`).
2. Per goal: `aggregate_goal` — all Supported → Supported, all Unsupported →
   Unsupported, otherwise Partial; nothing assessed → Unverified
   (`crates/codecaddie-domain/src/scoring.rs`). `score_report` computed a
   priority-weighted coverage score.
3. For display: six categories — strong, functional, incomplete, broken,
   missing, not-applicable
   (`crates/codecaddie-core/src/local_state/heatmap.rs`).

**Leak defenses** enforced the invariant that IPC, reports, and exports never
carry source text (`AGENTS.md`): a credential-marker list, a whole-repository
fingerprint check over narrative text (`repository.rs`, `each_fingerprint`
and `field_fingerprints`), and a cited-excerpt check that rejected any quoted
span of 16 or more characters from a cited range or any cited line of 24 or
more characters (`report_materialize.rs`, `screen_provider_narrative`;
contract published in
`plugin/skills/codecaddie-analysis/references/evidence-rules.md`).

### I.6 Results rendering

`workspace.recent` and `workspace.open` returned the approved goals, the full
latest `Report`, and a heatmap projection of recent reports
(`workspace_store.rs`, `LocalWorkspaceStore::recent_workspace`;
`heatmap.rs`, `HeatmapWeek`). The desktop rendered a report screen (overall,
improvement, and trend cards, the goals × runs heatmap, architecture findings,
recommendations) and a finding-detail screen per goal
(`apps/desktop/src/app.native`).

The finding-detail screen already rendered hash-verified source snippets: a
dedicated worker (`apps/desktop/src/snippet_worker.zig`) ran
`git show <commit>:<path>` against the user's local checkout, sliced the cited
lines, re-hashed with BLAKE3 against the stored `content_hash`, and zeroized
the buffer on any mismatch (`apps/desktop/src/model.zig`,
`SensitiveSnippet`). Source text resolved locally at render time and never
crossed core IPC — the mechanism that made "snippets as evidence" legal under
the privacy model, and it existed before this decision.

## Part II — Context at the time: the gaps

Findings 3–6 were bugs; the rest were design limits.

1. **Sampling, not surveying.** One bounded pass per two-goal batch, roughly a
   dozen tool calls, and a prompt that told the model to stop after two failed
   searches. On any non-trivial repository this sampled the tree; it could
   not survey it.

2. **No cross-batch reasoning and no memory.** Batches ran concurrently and
   mutually blind (`analyzer/scan.rs`, `run_scan`); a nine-goal scan was five
   disjoint mini-audits. Nothing persisted between scans — every run started
   from a fresh disposable snapshot with zero structural memory. There was no
   codebase map, index, or summary anywhere in the system; the only
   architecture output was at most five goal-scoped claims inside each report.

3. **Prompt/tool mismatch (bug).** The prompt hard-coded the Codex MCP tool
   names, but only Codex was given those tools (`provider/codex.rs`). Claude
   received `Read`/`Glob`/`Grep` (`provider/claude.rs`) and Grok
   `list_dir`/`grep`/`read_file` (`provider/grok.rs`) — two of three providers
   got search instructions addressed to tools that did not exist in their run.
   The turn limit was likewise only enforced for Grok (`--max-turns` in
   `provider/grok.rs`); the tool-call and stopping-turn limits were enforced
   nowhere.

4. **Silent losses (bugs).** Three places where valid provider work vanished
   without a trace: (a) an architecture claim or recommendation whose
   evidence failed to bind was dropped with no warning and no `partial` flag
   (`report_materialize.rs`); (b) architecture claims from several batches
   were truncated to five with no warning (`analyzer/scan.rs`); (c) bind
   failures were indistinguishable — an 81-line citation and a hallucinated
   path both produced the same "could not bind the provider's citation"
   rationale, so the user could not tell real-but-oversized evidence from
   fabrication. Batch-level failures, by contrast, were not silent: they set
   `partial` and appended warnings.

5. **Over-broad leak-check redaction (a bug in the response, not the
   detection).** The fingerprint check hashed every trimmed repository line
   of 16 or more characters, every single word of 16 or more characters, and
   every 2–4-word phrase of 16 or more characters (`repository.rs`,
   `each_fingerprint`). One hit — for example the phrase "authentication
   middleware" appearing in both a rationale and a README — triggered
   `redact_provider_narrative` (`report_materialize.rs`), which replaced every
   narrative field in the entire report with boilerplate. Verdicts and
   coordinates survived; all explanatory value was destroyed.

6. **Weak dedup.** The merge removed only adjacent duplicates after a sort
   keyed on goal priority, so identical claims attached to
   differently-prioritized goals survived as duplicates
   (`analyzer/scan.rs`).

7. **The results page starved the user.** Architecture-claim evidence
   rendered as a bare count of verified references (`apps/desktop/src/model.zig`)
   while the coordinates sat unrendered in the model and the snippet worker
   sat unwired. The full `Report` already crossed IPC, but the desktop parsed
   with narrow structs (`apps/desktop/src/core_ipc.zig`), silently
   discarding the priority-weighted coverage score, per-criterion
   `confidence`, `unverifiedCriteria`, provider and version, repositories at
   commit, `EvidenceRef.kind`, and `blobOid`. The Word export
   (`crates/codecaddie-core/src/export.rs`, `write_goal_report`) showed
   strictly more than the app.

8. **Retrieval quality (Codex path only).** Literal substring search with a
   `MAX_SEARCH_RESULTS` cap walked in sorted path order — common terms
   returned only alphabetically-early matches
   (`provider_repository_mcp.rs`). Improving this server helped only Codex;
   Claude and Grok used their own tools against the snapshot.

9. **Degradation semantics.** The wall clock converted unfinished batches to
   all-Unverified goals; batch fatality was classified by substring matching
   on error text (`analyzer/scan.rs`, `provider_batch_is_fatal`). The
   standalone (unattested) plugin mode enforced evidence rules by honor
   system plus a banner (`SKILL.md`).

The structural conclusion: the product's validation machinery (freezing,
binding, hashing, signing) was genuinely strong — the weakness was everything
upstream of it (shallow retrieval, no shared representation) and downstream
of it (evidence-starved display). Both ends were addressable without touching
the trust chain.

## Part III — Target architecture (as built)

Two stages. Stage 1 gives the analysis a durable, evidence-bound picture of
the codebase. Stage 2 makes every goal assessment explain itself against that
picture, on screen, with verifiable snippets.

### III-a. Stage 1: the codebase map

**What it is.** A typed graph, not a prose document
(`crates/codecaddie-domain/src/map.rs`):

- `MapOverview` — system summary (≤700 chars), architecture style, ≤16
  technology observations, each citing a manifest or configuration line.
- ≤24 `Component`s — name, `ComponentKind` (service, library, UI surface,
  data store, pipeline, infrastructure, external interface, test suite,
  build tooling), owning repository, ≤8 repository-relative root paths,
  responsibility (≤480 chars), ≤6 key interfaces, ≤3 concerns, 1–6 evidence
  anchors. Component ids are content-addressed (BLAKE3 of repository and
  name) so they are stable across regenerations.
- ≤48 `ComponentRelationship`s (calls, spawns, reads, writes, validates,
  depends-on, builds, serializes-to, endpoints restricted to declared
  components), ≤8 `DataFlow`s of 2–10 ordered steps, ≤24 `EntryPoint`s (CLI,
  IPC method, UI screen, MCP tool, build target).

Every item carries `EvidenceRef`s bound through the existing machinery — the
map contains paths, ranges, hashes, and derived prose, never source text, so
it is legal in IPC, the event log, and exports by construction. Worst case is
roughly 300 KiB serialized — far more architectural information than the
five-claim cap, while every individual narrative field stays small enough for
field-level screening. The sketch that was approved is in Appendix B.

**How it is generated** (`crates/codecaddie-core/src/analyzer/map_generate.rs`,
`generate_codebase_map`, reusing `ProviderRunner` and the batch patterns from
`analyzer/scan.rs`):

- **Phase A — inventory digest** (no provider): `inventory_digest` walks the
  already-materialized snapshot into a bounded metadata digest — a two-level
  directory tree with file counts, an extension histogram, and recognized
  manifests. Pure paths and counts, so the survey call spends its tool budget
  reading rather than discovering the layout.
- **Phase B — survey call** (one provider call): skeleton schema only
  (overview, components with one or two anchors, entry points), deliberately
  deeper than a goal batch because this pass is the survey the product lacked.
- **Phase C — component deep-dives** (chunks of `DEEP_DIVE_COMPONENTS_PER_CHUNK`
  components, at most `MAX_CONCURRENT_DEEP_DIVES` concurrent): each call gets
  the compact component index plus its chunk, and returns interfaces,
  concerns, relationships, and flows. Failed chunks degrade exactly like goal
  batches: skeleton entries survive and the map is marked partial with a
  warning.
- **Phase D — deterministic merge and validation** (no provider):
  `analyzer/map_materialize.rs`, `materialize_codebase_map`, mirrors
  `report_materialize`: `bind_evidence` for every item (drops emit named
  warnings, fixing gap 4's pattern at the source), cross-reference validation,
  narrative limits, and leak screening under `MapNarrativePolicy`. Merging a
  typed graph is deterministic; a synthesis pass would reintroduce exactly the
  ambiguous multi-document context that the bounded prompt contract removed.
- **Phase E — rephrase screened fields** (`rephrase_screened_fields`): the
  only provider call after the merge asks the provider to re-express
  narrative fragments that failed screening in its own words, re-screens each
  rewrite with the same credential and source-match defenses, and applies
  only the rewrites that pass.

The whole generation runs under `MAP_WALL_CLOCK`.

**Where it lives** (decision D1, Appendix D). A hybrid: a slim signed
`DomainEvent::CodebaseMapRecorded { descriptor }` in the event log
(`crates/codecaddie-domain/src/event.rs`) — `CodebaseMapDescriptor` carries
the map id, the frozen repository set (the invalidation key), the provider,
the content hash (BLAKE3 of the canonical map JSON), and a supersedes chain —
plus the content-addressed body under the data root's `codebase-maps-v1`
directory (`workspace_store.rs`, `LocalWorkspaceStore::record_codebase_map`
and `load_codebase_map_for`), written with the existing owner-only atomic
write posture (`persistence.rs`, `write_private_atomic_new`) and
hash-verified on every read, with superseded bodies garbage-collected. This
is not a second storage system under the `AGENTS.md` rule: everything stays
in the single data root, exactly as `agent-sessions/` and
`local-state-v2.json` already do, while the signed descriptor keeps
provenance in the ledger — which a bare cache file would not — and the
prunable body avoids growing an append-only log by roughly 150 KiB per commit,
which a full-map event would. `Report` gained optional `codebase_map_id` and
`codebase_map_hash` provenance fields following the `LegacyReport`
serde-default precedent (`crates/codecaddie-domain/src/event.rs`).

**Reuse and invalidation.** The key is the exact frozen repository set — the
same shape a `Report` pins — so "is this map current for this scan" is a
set-equality check (`WorkspaceProjection::latest_map_for`). Goal edits never
invalidate the map; it is goal-independent by design, and its projection arm
performs no approved-goal-set check. Any commit change makes it stale: v1
policy is full regeneration, mitigated by reuse across scans at the same
commit — the common "re-run after editing goals" case is map-free — and by the
explicit `map.generate` method (`service/map.rs`, `generate`) so the desktop
can pre-warm in the background on a new HEAD. A v2 incremental path is
designed for but not built: `git diff --name-only old..new` intersected with
`root_paths` selects dirty components, and re-running `bind_evidence` on
clean components' refs makes the existing binding machinery the staleness
oracle. The supersedes chain and content-addressed bodies already support it
without schema change.

**How Stage 2 consumes it without prompt bloat.** Two channels, and the full
map never rides in a goal prompt:

1. The validated map JSON is written to `codecaddie-map.json` in the
   disposable scan workspace (`analyzer/scan.rs`, `run_scan_with_map`). All
   three providers can read it there — Claude's `Read(./**)` allowlist,
   Codex's MCP server rooted at the workspace, Grok's `read_file` with the
   workspace as cwd — so this required zero new tool plumbing and adds zero
   exposure (the provider already sees the full source).
2. Each goal-batch prompt gains one bounded data section, `ARCHITECTURE MAP
   DIGEST (UNTRUSTED DERIVED DATA)`, built by `map_prompt_digest` in
   `analyzer/scan.rs`: the system summary plus a one-line-per-component index,
   and one sentence pointing at the workspace file. The digest is an index,
   not a competing document with its own output contract — the distinction
   that keeps this on the right side of the bounded-context lesson.

**Protocol and the agent path.** `scan.run` gained a Phase 0 "ensure map"
(`service/scan.rs`, `ensure_codebase_map`: reuse or generate; optional
`refreshMap` param, `ScanRequest::refresh_map`; on generation failure the scan
degrades to mapless with `partial` and a warning — the same honesty posture as
batch degradation). Two new scoped methods: `map.generate` (streaming, slim
receipt) and `map.get` (descriptor plus hash-verified body), registered in
`service.rs` (`DISPATCH`, `METHODS`, `streams_progress`), documented in
`protocol/README.md`, and covered by request fixtures. Progress messages stay
static-phase-only. The agent-session path participates symmetrically — MCP
tools `get_codebase_map` and `submit_codebase_map` (`mcp.rs`,
`agent_gateway.rs`; validated by the same `materialize_codebase_map` under
`MapNarrativePolicy::Reject`, evidence re-bound against the session's pinned
commits), mirrored as the `agent map` and `agent submit-map` CLI verbs
(`agent_cli.rs`), with `SKILL.md` updated so attested agents consume or
produce the map before evaluating goals.

**Prerequisite: fix the leak-check response globally** (decision D3). Before
any artifact with rich technical narrative could survive screening: drop the
single-word fingerprint tier (a lone identifier is coordinate-adjacent
vocabulary — reports already legally carry paths, which reveal more); raise
line and phrase thresholds toward the 24-character threshold
`evidence-rules.md` already published; and replace whole-report redaction
with field-granular redaction plus a warning (`report_materialize.rs`,
`redact_narrative_fields` and `NarrativeField`), failing the whole artifact
closed only for credential markers (`map_materialize.rs`,
`contains_credential_marker`) or when too large a share of fields redact. The
precise cited-excerpt check and the agent path's fail-closed `Reject` are
untouched. This fixed a standing false-positive bug rather than loosening the
privacy model — the old behavior destroyed entire valid reports over
dictionary phrases, which taught users to distrust redaction warnings.

### III-b. Stage 2: per-goal architecture narrative and snippet evidence

**Report model.** `GoalAssessment` gained `architecture_narrative` (wire
`architectureNarrative`, ≤700 chars, serde-default — the pattern the
`summary` field used) and `related_component_ids` (≤4, validated against the
seeding map by `validate_component_references` in `analyzer/scan.rs`; unknown
ids dropped, narrative kept). `ArchitectureClaim` gained
`component_id: Option<ComponentId>` as a cross-reference into the map. For
every historical report — which predates the narrative — the UI joins
`ArchitectureClaim.affected_goal_version_ids` to goals at render time, so the
architecture section appears for old reports too, populated from claim
summaries. The provider schema added both fields as required-nullable (the
strict-schema test demands every property required with
`additionalProperties: false`; optionality is expressed by nullability), and
serde defaults on the raw types (`RawGoalAssessment`,
`RawArchitectureClaim` in `analysis_contract.rs`) keep older plugin-skill
submissions parsing.

**Caps raised coherently.** The architecture-claim cap went from 5 to 12 flat
per report; that meant changing, together: `analysis.schema.json`
`maxItems`, the prompt sentence, the merge truncation
(`MAX_REPORT_ARCHITECTURE_CLAIMS` in `analyzer/scan.rs`), and the desktop's
`max_decision_items` (`apps/desktop/src/model.zig`) — raising the backend
alone would generate claims the UI cannot show. Batch-level schema stays at
five per call.

**Results page.** The finding-detail screen gained an `ARCHITECTURE SUPPORT`
section (`apps/desktop/src/app.native`) between WHAT CHANGED and CRITERION
RESULTS: the per-goal narrative, then a card per linked architecture claim
(component, summary, relationship, and each evidence coordinate with an
`EvidenceKind` badge and a snippet button). Snippets load on demand through
the existing worker — `max_snippet_slots` grew by `arch_snippet_slots`
(`model.zig`), architecture claims map to the slots starting at
`arch_snippet_slot`, and `snippet_worker.zig` needed no mechanical change:
the job struct already carried repository, path, commit, hash, and lines plus
a routing index, and the zeroization and generation-counter model is
untouched. The report screen's architecture cards upgraded from the bare
count to affected-goal chips and coordinate rows (no snippets there — live
source buffers stay confined to one screen, preserving the `SensitiveSnippet`
posture).

**IPC.** Most of the needed data already crossed the wire and was dropped at
parse. Rust added only `HeatmapCell.architecture_narrative`,
`HeatmapWeek.architecture` (reusing the domain `ArchitectureClaim` — ids,
coordinates, prose only) and `HeatmapWeek.coverage`, and
`HeatmapCriterion.confidence` (`local_state/heatmap.rs`). Zig started parsing
what was already sent: provider, `providerVersion`, `unverifiedCriteria`,
repositories, goal version id, `confidence`, `blobOid`, and evidence kind
(`apps/desktop/src/core_ipc.zig`). All new Zig fields default, so an old core
binary with a new desktop degrades gracefully. Because the Word export is
built entirely from `HeatmapWeek` (`export.rs`, `write_goal_report`), it
gained the architecture sections and a weighted-coverage line for free —
coordinates only, per the existing export rule.

**Quick wins** (independent, all display-only): bind the already-populated
`FindingCell.checks` and `.references`; bind the already-computed snippet
language instead of a hard-coded plain language; kind badges on evidence rows;
a COVERAGE card next to OVERALL; unverified-criteria and provider/commit
provenance chips; a confidence chip when confidence is low.

## Part IV — Phased roadmap (as followed)

The sequencing principle: land correctness first, because until then one
could not distinguish "the model was shallow" from "the pipeline discarded
the model's work" — and that distinction determined how much the map was
worth.

**Phase 0 — correctness (days).** Parameterize the prompt's tool-name and
turn-limit sentences by provider contract (`provider_tool_text` in
`analysis_contract.rs`; gap 3); surface the three silent losses with warnings
and propagated bind-failure reasons (gap 4); clamp over-long citations to
`MAX_EVIDENCE_LINES` (keep the start, truncate, mark clamped) instead of
discarding them; full non-adjacent dedup of merged claims (gap 6).

**Phase 1 — the visible half, without the map (about a week).**
Field-granular leak redaction (gap 5; the hard prerequisite for the map); the
coherent cap raise (tool calls roughly 12 → 30, architecture 5 → 12,
citations 3 → 5, schema, prompt, truncation, and `model.zig` together); the
results-page ARCHITECTURE SUPPORT section fed by existing
`ArchitectureClaim`s via the render-time join, with on-demand snippets; the
quick wins. This shipped most of what a user perceives as "more thorough" and
de-risked the map's two biggest hazards before it existed.

**Phase 2 — the minimal viable map.** As III-a: structured component
inventory, a bounded number of provider calls, cache-first (never block a
scan that has a valid cached map), goal batches stay parallel,
degrade-not-fail. Reports gained map provenance; assessments gained
map-seeded narratives and component links.

**Phase 3 — only if Phase 2 earned it.** A browsable Architecture map screen
in the desktop (`app.native`, the `open_architecture` action) shipped. Map
history and diffing across commits, and a reflection or verification pass per
batch (valuable, but roughly doubling scan time), remain unbuilt.

**Explicit non-goals.** A prose architecture essay (the bounded-context
lesson, and prose maximizes leak-check surface); map generation blocking
every scan; AST, symbol, or embedding indexing (the pipeline's pure-git
simplicity is a deliberate privacy-surface choice — no code-intelligence
engine bolted onto a per-request process); per-goal map queries (multiplies
provider calls); sequential cross-batch digest passing (three times the wall
clock — strictly dominated by the map); and any weakening of leak detection —
only the redaction response changed.

## Appendix A — Target pipeline diagram

```
scan.run {reportId, repositories, provider, refreshMap?, stream?}
│
├─ resolve commits, freeze disposable multi-repo workspace          (existing)
│
├─ PHASE 0 — ensure map                     service/scan.rs · ensure_codebase_map
│    projection.latest_map_for(frozen set)
│      ├─ hit + body hash verifies + !refreshMap ─────────► reuse (0 provider calls)
│      └─ miss / stale / refreshMap ──► GENERATE           map_generate.rs
│            A  inventory digest      core walk, no provider
│            B  survey call           1 call, skeleton schema
│            C  deep-dives            chunked, bounded concurrency
│            D  merge → map_materialize (bind, xref, field-level screen)
│            E  rephrase screened fields, re-screen, apply passes only
│            persist: CodebaseMapRecorded{descriptor} event
│                     + codebase-maps-v1/<hash>.json body
│            (on failure: continue mapless, report.partial + warning)
│
├─ write <workspace>/codecaddie-map.json  (validated map, read-only)
│
├─ PHASE 1 — goal evaluation (2-goal batches, bounded concurrency)
│    prompt += MAP DIGEST section + pointer to codecaddie-map.json
│    RawGoalAssessment += architectureNarrative?, relatedComponentIds[]
│    RawArchitectureClaim += componentId?
│
├─ PHASE 2 — materialize report (existing, with field-granular screening)
│    validate component ids against seeding map; architecture cap 12
│    report.codebase_map_id / codebase_map_hash set
│
└─ ReportCompleted event → projection → desktop / Word export
     finding detail: per-goal narrative + component cards
     evidence → snippet_worker (local git show + BLAKE3 verify) at render time

map.generate ──► PHASE 0 generate branch only, standalone receipt
map.get      ──► descriptor + hash-verified body
agent session ─► get_codebase_map / submit_codebase_map (Reject policy)
```

## Appendix B — Domain types as approved (sketch)

This is the sketch the decision was taken on. The shipped types live in
`crates/codecaddie-domain/src/map.rs` and `event.rs`; field names below match
those types, and the caps are enforced there.

```rust
pub struct CodebaseMap {
    pub id: CodebaseMapId,
    pub schema_version: u32,
    pub generated_at: OffsetDateTime,
    pub repositories: Vec<FrozenRepository>,   // the invalidation key
    pub provider: String,
    pub provider_version: String,
    pub origin: ReportOrigin,                  // Scan | AgentSession (reused)
    pub overview: MapOverview,                 // summary ≤700, style ≤240, ≤16 techs
    pub components: Vec<Component>,            // ≤24
    pub relationships: Vec<ComponentRelationship>, // ≤48, endpoints validated
    pub data_flows: Vec<DataFlow>,             // ≤8 × ≤10 steps
    pub entry_points: Vec<EntryPoint>,         // ≤24
    pub partial: bool,
    pub analysis_warnings: Vec<String>,
    pub supersedes: Option<CodebaseMapId>,
}

pub struct Component {
    pub id: ComponentId,                       // content-addressed
    pub name: String,                          // ≤120
    pub kind: ComponentKind,
    pub repository_id: RepositoryId,
    pub root_paths: Vec<String>,               // ≤8, repository-relative
    pub responsibility: String,                // ≤480 derived prose
    pub key_interfaces: Vec<KeyInterface>,     // ≤6, each 1-2 EvidenceRefs
    pub concerns: Vec<MapConcern>,             // ≤3, each 1-2 EvidenceRefs
    pub evidence: Vec<EvidenceRef>,            // 1..=6 anchors
}

// Slim signed event; body content-addressed on disk beside the log.
DomainEvent::CodebaseMapRecorded {
    descriptor: CodebaseMapDescriptor { /* map id, schema version,
        repositories, generated_at, provider, origin, content_hash,
        component_count, partial, supersedes */ }
}

// Report / assessment additions (all serde-default, LegacyReport precedent)
Report            += codebase_map_id: Option<CodebaseMapId>,
                     codebase_map_hash: Option<String>
GoalAssessment    += architecture_narrative: String,          // ≤700
                     related_component_ids: Vec<ComponentId>  // ≤4
ArchitectureClaim += component_id: Option<ComponentId>
```

## Appendix C — Risk register for the map

| Risk | Mitigation |
|---|---|
| Scan latency (each provider call is minutes) | One bounded generation sized like a scan; cache-first by frozen repository set; pre-warm via `map.generate`; never block on refresh when a valid map exists |
| Staleness across commits | Evidence-bound claims plus `read_evidence` re-verification as the staleness oracle; v1 full regeneration, v2 incremental designed for (diff intersected with root paths) |
| Prompt bloat | Map enters prompts only as a bounded index; full map via the workspace file the provider reads on demand |
| Leak-check false positives on rich narrative | Field-granular redaction fix landed first (Phase 1); the map is validated through the same gate at generation time, so stored text is pre-cleared |
| Injection via the map (derived from untrusted repository text, fed back into prompts) | UNTRUSTED DERIVED DATA framing after trusted sections (existing ordering pattern and test); schema-validated on load; hash-verified body |
| A wrong map poisons every batch | Every claim binds evidence or is dropped with a named warning; unverifiable claims never enter prompts |
| Hidden cost of a new artifact type | Honest budget: domain event, projection, storage, protocol, MCP, and desktop are the bulk of the work, not the prompts — hence Phases 0–1 first |

## Appendix D — Decisions taken

| # | Decision | Outcome |
|---|---|---|
| D1 | Map storage | Descriptor event plus content-addressed body in the data root — signed provenance without unbounded log growth; precedent `agent-sessions/` |
| D2 | Map → Stage 2 delivery | Workspace file plus a bounded inline digest — provider-uniform, zero new tool plumbing |
| D3 | Leak-fix scope | Global (reports and maps) — map-only special-casing would fork the single validator |
| D4 | Agent sessions and maps in v1 | Serve and submit (`get_codebase_map` / `submit_codebase_map`, Reject policy) — keeps the two paths symmetric |
| D5 | Refresh strategy | Full regeneration in v1 (`refreshMap`); incremental is v2, already supported by the descriptor design |
| D6 | Map failure during a scan | Degrade to mapless with `partial` and a warning — matches the batch-degradation posture |
| D7 | Architecture-claim cap | Flat 12 per report (`MAX_REPORT_ARCHITECTURE_CLAIMS`, `max_decision_items`), changed with schema, prompt, truncation, and desktop cap together |
| D8 | Provider synthesis pass in the map merge | No — deterministic graph merge; the only post-merge provider call is the bounded rephrase of screened fields, which is re-screened before use |
