---
name: verify-codecaddie
description: Launch CodeCaddie's headless Rust core and desktop state machine, check health, drive one user-facing feature, capture source-free evidence, and clean up. Use before declaring any UI or core change done, or when asked to prove a CodeCaddie change works at runtime.
---

# Verify CodeCaddie

This skill proves a change works at runtime, not only that it compiles. Work
through the six phases in order; stop at the first failure and report it with
the evidence collected so far. Every command runs from the repository root.

Feature-specific routes (what to drive and what to expect) live in
`features/`; read `features/README.md` first and pick one file.

## Launch

CodeCaddie has no daemon. The desktop starts one `codecaddie-core` process per
request over stdin/stdout, so "launched" means: the core binary is built, a
private data root exists, and the desktop state machine passes.

1. Build the core and record the tree state:

   ```sh
   cargo build --workspace --locked
   CORE=target/debug/codecaddie-core
   T="${TMPDIR:-/tmp}"; T="${T%/}"
   SHA=$(git rev-parse --short HEAD); [ -z "$(git status --porcelain)" ] || SHA="$SHA-dirty"
   ```

2. Create a fresh owner-only data root and export it for every later command.
   Never reuse the real per-platform data directory and never place the root
   inside the checkout:

   ```sh
   export CODECADDIE_DATA_DIR="$(mktemp -d "$T/codecaddie-verify-data.XXXXXX")"
   chmod 700 "$CODECADDIE_DATA_DIR"
   ```

3. Create the evidence directory now (layout under Evidence) so every phase
   can write into it:

   ```sh
   export EVIDENCE="$T/codecaddie-verify/$(date -u +%Y%m%dT%H%M%SZ)-$SHA"
   mkdir -p "$EVIDENCE"
   ```

4. Desktop state machine. Required for any change under `apps/desktop/`, and
   cheap enough to run every time:

   ```sh
   pnpm exec native check apps/desktop --strict
   pnpm exec native test apps/desktop --yes 2>&1 | tee "$EVIDENCE/native-test.log"
   ```

   The suites in `apps/desktop/src/tests.zig` and
   `apps/desktop/src/first_report_journey_assurance.zig` drive the real
   `update` function with a fake executor, so they cover every screen without
   a display. `native test` has no filter flag; it always runs the whole suite.

5. `pnpm dev` (or `pnpm dev:isolated`) only when the change touches what a
   user sees. It launches the real window against `target/debug/codecaddie-core`;
   `.native` markup hot-reloads, Zig changes need a restart. Record the PID so
   Cleanup stops exactly that process.

## Doctor

Confirm the built core is the one under test and answers the protocol before
driving anything.

1. Health line. A development build reports build `0`:

   ```sh
   "$CORE" --health-check | tee "$EVIDENCE/health.txt"
   # CodeCaddie 0.4.0+0 <commit>
   ```

2. `system.ping` must be the first framed request. Use the helper described
   under Helpers:

   ```sh
   node .agents/skills/verify-codecaddie/frame.mjs system.ping > "$EVIDENCE/ping.json"
   ```

   Expect `ok: true`, `result.service` = `codecaddie-core`,
   `result.protocolVersion` = `2`, and `result.build.channel` = `dev`.

3. Providers and privacy:

   ```sh
   node .agents/skills/verify-codecaddie/frame.mjs providers.detect > "$EVIDENCE/providers.json"
   node .agents/skills/verify-codecaddie/frame.mjs privacy.promise  > "$EVIDENCE/privacy.json"
   ```

   `providers.detect` lists `claude`, `codex`, and `grok` with `installed` and
   `version`; a provider-backed Drive needs at least one `installed: true`.
   `privacy.promise` must include `attachedContextSentToProvider`.

If the serialized ping response contains `source`, `attachment`, `goaltext`,
`prompt`, `credential`, or `keychain` (case-insensitive), stop: that is the
negative check `scripts/exercise-installed-core.mjs` enforces in CI.

## Drive

Drive one feature per run unless the change spans several. The deterministic
path needs no AI provider and finishes in seconds; the provider-backed path is
optional and slow.

Fixture: a throwaway Git repository built from `testdata/golden/monolith`,
never the CodeCaddie checkout itself:

```sh
REPO="$(mktemp -d "$T/codecaddie-verify-repo.XXXXXX")"
cp -R testdata/golden/monolith/. "$REPO"
git -C "$REPO" init --quiet && git -C "$REPO" add . \
  && git -C "$REPO" -c user.name=verify -c user.email=verify@invalid.example commit --quiet -m "verify fixture"
COMMIT=$(git -C "$REPO" rev-parse HEAD)
```

Deterministic journey (the same one `pnpm verify:core` runs end to end):

1. `workspace.create` with `name`, `repositoryDisplayName`, `repositoryPath`
   = `$REPO`, `productBrief`, `context: {}`. Keep `result.workspaceId`.
2. `goals.approve` (scoped with `workspaceId`) once per goal. The three
   synthetic goals in `fixtures/journeys/synthetic-goals.json` are a known-good
   shape (`goalId`, `title`, `businessOutcome`, `criteria[]`, `priority`,
   `position`, `rubricDimensions`). Keep `result.goalVersion.id` and
   `result.goalVersion.criteria[0].id`.
3. `"$CORE" agent status --workspace <id>` and note `exchange.inbox` and
   `exchange.outbox`.
4. `"$CORE" agent begin-analysis --repo attached-repository@$COMMIT --workspace <id>`
   and note `analysisSessionId`.
5. Write an analysis payload into `exchange.inbox` (one assessment citing a
   `path` that exists at `$COMMIT`, a `startLine`/`endLine` inside that file,
   `kind: "test"`, verdict `supported`), then
   `"$CORE" agent submit-analysis --session <sessionId> --file <payload>`.
   Expect `recorded: true`; the inbox file is deleted on success.
6. `workspace.open`: `result.workspace.latestReport.id` equals the submitted
   `reportId` and `repositories[0].commitSha` equals `$COMMIT`.
7. `reports.history.list` and `reports.finding.get` (scoped; `reportEventId`
   from the history row, `goalVersionId` from step 2). Save the finding as
   `report-finding.json`.
8. `"$CORE" agent export --kind word --out "$EVIDENCE/export.docx"`; the file
   starts with the bytes `PK`.

Shortcut for the whole deterministic journey, with evidence retained:

```sh
pnpm verify:core 2>&1 | tee "$EVIDENCE/verify-core.log"
```

Provider-backed drive (optional; only when the change touches `analyzer/`,
`provider/`, or goal generation): send `scan.run` with `"stream": true`,
`provider` set to an installed CLI, the approved `goals`, `productBrief`, and
`repositories: [{repositoryId, repositoryPath, commit}]`. Progress arrives as
NDJSON `scan.progress` lines; the terminal line is a slim receipt and the
report is re-read through `workspace.recent`. Budget several minutes. A
missing provider must fail with error code `scan_failed` and never with
provider stderr.

For the screen-level route of each feature, follow `features/<feature>.md`.

## Evidence

Everything a reviewer needs in order to believe the run lives in one directory
that survives Cleanup:

```
$TMPDIR/codecaddie-verify/<utc-timestamp>-<short-sha>[-dirty]/
  health.txt            --health-check line
  ping.json             system.ping response
  providers.json        providers.detect response
  privacy.json          privacy.promise response
  report-finding.json   reports.finding.get response for the driven goal
  export.docx           metadata-only Word export
  native-test.log       desktop state-machine run (when executed)
  verify-core.log       pnpm verify:core output (when executed)
  screenshots/          only from a pnpm dev session, named <feature>-<step>.png
  summary.md            what was driven, what passed, what did not
```

`summary.md` names the feature file used, the commit, the data root, and each
phase's outcome in one line each. Write it last.

Negative checks; both must print nothing:

```sh
grep -rIl "PRIVATE SOURCE CANARY" "$EVIDENCE" || true
grep -rIl --exclude=summary.md --exclude='*.log' "$(git rev-parse --show-toplevel)" "$EVIDENCE" || true
```

The first proves no repository source leaked into a response or export; the
second proves no absolute checkout path leaked into report data. Then run the
same checker CI uses over the data root, which also covers the sentinel strings
the Rust privacy tests plant and the fixture repository's absolute path:

```sh
node scripts/assert-source-free.mjs --directory "$CODECADDIE_DATA_DIR" --fixture-repo "$REPO"
```

## Cleanup

1. Stop only processes this run started (`pnpm dev`, if launched). Core
   processes are one-shot and exit on their own; never `pkill codecaddie`.
2. Remove the data root and fixture only if they are the temporary
   directories created in Launch and Drive:

   ```sh
   case "$CODECADDIE_DATA_DIR" in "$T"/codecaddie-verify-data.*) rm -rf "$CODECADDIE_DATA_DIR";; esac
   case "$REPO" in "$T"/codecaddie-verify-repo.*) rm -rf "$REPO";; esac
   unset CODECADDIE_DATA_DIR
   ```

3. Never touch `~/Library/Application Support/CodeCaddie*`,
   `%APPDATA%\CodeCaddie*`, or `$XDG_DATA_HOME/codecaddie*`.
4. Keep `$EVIDENCE`. Report its path.

## Helpers

`frame.mjs` (beside this file) sends one length-prefixed request to the core
and prints the decoded response as JSON. It imports `encodeFrame` and
`decodeSingleFrame` from `scripts/exercise-installed-core.mjs`, so the framing
is the same code the CI harness uses.

```sh
node .agents/skills/verify-codecaddie/frame.mjs <method> [params-json] [--workspace <id>] [--binary <path>] [--id <request-id>]
node .agents/skills/verify-codecaddie/frame.mjs workspace.open '{"workspaceId":"<id>"}'
node .agents/skills/verify-codecaddie/frame.mjs goals.approve "$(cat goal.json)" --workspace "$WS"
```

Exit status is 0 when `ok` is true, 1 when the core answered with an error
object, and 2 when the process itself failed. `CODECADDIE_DATA_DIR` must be
set; the helper refuses to run without it so a stray call can never touch the
real data root.

Other helpers already in the repository:

- `scripts/exercise-installed-core.mjs`: the full deterministic journey
  (`pnpm verify:core` wraps it).
- `scripts/dev-isolated.mjs`: `pnpm dev` against a data root derived from the
  worktree path.
- `protocol/fixtures/`: valid request and event envelopes for every method.
