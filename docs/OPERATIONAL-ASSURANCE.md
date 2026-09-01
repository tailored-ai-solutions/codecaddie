# Operational assurance

CodeCaddie turns local reliability behavior into repository-verifiable
controls. The controls are not claims about production adoption or customer
satisfaction. They are executable contracts for supported environments,
service objectives, source-safe failures, fault injection, and release
recovery.

## Evidence discovery

Any repository may add `.codecaddie/assurance.json` as an optional routing
index. CodeCaddie validates its size, identifiers, relative paths, file types,
and snapshot confinement before placing a metadata-only digest in the analysis
prompt. The index cannot support a criterion by itself: the provider must read
the referenced artifact in the immutable checkout and cite that artifact's
coordinates. Missing, invalid, escaping, or oversized indexes are ignored.

The generic format is versioned in
`protocol/repository-assurance-index-v1.schema.json`. It contains only control
IDs, topics, and repository-relative artifact paths—never goal text, desired
verdicts, source excerpts, or product-specific analyzer exceptions.

## Service objectives and local traces

`config/service-level-objectives.json` defines owners, review cadence,
availability, latency, alert thresholds, and measurement events for the first
saved report, report persistence, provider execution, covered core requests,
and crash-free desktop sessions.

Each covered core request emits a signed, content-free operation record. Its
generated correlation ID is copied into the associated local SLO alert, making
the pair a local trace. The report projection turns these signed records into
availability, latency, failure, cancellation, crash-free, and alert metrics.
The Native SDK panic marker is the authoritative crash record. Nothing is sent
to an endpoint, and the schema cannot serialize source, prompts, goal text,
attachments, or free-form messages.

## Actionable failure states

Repository, provider, storage, migration, and export failures are rendered
from core-owned error codes. The desktop shows a safe category summary,
retry/recovery guidance, and the local correlation reference. It never renders
provider or repository text as the primary error. Full analysis diagnostics
may be inspected in the local activity view, while the saved report and goals
remain unchanged after failure.

## Fault injection and release control

`config/operational-fault-matrix.json` binds provider timeout, malformed
provider output, disk exhaustion, interrupted writes, local telemetry outage,
and paused-release restart to production surfaces, executable test names,
expected metrics, and expected alerts. Tests verify both the customer result
and the signed local records, including the shared correlation ID and the
absence of private content. The focused
`operational_fault_matrix_proves_metrics_errors_and_alerts_end_to_end` journey
drives the real provider timeout and malformed-result parser, persistence sync
boundaries, and local-ledger outage before reopening the signed aggregates.

Release incident containment and fix-forward recovery are governed by
`docs/INCIDENT-RESPONSE.md`. Published releases are immutable and `latest`
never moves to a lower version/build. The updater journey proves that the
current verified version keeps opening its real encrypted state and that a
failed local replacement restores the prior installed application without
losing customer state. A product rollback is a reviewed `main` commit that
publishes a newer build containing the restored source.
