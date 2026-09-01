# CodeCaddie development workflow

## Toolchain

CodeCaddie is a Native SDK desktop app with a Zig host and bundled Rust core.
The supported contributor toolchain is Node.js 24, pnpm 11.22.0, Rust 1.95 or newer,
Git, and Native SDK 0.10.1. Ordinary Native SDK commands use its pinned Zig
toolchain; `native doctor --strict` additionally checks for a system Zig.

pnpm applies `patches/@native-sdk__cli@0.10.1.patch`. The patch adds
target-aware external-file drag enter, move, leave, and drop events for AppKit
and Windows OLE drop targets. It also adds a bounded per-spawn collect limit:
ordinary child processes retain Native SDK's 512 KiB default while the local
core resume request may receive one complete 16 MiB-framed response. Keep the
patch enabled and rerun the native tests when updating dependencies.

Confirm the basics:

```sh
node --version
pnpm --version
rustc --version
cargo --version
git --version
```

## First run

```sh
git clone https://github.com/tailored-ai-solutions/codecaddie.git
cd codecaddie
pnpm install --frozen-lockfile
cargo build --workspace --locked
pnpm dev
```

The desktop host expects `target/debug/codecaddie-core` beside the repository
during development. `.native` markup hot-reloads without resetting the current
model. Restart `pnpm dev` after Zig changes. Rebuild Rust and restart after core
changes:

```sh
cargo build --workspace --locked
pnpm dev
```

Set `CODECADDIE_DATA_DIR` to an absolute directory outside the checkout when a
test should not touch normal developer state:

```sh
CODECADDIE_DATA_DIR=/absolute/path/outside-the-repository pnpm dev
```

## Validate a change

`pnpm check` is the contribution gate. It runs these steps in order and stops
at the first failure:

1. `pnpm version:check`: `package.json`, the Cargo workspace, and the release
   metadata agree on one version.
2. `pnpm brand:check`: generated desktop brand assets match their sources.
3. `node --test scripts/tests/*.test.mjs`: the Node suites for scripts,
   policies, and agent configuration.
4. `pnpm reliability:check`: `scripts/check-reliability-gates.mjs` and the
   `performance_gate` Rust test.
5. `pnpm evidence:check`: saved evidence survives a checkout switch
   (`scripts/exercise-saved-evidence-checkout.mjs`).
6. `pnpm recovery:check`: the executable recovery matrix
   (`scripts/exercise-recovery-matrix.mjs`).
7. `./scripts/check-public-safety.sh`: credential shapes, personal paths,
   customer-shaped fixtures, and the untracked private denylist when present.
8. `pnpm audit --audit-level high`: Node dependency advisories.
9. `cargo metadata --locked`: the Cargo lockfile is current.
10. `cargo fmt --all --check` and
    `cargo clippy --workspace --all-targets --locked -- -D warnings`.
11. `pnpm privacy:check`: the adversarial privacy tests, with output shown.
12. `cargo test --workspace --locked`.
13. `pnpm native:check`: `native check apps/desktop --strict` followed by
    `native test apps/desktop --yes`.

`pnpm check:fast` is the offline subset (steps 1, 2, 3, 7, 10, and 12) for
iterating. `pnpm check:release` runs the slow release gates (reliability,
evidence, recovery, and `pnpm compatibility:check --require-clean`, which
needs a clean tree); CI runs them on every pull request. `pnpm build` then
builds the Rust workspace and desktop host.

```sh
pnpm check:fast
pnpm check
pnpm build
```

Run narrower checks while iterating:

```sh
pnpm version:check
pnpm brand:check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pnpm exec native check apps/desktop --strict
pnpm exec native test apps/desktop --yes
pnpm exec native build apps/desktop --yes
node --test scripts/tests/*.test.mjs
pnpm verify:core
```

## Agent workflows

Coding agents follow `AGENTS.md`; `CLAUDE.md` imports it for Claude Code.
Install pstack for your tool as described in
[CONTRIBUTING.md](../CONTRIBUTING.md) under "Agent workflows", then:

- `pnpm verify:core` drives the built `target/debug/codecaddie-core` through
  `system.ping` and the deterministic first-report journey in a throwaway
  data root and keeps the evidence directory
  (`scripts/exercise-installed-core.mjs --dev --only ping,journey --json --keep`).
- `/verify-codecaddie` (`.agents/skills/verify-codecaddie`) is the launch,
  doctor, drive, evidence, and cleanup recipe to run before declaring UI or
  core work done.
- `pnpm dev:isolated` runs the desktop against an owner-only data root
  derived from the worktree path. Use one Git worktree per parallel worker.
- `pnpm agents:setup --codex` (or `--grok`) links the pinned upstream pstack
  skills into `~/.agents/skills`; `--dry-run` prints the plan. It never runs
  from install or CI.
- Decision records live in `docs/decisions/` and the module map in
  `docs/MODULE-MAP.md`.

The documentation and download site is available at
[`codecaddie.ai`](https://codecaddie.ai).

## One-command developer installation

```sh
pnpm install:local
```

The dispatcher finds the repository root regardless of the current working
directory and chooses the native installer for the current platform. It creates
a release-optimized, ad-hoc-signed Mac build or unsigned local Windows build,
stages and validates it, closes only a running developer edition, atomically
replaces the application, and launches it.

Options:

```sh
pnpm install:local -- --no-launch
pnpm install:local -- --no-build
pnpm install:local -- --uninstall
```

On macOS, `--destination /Applications` is an explicit opt-in to a shared
Applications destination and may require the user's existing filesystem
permissions. The default never requests administrator access.

| Platform | Developer application | Developer data |
|---|---|---|
| macOS | `~/Applications/CodeCaddie Dev.app` | `~/Library/Application Support/CodeCaddie Dev` |
| Windows | `%LOCALAPPDATA%\Programs\CodeCaddie Dev` | `%APPDATA%\CodeCaddie Dev` |

The developer edition uses `org.codecaddie.desktop.dev` and `codecaddie-dev:`
URLs. It cannot overwrite the
stable application. Uninstall removes the developer application, shortcut, and
associations, but intentionally preserves workspace data.

## State and update safety

Production state lives in `~/Library/Application Support/CodeCaddie` on macOS
or `%APPDATA%\CodeCaddie` on Windows; `CODECADDIE_DATA_DIR` overrides the
location entirely. Approved goals, analysis reports, action history, and
content-free decision-funnel markers are encrypted JSONL records under
`events-v2/`; historical numeric outcome-rating events created by older builds
remain readable for ledger compatibility, but current builds do not create new
ones. Device-local workspace context
lives in `local-state-v2.json` with the current v3 format marker. [PLATFORMS.md](PLATFORMS.md) documents the full
per-platform layout. Files use owner-only permissions where supported, and the
content key is an owner-only regular file in the same data root. The application
does not request Keychain authorization or store or retrieve CodeCaddie data or
secrets through Keychain, Credential Manager, or Secret Service. The HTTPS
updater may use the operating system's public trust-root verifier. Xcode Cloud
retains the non-exportable Developer ID credential, and GitHub Actions uses
OIDC for keyless Sigstore signing of the release manifest. No platform or
manifest signing private key enters a runner credential store. Windows public
distribution remains disabled until SignPath Foundation approves open-source
signing. None of those services is an application-data credential store. A
build that encounters newer state or a missing, malformed, or
mismatched key fails closed instead of attempting a downgrade.

Local-state replacements and immutable body/session creation stage a private
file, sync it, rename it, and sync the containing directory (or use the Windows
write-through replacement primitive). On startup, a valid destination removes
stale sidecars; when the destination is missing or malformed, the newest valid
temporary or quarantined JSON sidecar is promoted before loading. Event JSONL
appends first remove an unterminated crash tail and treat an identical final
signed record as an idempotent retry. Fault-injection tests interrupt temporary
file sync, quarantine and destination renames, append writes, and append syncs
to prove retries converge without duplicate events. Legacy plaintext state and
event logs are preserved record-for-record by the locked startup encryption
sweep and use the same replacement boundaries; normal readers perform schema,
signature, and hash validation only after decryption.

Update manifests are signed over their exact bytes with a GitHub OIDC-backed
Sigstore bundle. The embedded Rust verifier validates the Fulcio chain, Rekor
inclusion proof, rotating Sigstore trust roots, protected-main repository and
workflow identity, source commit, and manifest bytes without invoking a local
`cosign` executable. The core selects the exact OS, architecture, and updater
format; enforces SemVer and monotonic build rollback rules; downloads into the
private data root; and checks declared size and SHA-256 before staging. The
external updater then requires the candidate macOS application to match the
verified Apple team, bundle ID, semantic version, and build number before
atomic replacement.
