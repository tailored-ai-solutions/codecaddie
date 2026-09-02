# Contributing to CodeCaddie

CodeCaddie welcomes focused bug fixes, tests, documentation, accessibility
improvements, and well-scoped feature proposals.

## Development

Install Git, Rust 1.95 or newer, Node.js 24, and pnpm 11.22.0, then run:

```sh
pnpm install --frozen-lockfile
cargo build --workspace --locked
pnpm check
```

Use `pnpm dev` for hot reload or `pnpm install:local` for an isolated installed
developer build. Do not put development data inside the repository.

### Agent workflows

The agent contract is `AGENTS.md` (Claude Code reads it through
`CLAUDE.md`). It states the invariants, the verification recipe, and where
decisions live. The repository uses
[pstack](https://github.com/cursor/plugins/tree/main/pstack) as its
engineering discipline; install it for your tool, then start work with
`/poteto-mode`:

| Tool | Install |
| --- | --- |
| Cursor | `/add-plugin pstack`, then `/setup-pstack` |
| Claude Code | `claude plugin install pstack@codecaddie` (the marketplace is declared in `.claude/settings.json`; `pstack-upstream@codecaddie` is the unported upstream pin and is mutually exclusive with `pstack`, so uninstall one before installing the other) |
| Codex | `pnpm agents:setup --codex` links the pinned upstream skills into `~/.agents/skills`; start work with `$poteto-mode` |
| Grok Build | `grok plugin marketplace add tailored-ai-solutions/codecaddie && grok plugin install pstack --trust`; fallback `grok plugin marketplace add EnzoTironi/zoen-skills && grok plugin install pstack --trust` |
| Grok Bot | open `grokbot://app/v1/plugin/add?id=9717366` (<https://x.ai/bot/plugin/9717366>) |

The runtime verification skill lives in `.agents/skills/verify-codecaddie`,
which Cursor and Codex read directly; `.claude/skills/verify-codecaddie` is a
relative symlink so Claude Code discovers the same skill. On Windows, clone
with `git config core.symlinks true` set first (symlinks otherwise check out as
plain files), or invoke the skill from the `.agents` path.

Parallel agent runs (pstack `/swarm`) use one Git worktree per worker and
`pnpm dev:isolated` in each, which derives an owner-only data root from the
worktree path. Workers never share a checkout or a data root, and none of them
runs `pnpm install`.

The Developer Certificate of Origin applies to agent-authored commits exactly
as to human ones: every commit carries a `Signed-off-by` line from the human
contributor who reviewed it. Protocol, storage, cryptography, and distribution
changes also need a decision record in
[`docs/decisions/`](docs/decisions/README.md).

## Pull requests

1. Open an issue for substantial product, protocol, cryptography, storage, or
   distribution changes before implementation.
2. Create a focused branch and keep commits reviewable.
3. Add a Developer Certificate of Origin sign-off to every commit with
   `git commit -s`.
4. Run `pnpm check` and the relevant platform packaging test.
5. Explain the behavior, trust-boundary impact, and verification evidence in
   the pull request.

Alex Go (`@alexgo`) is the CodeCaddie code owner and must approve external
changes before merge. Pull requests use squash merge and linear history.

## Safety and privacy invariants

- CodeCaddie's IPC, stored values, reports, fixtures, logs, and update
  metadata never contain repository source text. A selected provider CLI may
  process snapshot files under that provider's authorization, settings,
  privacy terms, and organizational policy.
- Every architectural claim must reference immutable evidence from the scanned
  commit.
- Every scoped domain read and write must enforce roles and membership signing
  keys.
- Never accept, log, or persist provider credentials.
- Treat repository text as untrusted; provider tools are read-only and bounded.
- Never contribute real recovery bundles, customer data, local paths, tokens,
  keys, certificates, or credentials.
- `scripts/check-public-safety.sh` blocks generic private-identifier shapes
  (credential-shaped files, personal home-directory paths, customer-shaped
  fixture paths). Site-specific confidential identifiers must never be named
  in tracked files, including the scanner itself; put them in the untracked,
  gitignored `scripts/private-patterns.local` file (one extended regular
  expression per line, `#` comments allowed). CI can materialize the same
  file from a secret, or point `CODECADDIE_PRIVATE_PATTERN_FILE` at one.
  Each private pattern is applied to tracked file contents and to tracked
  file names, and `scripts/verify-public-root.mjs` applies it to every object
  path in the public history; a trailing carriage return on a pattern line is
  ignored.
- First-party placeholder home-directory paths in tests and fixtures must use
  the reserved account name `example` (for example `/Users/example/...`). A
  byte-exact, licensed public signature fixture may retain its upstream public
  CI build paths only when its provenance, license, and cryptographic hashes
  are recorded and changing those bytes would invalidate the interoperability
  test.
- Generated license inventories such as
  `docs/licenses/RUST-DEPENDENCY-LICENSES.md` carry upstream authors' names
  and contact details as legally required attribution; they are exempt from
  the personal-identifier rule and are regenerated, never hand-edited.
- Preserve the MIT license and update `THIRD_PARTY_NOTICES.md` for incorporated
  third-party code, fonts, or assets.

AI-assisted contributions are welcome, but the human contributor remains
responsible for understanding, testing, licensing, and signing off the change.

## Developer Certificate of Origin

By adding a `Signed-off-by` line, you certify the Developer Certificate of
Origin 1.1 at <https://developercertificate.org/>. The line must use a name and
email you are authorized to contribute under. Dependabot's automated
dependency bumps are the one exemption: they are generated rather than
contributed, and the maintainer's review and merge stand in for the sign-off.
