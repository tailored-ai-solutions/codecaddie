# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A short code of conduct, an `.editorconfig`, and repository-wide line-ending
  and language attributes for contributors.
- Decision records under `docs/decisions/`, starting with the codebase-map and
  goal-analysis pipeline record, and a module map linked from the
  architecture document.
- A worked example goal for AI-assisted engineering (agent contract, runtime
  verification, shared CI gates, decision log) in the demo goal set.
- Documentation for every protocol schema, including the current
  `local-product-events-v2` shape, the portable backup envelope, the provider
  execution contract, and the repository assurance index.

### Changed

- The cryptography and utility crates moved to their current majors (argon2
  0.6, base64 0.23, chacha20poly1305 0.11, ed25519-dalek 3, rand 0.10,
  x509-cert 0.3) with byte-identical outputs across the upgrade. An
  unavailable operating-system random generator is now reported as an error
  instead of aborting the core; see decision record 0012.
- The demo fixture is now `testdata/acme-demo`: nine goals for Acme, a
  document-search service whose shape matches the bundled demo repository,
  replacing the previous example company.
- The getting-started guide, README, support, security, and governance pages
  now link every project document, name the pinned Rust toolchain, point usage
  questions at GitHub Discussions, and describe the squash-only, linear,
  DCO-signed history of protected `main`.
- The release runbook spells out the order in which repository controls are
  applied, lists all eleven required status checks, and documents how the
  release channel is derived from the package version.
- The plugin documentation reflects the current platform status (macOS today;
  Windows from source) and gives concrete install commands.
- Protocol schema identifiers are all namespaced under `https://codecaddie.ai/protocol/`.
- Third-party notices name the vendored rubric locations and the `plist` crate.

### Removed

- The standalone goal-analysis design document; its history now lives in the
  decision record and its current-state summary in the architecture document.
- Brand-lineage notes and a reference to a website repository that is not part
  of this project.

## [0.4.0] - 2026-08-30

Initial open-source release. The public repository starts at build `0.4.0+2001`,
the snapshot commit. The first published desktop build is the first protected-
`main` commit archived and notarized through Xcode Cloud after the Apple signing
chain was connected; `config/supported-upgrade-matrix.json` records its exact
build once the baseline is established.

### Added

- Local-first native CodeCaddie for Apple Silicon and Intel Macs, distributed as
  one signed and notarized universal application ZIP.
- The Goals → Evidence → Action → Repeat workflow, with encrypted local state,
  commit-bound evidence, report history, recommendations, and Word export.
- Bounded integrations with locally installed Codex, Claude, and Grok command-line
  tools; CodeCaddie stores no provider credentials or repository source text.
- Startup and six-hour update checks with explicit “Not now” and
  “Update and restart” choices.
- Immutable GitHub Releases containing checksums, an SBOM, provenance,
  attestations, and a source-commit-bound update manifest.

### Security

- Update manifests are signed keylessly through GitHub OIDC and Sigstore.
- The updater verifies Fulcio identity, Rekor inclusion, source commit, artifact
  digest, semantic version, build number, Apple team, and bundle identifier
  before atomic replacement.
- Signing credentials remain outside the repository. Apple retains the
  non-exportable Developer ID key.

### Platform status

- macOS Apple Silicon and Intel are supported.
- Windows is coming soon, pending SignPath Foundation open-source signing.
- Linux remains an unsupported source-built experiment.

[Unreleased]: https://github.com/tailored-ai-solutions/codecaddie/compare/066b8cd223034e41fd587035ac71479caeb7c76c...HEAD
[0.4.0]: https://github.com/tailored-ai-solutions/codecaddie/releases
