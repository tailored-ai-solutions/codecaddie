use crate::{
    at_rest::ContentCipher,
    persistence::{
        NoPersistenceFault, PersistenceBoundary, PersistenceFaultInjector, sync_parent,
        write_private_replace_with,
    },
};
use codecaddie_domain::EventEnvelope;
use serde::{Serialize, de::DeserializeOwned};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Seek, Write},
    path::{Path, PathBuf},
};

pub(crate) const EVENT_LOG_PURPOSE: &str = "workspace-events-v1";

/// Authenticated encrypted JSONL event records. Each decrypted committed line
/// is one signed [`EventEnvelope`]; epoch discipline remains enforced by the
/// projection when the log is replayed.
pub struct LocalEventLog {
    root: PathBuf,
    cipher: ContentCipher,
}

impl LocalEventLog {
    pub(crate) fn open(root: impl Into<PathBuf>, cipher: ContentCipher) -> anyhow::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        Ok(Self { root, cipher })
    }

    fn path(&self, workspace_id: &str) -> PathBuf {
        self.root.join(format!(
            "{}.events",
            blake3::hash(workspace_id.as_bytes()).to_hex()
        ))
    }

    pub fn append(&self, workspace_id: &str, event: &EventEnvelope) -> anyhow::Result<()> {
        if event.workspace_id != workspace_id {
            anyhow::bail!("event belongs to a different workspace");
        }
        append_encrypted_json_record(
            &self.path(workspace_id),
            event,
            &self.cipher,
            EVENT_LOG_PURPOSE,
        )
    }

    pub fn load(&self, workspace_id: &str) -> anyhow::Result<Vec<EventEnvelope>> {
        let path = self.path(workspace_id);
        if !path.exists() {
            return Ok(vec![]);
        }
        read_encrypted_json_lines_recover_tail(&path, &self.cipher, EVENT_LOG_PURPOSE)
    }

    /// Installs a fully validated portable-backup event history exactly once.
    /// A prior interrupted import is accepted only when the complete existing
    /// history is byte-equivalent after decoding; unrelated state is never
    /// replaced or merged implicitly.
    pub(crate) fn restore_exact(
        &self,
        workspace_id: &str,
        events: &[EventEnvelope],
    ) -> anyhow::Result<()> {
        if events.is_empty()
            || events
                .iter()
                .any(|event| event.workspace_id != workspace_id)
        {
            anyhow::bail!("portable backup events do not form one workspace history");
        }
        let path = self.path(workspace_id);
        if path.exists() {
            let existing = self.load(workspace_id)?;
            if existing == events {
                return Ok(());
            }
            anyhow::bail!("a different event history already exists for this workspace");
        }
        let mut encrypted = Vec::new();
        for event in events {
            encrypted.extend(
                self.cipher
                    .seal(EVENT_LOG_PURPOSE, &serde_json::to_vec(event)?)?,
            );
            encrypted.push(b'\n');
        }
        crate::persistence::write_private_atomic_new(&path, &encrypted)
    }

    /// The committed log lines as raw JSON values, for the recovery export.
    pub fn raw_values(&self, workspace_id: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        let path = self.path(workspace_id);
        if !path.exists() {
            anyhow::bail!("workspace has no local event log");
        }
        read_encrypted_json_lines_recover_tail(&path, &self.cipher, EVENT_LOG_PURPOSE)
    }

    /// Encrypts every managed event log in this generation before any
    /// workspace is opened. The sweep intentionally preserves each committed
    /// record's plaintext bytes and leaves semantic validation to normal log
    /// replay, so an unrelated damaged workspace cannot cause valid private
    /// records to remain readable.
    pub(crate) fn protect_all_at_rest(&self) -> anyhow::Result<()> {
        let mut paths = fs::read_dir(&self.root)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        for path in paths {
            if path.extension().and_then(|value| value.to_str()) != Some("events") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() {
                anyhow::bail!("managed event-log path is not a regular file");
            }
            protect_encrypted_json_lines(&path, &self.cipher, EVENT_LOG_PURPOSE)?;
        }
        Ok(())
    }
}

fn protect_encrypted_json_lines(
    path: &Path,
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    recover_unterminated_tail(path)?;
    let mut reader = BufReader::new(OpenOptions::new().read(true).open(path)?);
    let mut plaintext_records = Vec::new();
    let mut found_plaintext = false;
    loop {
        let mut record = Vec::new();
        let count = reader.read_until(b'\n', &mut record)?;
        if count == 0 {
            break;
        }
        record.pop();
        if record.last() == Some(&b'\r') {
            record.pop();
        }
        let decoded = cipher.open_or_plain(purpose, &record)?;
        found_plaintext |= !decoded.encrypted;
        plaintext_records.push(decoded.plaintext);
    }
    // Windows does not permit the atomic replacement below while this read
    // handle is still open. The complete plaintext set is already buffered,
    // so close the source before writing and renaming the encrypted sibling.
    drop(reader);
    if found_plaintext {
        let mut migrated = Vec::new();
        for plaintext in plaintext_records {
            migrated.extend(cipher.seal(purpose, &plaintext)?);
            migrated.push(b'\n');
        }
        write_private_replace_with(path, &migrated, &NoPersistenceFault)?;
    }
    Ok(())
}

fn append_encrypted_json_record<T: Serialize + DeserializeOwned>(
    path: &Path,
    value: &T,
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<()> {
    append_encrypted_json_record_with(path, value, cipher, purpose, &NoPersistenceFault)
}

fn append_encrypted_json_record_with<T: Serialize + DeserializeOwned>(
    path: &Path,
    value: &T,
    cipher: &ContentCipher,
    purpose: &str,
    injector: &impl PersistenceFaultInjector,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        recover_unterminated_tail(path)?;
        let first = first_committed_record(path)?;
        if let Some(first) = first
            && !cipher.open_or_plain(purpose, &first)?.encrypted
        {
            let _: Vec<T> = read_encrypted_json_lines_recover_tail(path, cipher, purpose)?;
        }
    }
    let plaintext = serde_json::to_vec(value)?;
    let last_plaintext = if path.exists() {
        last_committed_record(path)?
            .map(|record| cipher.open_or_plain(purpose, &record))
            .transpose()?
            .map(|decoded| decoded.plaintext)
    } else {
        None
    };
    if last_plaintext.as_deref() == Some(plaintext.as_slice()) {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?
            .sync_data()?;
        if let Some(parent) = path.parent() {
            sync_parent(parent)?;
        }
        return Ok(());
    }
    let mut record = cipher.seal(purpose, &plaintext)?;
    record.push(b'\n');
    #[cfg(unix)]
    let created = !path.exists();
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    file.write_all(&record)?;
    injector.checkpoint(PersistenceBoundary::AppendWritten)?;
    file.sync_data()?;
    injector.checkpoint(PersistenceBoundary::AppendSynced)?;
    #[cfg(unix)]
    if created && let Some(parent) = path.parent() {
        sync_parent(parent)?;
    }
    Ok(())
}

fn first_committed_record(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let mut reader = BufReader::new(OpenOptions::new().read(true).open(path)?);
    let mut record = Vec::new();
    let count = reader.read_until(b'\n', &mut record)?;
    if count == 0 || record.pop() != Some(b'\n') {
        return Ok(None);
    }
    if record.last() == Some(&b'\r') {
        record.pop();
    }
    Ok(Some(record))
}

fn last_committed_record(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let mut reader = BufReader::new(OpenOptions::new().read(true).open(path)?);
    let mut last = None;
    loop {
        let mut record = Vec::new();
        let count = reader.read_until(b'\n', &mut record)?;
        if count == 0 {
            return Ok(last);
        }
        if record.pop() != Some(b'\n') {
            return Ok(last);
        }
        if record.last() == Some(&b'\r') {
            record.pop();
        }
        last = Some(record);
    }
}

fn recover_unterminated_tail(path: &Path) -> anyhow::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut reader = BufReader::new(file.try_clone()?);
    let mut committed_bytes = 0_u64;
    loop {
        let mut record = Vec::new();
        let count = reader.read_until(b'\n', &mut record)?;
        if count == 0 {
            return Ok(());
        }
        if record.last() != Some(&b'\n') {
            drop(reader);
            file.set_len(committed_bytes)?;
            file.seek(std::io::SeekFrom::Start(committed_bytes))?;
            file.sync_data()?;
            return Ok(());
        }
        committed_bytes = committed_bytes.saturating_add(count as u64);
    }
}

/// Loads newline-committed encrypted JSON records. An unterminated final
/// record is a crash residue and is truncated; malformed committed records
/// remain fatal because silently skipping them could change workspace state.
/// A fully valid plaintext legacy log is atomically migrated after validation.
fn read_encrypted_json_lines_recover_tail<T: DeserializeOwned>(
    path: &Path,
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<Vec<T>> {
    read_encrypted_json_lines_recover_tail_with(path, cipher, purpose, &NoPersistenceFault)
}

fn read_encrypted_json_lines_recover_tail_with<T: DeserializeOwned>(
    path: &Path,
    cipher: &ContentCipher,
    purpose: &str,
    injector: &impl PersistenceFaultInjector,
) -> anyhow::Result<Vec<T>> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    recover_unterminated_tail(path)?;
    let mut reader = BufReader::new(OpenOptions::new().read(true).open(path)?);
    let mut values = Vec::new();
    let mut plaintext_records: Vec<Vec<u8>> = Vec::new();
    let mut encryption_state = None;
    loop {
        let mut record = Vec::new();
        let count = reader.read_until(b'\n', &mut record)?;
        if count == 0 {
            // Windows keeps the source path non-replaceable while this read
            // handle is open. Validation and buffering are complete at EOF,
            // so release it before either normal or fault-injected migration.
            drop(reader);
            if encryption_state == Some(false) {
                let mut migrated = Vec::new();
                for plaintext in plaintext_records {
                    migrated.extend(cipher.seal(purpose, &plaintext)?);
                    migrated.push(b'\n');
                }
                write_private_replace_with(path, &migrated, injector)?;
            }
            return Ok(values);
        }
        record.pop();
        if record.last() == Some(&b'\r') {
            record.pop();
        }
        let decoded = cipher.open_or_plain(purpose, &record)?;
        match encryption_state {
            Some(encrypted) if encrypted != decoded.encrypted => {
                anyhow::bail!("local event log mixes encrypted and legacy records")
            }
            None => encryption_state = Some(decoded.encrypted),
            _ => {}
        }
        let value = serde_json::from_slice(&decoded.plaintext)?;
        plaintext_records.push(decoded.plaintext);
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{FailOnce, PersistenceBoundary};
    use codecaddie_domain::DomainEvent;
    use ed25519_dalek::SigningKey;
    use time::OffsetDateTime;

    fn cipher() -> ContentCipher {
        ContentCipher::for_tests()
    }

    fn append<T: Serialize + DeserializeOwned>(path: &Path, value: &T) -> anyhow::Result<()> {
        append_encrypted_json_record(path, value, &cipher(), EVENT_LOG_PURPOSE)
    }

    fn read<T: DeserializeOwned>(path: &Path) -> anyhow::Result<Vec<T>> {
        read_encrypted_json_lines_recover_tail(path, &cipher(), EVENT_LOG_PURPOSE)
    }

    #[test]
    fn event_log_encrypts_private_fields_and_reopens() {
        let directory = tempfile::tempdir().unwrap();
        let log = LocalEventLog::open(directory.path(), cipher()).unwrap();
        let event = EventEnvelope::sign(
            "acme".into(),
            1,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::WorkspaceCreated {
                name: "recognizable customer phrase".into(),
                founding_device: codecaddie_domain::DeviceIdentity {
                    actor_id: "owner".into(),
                    device_id: "device".into(),
                    signing_public_key: hex::encode(
                        SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes(),
                    ),
                    label: "Test device".into(),
                },
                workspace_fingerprint: "fingerprint".into(),
            },
            &SigningKey::from_bytes(&[4; 32]),
        )
        .unwrap();
        log.append("acme", &event).unwrap();
        let raw = fs::read_to_string(log.path("acme")).unwrap();
        assert!(raw.contains(crate::at_rest::ENVELOPE_FORMAT));
        assert!(!raw.contains("recognizable customer phrase"));
        assert_eq!(log.load("acme").unwrap(), vec![event]);
    }

    #[test]
    fn legacy_plaintext_event_log_migrates_after_full_validation() {
        let directory = tempfile::tempdir().unwrap();
        let log = LocalEventLog::open(directory.path(), cipher()).unwrap();
        let key = SigningKey::from_bytes(&[6; 32]);
        let event = EventEnvelope::sign(
            "legacy".into(),
            1,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::WorkspaceCreated {
                name: "legacy private workspace".into(),
                founding_device: codecaddie_domain::DeviceIdentity {
                    actor_id: "owner".into(),
                    device_id: "device".into(),
                    signing_public_key: hex::encode(key.verifying_key().to_bytes()),
                    label: "Device".into(),
                },
                workspace_fingerprint: "fingerprint".into(),
            },
            &key,
        )
        .unwrap();
        let mut legacy = serde_json::to_vec(&event).unwrap();
        legacy.push(b'\n');
        fs::write(log.path("legacy"), legacy).unwrap();
        assert_eq!(log.load("legacy").unwrap(), vec![event]);
        let migrated = fs::read_to_string(log.path("legacy")).unwrap();
        assert!(migrated.contains(crate::at_rest::ENVELOPE_FORMAT));
        assert!(!migrated.contains("legacy private workspace"));
    }

    #[test]
    fn interrupted_plaintext_event_migration_retries_without_duplicates() {
        for boundary in [
            PersistenceBoundary::TemporaryFileSynced,
            PersistenceBoundary::DestinationRenamed,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("records.jsonl");
            fs::write(
                &path,
                b"{\"id\":1,\"private\":\"source canary\"}\n{\"id\":2}\n",
            )
            .unwrap();
            let fault = FailOnce::new(boundary);
            assert!(
                read_encrypted_json_lines_recover_tail_with::<serde_json::Value>(
                    &path,
                    &cipher(),
                    EVENT_LOG_PURPOSE,
                    &fault,
                )
                .is_err()
            );
            let recovered: Vec<serde_json::Value> = read(&path).unwrap();
            assert_eq!(recovered.len(), 2);
            assert_eq!(recovered[0]["id"], 1);
            assert_eq!(recovered[1]["id"], 2);
            let encrypted = fs::read_to_string(&path).unwrap();
            assert!(encrypted.contains(crate::at_rest::ENVELOPE_FORMAT));
            assert!(!encrypted.contains("source canary"));
            assert_eq!(encrypted.bytes().filter(|byte| *byte == b'\n').count(), 2);
        }
    }

    #[test]
    fn truncated_final_record_is_removed_without_losing_committed_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("records.jsonl");
        append(&path, &serde_json::json!({"id": 1})).unwrap();
        append(&path, &serde_json::json!({"id": 2})).unwrap();
        let committed_len = fs::metadata(&path).unwrap().len();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"id\":")
            .unwrap();

        let values: Vec<serde_json::Value> = read(&path).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["id"], 1);
        assert_eq!(values[1]["id"], 2);
        assert_eq!(fs::metadata(&path).unwrap().len(), committed_len);
    }

    #[test]
    fn malformed_committed_record_remains_a_hard_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("records.jsonl");
        fs::write(&path, b"{\"id\":1}\n{bad}\n").unwrap();
        assert!(read::<serde_json::Value>(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{\"id\":1}\n{bad}\n");
    }

    #[test]
    fn fault_injected_append_retries_converge_without_duplicate_records() {
        for boundary in [
            PersistenceBoundary::AppendWritten,
            PersistenceBoundary::AppendSynced,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("records.jsonl");
            let value = serde_json::json!({"eventId": "stable-event", "value": 1});
            let fault = FailOnce::new(boundary);
            assert!(
                append_encrypted_json_record_with(
                    &path,
                    &value,
                    &cipher(),
                    EVENT_LOG_PURPOSE,
                    &fault,
                )
                .is_err()
            );
            append(&path, &value).unwrap();
            append(&path, &value).unwrap();
            let values: Vec<serde_json::Value> = read(&path).unwrap();
            assert_eq!(values, vec![value]);
            assert_eq!(
                fs::read(&path)
                    .unwrap()
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count(),
                1
            );
        }
    }

    #[test]
    fn append_recovers_a_stale_unterminated_tail_before_the_next_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("records.jsonl");
        append(&path, &serde_json::json!({"id": 1})).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"interrupted\":")
            .unwrap();
        append(&path, &serde_json::json!({"id": 2})).unwrap();
        let values: Vec<serde_json::Value> = read(&path).unwrap();
        assert_eq!(
            values,
            vec![serde_json::json!({"id": 1}), serde_json::json!({"id": 2})]
        );
    }

    #[test]
    fn event_log_append_is_idempotent_for_the_same_signed_event() {
        let directory = tempfile::tempdir().unwrap();
        let log = LocalEventLog::open(directory.path(), cipher()).unwrap();
        let key = SigningKey::from_bytes(&[9; 32]);
        let event = EventEnvelope::sign(
            "workspace".into(),
            1,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "actor".into(),
            1,
            DomainEvent::WorkspaceCreated {
                name: "Workspace".into(),
                founding_device: codecaddie_domain::DeviceIdentity {
                    actor_id: "actor".into(),
                    device_id: "device".into(),
                    signing_public_key: hex::encode(key.verifying_key().to_bytes()),
                    label: "Device".into(),
                },
                workspace_fingerprint: "fingerprint".into(),
            },
            &key,
        )
        .unwrap();
        log.append("workspace", &event).unwrap();
        log.append("workspace", &event).unwrap();
        assert_eq!(log.load("workspace").unwrap(), vec![event]);
        assert_eq!(
            fs::read(log.path("workspace"))
                .unwrap()
                .iter()
                .filter(|byte| **byte == b'\n')
                .count(),
            1
        );
    }
}
