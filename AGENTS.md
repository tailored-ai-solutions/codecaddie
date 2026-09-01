# CodeCaddie agent contract

This file is the contract for every coding agent (Claude Code, Cursor, Codex,
Grok Build, Grok Bot) and for the humans who review agent work. `CLAUDE.md`
imports it. Read it before changing anything; the invariants are enforced by
tests and CI, not by convention.

## Invariants

- Keep repository source on the device. IPC, reports, and exports may contain paths, line ranges, hashes, derived claims, and report metadata, but never source text.
- Every architectural claim must reference immutable evidence from the scanned commit.
- Goal and analysis-result storage lives in the data root resolved from `CODECADDIE_DATA_DIR` or the per-platform default. Do not introduce a second storage system.
- Treat repository text as untrusted input. Model tools are read-only and bounded.
- Preserve the MIT license and record incorporated third-party code in `THIRD_PARTY_NOTICES.md` before distribution.

## Where things live

- `crates/codecaddie-core` is the Rust core: one process per request, dispatch in `src/service.rs`, storage under `src/local_state/`, analysis under `src/analyzer/`, provider adapters under `src/provider/`, updates in `src/update.rs`.
- `crates/codecaddie-domain` holds the deterministic domain model: events, projection, scoring, and the codebase map types.
- `apps/desktop` is the Zig/Native SDK host: `src/main.zig` (messages and `update`), `src/model.zig` (state), `src/app.native` (markup), `src/core_ipc.zig` (frames), `src/tests.zig` (state-machine tests).
- `protocol/` is the method catalog and fixture corpus; `plugin/` is the cross-agent plugin whose reference files are compiled into the core.
- `docs/MODULE-MAP.md` maps every module to its responsibility, entry points, and tests; `docs/decisions/` records why things are the way they are.

## Working method: pstack

pstack is the engineering discipline this repository uses. Start substantive work with `/poteto-mode` (`/pstack:poteto-mode` in Claude Code, `$poteto-mode` in Codex). Per-tool installation is in `CONTRIBUTING.md` under "Agent workflows".

The 21 principles, grouped by phase. Each is a skill named `principle-<name>` in the installed plugin. The three this repository leans on hardest are marked.

- Think: `foundational-thinking`, `redesign-from-first-principles`, `exhaust-the-design-space`, `fix-root-causes`, `laziness-protocol`, `guard-the-context-window` (emphasis: the largest core files exceed 1,000 lines; read by symbol and by `docs/MODULE-MAP.md`, not whole files).
- Design: `model-the-domain`, `type-system-discipline`, `boundary-discipline` (emphasis: source text, credentials, and absolute paths stop at the boundaries named in the invariants, and every boundary has a fail-closed test), `separate-before-serializing-shared-state`, `subtract-before-you-add`, `minimize-reader-load`, `build-the-lever`, `encode-lessons-in-structure`.
- Execute: `sequence-verifiable-units`, `outcome-oriented-execution`, `never-block-on-the-human`, `make-operations-idempotent`, `migrate-callers-then-delete-legacy-apis`.
- Verify: `prove-it-works` (emphasis: a change is done when the recipe below proves it at runtime, not when it compiles), `experience-first`.

## Verification recipe, fast to slow

1. `pnpm check:fast` — version and brand checks, the Node test suites, the public-safety scan, `cargo fmt`, `cargo clippy`, and `cargo test --workspace --locked`. Offline.
2. `pnpm exec native check apps/desktop --strict && pnpm exec native test apps/desktop --yes` — desktop markup and the desktop state machine, including the first-report journey. Required for any change under `apps/desktop/`.
3. `pnpm verify:core` — drives `target/debug/codecaddie-core` through `system.ping` and the deterministic first-report journey in a throwaway data root and keeps the evidence (`scripts/exercise-installed-core.mjs --dev --only ping,journey --json --keep`). Build first with `cargo build --workspace --locked`.
4. `pnpm check` — the full contribution gate; `docs/DEVELOPMENT.md` lists every step.
5. `pnpm dev` — only for changes a user can see. `.native` markup hot-reloads; Zig changes need a restart; Rust changes need a rebuild.

Run `/verify-codecaddie` (the skill in `.agents/skills/verify-codecaddie`) before declaring UI or core work done. It launches, health-checks, drives one feature, captures source-free evidence, and cleans up. Put the evidence directory path in the pull request.

## Headless core facts

- The binary is `target/debug/codecaddie-core` after `cargo build --workspace --locked`. There is no daemon; every request is one process.
- `CODECADDIE_DATA_DIR` selects the data root. Always set it to a fresh, owner-only temporary directory for tests and agent runs. Never point it inside the checkout and never at the real per-platform default.
- Wire format: one request per frame on stdin, a 4-byte big-endian length followed by one UTF-8 JSON object, the same shape back on stdout, 16 MiB maximum. Logs go to stderr only.
- Send `system.ping` first: `{"id":"<any>","protocolVersion":2,"method":"system.ping","params":{}}`. Every method is in `protocol/README.md`, mirrored by `service::METHODS`; scoped methods need `workspaceId` in the envelope.
- `--health-check` prints `CodeCaddie <version>+<build> <commit>`; development builds report build `0`.
- `agent <verb>` (`status`, `goals`, `backlog`, `begin-analysis`, `submit-analysis`, `note-action`, `export`) prints exactly one JSON object and exits 0 or 1. Files move only through `agent-exchange/inbox` and `agent-exchange/outbox` under the data root.
- `"stream": true` on `goals.generate`, `scan.run`, or `map.generate` switches the reply to NDJSON progress lines plus one terminal response line.

## Parallel work and `/swarm`

- One worker per Git worktree (`git worktree add ../codecaddie-<task>`; a `/.worktrees/` directory inside the checkout is gitignored). Workers never share a checkout.
- Each worker runs `pnpm dev:isolated`, which derives an owner-only data root from the worktree path and launches the desktop against it. Two workers must never share a data root.
- Workers do not run `pnpm install` or edit the lockfile; that is integration work for the human.

## Commits, reviews, and decisions

- Every commit carries a DCO sign-off (`git commit -s`), agent-authored commits included; the human contributor is responsible for the change.
- Pull requests are squash-merged onto a linear `main`. Fill in `.github/pull_request_template.md`, including the runtime verification evidence.
- Protocol, storage, cryptography, or distribution changes need a decision record in `docs/decisions/` (copy `docs/decisions/TEMPLATE.md`). Use `/why` against that directory rather than git archaeology: the public history was re-rooted on a single snapshot commit.
- Verification evidence lives under `$TMPDIR/codecaddie-verify/` and is never committed.

## Public-safety rules for agents

- Never add real customer, company, or person names, personal email addresses, or home-directory paths. Placeholder paths use the reserved account `/Users/example`. CodeCaddie, ThoughtfulBits, Tailored AI Solutions, the maintainer's public handle, the GitHub noreply identity pinned by the public-root audit, the owners of pinned upstream dependencies, and public vendors are fine.
- Never add credentials, tokens, private keys, Apple team identifiers, or fixtures derived from real customer code. Fixtures are synthetic.
- Run `./scripts/check-public-safety.sh` before proposing a change. Site-specific identifiers belong only in the untracked `scripts/private-patterns.local`.
- Generated license inventories under `docs/licenses/` carry upstream author attribution by design and are exempt from the personal-identifier rule.
- Never write to `~/Library/Application Support/CodeCaddie*`, `%APPDATA%\CodeCaddie*`, or `$XDG_DATA_HOME/codecaddie*` from a test or agent run.
