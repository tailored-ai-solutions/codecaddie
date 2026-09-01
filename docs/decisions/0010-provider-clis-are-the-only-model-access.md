# 0010. Installed provider CLIs are the only model access

- Status: Accepted
- Date: 2026-09-01

## Context

Model access needs authorization, and a desktop app that accepted API keys
would have to store, protect, rotate, and eventually leak them. Users already
authorize `claude`, `codex`, or `grok` on their machines under terms their
organization approved, and each CLI ships its own sandboxing and read-only
tool modes.

## Decision

The core never calls a model API and never accepts, logs, or persists a
credential. `provider::detect_all` finds the supported CLIs on `PATH`, probes
each one, and admits it only if it supports the required read-only or
sandboxed contract (`contract_supported` in `provider/claude.rs`,
`provider/codex.rs`, `provider/grok.rs`). `ProviderRunner` runs the chosen
CLI against a disposable, read-only snapshot of the frozen commit with a
bounded tool budget; Codex reaches the snapshot only through the bundled
`provider_repository_mcp` server, whose listing, search, and read limits are
constants in that module. Output is parsed as bounded NDJSON
(`provider/stream.rs`) against `protocol/provider-contract-v1.schema.json`.
`settings.provider.set` stores only the provider name.

## Consequences

There is no network client in the analysis path and nothing for a secret
scanner to find. Adding a provider means an adapter, a contract assurance
test, and a row in the tool-contract table in `docs/ARCHITECTURE.md`. Users
without any CLI are offered an install path rather than a key field.

## Evidence

- `crates/codecaddie-core/src/provider/mod.rs`: `ProviderKind`, `ProviderCapability`, `detect_all`; `crates/codecaddie-core/src/provider/runner.rs`: `ProviderRunner`.
- `crates/codecaddie-core/src/provider/contract.rs`; `crates/codecaddie-core/src/provider/contract_assurance.rs`; `crates/codecaddie-core/src/provider_repository_mcp.rs`.
- `protocol/provider-contract-v1.schema.json`; `docs/SECURITY_MODEL.md` "Trust boundaries".
