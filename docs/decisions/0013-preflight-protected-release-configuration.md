# 0013. Validate protected release configuration before waiting for builds

- Status: Accepted
- Date: 2026-09-06

## Context

The release pipeline can spend an hour waiting for exact-commit CI before the
archive importer discovers a missing Apple artifact-reader credential.
Credentials must remain within `release-apple`, and release verification must
continue to fail closed.

## Decision

Run a bounded, read-only configuration preflight in the protected
`release-apple` environment before the prepare job. Check required variable
formats and that the secret decodes to an unencrypted P-256 private key. Report
only fixed field names and remediation instructions, never values or crypto
parser errors. Keep the existing canonical repository identity check, protected
ref restriction, and exact-commit CI and archive gates.

Preflight is offline validation, not an authentication or release-readiness
claim. Publication continues to depend on the existing exact-commit archive,
notarization, manifest, attestation, immutable asset, and Latest checks.

## Consequences

Missing configuration fails quickly without a long build wait. One additional
read-only job accesses the artifact-reader credential, confined to the same
protected environment as the importer; no job receives extra signing or
publication authority. Valid format cannot establish API authorization or
prove that a workflow has an available notarized archive.

## Evidence

- `.github/workflows/release.yml`: `preflight` gates `prepare`.
- `scripts/check-release-configuration.mjs`: bounded, sanitized validation.
- `scripts/tests/release-configuration.test.mjs`: missing settings, malformed
  input, wrong curves, credential removal, and safe CLI failures.
- `scripts/tests/release-workflows.test.mjs`: protected environment and
  read-only authority assertions.
