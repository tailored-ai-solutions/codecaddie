# Core protocol

The Zig desktop host and the Rust core communicate only through private
stdin/stdout pipes. Every frame is a four-byte unsigned big-endian length
followed by one UTF-8 JSON object. The maximum payload is 16 MiB. Logs are
written to stderr and must never be mixed with protocol output.

`fixtures/` is the cross-language compatibility corpus. Both runtimes validate
these envelopes, reject unknown protocol versions, and reject oversized or
truncated frames. Every request fixture must name a method the core actually
implements; the Rust fixture test walks the whole directory and checks each
method against the catalog below (mirrored by `service::METHODS`).

Protocol version 2 replaced the pre-release shared-workspace-key formats;
unsafe development workspaces are intentionally recreated rather than
migrated.

`core.schema.json` is the envelope contract itself: the request, response,
and event shapes that every length-prefixed frame must match.

`local-product-events-v2.schema.json` is the current schema for the
content-free lifecycle records used for device-local product measurement; v2
adds the `workspaceId` field that scopes each record to one workspace.
`local-product-events-v1.schema.json` is retained as the historical shape so
older ledgers still validate. Both remain inside the signed workspace ledger;
`workspace.recent` returns only aggregate counts and elapsed times.

`first-report-activation-v1.schema.json` is the versioned local calculation
contract for the first-saved-report activation metric (the share of workspaces
that save a first report within ten minutes) and its repeat-review and
decision-cycle companions, including the privacy denylist those calculations
must respect.

`portable-backup-v1.schema.json` closes the encrypted portable backup
envelope produced by `workspace.backup.export`: the format constant, the
XChaCha20-Poly1305 algorithm, the Argon2id parameters, salt, nonce, and
ciphertext, and nothing else.

`provider-contract-v1.schema.json` records the execution contract CodeCaddie
holds each installed provider CLI to: the request it receives, the response
shape it must return, the evidence rules, the process lifecycle, and the
fallback behavior when a provider fails.

`repository-assurance-index-v1.schema.json` describes the optional
`.codecaddie/assurance.json` routing index a repository may ship. It only
points analysis at artifacts to inspect; it is never treated as evidence.

`local-reliability-event-v1.schema.json` defines the content-free operation,
desktop-session, and local SLO-alert records. The same signed ledger stores
them, and `workspace.recent` returns only aggregate availability, latency,
failure, cancellation, alert, and crash-free-session measurements.

`persisted-report-evidence-v1.schema.json` documents the only repository
coordinate shape accepted at the signed report boundary. The core re-resolves
the full commit, blob, path, line range, and excerpt hash locally before
persisting any supported or partial claim, architecture finding, or ranked
recommendation. Repeat-history projections retain prior and current evidence
arrays so field-level comparisons never overwrite the earlier proof set.

`updater-result-v1.schema.json` closes the content-free one-shot result shared
by the external updater helper, its private local mailbox, and the desktop's
startup handshake. It permits only schema version 1, `status: "failed"`, and a
fixed result code; paths, source or attachment text, secrets, raw installer or
OS errors, and all other free-form fields are rejected.

## Method catalog

Every method the core dispatches, kept in step with `service::METHODS` and
the request fixtures. "Scoped" methods require the envelope's `workspaceId`.
All methods exist since protocol version 2 (the earliest supported version);
none carry repository source text in either direction.

| Method | Params | Result | Since |
| --- | --- | --- | --- |
| `system.ping` | optional startup-only `consumeUpdaterResult: true`; absent, false, and non-boolean values are read-only | `protocolVersion`, `service`, `build` summary; an opted-in startup also receives nullable `updaterResult` | 2 |
| `updates.check` | none | update check result (`currentVersion`, `latestVersion`, `available`, `required`, `releaseNotesUrl`, optional `artifact`) | 2 |
| `updates.download` | none | staged update (`version`, `build`, `artifactPath`, `size`, `sha256`) | 2 |
| `updates.install` | `stagedPath`, `parentPid` | `status: "readyToRestart"`, `version`, `build` | 2 |
| `settings.launchAtLogin.get` | none | `enabled`, `supported` | 2 |
| `settings.launchAtLogin.set` | `enabled` | `enabled`, `supported` | 2 |
| `settings.provider.get` | none | `provider` (`claude`, `codex`, `grok`, or `null`) | 2 |
| `settings.provider.set` | `provider` | `provider` | 2 |
| `providers.detect` | none | array of provider capabilities (`kind`, `installed`, `version`) | 2 |
| `privacy.promise` | none | privacy invariants, including `attachedContextSentToProvider` | 2 |
| `context.files.inspect` | `paths` (up to 10 absolute local paths) | metadata-only `files` and `status` | 2 |
| `workspace.create` | `name`, `repositoryDisplayName`, `repositoryPath`, `productBrief`, optional `context` | `workspaceId`, `name`, `encryptedAtRest` (`true`), `storage` (`local-encrypted-json`), `role`, `contextFiles` | 2 |
| `workspace.recent` | none | `workspace` (most recent local workspace or `null`) | 2 |
| `workspace.open` | `workspaceId` | `workspace` | 2 |
| `workspace.context.update` (scoped) | `name`, optional `repositoryPath`, `productBrief`, optional `context` | `workspaceId`, `updated`, `contextFiles` | 2 |
| `workspace.recovery.export` (scoped) | `destination` | `destination`, `format` (`plain-json`) | 2 |
| `workspace.backup.export` (scoped) | `destination`, `passphrase` (12–1024 bytes) | `workspaceId`, `eventCount`, `manifestBlake3`, `format` (`codecaddie-portable-backup-v1`) | 2 |
| `workspace.backup.import` | `source`, `repositoryPath`, `passphrase` (12–1024 bytes) | `workspaceId`, `eventCount`, `manifestBlake3`, `status: "restored"` | 2 |
| `workspace.backup.schedule.status` (scoped) | none | content-free enablement, destination, 24-hour cadence/RPO, 30-minute RTO, retention, and last/next timestamps; never the passphrase | 2 |
| `workspace.backup.schedule.enable` (scoped) | `destinationDirectory`, `passphrase` (12–1024 bytes) | first `created` backup receipt and sanitized schedule | 2 |
| `workspace.backup.schedule.disable` (scoped) | none | disabled schedule; existing backup files remain | 2 |
| `workspace.backup.schedule.run` (scoped) | optional `force` (defaults false) | `created`, `not_due`, or `disabled` receipt and sanitized schedule | 2 |
| `reports.export_word` (scoped) | `destination` | `destination`, `format: "docx"` | 2 |
| `reports.history.list` (scoped) | optional `beforeEventId`, optional `limit` (1–100; defaults 50) | lightweight active `runs`, `totalActiveRuns`, `hasOlder`, optional `nextBefore`; never criteria or evidence | 2 |
| `reports.finding.get` (scoped) | immutable `reportEventId`, current `goalVersionId` | one full saved `finding` with bounded criteria, evidence coordinates, and architecture claims | 2 |
| `reports.delete` (scoped) | immutable `reportEventId` | `reportEventId`, `deleted: true`; rejects the latest run | 2 |
| `goals.approve` (scoped) | goal fields (`goalId`, `title`, `businessOutcome`, `criteria`, `priority`, `position`, `rubricDimensions`) | `goalVersion`, `status: "approved"` | 2 |
| `goals.replace` (scoped) | `goals` (array of goal fields) | `goals`, `status: "approved"` | 2 |
| `actions.ready` (scoped) | `recommendationId`, `title`, `note` | `action` | 2 |
| `instrumentation.record` (scoped) | content-free UI action (`event: "report_opened"`) | `recorded`; the signed envelope supplies time and workspace provenance | 2 |
| `reliability.record` (scoped) | allowlisted `kind` (`session_started`, `session_ended`, or `operation_cancelled`), opaque `sessionId`, and core-owned cancellation `operation` | content-free `correlationId`, `crashDetected`, and opaque `sessionId` | 2 |
| `recommendations.prompt` (scoped) | `recommendationIds` (one to five ids from the latest report), optional `intent` (`implementation`, `goal_contract`, or `analysis_audit`; defaults to `implementation`) | deterministic metadata-only action `prompt`, selected ids, report id, repository commit state, and drift warnings | 2 |
| `recommendations.copy_prompt` (scoped) | editable `prompt` (maximum 64 KiB) | `bytesCopied`; the prompt is never echoed or stored | 2 |
| `scan.run` (optionally scoped) | `reportId`, `repositories` (`repositoryId`, `repositoryPath`, optional `commit`), `provider`, `goals`, `productBrief`, optional `stream`, optional `refreshMap` | full report, or a slim receipt (`reportId`, `recorded`, `partial`, `warnings`) when streaming | 2 |
| `goals.generate` | scoped: `provider`, optional `existingGoals`/`stream` (stored context is authoritative); unscoped legacy: `provider`, `productBrief`, optional `existingGoals`/`stream` | `goals`, `status: "draft"`, `contextSourcesUsed` | 2 |
| `map.generate` (scoped) | `repositories` (`repositoryId`, `repositoryPath`, optional `commit`), `provider`, optional `refresh`, optional `stream` | full codebase map, or a slim receipt (`mapId`, `generated`, `partial`, `componentCount`, `warnings`) when streaming | 2 |
| `map.get` (scoped) | optional `mapId` | `descriptor` plus the hash-verified `map` body | 2 |

The public updater uses three methods that never carry repository content:

- `updates.check` verifies the signed channel manifest and selects the exact
  OS/architecture artifact.
- `updates.download` re-verifies the manifest, downloads into the private data
  directory, and stages only after size and SHA-256 checks succeed.
- `updates.install` re-verifies the staged payload, starts the external updater,
  and returns `readyToRestart`; the desktop must then quit explicitly.

The desktop's first `system.ping` may explicitly set
`consumeUpdaterResult: true`. That exact boolean consumes the helper's private
one-shot mailbox and adds `updaterResult`, either `null` or the closed
`updater-result-v1` object, to the ping result. Ordinary pings omit the field
and never consume the mailbox, preserving the historical protocol-v2 response
shape. The desktop maps fixed codes to desktop-owned guidance instead of
displaying or persisting helper, filesystem, installer, or operating-system
error text.

Long-running hosts may surface `updates.download.progress` events with
`receivedBytes` and `totalBytes`. These events are derived transport metadata,
never source-bearing content.

## Streaming requests

A host may add `"stream": true` to the params of `goals.generate`,
`scan.run`, or `map.generate`. The core then answers that request in NDJSON
instead of a length-prefixed frame: zero or more `CoreEvent` lines (topics
`goals.generate.progress`, `scan.progress`, and `map.generate.progress`,
payload `{"message": "..."}`) while the provider runs, followed by exactly
one terminal `CoreResponse` line. Event messages are sanitized, display-ready
derived text — provider stderr is never forwarded. Repository-reading passes
may report a safe repository-relative filename, a distinct-file ordinal for
that provider pass, and the number of regular files available in the disposable
snapshot. Listing and search events never include provider queries or source
text, and an ordinal is not represented as an exhaustive sequential scan. The streaming `scan.run`
terminal response is a slim receipt (`reportId`, `recorded`); the report
itself is persisted and re-read through `workspace.recent`. All other
methods, and these methods without the flag, remain length-prefixed.

## Project context

`workspace.create` accepts an optional structured `context` object
(`company`, `website`, `notes`, transient `contextFilePaths`, structured
`contextFiles`, and legacy `contextFileNames`) alongside the flattened
`productBrief`. `workspace.context.update` (workspace-scoped) rewrites the
name, repository path, brief, and context of an existing workspace in place
— same workspace id, no events appended — so approved goals and report
history survive edits; blank `name`/`repositoryPath` values leave the
stored ones unchanged.
`workspace.recent` and `workspace.open` return the stored `context` so hosts
can rehydrate their setup forms. Context is device-local settings data, like
`repositoryPath`; it never enters the signed event log.

The core validates transient paths and persists only a canonical device-local
reference: display name, path, media type, byte size, BLAKE3 hash, and page/
slide/section count. `context.files.inspect` exposes the same metadata without
persisting it. Extracted document text never appears in protocol responses,
progress events, diagnostics, reports, or exports. Legacy filename-only
references remain decodable but cannot be used for generation until reattached.
Scoped goal generation re-reads and hashes the stored references; its
`contextSourcesUsed` omits absolute paths and source text.

## Settings

`settings.launchAtLogin.get`/`.set` manage the login item.
`settings.provider.get`/`.set` persist the explicitly selected provider
(`claude`, `codex`, or `grok`) as a plaintext one-word file beside the
`recent-workspace` pointer; invalid stored values read back as `null`.

## Staged request files

Hosts whose spawn-stdin budget is smaller than a request (large goal sets,
long product briefs) may write the same length-prefixed frames to a private
file and start the core with `--request-file <path>`. The core reads every
frame, deletes the file, and answers on stdout exactly as if the frames had
arrived on stdin.
