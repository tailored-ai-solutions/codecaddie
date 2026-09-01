# Supported desktop environments

This matrix is the release contract for CodeCaddie 0.4. A combination not
listed as supported is not silently treated as supported. The release owner
updates this file and the cross-platform CI matrix together.

| Area | Supported | Explicitly unsupported |
| --- | --- | --- |
| macOS | macOS 15 on Apple silicon (`arm64`) and Intel (`x86_64`) | macOS 14 and earlier; case-sensitive system volumes are not exercised by the release gate |
| Windows | Source-built developer preview exercised on Windows Server 2025 CI and Windows 11-compatible `x86_64` desktop execution | Public Windows installation and automatic updates until open-source signing is approved; 32-bit Windows, Windows on Arm, and system-wide MSI installation |
| Repository filesystem | Local APFS/HFS+ on macOS and local NTFS on Windows; a readable Git worktree with a resolvable commit | Network shares, virtual filesystems that do not preserve ordinary-file semantics, bare repositories, and repositories whose selected commit is unavailable |
| Local state | `~/Library/Application Support/CodeCaddie` on macOS and `%APPDATA%\CodeCaddie` on Windows; the documented development variants and `CODECADDIE_DATA_DIR` override | Shared multi-user data directories, remote state stores, and a data root that cannot provide private regular files and atomic replace semantics |
| AI providers | Installed Codex, Claude, or Grok command-line tools that satisfy the versioned provider contract | API-key discovery, network service fallback, switching providers after a failure, or an adapter that cannot enforce bounded output, timeout, cancellation, and read-only repository access |
| Repository capacity | Up to 100,000 regular files and 2 GiB of eligible repository content per the reliability budget | Symlink escapes, device files, sockets, or inputs above the bounded scan limits |

Every Tier 1 target runs the native model and UI tests, package creation,
repeat installation, uninstall, and data-preservation checks. Windows preview
CI runs its source and packaging tests without creating a public release. The common Rust
and protocol suites cover repository analysis, saved-report persistence,
restart recovery, export, cancellation, malformed provider output, timeouts,
and provider failure. Physical release acceptance remains required for signed
artifacts as described in [RELEASING.md](RELEASING.md).

Provider execution never falls back silently. A requested adapter either
returns a result that satisfies
[`provider-contract-v1.schema.json`](../protocol/provider-contract-v1.schema.json)
or returns a typed, source-safe failure for that adapter. The focused
[`contract_assurance.rs`](../crates/codecaddie-core/src/provider/contract_assurance.rs)
matrix runs Codex, Claude, and Grok through valid, malformed, timeout,
cancellation, typed-error, and forbidden-fallback cases.
