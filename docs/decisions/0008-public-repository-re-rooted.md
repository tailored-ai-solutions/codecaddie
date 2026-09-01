# 0008. Public repository re-rooted on a single snapshot commit

- Status: Accepted
- Date: 2026-08-30

## Context

The release identity is derived from history: `config/release-version.json`
fixes the build epoch at 2000 and the build number is that epoch plus the
first-parent commit count of protected `main`. The Sigstore policy (0006)
pins a numeric repository id. A public repository therefore needs a history
whose every commit was reviewed for publication and whose count is known.

## Decision

The public `tailored-ai-solutions/codecaddie` repository starts from exactly
one parentless commit, `Open source CodeCaddie snapshot`, authored and
signed off by the maintainer under the GitHub noreply address, built in a
fresh directory from the sanitized tree. `scripts/verify-public-root.mjs`
checks that shape mechanically (one root, one branch, no tags, no alternate
objects, no private refs). The first release is build 2001 and every later
squash merge increments the build by one. The private original repository
stays private and is never pushed with `--mirror`.

## Consequences

`git log` cannot explain earlier decisions, which is why this directory
exists and why `AGENTS.md` points `/why` at it. After the first squash merge,
`node scripts/verify-public-root.mjs --expected-commits 2` must still pass.
`config/release-trust.json` carries the new repository id, and the
`CODECADDIE_PRIVATE_PATTERNS` secret keeps the denylist out of the tree.

## Evidence

- `scripts/verify-public-root.mjs`; `scripts/tests/verify-public-root.test.mjs`.
- `config/release-version.json`; `scripts/release-build-number.mjs`.
- `docs/RELEASING.md` "Release identity" and "Repository controls"; `CHANGELOG.md` entry 0.4.0.
