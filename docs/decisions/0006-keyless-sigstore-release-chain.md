# 0006. Keyless Sigstore release and update chain

- Status: Accepted
- Date: 2026-09-01

## Context

The updater downloads code that runs with the user's privileges. A public
repository cannot hold a manifest signing key, and no runner credential store
should hold one either. Apple code signing needs a Developer ID that must not
be exportable from CI.

## Decision

GitHub Actions signs the exact bytes of the release manifest with a
short-lived GitHub OIDC identity through Sigstore, producing a bundle beside
it. The core's `update::verify_release` verifies the Fulcio certificate
chain, the Rekor inclusion proof, the TUF-refreshed trust roots, and the
identity policy pinned in `config/release-trust.json` and baked in at build
time (repository, numeric repository id, `refs/heads/main`, the release
workflow, the OIDC issuer, and the source commit). Only then does it check
artifact size and SHA-256, semantic-version and build monotonicity, HTTPS
URL, and platform. The external updater in `bin/codecaddie-updater.rs`
re-checks the Apple team, bundle identifier, version, and build before atomic
replacement. Xcode Cloud retains the non-exportable Developer ID and returns
a notarized archive; GitHub never signs the application.

## Consequences

No `cosign` binary runs on the user's machine and no private key exists in
the pipeline. The repository id is part of the trust policy, so re-rooting
the repository (0008) requires a new id in `release-trust.json` before any
release; a new channel extends the policy and its tests, never adds a key.

## Evidence

- `crates/codecaddie-core/src/update.rs`: `verify_release`, `SigstoreIdentityPolicy`, `ReleaseManifestV2`.
- `crates/codecaddie-core/src/bin/codecaddie-updater.rs`; `crates/codecaddie-core/src/testdata/public-github-actions.sigstore.json` (interoperability fixture); `config/release-trust.json`; `scripts/verify-release-manifest.mjs`; `scripts/tests/release-manifest.test.mjs`.
- `docs/RELEASING.md` "Keyless manifest identity"; `docs/ARCHITECTURE.md` "Release and update chain".
