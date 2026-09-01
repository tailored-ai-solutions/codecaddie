# 0003. Evidence is immutable Git coordinates, validated fail-closed

- Status: Accepted
- Date: 2026-09-01

## Context

Providers can assert anything. A report is only worth sharing if every claim
can be checked later, after the working tree has moved on, without trusting
the provider or the report author. Reports must also be comparable across
runs at different commits.

## Decision

Every supported or partial criterion assessment, every architecture claim,
and every ranked recommendation must carry an `EvidenceRef`: repository id,
full frozen commit, blob id, repository-relative path, line range, and
excerpt hash. `validate_report_for_persistence` re-resolves each reference
against the local Git object database and fails closed on a missing path,
wrong commit, stale hash, unregistered repository, incomplete goal coverage,
duplicate action rank, or score mismatch. Unsupported and unverified verdicts
carry no evidence and never receive invented proof. History projections keep
prior and current evidence side by side so comparisons never overwrite the
earlier proof set.

## Consequences

The persistence boundary is the single place where trust is established, so
Word export and the agent-submission path reuse it instead of validating
again. Reports remain valid after branch switches; the checkout test proves
it. Adding a claim type means adding it to the fail-closed table-driven test.

## Evidence

- `crates/codecaddie-domain/src/model.rs`: `EvidenceRef`, `EvidenceKind`, `Verdict`, `CriterionAssessment`, `ArchitectureClaim`, `Recommendation`.
- `crates/codecaddie-core/src/report_integrity.rs`: `validate_report_for_persistence`.
- `protocol/persisted-report-evidence-v1.schema.json`.
- `scripts/exercise-saved-evidence-checkout.mjs` (`pnpm evidence:check`); `docs/EVIDENCE-AND-COMPARISONS.md`; `docs/REPORT-INTEGRITY.md`.
