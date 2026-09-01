# Report integrity and exact-commit comparison

CodeCaddie treats a report as an immutable decision record, not a cache of
provider output. A report is accepted into the signed workspace ledger only
after the core independently proves that its metadata can still resolve to the
frozen repository state.

## Persistence gate

`report_integrity::validate_report_for_persistence` runs immediately before a
`ReportCompleted` event is signed and appended. It requires:

- a registered repository and a full 40- or 64-character commit object ID;
- the report's goal versions to equal the approved frozen goal set;
- the goal-set hash, per-goal verdicts, weighted coverage, and unverified count
  to recompute exactly from those frozen goals and criterion results;
- one assessment and one result for every approved success check;
- immutable evidence for every Supported or Partial check, architecture claim,
  and ranked recommendation;
- architecture and recommendation IDs to be unique, with every supplied goal
  link resolving to the frozen set and every ranked recommendation linked; and
- every evidence reference to name the report's repository and exact commit.

The local Git verifier then resolves every coordinate again and compares its
blob object ID and excerpt BLAKE3 hash. Missing commits, moved paths, invalid
line ranges, stale hashes, and cross-repository references fail before the
event log changes. The verifier returns metadata-only errors and never includes
source text in a report, IPC response, log, or diagnostic.

Unsupported checks may have no citation when the analyzer honestly found no
relevant implementation. Unsupported checks with contrary evidence still
carry that evidence. Unverified checks remain explicit and do not masquerade
as resolved claims.

The serialized coordinate allowlist is documented by
`protocol/persisted-report-evidence-v1.schema.json`.

## Repeat-analysis comparison

The history projection keeps the latest twelve reports but never rewrites the
signed ledger. For each logical goal and stable criterion ID, it now emits a
field-level comparison containing:

- previous and current verdict;
- previous and current immutable evidence arrays;
- a change category (`first`, `improved`, `declined`, `evidence_changed`, or
  `unchanged`); and
- a source-free explanation naming the number of references and their short
  frozen commits.

The goal-level summary counts improved, declined, evidence-changed, and
unchanged checks. Finding details show the comparison next to the current
criterion, while every prior report and evidence set remains independently
selectable in history.

## Verification

Rust tests create a real temporary Git repository and prove that valid evidence
survives the persistence gate while missing evidence and stale content hashes
are rejected. History tests retain twelve reports, compare stable criterion
IDs, preserve both proof sets, and map their absolute indexes correctly. Native
tests parse and render the field-level comparison in finding details.
