# 0007. The analysis contract is embedded from the plugin at compile time

- Status: Accepted
- Date: 2026-09-01

## Context

The same analysis contract serves two consumers: the packaged desktop core,
which prompts a provider CLI and validates its output, and the cross-agent
plugin under `plugin/`, which teaches Claude Code, Codex, and Grok the same
analysis and submits results through `codecaddie mcp` or the agent CLI. If
the two copies drift, the core rejects reports the plugin produced.

## Decision

`plugin/skills/codecaddie-analysis/references/` is the single source of
truth. `crates/codecaddie-core/src/analyzer/analysis_contract.rs` embeds
those files with `include_str!` (`ANALYSIS_SCHEMA`,
`GOAL_GENERATION_SCHEMA`, `CODEBASE_MAP_SCHEMA`,
`CODEBASE_MAP_DEEP_DIVE_SCHEMA`, `GOAL_GENERATION_RUBRIC`,
`ENGINEERING_HEALTH_CHECKLIST`), so a packaged build and a marketplace install
can never disagree. Edits happen in the plugin files, never in Rust string
literals. The golden `goal-template-catalog.md` is regenerated from the Rust
catalog with `UPDATE_GOLDEN=1 cargo test -p codecaddie-core rendered_catalog`
so the plugin and the core describe the same goal archetypes.

## Consequences

The workspace crates are packaged by the repository scripts rather than
`cargo publish`, because the `include_str!` paths cross the crate root. The
plugin version tracks the application version. Vendored product rubrics under
`crates/codecaddie-core/rubrics/` are pinned by BLAKE3 hash in tests so an
accidental edit fails the build.

## Evidence

- `crates/codecaddie-core/src/analyzer/analysis_contract.rs`: the `include_str!` constants and `analysis_prompt`.
- `plugin/skills/codecaddie-analysis/SKILL.md`; `plugin/skills/codecaddie-analysis/references/goal-template-catalog.md`.
- `plugin/README.md` "Single source of truth".
