# Local product measurement

CodeCaddie measures its decision funnel without an analytics service. Version 1 lifecycle records are signed into the existing workspace event ledger, are read only from the device, and are reduced to aggregate counts and elapsed times by `workspace.recent`.

## Event contract

`protocol/local-product-events-v2.schema.json` is the current serialization
allowlist. Every new record has a stable opaque workspace ID, an opaque session
ID, product version, operating-system/architecture platform, and release cohort.
The workspace ID must match the enclosing signed event before projection; the
signed envelope supplies the time. Analysis events use the immutable report ID
as their shared session and report identity. Schema-1 records remain readable
for signed-history compatibility but no new schema-1 records are emitted.

The lifecycle is `workspace_created`, `goal_approved`, `analysis_started`, `scorecard_generated`, `report_saved`, and `time_to_first_saved_report`. Repeat review adds `repeat_analysis_started`, `comparison_generated`, `report_revisited`, and `evidence_opened`; recommendation use adds `prompt_copied` without prompt content.

Repository paths and source, attachment contents, goal text, prompts, free text, personal identifiers, and network destinations are forbidden. Tests seed privacy sentinels and inspect the serialized signed records. There is no network transmitter or second analytics store.

The Rust event records also reject unknown serialized fields. The JSON schemas
and production deserializers therefore fail together when a new field is added:
the field cannot enter signed history until the governance contract, privacy
matrix, and executable tests are deliberately updated.

## Metrics

`config/product-metrics.json` is the executable metric definition and `protocol/first-report-activation-v1.schema.json` freezes its first-report calculation. The denominator is one unique signed workspace ledger with `workspace_created`. The numerator is that workspace's first `time_to_first_saved_report` event when `elapsedMilliseconds` is at most 600,000; a missing event is a miss. The result is grouped by product version, the platform family derived from the event's platform prefix, and release cohort. Version 1 keeps separate macOS and Windows groups and a success rate threshold of at least 80 percent. Public release gating currently applies only to the macOS groups; Windows remains a source-built preview cohort until public signing is available.

`scripts/first-report-metric.mjs` executes that contract over metadata-only observations. Its checked fixture proves the inclusive ten-minute boundary, missing-result behavior, unique-workspace denominator, duplicate rejection, macOS and Windows segmentation, and the exact 80-percent threshold. Repeat review requires a repeat-analysis start followed by a generated comparison or revisited report. Decision-cycle time runs from the latest approved goal set to the next saved report.

The product owner also reviews `validated-criterion-evidence-rate` after every
saved report and monthly. Its denominator is every current success check; the
numerator requires a validated verdict plus at least one commit-resolved
evidence anchor. Unverified checks remain in the denominator and never enter
the numerator. Any rate below 100 percent triggers an evidence investigation
before the scorecard is used for a decision.

## First-report journey matrix

`config/first-report-journey-v1.json` binds the journey contract to one native end-to-end test. That test crosses all six states in order: report empty state, invalid goals, cancellation, provider failure, retry, and saved success. It asserts that editable goals survive cancellation and failure, then reopens the saved report with history and an evidence-grounded recommendation. `pnpm native:check` and the full `pnpm check` gate execute the matrix.

The app displays only local aggregates. An event remains for the workspace lifetime and is removed when that workspace is deleted. Recovery material retains the signed ledger under the same local-only privacy boundary; normal report and diagnostic exports contain aggregate measurements only. CodeCaddie does not request a review rating or transmit this data.
