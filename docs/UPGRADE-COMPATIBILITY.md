# Upgrade compatibility

CodeCaddie supports upgrade and local transaction rollback from every public build listed in
[`config/supported-upgrade-matrix.json`](../config/supported-upgrade-matrix.json).
The matrix is intentionally explicit: adding or removing a supported prior
build is a reviewed product decision, not an inference from whichever fixture
happens to remain in a test directory.

A product version in this contract is the semantic version plus its monotonic
build number. Only the identities listed in the matrix are supported prior
versions. An older tag or build that is not listed is outside the supported
set; release history alone does not silently expand the compatibility promise.

The one-commit `0.4.0+2001` public snapshot is the only permitted empty matrix:
there is no earlier signed public build that can safely participate in the new
trust chain. The next public commit must change `firstPublicBaseline.status` to
`established` and add `0.4.0+2001` with the immutable root commit SHA. CI fails
closed if a pending baseline contains an entry or an established baseline does
not begin with that exact version and build.

The `codecaddie-updater` test named
`supported_prior_version_upgrade_and_rollback_matrix_preserves_real_encrypted_workspace_state`
runs the production application-replacement transaction once for every matrix
entry. Each journey creates a real local workspace and approved goal, saves a
report bound to a full Git commit and resolvable evidence coordinate, closes
and reopens the store after each transition, and proves that:

- a failed upgrade restores and reopens the exact prior application version;
- a healthy upgrade opens the existing encrypted state after restart;
- a local failed-install rollback reopens the same report, configuration,
  history, and immutable evidence;
- owner-only managed state never contains the recognizable privacy canary.

The test is part of the normal Rust gate. Release changes must extend the
matrix and its executable journey before declaring another prior build
supported. This contract covers recovery inside one failed installation; a
product rollback is always a newer fix-forward release from protected `main`.
