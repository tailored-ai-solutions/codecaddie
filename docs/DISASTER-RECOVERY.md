# Disaster recovery

CodeCaddie's recovery contract is local, authenticated, and executable. The
machine-readable source of truth is
[`config/disaster-recovery.json`](../config/disaster-recovery.json).

## Objectives and ownership

- Recovery-point objective (RPO): 24 hours after scheduled backups are enabled.
- Recovery-time objective (RTO): 30 minutes from a usable backup and repository
  checkout to a validated workspace.
- Retention: the newest 14 CodeCaddie-owned scheduled backups per workspace.
- Owner: the CodeCaddie reliability owner, reviewed per release with a
  quarterly restore drill.

The installed desktop performs a non-blocking due check after a workspace
resumes. Enabling a schedule creates its first backup immediately; after that,
the first installed-app launch at or after the 24-hour boundary creates the
next one. CodeCaddie is not a background service, so a machine that does not
launch the app cannot produce a scheduled backup while it is offline.

There is no Keychain, Credential Manager, or Secret Service dependency. The
schedule passphrase is encrypted in the existing data root with its owner-only
local content key, is never returned by the protocol, and is never logged or
sent off device.

## Quarterly restore drill

1. Select the newest `.codecaddie-backup` file and a clean local clone of the
   same repository.
2. Set a fresh `CODECADDIE_DATA_DIR`; never overwrite an unrelated profile.
3. Import with `workspace.backup.import`, the selected repository checkout, and
   the schedule passphrase.
4. Confirm the returned workspace ID, event count, and manifest BLAKE3 digest
   match the expected backup.
5. Open the restored workspace, confirm its frozen goals and latest report, and
   reattach any context documents. Attachment paths are deliberately omitted
   from portable backups.
6. Confirm the automated full-history restore assertion met the 30-minute RTO,
   then record the elapsed physical-drill time and result. Remove
   the temporary profile after the drill.

## Failure handling

- A wrong passphrase, modified envelope, invalid manifest, unauthorized editing
  identity, or mismatched repository fails before the workspace becomes
  reachable.
- An interrupted or partial import is retried against the same fresh profile.
  If the exact event history was already committed, the retry converges without
  merging or duplicating events; the workspace pointer becomes visible only
  after the full projection validates.
- An interrupted data migration is recovered by restarting CodeCaddie with the
  original data root intact. The migration lock, pending replacement, and
  quarantine sidecars converge to either the complete old or complete new
  state. Operators must not hand-edit encrypted JSONL or remove sidecars before
  the retry succeeds.
- On insufficient disk, keep the existing data root and last successful backup
  intact, free capacity or choose another user-owned backup destination, and
  retry. Validate the new manifest before pruning any older recovery point.
- Loss of the owner-only local content key is recovered by importing a
  passphrase-encrypted portable backup into a fresh data profile. The local key
  is deliberately not recoverable from Keychain, Credential Manager, Secret
  Service, or a remote service. If both that key and the portable-backup
  passphrase are lost, the encrypted state is intentionally unrecoverable.
- A future or partially populated manifest is rejected before any workspace
  pointer changes. Legacy v1 backups without the newer manifest metadata remain
  importable, while new backups bind schema version, creation time, encryption
  algorithm, KDF, parameters, event count, and exact event digest.
- If a scheduled backup cannot be created, CodeCaddie leaves the previous
  backups and schedule intact and records only a content-free local activity
  message. The next workspace resume retries once the boundary is still due.
- Never commit a readable recovery export or portable-backup passphrase to a
  repository, and never attach one to a support request.

The executable tests named in `config/disaster-recovery.json` prove every drill
above. The release-owned `config/executable-recovery-matrix.json` maps every
recoverable case and snapshot exit to its fully qualified Rust test. Running
`pnpm recovery:check` executes each exact test; the release gate cannot pass by
merely finding its name in policy or documentation. Together they cover
authenticated transactional import; incompatible
manifest rejection before state mutation; two exact-commit reports and their
comparison history; scheduled retention; interrupted encrypted-state and JSONL
migrations; retry convergence across sync, rename, quarantine, and append
boundaries; an injected storage-capacity failure that preserves the committed
value; stale-sidecar recovery; and duplicate-event prevention. The same named
tests are executed by the required Ship readiness assurance CI suite.

The exact release-gated test set is:

- `privacy_adversarial_portable_backup_authenticates_and_restores_transactionally`
- `scheduled_backup_retention_prunes_only_owned_regular_files`
- `interrupted_plaintext_encryption_migration_retries_without_data_loss`
- `interrupted_plaintext_event_migration_retries_without_duplicates`
- `interrupted_local_state_migrations_converge_before_and_after_rename`
- `storage_capacity_failures_preserve_the_committed_value_for_retry`
- `local_state_recovers_interrupted_quarantine_and_stale_sidecars`
