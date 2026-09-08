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

The first public baseline is established at
[`0.4.0+2015`](https://github.com/tailored-ai-solutions/codecaddie/releases/tag/v0.4.0+2015),
built from immutable source commit
`2ee34e1515b76da58839c935ec1a29cb4c000df1`. It uses
`codecaddie-local-state-v3` and is the first supported prior build in the matrix.
The `0.4.0+2001` snapshot was never published and is outside that set.
CI fails closed if the established matrix is empty or does not begin with the
baseline's exact version and build.

The supported-prior binary harness builds that source commit and the candidate
commit from exact Git archives, using canonical release build numbers. It then
opens state written by each binary with the other binary and checks report
history, provider settings, immutable evidence, and source privacy after restart.
This source-build compatibility check complements verification of the signed
public download; it does not replace release-signature or notarization checks.

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
