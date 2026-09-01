# Platform support

## Support tiers

| Tier | Platform | What ships |
|---|---|---|
| Tier 1 | macOS Apple Silicon (arm64) | Xcode Cloud-signed and notarized universal-app ZIP, release CI, and physical acceptance testing before publication |
| Tier 1 | macOS Intel (x64) | The same Xcode Cloud-signed and notarized universal-app ZIP, release CI, and physical acceptance testing before publication |
| Coming soon | Windows x64 | Source-built developer preview only until SignPath Foundation approves open-source signing; no public installer or update artifact |
| Experimental | Linux | Source builds only. No packaged desktop app. |

Tier 1 platforms receive signed installers on
[GitHub Releases](https://github.com/tailored-ai-solutions/codecaddie/releases),
plus credential-free public updater payloads whose manifests use keyless
Sigstore signing tied to the protected GitHub release workflow. The app never
embeds a GitHub, Apple, or signing credential. See the update flow described in
[RELEASING.md](RELEASING.md).

## Linux status

The Rust workspace builds and its tests run on Linux (repository CI runs the
Rust job on Ubuntu), and the storage layer defines Linux paths. The desktop
application on Linux is **unsupported and experimental** until all of the
following exist, none of which ship today:

- **Updater.** `updates.check` resolves the platform to `macos` or `windows`
  and reports anything else as unsupported.
- **Launch at login.** `settings.launchAtLogin.set` fails with
  "Launch at Login is supported only on macOS and Windows".
- **Packaging.** There are no Linux packaging or local-install scripts;
  `scripts/` covers macOS and Windows only.
- **Desktop CI and release artifacts.** CI builds and tests the desktop app on
  macOS and Windows runners. The release workflow currently distributes only
  the signed universal macOS artifact.

On Linux, expect to build from source per [DEVELOPMENT.md](DEVELOPMENT.md),
run development-channel builds, and set the environment variables described
under "Headless and CI use" below. Folder pickers are unavailable; enter
absolute repository paths directly.

## Data locations

All workspace state lives on the device in one data directory. The location
is resolved in this order:

1. `CODECADDIE_DATA_DIR`, when set, is used verbatim. This is the single
   configuration knob for relocating storage (tests, portable setups,
   alternate profiles).
2. Otherwise the per-user platform default for the running channel:

| Platform | Stable | Developer edition |
|---|---|---|
| macOS | `~/Library/Application Support/CodeCaddie` | `~/Library/Application Support/CodeCaddie Dev` |
| Windows | `%APPDATA%\CodeCaddie` | `%APPDATA%\CodeCaddie Dev` |
| Linux (experimental) | `$XDG_DATA_HOME/codecaddie` | `$XDG_DATA_HOME/codecaddie-dev` |

The Linux row applies to experimental source builds only. On Linux, an
unset, empty, or relative `$XDG_DATA_HOME` falls back to `~/.local/share`
per the XDG Base Directory Specification, so storage works with only `HOME`
set and no extra environment variables.

Inside the data directory:

- `events-v2/` holds the authenticated encrypted append-only JSONL event log per workspace —
  approved goals, analysis reports, action history, content-free decision
  funnel markers, and any historical outcome-rating events from older builds.
  Current builds replace rating collection with editable recommendation-fix
  prompts and create no new rating events. This is the authoritative store for
  goals and analysis results. Each committed line encrypts one signed event envelope.
- `local-state-v2.json` holds the local device identity and device-specific
  workspace context such as the repository path, product brief, and structured
  attachment references (canonical path, display name, type, size, page/slide/
  section count, and BLAKE3 hash). The filename remains stable, while current
  writes use the `codecaddie-local-state-v3` format marker. A v2 file migrates
  atomically on the next context write; older builds reject v3 rather than
  erasing attachment references. Raw extracted document text is never stored.
- User-selected portable backup files live outside this directory. They are
  independently encrypted with an Argon2id passphrase key and never rely on
  the data-root content key or an operating-system credential manager.
- `backup-schedules-v1/` holds encrypted schedule configuration. The installed
  app checks for a due backup after workspace resume; the stored passphrase is
  never returned or logged, and no operating-system credential manager is used.
- `recent-workspace-v1` and `provider-preference-v1` are small encrypted pointer
  files; `codebase-maps-v1/` holds encrypted metadata-only architecture maps;
  `locks-v1/` holds advisory lock files; `agent-sessions/` holds encrypted
  metadata-only agent analysis sessions.

Workspace JSON and JSONL payloads are encrypted with XChaCha20-Poly1305. On
Unix-like platforms, CodeCaddie also creates state files with owner-only
permissions. The enclosing data directory should remain inside the user's
private application-data location.

## Owner-only local content key

CodeCaddie generates one random 256-bit content key per data root and stores it
as `local-content-key-v1` with owner-only permissions in that same root. Stable
and developer builds have separate roots and therefore separate keys. The app
does not access macOS Keychain, Windows Credential Manager, or Linux Secret
Service. A missing, malformed, or mismatched key fails closed without modifying
ciphertext. CodeCaddie still never accepts or stores AI-provider credentials;
installed provider tools retain their own authorization.

Valid plaintext `local-state-v2.json` and event logs from earlier builds are
atomically migrated after the key is available. A locked, one-time startup
sweep covers every active workspace log, architecture map, pointer, preference,
agent session, and backup schedule before the app serves a request, including
workspaces the user has not opened since upgrading. Normal readers still
validate decrypted schemas and signed records before using them. Interrupted
migrations preserve either the complete old or complete new file; newer
encrypted state is intentionally unreadable to older builds.

Co-locating the key avoids authorization prompts and keeps the local data root
self-contained for this local-first phase. It prevents managed payload files
from being readable on their own, but it is not a defense against a process
that can copy the entire data root as the current operating-system user.

This format is a clean break from earlier pre-release layouts. Old
`device-keyring-v1.json`, `events/`, `events-v1/`, `published/`,
`published-v1/`, `local-state-v1.json`, `development-keys/`, and
`device-keys/` entries are not loaded by the current application; the app
starts fresh beside them.

## Headless and CI use

The Rust core and its CLI surfaces run headless on all three platforms for
tests and automation:

- `CODECADDIE_DATA_DIR=/absolute/path` relocates all storage to a directory
  you control. Use a path outside any repository checkout.
- `CODECADDIE_RELEASE_CHANNEL=stable|dev` overrides channel detection when
  the executable path does not indicate the channel.

Repository CI (`.github/workflows/ci.yml`) runs the Rust workspace tests on
Ubuntu with these mechanisms, and builds and tests the desktop app on macOS
(arm64 and x64) and Windows x64 runners.
