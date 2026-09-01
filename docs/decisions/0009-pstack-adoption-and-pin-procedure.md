# 0009. pstack adoption and pin procedure

- Status: Accepted
- Date: 2026-09-01

## Context

Agent-driven development needs one discipline across Claude Code, Cursor,
Codex, Grok Build, and Grok Bot. pstack's canonical source,
`cursor/plugins/pstack`, ships a Cursor manifest only, Claude Code needs a
`.claude-plugin` manifest, and a floating ref would change agents without review.

## Decision

`.claude-plugin/marketplace.json` pins two entries that both derive the plugin
namespace `pstack` and are therefore mutually exclusive: `pstack`, the Claude
Code port `michael-denyer/pstack-claude` at
`1716d5d6e0a8f50f137626d6f480d0f8533959c9` (`plugins/pstack`), and
`pstack-upstream`, `cursor/plugins` at
`23a56e2dac2efd54788056db8eced26e371d7b5e` (`pstack`). `.claude/settings.json`
enables only the port. Codex and Grok get the upstream skills through
`scripts/agents-setup.mjs` at the same sha (`PSTACK_PIN`). `AGENTS.md` is the
agent contract, `CLAUDE.md` imports it, and the verification skill is canonical
in `.agents/skills/verify-codecaddie`. Bump procedure:

1. Resolve the new commits read-only: `gh api repos/michael-denyer/pstack-claude/commits/main --jq .sha` and `gh api "repos/cursor/plugins/commits?path=pstack&per_page=1" --jq '.[0].sha'`.
2. Update both `sha` values in `.claude-plugin/marketplace.json`, `PSTACK_PIN.sha` and `KNOWN_PSTACK_SKILLS` in `scripts/agents-setup.mjs`, and the shas above.
3. Run `claude plugin validate .` and `node --test scripts/tests/agent-config.test.mjs scripts/tests/agents-setup.test.mjs`.
4. In a fresh clone, `claude plugin marketplace update codecaddie` then `claude plugin install pstack@codecaddie`; confirm `/pstack:poteto-mode` and `/verify-codecaddie` are listed. Repeat for `pstack-upstream@codecaddie` after uninstalling the port.
5. `pnpm agents:setup --codex --dry-run` prints the new checkout plan.

## Consequences

Agents see the same skill text until a reviewed bump. The port lags upstream
by design; try `pstack-upstream` before bumping the port.

## Evidence

- `.claude-plugin/marketplace.json`; `.claude/settings.json`; `scripts/agents-setup.mjs`; `scripts/tests/agent-config.test.mjs`.
