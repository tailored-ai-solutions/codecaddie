# Security model

## Protected assets

CodeCaddie protects repository source, provider authorization, device signing
keys, local workspace state, report integrity, and the release update
channel.

## Trust boundaries

The desktop and bundled core are trusted local components. Repository text and
provider output are untrusted. Download hosts and update networks are
untrusted transports. Device signatures, immutable Git coordinates,
artifact hashes, and publisher signatures establish
authority and integrity.

An installed AI provider is a separate data processor. CodeCaddie gives it a
history-free snapshot of the selected commit. The provider may process those
files under its own authorization, settings, privacy terms, network policy, and
organization controls. Users must select a provider configuration approved for
the repository's data classification.

Explicitly selected product-context files form a second, narrower provider
boundary. Selecting or dropping a supported file authorizes the core to extract
bounded text and send it to the selected provider during goal generation. The
core re-reads and BLAKE3-verifies each file at generation time; missing,
changed, symlinked, encrypted, corrupt, image-only, empty, or oversized files
fail closed. Website values are metadata and are never fetched. Document text
exists only in core memory for that run and never crosses back to the desktop.

## Data retained by CodeCaddie

Device-local decrypted state may include workspace context, attachment paths
and hashes, goals, reports, actions, device identities, and the local repository path. Raw
attachment text is not retained. The
private desktop-to-core IPC returns the device-local repository path so the
app can reopen the workspace. Reports and exports may include derived claims,
hashes, repository-relative paths, line ranges, blob IDs, and report
metadata. They must not include repository source excerpts, provider
credentials, absolute repository paths, or absolute attachment paths.
Provider activity may include sanitized repository-relative filenames and
application-counted file totals/ordinals. Absolute disposable-clone paths are
reduced to their `repository-N` suffix; search terms, source text, and provider
stderr are never forwarded.

## Authorization and key lifecycle

Every domain write requires this device's Editor signature. Local workspace
state and event records are authenticated encrypted envelopes written with
owner-only permissions where supported. A random 256-bit content key is stored
as an owner-only regular file in this same data root; the application never
calls an operating-system credential manager. Existing plaintext state migrates only after complete
record preservation under a one-time startup lock, using the same atomic
replacement and recovery boundaries. Normal consumers still validate the
decrypted schema, signatures, hashes, and evidence before use.

This boundary protects a managed ciphertext file when it is separated from its
data root. It does not claim to resist malware, another process running as the
same user, or an attacker who can read both the ciphertext and local key file.
User-requested recovery exports remain readable JSON so they can be inspected
and require destination care. Portable backups instead use an independent
Argon2id-derived key and XChaCha20-Poly1305 envelope; their passphrase and key
remain memory-only and never enter the data root or an operating-system
credential manager. Imports authenticate the complete payload and replay its
signed events before committing any reachable workspace state.

## Failure policy

Malformed signatures, wrong epochs, invalid evidence, source excerpts, unknown
protocol versions, oversized values, and invalid updates fail closed. A scan may
retain successful provider batches when another bounded batch fails; missing
criteria remain unverified. A scan with no successful batch fails.

Analysis and architecture-map providers receive only history-free files from
an exact resolved commit. A shared snapshot-workspace guard chooses the
confined `repository-N` destinations, makes materialized source files
read-only, and removes the complete workspace when the operation succeeds,
fails, times out, or is cancelled. The registered checkout is never a provider
working directory and is never mutated by snapshot creation.

## Adversarial privacy gate

Test-only repository and attachment fixtures carry private sentinels and a
hostile instruction. The versioned
[`source-canary matrix`](../config/source-canary-matrix-v1.json) binds local
ciphertext, reports, framed IPC, progress and failure diagnostics, logs, Word
and recovery exports, prompts, local analytics, crash markers, and provider
snapshot retention to executable tests. `pnpm privacy:check` runs the complete
named filter and proves those surfaces preserve the local-source boundary. The
same filtered suite runs as the separately named **Adversarial privacy and
prompt-injection gate** in GitHub Actions so a regression is visible without
interpreting the general Rust test job. Recommendation prose is explicitly
delimited as untrusted planning data before any generated prompt's fixed
working instructions.

Report security issues through [SECURITY.md](../SECURITY.md). Operational and
release checks are documented in [Development](DEVELOPMENT.md) and
[Releasing](RELEASING.md).
