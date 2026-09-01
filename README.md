# CodeCaddie

CodeCaddie is a local-first desktop application that evaluates whether a
software system is delivering its approved business goals. Its operating loop
is **Goals → Evidence → Action → Repeat**.

The application is MIT licensed. It has no hosted application tier, account
service, billing service, or model API integration.

## Trust boundary

CodeCaddie's storage, reports, and IPC never contain repository source text —
reports cite immutable coordinates (paths, line ranges, hashes) from the
scanned commit, never excerpts. A selected, already-installed Claude, Codex,
or Grok CLI may process a disposable single-commit snapshot of the repository
under that provider's own authorization, settings, privacy terms, and
organizational policy. CodeCaddie never accepts or stores provider
credentials. Everything it records stays in one data directory on the device.
Workspace state and signed event history are authenticated and encrypted with
an owner-only local content-key file inside the same data root; CodeCaddie does
not call Keychain, Credential Manager, or Secret Service, and the key is never
sent to an AI provider.

If the user explicitly attaches a PDF, PPTX, DOCX, TXT, or Markdown product
document, that selection authorizes bounded extracted text to be sent to the
selected provider during goal generation. CodeCaddie stores only the local
path and metadata/hash reference—not extracted contents—and never fetches the
optional website field.

## Platform support

- **Tier 1** — macOS Apple Silicon and macOS Intel use one signed, notarized
  universal download from
  [GitHub Releases](https://github.com/tailored-ai-solutions/codecaddie/releases).
- **Windows** — coming soon. Source builds are available for contributors, but
  public Windows downloads wait for open-source code-signing approval.
- **Linux** — builds from source and the storage layer defines Linux paths,
  but the desktop app is **unsupported and experimental** until the updater,
  launch-at-login, packaging, and desktop CI exist for Linux.

See [Platforms](docs/PLATFORMS.md) for the full support statement, per-OS
data locations, and local storage behavior.

## Quickstart

New users: follow [Getting started](docs/GETTING-STARTED.md) — install,
attach a repository, add product notes or documents, generate and approve goals,
run the first analysis, and read the report. Provider analysis commonly takes
several minutes and varies with repository size and the installed provider.

Contributors need Git, Node.js 24, pnpm 11.22.0, and Rust 1.95.0 (pinned by
`rust-toolchain.toml`).
Native SDK 0.10.1 downloads its pinned Zig toolchain on first use.

```sh
pnpm install --frozen-lockfile
cargo build --workspace --locked
pnpm dev
```

The full local gate is:

```sh
pnpm check
pnpm build
```

Install a release-optimized, visibly labeled developer edition beside any
stable installation with `pnpm install:local`. See
[Development](docs/DEVELOPMENT.md) for the complete workflow, isolated data
directories, and per-platform install destinations.

## Architecture

- `apps/desktop` is a Native SDK 0.10.1 application written in Zig and native
  markup. It has no browser runtime or WebView.
- `crates/codecaddie-core` is the bundled Rust process. It owns disposable Git
  single-commit source snapshots, installed-provider execution, evidence validation,
  local persistence, schedules, and Word report export.
- `crates/codecaddie-domain` contains immutable domain events, deterministic
  projections, role enforcement, goal history, action lifecycle, and scoring.
- `protocol` defines the bounded length-prefixed JSON contract between Zig and
  Rust. Provider progress uses sanitized phase messages and repository-relative
  file counters; stdout is reserved for bounded protocol frames.

The desktop binary and Rust core are packaged beside each other. Local
repository attachments are device-specific and never leave the device as
absolute paths.

## Where goals and analysis results are stored

All workspace state lives on the device as authenticated encrypted JSON and
JSONL envelopes in one data directory:
`CODECADDIE_DATA_DIR` when set, otherwise the per-user platform default
(for example `~/Library/Application Support/CodeCaddie` on macOS). Files are
created with owner-only permissions where the operating system supports them.
The 256-bit content key lives in an owner-only file inside that data directory.
Existing plaintext active state is atomically migrated by a locked one-time startup
sweep, including unopened workspace logs, maps, pointers, preferences, and
agent sessions; normal readers validate decrypted schemas and signatures
before use. A missing, malformed, or mismatched key leaves ciphertext untouched
and fails closed. This protects individual managed files from casual disclosure;
it does not protect against a process that can read the current user's complete
CodeCaddie data directory.
Per-OS locations and the directory layout are in
[Platforms](docs/PLATFORMS.md); backups and machine moves are covered in
[Backup and portability](docs/BACKUP-AND-PORTABILITY.md).

## Releases and updates

`package.json` is the canonical semantic version source. Every protected-main
build receives a unique `vX.Y.Z+N` release identity, and GitHub Releases holds
immutable signed installers, checksums, SBOM, provenance, and the signed release
manifest. The core verifies signed update manifests and fails safely: an
offline, malformed, tampered, wrong-publisher, wrong-architecture, or downgrade
update is rejected without blocking normal use. Download and installation
always require explicit user
actions. Release mechanics live in [Releasing](docs/RELEASING.md).
[`codecaddie.ai`](https://codecaddie.ai) is the branded documentation and
download front door.

## Documentation

- [Getting started](docs/GETTING-STARTED.md) — first install to first report.
- [Platforms](docs/PLATFORMS.md) — support tiers, data locations, local storage.
- [Troubleshooting](docs/TROUBLESHOOTING.md) — providers, local state,
  locks, recovery.
- [Backup and portability](docs/BACKUP-AND-PORTABILITY.md) — recovery
  exports and machine moves.
- [Local product measurement](docs/LOCAL-PRODUCT-MEASUREMENT.md) — the
  metadata-only first-report, repeat-review, and decision-cycle contracts.
- [Operational assurance](docs/OPERATIONAL-ASSURANCE.md) — support, SLO,
  source-safe failure, fault-injection, and release-publication evidence.
- [Local data governance](docs/DATA-GOVERNANCE.md) — consent, retention,
  deletion, minimization, exception, and local audit controls.
- [Support matrix](docs/SUPPORT-MATRIX.md) — the exact desktop environments
  each release supports.
- [Upgrade compatibility](docs/UPGRADE-COMPATIBILITY.md) — the supported
  prior-build matrix and upgrade rollback.
- [Evidence and comparisons](docs/EVIDENCE-AND-COMPARISONS.md) and
  [Report integrity](docs/REPORT-INTEGRITY.md) — immutable evidence,
  exact-commit comparison, and the report acceptance gate.
- [Local reliability](docs/LOCAL-RELIABILITY.md),
  [Runtime health](docs/RUNTIME-HEALTH.md), and
  [Reliability and performance](docs/RELIABILITY-AND-PERFORMANCE.md) — on-device
  reliability measurement and the release performance contract.
- [Disaster recovery](docs/DISASTER-RECOVERY.md),
  [Incident response](docs/INCIDENT-RESPONSE.md), and the
  [incident index](docs/incidents/README.md).
- [Reproducible builds](docs/REPRODUCIBLE-BUILDS.md) — the double-build gate
  that separates compilation from signing.
- [Architecture](docs/ARCHITECTURE.md), [Module map](docs/MODULE-MAP.md),
  [Decision records](docs/decisions/README.md),
  [Security model](docs/SECURITY_MODEL.md), [Development](docs/DEVELOPMENT.md),
  [Releasing](docs/RELEASING.md), [Design QA](docs/DESIGN-QA.md), and
  [Brand](docs/BRAND.md).
- [LICENSE](LICENSE), [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), and
  [protocol/README.md](protocol/README.md).

## Project

- [Contributing](CONTRIBUTING.md) — how to build, test, sign off, and submit
  changes.
- [Governance](GOVERNANCE.md) — who decides, and how protected `main` works.
- [Code of conduct](CODE_OF_CONDUCT.md).
- [Support](SUPPORT.md) — where to ask questions and report bugs.
- [Security policy](SECURITY.md) — supported versions and private
  vulnerability reporting.
- [Trademarks](TRADEMARKS.md).
- [Changelog](CHANGELOG.md).
