# Immutable evidence and repeat-analysis comparisons

CodeCaddie keeps every saved report in the existing signed workspace event
history. A later analysis never updates or replaces an earlier report. Each
report retains its own repository identifier, full analyzed commit, goal-set
hash, criterion verdicts, and immutable evidence coordinates.

## Save and export boundary

Before a report is signed, `report_integrity` validates all three kinds of
repository claim:

- every Supported or Partial success-check assessment;
- every architecture claim; and
- every ranked recommendation.

Each must name a registered repository, the report's full frozen commit, a Git
blob, line range, and content hash. The verifier re-resolves the commit and
evidence from local Git. Missing evidence, wrong commits, stale hashes,
unregistered repositories, incomplete goal coverage, duplicate action ranks,
or recomputed score mismatches fail closed. Unsupported and Unverified checks
remain explicit rather than receiving invented proof. Word export accepts only
a report that already passed this persistence boundary.

The table-driven persistence test removes and corrupts evidence independently
for success checks, architecture claims, and recommendations, then proves each
variant is rejected.

## Checkout-switch boundary

The saved-report checkout test persists a report, changes both the repository
branch and working-tree content, restarts the local store, and proves that the
displayed commit and evidence coordinates still identify the original blob.
It then reopens that original line through local Git rather than the changed
checkout. `pnpm evidence:check` executes the fully qualified test with an exact
filter, and the Ship readiness assurance job runs the same command before a
release can pass.

## Comparison boundary

The history projection orders signed reports by completion time and retains the
latest twelve without deleting prior ledger events. Each projected analysis
includes its report ID and the complete `repository @ full-commit` identity.
For every stable criterion ID, the next report carries:

- the prior verdict and current verdict;
- prior immutable evidence and current immutable evidence;
- a change kind of improved, declined, evidence changed, or unchanged; and
- a metadata-only summary naming the abbreviated commits on both sides.

The repeat-history test creates thirteen reports for one workspace at thirteen
different commits. It proves the latest twelve are projected, both compared
reports retain different full commit identities, previous and current evidence
remain separate, and the original thirteen saved reports are not overwritten.
Native tests also verify that field-level changes, absolute report indexes, and
historical N/A cells render correctly.

This document and `.codecaddie/assurance.json` are routing metadata. They do not
establish a verdict by themselves; an analysis must inspect and cite the
implementation and tests at the frozen commit.
