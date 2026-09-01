# Local reliability and error reporting

CodeCaddie measures desktop and bundled-core reliability without sending data
off the device. Reliability records are signed events in the existing
workspace ledger. There is no remote telemetry endpoint and no second local
database.

## Contract

`protocol/local-reliability-event-v1.schema.json` is the serialization
allowlist. A record may contain only:

- a generated correlation ID;
- a core-owned operation, category, error, or alert identifier;
- success, failure, or cancellation state and retryability;
- elapsed milliseconds, product version, platform, and an opaque session ID.

Each signed `operation_completed` record is also its local trace span. Its
correlation ID is the trace identity, its core-owned operation is the span
name, and it reuses the same closed metadata allowlist. This avoids a second
event shape that a supported prior binary could not read. Current builds still
recognize historical `trace_span_completed` records without double-counting
their paired operation. There are no arbitrary span attributes or
source-bearing trace payloads.

Repository paths and source, provider output, prompts, attachment contents,
goal text, free-form error messages, usernames, and secrets are forbidden.
IPC errors are rendered from their core-owned error code rather than arbitrary
provider or repository text. The signed reliability record stores only the
categorized error code.

## Measurement and alerts

`config/service-level-objectives.json` names the owner, review cadence,
availability and latency objectives, alert thresholds, and the crash-free
desktop-session target. A failed covered operation or a latency breach appends
a source-free local SLO alert next to the operation record. The report screen
derives availability, average core latency, crash-free sessions, failures,
cancellations, trace-span counts, and alert counts from signed events.

The same file names customer-journey objectives for first report creation,
report persistence, provider execution, and crash-free desktop sessions. Each
journey has an owner, alert code, and explicit local events used to measure it.
The full artifact and fault-injection map is in
[Operational assurance](OPERATIONAL-ASSURANCE.md).

The Native SDK writes `last-panic.txt` before terminating on an uncaught native
panic. At the next desktop session start, the core checks only the fixed,
channel-owned marker path, atomically claims the file without reading its
contents, appends a content-free `native_panic_detected` event and alert, and
then deletes the marker. Missing session-end events are retained as lifecycle
data but are never inferred to be crashes, so a normal quit cannot lower the
crash-free rate. A claimed marker remains pending if ledger persistence fails
and is retried on the next start.

## Failure behavior

Core failures include a correlation ID and whether the local reliability write
succeeded. Desktop error states preserve the safe error message, add recovery
guidance for repository, provider, storage or migration, and export failures,
and display the correlation reference. If the reliability ledger itself is
unavailable, the customer operation keeps its original result and the IPC
response reports `local_reliability_unavailable`; CodeCaddie never substitutes
or uploads a fallback payload.
