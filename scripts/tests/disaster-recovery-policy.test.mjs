import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);

test("disaster recovery policy and runbook stay executable and no-Keychain", async () => {
  const policy = JSON.parse(
    await readFile(new URL("config/disaster-recovery.json", root), "utf8"),
  );
  const runbook = await readFile(
    new URL("docs/DISASTER-RECOVERY.md", root),
    "utf8",
  );
  const backupGuide = await readFile(
    new URL("docs/BACKUP-AND-PORTABILITY.md", root),
    "utf8",
  );

  assert.equal(policy.schemaVersion, 1);
  assert.equal(policy.localOnly, true);
  assert.equal(policy.scheduledBackup.cadenceHours, 24);
  assert.equal(policy.scheduledBackup.recoveryPointObjectiveHours, 24);
  assert.equal(policy.scheduledBackup.recoveryTimeObjectiveMinutes, 30);
  assert.equal(policy.scheduledBackup.retentionCount, 14);
  assert.match(policy.scheduledBackup.trigger, /installed-app launch/i);
  assert.deepEqual(policy.failureDrills, [
    "interrupted_or_partial_import",
    "interrupted_data_migration",
    "insufficient_disk_capacity",
    "lost_local_content_key",
    "corrupt_or_incompatible_manifest",
  ]);
  assert.equal(policy.portableManifest.schemaVersion, 1);
  assert.deepEqual(policy.portableManifest.binds, [
    "creation_time",
    "encryption_algorithm",
    "key_derivation_parameters",
    "workspace_identity",
    "event_count",
    "event_digest",
  ]);
  assert.equal(policy.portableManifest.legacyV1WithoutMetadata, "accepted");
  assert.equal(
    policy.portableManifest.futureOrPartialMetadata,
    "rejected_before_state_mutation",
  );
  assert.deepEqual(policy.executableTests, [
    "privacy_adversarial_portable_backup_authenticates_and_restores_transactionally",
    "scheduled_backup_retention_prunes_only_owned_regular_files",
    "interrupted_plaintext_encryption_migration_retries_without_data_loss",
    "interrupted_plaintext_event_migration_retries_without_duplicates",
    "interrupted_local_state_migrations_converge_before_and_after_rename",
    "storage_capacity_failures_preserve_the_committed_value_for_retry",
    "local_state_recovers_interrupted_quarantine_and_stale_sidecars",
  ]);

  for (const expected of [
    "24 hours",
    "30 minutes",
    "quarterly restore drill",
    "workspace.backup.import",
    "There is no Keychain",
    "insufficient disk",
    "Loss of the owner-only local content key",
    "future or partially populated manifest",
    ...policy.executableTests,
  ]) {
    assert.ok(runbook.includes(expected), `runbook must include ${expected}`);
  }
  assert.match(backupGuide, /first launch after the 24-hour boundary/i);
  assert.match(backupGuide, /newest 14 CodeCaddie-owned backup files/i);
  assert.match(backupGuide, /There is no Keychain/i);

  const serialized = JSON.stringify(policy);
  for (const forbidden of ["endpoint", "url", "token", "apiKey", "sourceText", "prompt"] ) {
    assert.equal(serialized.includes(forbidden), false);
  }
});
