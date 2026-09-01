# Analysis report

## Sub-features

- Frozen-commit scan (`scan.run`): the core resolves the exact commit,
  materializes a disposable history-free snapshot, runs the selected provider
  in bounded goal batches, and validates every claim against the Git object
  database before saving.
- Report sections: **Analysis summary** (per-goal status across saved runs),
  **Architecture findings**, **Recommendations**, and goal-by-goal detail with
  per-criterion verdicts and evidence coordinates (`path:start-end @ commit`).
- Recommendation prompts: select one to five recommendations, choose the
  implementation, goal-contract, or analysis-audit path, edit the deterministic
  metadata-only prompt, copy it.
- Failure and cancellation: the goal set is kept, the last saved report stays,
  and **Retry analysis** starts a new bounded run.
- Agent-submitted reports: the same validation through `agent begin-analysis`
  and `agent submit-analysis` (used by the plugin and the Grok Bot routine),
  badged as agent sessions in the report header.

## How to get to it (user POV)

1. On **Goals**, click **Analyze repository**. The report screen shows a
   **LIVE** badge beside "Analyzing the repository with <provider>" and the
   provider activity feed.
2. On success, "Analysis complete" appears with the summary. **Show all
   sections** and the section buttons narrow the view to architecture
   findings, recommendations, or goal details.
3. Click a summary cell or **View finding details** for one goal's criteria;
   **View snippet** loads a hash-verified snippet from the local repository,
   never from the report. **Previous goal** / **Next goal** move between
   findings.
4. **Select recommendations**, pick items, **Choose a fix**, **Create prompt**,
   edit, **Copy prompt**. **Edit goals directly** is the manual escape hatch.
5. On failure, "Analysis did not finish" shows source-free guidance and
   **Retry analysis**.

## Driving it with the harness

Deterministic (no provider), the path `pnpm verify:core` exercises:

1. `agent status --workspace <id>` returns `exchange.inbox` and
   `exchange.outbox`.
2. `agent begin-analysis --repo attached-repository@<commit> --workspace <id>`
   returns `analysisSessionId`; `repositories[0].commitSha` must equal the
   fixture commit.
3. Write `{providerVersion, assessments:[{goalVersionId, summary,
   criteria:[{criterionId, verdict, rationale, confidence,
   evidence:[{repositoryId:"attached-repository", path, startLine, endLine,
   kind}]}]}], architecture:[], recommendations:[]}` into `exchange.inbox`,
   then `agent submit-analysis --session <id> --file <path>`. Expect
   `recorded: true`, `coverage`, and `reportId`.
4. `workspace.open`: `workspace.latestReport.id` equals `reportId`.
   `reports.finding.get` (scoped; `reportEventId`, `goalVersionId`) returns
   the full finding.

Provider-backed: `scan.run` (optionally scoped) with `reportId`,
`repositories: [{repositoryId, repositoryPath, commit}]`, `provider`, `goals`
(approved goal versions), `productBrief`, and `stream: true`. To exercise
cancellation, kill the core process after the first `scan.progress` line, send
`reliability.record` with `kind: "operation_cancelled"` and
`operation: "scan.run"`, and confirm a fresh `workspace.open` still returns the
previous report.

Native tests: "end-to-end first-report journey proves repository selection
commit capture recovery and saved success", "analysis failure keeps goals safe
and exposes retry plus the last report", "the latest report renders
architecture findings and ranked actions", "recommendations create an editable
bundled prompt and copy through a private staged request".

## Gotchas

- Evidence must re-resolve: `path` must exist at the frozen commit and the
  line range must lie inside the blob, or
  `report_integrity::validate_report_for_persistence` rejects the whole report
  with a metadata-only error. Unsupported and unverified verdicts carry no
  evidence and must not invent any.
- The verdict vocabulary at the boundary is `supported`, `partial`,
  `unsupported`, and `unverified`; the desktop renders them with the labels
  listed in `docs/GETTING-STARTED.md`.
- A missing provider fails with error code `scan_failed`; provider stderr is
  never forwarded. Check `providers.detect` first.
- Streaming `scan.run` returns only a slim receipt; read the report back
  through `workspace.recent` or `workspace.open`.
- `agent submit-analysis` deletes the inbox payload on success and always
  records the fixed agent provider slug; there is no flag to override it.
- Reports cite coordinates only. A source excerpt anywhere in a response,
  report, export, or log is a defect; the `PRIVATE SOURCE CANARY` string in
  the harness fixture exists to catch exactly that.
