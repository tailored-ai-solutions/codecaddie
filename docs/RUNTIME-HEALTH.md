# Local runtime health measurement

CodeCaddie measures native client crashes and failures at its repository-owned
provider boundary without an external telemetry service. All records live in
the existing signed workspace ledger under the normal CodeCaddie data root.
The application does not use Keychain, Credential Manager, or Secret Service
for this data or its owner-only local content key.

## Native client crashes

The Native SDK writes its fixed panic marker when the desktop process panics.
On the next workspace session start, the core atomically claims that marker,
records a `desktop_crash_detected` event and local SLO alert, then deletes the
claimed marker. CodeCaddie never reads or stores the marker body. An unmatched
session start is not treated as a crash.

`desktopCrashesDetected` and `crashFreeSessionsPercent` are derived from signed
events and shown in the installed report. The adversarial test writes a source
canary into the marker and proves that only the content-free crash fact enters
the ledger and the marker is removed.

## Provider-boundary errors

Every framed core request finishes through one repository-owned finalization
path. Provider-backed operations are the closed CodeCaddie-selected set
`scan.run`, `goals.generate`, `map.generate`, and `provider.*`. Success,
failure, cancellation, latency, retryability, a stable correlation ID, and an
allowlisted error code are recorded locally. Provider failures are categorized
separately and projected as:

- `providerOperationSamples`;
- `providerOperationFailures`; and
- `providerAlertsRaised`.

The installed Local reliability card displays those provider-boundary totals
beside native crash counts. A failed response also receives its correlation ID
and whether local measurement succeeded, so the UI can give safe recovery
guidance without echoing provider output.

The same signed `operation_completed` record is also the local trace span. The
trace uses the request correlation ID as its identity and admits only the
operation code, outcome, timing, retryability, and categorized error fields.
This single-record representation remains readable by supported prior binaries;
current builds also recognize historical `trace_span_completed` records and do
not double-count the paired operation. The installed reliability card reports
the signed trace count; CodeCaddie never creates free-form span attributes.

## Privacy and verification

The contract is versioned in `config/runtime-health-measurement.json`. Records
contain code-owned identifiers, timing, version, and platform only. They never
contain repository source, attachment contents, provider output, prompts, goal
text, credentials, personal identifiers, or free-form errors, and nothing is
transmitted externally.

The governance contract enumerates disallowed names, contact details,
addresses and location, government and financial identifiers, authentication
secrets, biometric, health and demographic attributes, and all free-form user
or provider content. The adversarial trace surface runs through the same
source-canary gate as logs, metrics, alerts, and crash reports.

Executable proof lives in the Rust service and workspace-store adversarial
tests, the source-canary matrix, the native report test, and the dedicated
privacy CI gate. The repository assurance index is routing metadata only; an
analysis must still inspect and cite these actual artifacts at the frozen
commit.
