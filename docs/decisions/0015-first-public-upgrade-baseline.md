# 0015. Support upgrades from the first public release

- Status: Accepted
- Date: 2026-09-07

## Context

The supported-upgrade matrix was pending because no signed public build existed.
Release `0.4.0+2015` establishes that boundary at immutable source commit
`2ee34e1515b76da58839c935ec1a29cb4c000df1`. The unpublished snapshot build
`0.4.0+2001` remains outside the supported set.

## Decision

Mark the first public baseline established and list `0.4.0+2015` with its source
commit and `codecaddie-local-state-v3` format. Build both the candidate and prior
source archives for the compatibility journey. Derive the candidate's build
with the canonical release-build helper so the public release epoch is retained.

## Consequences

The previously empty matrix now runs encrypted-state upgrade and rollback
checks for a real public baseline. Adding another supported prior build remains
an explicit reviewed change. Source-built compatibility testing complements
signed-download verification and does not replace it.

## Evidence

- `config/supported-upgrade-matrix.json` records the supported identity.
- `scripts/exercise-supported-prior-binaries.mjs` runs both real binaries.
- `scripts/tests/supported-prior-binaries.test.mjs` checks the established
  baseline and the build passed to Cargo.
- `crates/codecaddie-core/src/bin/codecaddie-updater.rs` executes
  `supported_prior_version_upgrade_and_rollback_matrix_preserves_real_encrypted_workspace_state`.
