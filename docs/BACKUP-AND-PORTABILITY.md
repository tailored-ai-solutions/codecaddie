# Backup and portability

CodeCaddie is local-first: its encrypted workspace data and owner-only content
key live in one data directory (locations in
[PLATFORMS.md](PLATFORMS.md)). This page explains what is device-local and what
moving between machines looks like.

## What is device-local

- **Goals, reports, and action history** — authenticated encrypted JSONL
  envelopes under `events-v2/`.
- **Local project state** (`local-state-v2.json`) — the local device identity
  and workspace context, including the attached repository path and product
  brief. Current files carry the v3 format marker and may include canonical
  paths, metadata, and hashes for selected context documents, but never their
  extracted text.
- **Pointers, preferences, maps, and agent sessions** — encrypted managed files
  under `recent-workspace-v1`, `provider-preference-v1`, `codebase-maps-v1/`,
  and `agent-sessions/`.
- **The attached repository itself** — never copied into CodeCaddie storage.
  Analyses use disposable single-commit snapshots that are deleted after each
  run.

CodeCaddie encrypts workspace state and events with XChaCha20-Poly1305. A
random 256-bit content key is stored as the owner-only
`local-content-key-v1` file beside the ciphertext. The app never requests
Keychain, Credential Manager, or Secret Service access. Losing the key makes
the encrypted state unreadable; a missing, malformed, or mismatched key fails
closed without rewriting data.
The first upgraded core process takes a private migration lock and encrypts all
active managed files, including unopened workspace logs and retained maps or
agent sessions, before serving the request.

## Recovery exports

A recovery export is readable JSON created by the local core (protocol method
`workspace.recovery.export`). It contains the workspace identity, local access
metadata, and the workspace's signed events. It never contains repository
source text, attached-document text or paths, or AI-provider credentials.

Because the export is intentionally unencrypted, choose its destination with
the same care as the main data directory. Do not commit it to a repository or
attach it to a support request.

## Authenticated portable backups

`workspace.backup.export` creates a `codecaddie-portable-backup-v1` envelope.
It derives a unique 256-bit key from the user-provided passphrase with
Argon2id (64 MiB, three iterations, one lane), then encrypts and authenticates
the complete payload with XChaCha20-Poly1305. The payload includes a BLAKE3
manifest, the workspace's signed event history, workspace context, and the
editing identity needed to continue that history. The passphrase and derived
key are never stored, returned, logged, or sent to Keychain, Credential
Manager, or Secret Service.

`workspace.backup.import` decrypts and authenticates the envelope, verifies
the manifest, replays every signed event, confirms that the included editing
key is authorized, and validates the selected local Git checkout before it
writes anything. Attachment paths are cleared on import and must be
reattached. The event file is committed atomically before the workspace is
made reachable from local state; retrying after an interruption finishes the
same exact restore without merging or duplicating events. A profile that
already contains a different editing identity fails closed—use a fresh
`CODECADDIE_DATA_DIR` rather than overwriting unrelated local workspaces.

## Scheduled backups

`workspace.backup.schedule.enable` stores a backup destination and creates the
first authenticated portable backup immediately. After that, the installed
desktop checks `workspace.backup.schedule.run` whenever the workspace resumes.
The first launch after the 24-hour boundary creates the next backup, and the
retention pass keeps the newest 14 CodeCaddie-owned backup files without
touching unrelated files or directories. Status and manual execution are also
available through `workspace.backup.schedule.status` and
`workspace.backup.schedule.run`; disabling the schedule removes only its local
configuration.

There is no Keychain, Credential Manager, or Secret Service access. A scheduled
backup's passphrase is stored only inside an encrypted schedule file under the
existing CodeCaddie data root, protected by the same owner-only local content
key as the rest of managed state. It is never returned by the protocol or
written to logs. Manual portable-export passphrases remain memory-only and are
never persisted. The destination must be a real directory outside both the
CodeCaddie data root and attached repository.

The machine-readable recovery policy sets a 24-hour recovery-point objective
and a 30-minute recovery-time objective. See
[DISASTER-RECOVERY.md](DISASTER-RECOVERY.md) for the restore drill and failure
procedure.

## Moving machines

To move an authenticated backup, clone the repository on the destination
machine, import the bundle into a fresh CodeCaddie data profile using that
checkout and the original passphrase, then reattach any project-context
documents. Filename-only references cannot recover document contents and
appear as **Reattach to use contents**.

## Backups

The complete data directory is a self-contained local backup because it
includes the owner-only key file. Protect the copy as sensitive: anyone who can
read the full backup can also decrypt its managed state. Prefer the portable
format for an independently authenticated, passphrase-protected copy. The
explicit readable recovery export remains available for inspection and must be
protected at its chosen destination.
