# Report history and Word export

## Sub-features

- History: the latest twelve saved analyses per goal, four visible at a time
  with **Earlier** / **Later**; older pages load lazily.
- Per-run comparison: each later report carries the prior and current verdict
  and evidence for every stable criterion, with a change kind (improved,
  declined, evidence changed, unchanged).
- Deleting a saved analysis (never the latest) with confirmation and undo.
- Word export of the latest report to the Downloads folder; metadata and
  coordinates only.
- Portable backup and recovery export of the whole workspace
  (`workspace.backup.*`, `workspace.recovery.export`), harness-only in this
  route.

## How to get to it (user POV)

1. Open the **Report** screen; **Analysis summary** shows "Goal progress by
   analysis run". Use **Earlier** and **Later** to move through runs; the
   **Status legend** explains Missing, Broken, Incomplete, Functional, Strong,
   and N/A.
2. Each run column has a delete control whose label names the run; it opens
   **Remove saved analysis** with **Delete** and **Keep run**. **Undo delete**
   appears briefly afterwards.
3. Click **Download Word report**; the button reads "Preparing Word report..."
   and then **Show in folder** reveals the file in Downloads.

## Driving it with the harness

- `reports.history.list` (scoped): optional `beforeEventId`, `limit` (1 to
  100); returns lightweight `runs`, `totalActiveRuns`, `hasOlder`, and
  `nextBefore`, never criteria or evidence.
- `reports.finding.get` (scoped): `reportEventId`, `goalVersionId`; one full
  finding with bounded criteria and evidence coordinates.
- `reports.delete` (scoped): `reportEventId`; returns `deleted: true` and
  rejects the latest run.
- `reports.export_word` (scoped): `destination` (absolute path); returns
  `format: "docx"`. Through the agent CLI:
  `agent export --kind word --out <exchange.outbox>/report.docx`.
- `workspace.backup.export` / `workspace.backup.import`: `destination` or
  `source`, `repositoryPath`, `passphrase` (12 to 1024 bytes); results carry
  `eventCount` and `manifestBlake3`.
- Native tests: "analysis history virtualizes 28 runs across eight pinned goals
  and lazy-loads older pages", "older history pages prepend without moving the
  visible run", "run deletion confirms an exact historical event and protects
  the latest", "Word report download writes a named file to Downloads with
  bounded core export".

```sh
"$CORE" agent export --kind word --out "$EVIDENCE/export.docx"
head -c 2 "$EVIDENCE/export.docx"                          # PK
grep -c "PRIVATE SOURCE CANARY" "$EVIDENCE/export.docx"   # 0
```

## Gotchas

- History never deletes ledger events; `reports.delete` records a deletion and
  the projection hides the run. The latest run is protected so a workspace
  never loses its current report.
- A history cell is N/A when the goal did not exist at that run; that is data,
  not a rendering bug.
- Word export accepts only a report that already passed the persistence
  boundary, so it cannot smuggle unvalidated claims out.
- Export and backup destinations are absolute paths chosen by the caller; the
  core never writes into the repository checkout.
- The backup passphrase is memory-only; `workspace.backup.schedule.status`
  returns a sanitized schedule and never the passphrase.
