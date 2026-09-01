//! Crash-safe local persistence primitives.
//!
//! Workspace state is authenticated and encrypted before it is written. The
//! legacy plaintext format is accepted only long enough to perform an atomic,
//! one-way migration with the owner-only local content key.

use crate::at_rest::ContentCipher;
#[cfg(test)]
use crate::at_rest::ENVELOPE_FORMAT;
use serde::{Serialize, de::DeserializeOwned};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const LOCAL_STATE_FILE: &str = "local-state-v2.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceBoundary {
    TemporaryFileSynced,
    QuarantineRenamed,
    DestinationRenamed,
    AppendWritten,
    AppendSynced,
}

pub(crate) trait PersistenceFaultInjector {
    fn checkpoint(&self, _boundary: PersistenceBoundary) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct NoPersistenceFault;

impl PersistenceFaultInjector for NoPersistenceFault {}

#[cfg(test)]
pub(crate) struct FailOnce {
    boundary: PersistenceBoundary,
    fired: std::cell::Cell<bool>,
}

#[cfg(test)]
impl FailOnce {
    pub(crate) fn new(boundary: PersistenceBoundary) -> Self {
        Self {
            boundary,
            fired: std::cell::Cell::new(false),
        }
    }
}

#[cfg(test)]
impl PersistenceFaultInjector for FailOnce {
    fn checkpoint(&self, boundary: PersistenceBoundary) -> io::Result<()> {
        if boundary == self.boundary && !self.fired.replace(true) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("injected persistence interruption at {boundary:?}"),
            ))
        } else {
            Ok(())
        }
    }
}

pub struct LocalStateFile {
    path: PathBuf,
    cipher: ContentCipher,
}

impl LocalStateFile {
    pub(crate) fn for_data_root(root: &Path, cipher: ContentCipher) -> anyhow::Result<Self> {
        Ok(Self {
            path: root.join(LOCAL_STATE_FILE),
            cipher,
        })
    }

    #[cfg(test)]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            cipher: ContentCipher::for_tests(),
        }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
            || replace_sidecars(&self.path).is_ok_and(|sidecars| !sidecars.is_empty())
    }

    pub fn load<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        recover_json_replace::<T>(
            &self.path,
            &self.cipher,
            LOCAL_STATE_FILE,
            &NoPersistenceFault,
        )?;
        let decoded = self
            .cipher
            .open_or_plain(LOCAL_STATE_FILE, &fs::read(&self.path)?)?;
        let value = serde_json::from_slice(&decoded.plaintext)?;
        if !decoded.encrypted {
            let encrypted = self.cipher.seal(LOCAL_STATE_FILE, &decoded.plaintext)?;
            write_private_replace(&self.path, &encrypted)?;
        }
        Ok(value)
    }

    pub fn save<T: Serialize>(&self, value: &T) -> anyhow::Result<()> {
        let plaintext = serde_json::to_vec_pretty(value)?;
        let encrypted = self.cipher.seal(LOCAL_STATE_FILE, &plaintext)?;
        write_private_replace(&self.path, &encrypted)
    }

    #[cfg(test)]
    pub(crate) fn load_with_fault<T: DeserializeOwned>(
        &self,
        injector: &impl PersistenceFaultInjector,
    ) -> anyhow::Result<T> {
        recover_json_replace::<T>(&self.path, &self.cipher, LOCAL_STATE_FILE, injector)?;
        let decoded = self
            .cipher
            .open_or_plain(LOCAL_STATE_FILE, &fs::read(&self.path)?)?;
        let value = serde_json::from_slice(&decoded.plaintext)?;
        if !decoded.encrypted {
            let encrypted = self.cipher.seal(LOCAL_STATE_FILE, &decoded.plaintext)?;
            write_private_replace_with(&self.path, &encrypted, injector)?;
        }
        Ok(value)
    }

    #[cfg(test)]
    pub(crate) fn save_with_fault<T: Serialize>(
        &self,
        value: &T,
        injector: &impl PersistenceFaultInjector,
    ) -> anyhow::Result<()> {
        let plaintext = serde_json::to_vec_pretty(value)?;
        let encrypted = self.cipher.seal(LOCAL_STATE_FILE, &plaintext)?;
        write_private_replace_with(&self.path, &encrypted, injector)
    }
}

pub fn write_private_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn write_private_atomic_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    write_private_atomic_new_with(path, bytes, &NoPersistenceFault)
}

pub(crate) fn write_encrypted_atomic_new(
    path: &Path,
    bytes: &[u8],
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<()> {
    if path.exists() {
        let existing = cipher.open_or_plain(purpose, &fs::read(path)?)?;
        if existing.plaintext == bytes {
            return Ok(());
        }
    }
    write_private_atomic_new(path, &cipher.seal(purpose, bytes)?)
}

pub(crate) fn write_encrypted_replace(
    path: &Path,
    bytes: &[u8],
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<()> {
    write_private_replace(path, &cipher.seal(purpose, bytes)?)
}

pub(crate) fn read_encrypted_migrating(
    path: &Path,
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<Vec<u8>> {
    let decoded = cipher.open_or_plain(purpose, &fs::read(path)?)?;
    if !decoded.encrypted {
        write_encrypted_replace(path, &decoded.plaintext, cipher, purpose)?;
    }
    Ok(decoded.plaintext)
}

/// Protects an existing managed file without interpreting its contents.
///
/// Semantic validation remains the responsibility of the caller that later
/// consumes the decrypted bytes. This helper exists for the one-time startup
/// sweep so unopened maps, pointers, and agent sessions do not remain readable
/// merely because the user has not visited that part of the application yet.
pub(crate) fn protect_file_at_rest(
    path: &Path,
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        anyhow::bail!("managed local-state path is not a regular file");
    }
    let decoded = cipher.open_or_plain(purpose, &fs::read(path)?)?;
    if !decoded.encrypted {
        write_encrypted_replace(path, &decoded.plaintext, cipher, purpose)?;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub(crate) fn write_private_atomic_new_with(
    path: &Path,
    bytes: &[u8],
    injector: &impl PersistenceFaultInjector,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private file path has no parent"))?;
    fs::create_dir_all(parent)?;
    if path.exists() {
        if fs::read(path)? == bytes {
            cleanup_sidecars(path, ".pending")?;
            sync_parent(parent)?;
            return Ok(());
        }
        anyhow::bail!("private destination already exists with different contents");
    }
    let mut pending = sidecars(path, ".pending")?;
    pending.sort();
    pending.reverse();
    if let Some(candidate) = pending
        .iter()
        .find(|candidate| fs::read(candidate).is_ok_and(|existing| existing == bytes))
    {
        rename_new(candidate, path)?;
        set_private_permissions(path)?;
        injector.checkpoint(PersistenceBoundary::DestinationRenamed)?;
        sync_parent(parent)?;
        cleanup_sidecars(path, ".pending")?;
        return Ok(());
    }
    cleanup_sidecars(path, ".pending")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("private file name is invalid"))?;
    let temp = parent.join(format!(".{name}.{}.pending", Uuid::now_v7()));
    write_private_new(&temp, bytes)?;
    injector.checkpoint(PersistenceBoundary::TemporaryFileSynced)?;
    rename_new(&temp, path)?;
    set_private_permissions(path)?;
    injector.checkpoint(PersistenceBoundary::DestinationRenamed)?;
    sync_parent(parent)?;
    Ok(())
}

pub(crate) fn write_private_replace(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    write_private_replace_with(path, bytes, &NoPersistenceFault)
}

pub(crate) fn write_private_replace_with(
    path: &Path,
    bytes: &[u8],
    injector: &impl PersistenceFaultInjector,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private file path has no parent"))?;
    fs::create_dir_all(parent)?;
    cleanup_sidecars(path, ".tmp")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("private file name is invalid"))?;
    let temp = parent.join(format!(".{name}.{}.tmp", Uuid::now_v7()));
    write_private_new(&temp, bytes)?;
    injector.checkpoint(PersistenceBoundary::TemporaryFileSynced)?;
    rename_replace(&temp, path)?;
    set_private_permissions(path)?;
    injector.checkpoint(PersistenceBoundary::DestinationRenamed)?;
    sync_parent(parent)?;
    Ok(())
}

fn recover_json_replace<T: DeserializeOwned>(
    path: &Path,
    cipher: &ContentCipher,
    purpose: &str,
    injector: &impl PersistenceFaultInjector,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private file path has no parent"))?;
    fs::create_dir_all(parent)?;
    if valid_json::<T>(path, cipher, purpose) {
        cleanup_replace_sidecars(path)?;
        return Ok(());
    }
    let mut candidates = replace_sidecars(path)?;
    candidates.sort();
    candidates.reverse();
    let Some(candidate) = candidates
        .iter()
        .find(|candidate| valid_json::<T>(candidate, cipher, purpose))
    else {
        return Ok(());
    };
    if path.exists() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("private file name is invalid"))?;
        let quarantined = parent.join(format!(".{name}.{}.quarantined", Uuid::now_v7()));
        rename_new(path, &quarantined)?;
        sync_parent(parent)?;
        injector.checkpoint(PersistenceBoundary::QuarantineRenamed)?;
    }
    rename_new(candidate, path)?;
    set_private_permissions(path)?;
    injector.checkpoint(PersistenceBoundary::DestinationRenamed)?;
    sync_parent(parent)?;
    cleanup_replace_sidecars(path)?;
    Ok(())
}

fn valid_json<T: DeserializeOwned>(path: &Path, cipher: &ContentCipher, purpose: &str) -> bool {
    fs::read(path)
        .ok()
        .and_then(|bytes| cipher.open_or_plain(purpose, &bytes).ok())
        .and_then(|decoded| serde_json::from_slice::<T>(&decoded.plaintext).ok())
        .is_some()
}

fn replace_sidecars(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = sidecars(path, ".tmp")?;
    paths.extend(sidecars(path, ".quarantined")?);
    Ok(paths)
}

fn cleanup_replace_sidecars(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private file path has no parent"))?;
    let mut removed = false;
    for candidate in replace_sidecars(path)? {
        match fs::remove_file(candidate) {
            Ok(()) => removed = true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if removed {
        sync_parent(parent)?;
    }
    Ok(())
}

fn cleanup_sidecars(path: &Path, suffix: &str) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private file path has no parent"))?;
    let mut removed = false;
    for candidate in sidecars(path, suffix)? {
        match fs::remove_file(candidate) {
            Ok(()) => removed = true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if removed {
        sync_parent(parent)?;
    }
    Ok(())
}

fn sidecars(path: &Path, suffix: &str) -> anyhow::Result<Vec<PathBuf>> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private file path has no parent"))?;
    if !parent.exists() {
        return Ok(Vec::new());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("private file name is invalid"))?;
    let prefix = format!(".{name}.");
    Ok(fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter(|entry| {
            let candidate = entry.file_name();
            let candidate = candidate.to_string_lossy();
            candidate.starts_with(&prefix) && candidate.ends_with(suffix)
        })
        .map(|entry| entry.path())
        .collect())
}

fn set_private_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) fn sync_parent(parent: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

#[cfg(not(windows))]
fn rename_new(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(not(windows))]
fn rename_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn windows_move(source: &Path, destination: &Path, flags: u32) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that live
    // for the duration of the call. MoveFileExW does not retain the pointers.
    let moved = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn rename_new(source: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    windows_move(source, destination, MOVEFILE_WRITE_THROUGH)
}

#[cfg(windows)]
fn rename_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    windows_move(
        source,
        destination,
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Example {
        goal: String,
    }

    #[test]
    fn local_state_is_encrypted_and_reopens() {
        let directory = tempfile::tempdir().unwrap();
        let cipher = ContentCipher::for_tests();
        let store = LocalStateFile::for_data_root(directory.path(), cipher.clone()).unwrap();
        store
            .save(&Example {
                goal: "Improve reliability".into(),
            })
            .unwrap();
        let raw = fs::read_to_string(directory.path().join(LOCAL_STATE_FILE)).unwrap();
        assert!(raw.contains(ENVELOPE_FORMAT));
        assert!(!raw.contains("Improve reliability"));
        let reopened = LocalStateFile::for_data_root(directory.path(), cipher).unwrap();
        assert_eq!(
            reopened.load::<Example>().unwrap(),
            Example {
                goal: "Improve reliability".into()
            }
        );
    }

    #[test]
    fn legacy_plaintext_state_migrates_once_after_successful_validation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCAL_STATE_FILE);
        fs::write(
            &path,
            serde_json::to_vec_pretty(&Example {
                goal: "Legacy private goal".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let store =
            LocalStateFile::for_data_root(directory.path(), ContentCipher::for_tests()).unwrap();
        assert_eq!(store.load::<Example>().unwrap().goal, "Legacy private goal");
        let migrated = fs::read_to_string(path).unwrap();
        assert!(migrated.contains(ENVELOPE_FORMAT));
        assert!(!migrated.contains("Legacy private goal"));
    }

    #[test]
    fn interrupted_plaintext_encryption_migration_retries_without_data_loss() {
        for boundary in [
            PersistenceBoundary::TemporaryFileSynced,
            PersistenceBoundary::DestinationRenamed,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(LOCAL_STATE_FILE);
            fs::write(
                &path,
                serde_json::to_vec(&Example {
                    goal: "Migration must survive interruption".into(),
                })
                .unwrap(),
            )
            .unwrap();
            let store = LocalStateFile::for_data_root(directory.path(), ContentCipher::for_tests())
                .unwrap();
            let fault = FailOnce::new(boundary);
            assert!(store.load_with_fault::<Example>(&fault).is_err());

            let recovered: Example = store.load().unwrap();
            assert_eq!(recovered.goal, "Migration must survive interruption");
            let encrypted = fs::read_to_string(path).unwrap();
            assert!(encrypted.contains(ENVELOPE_FORMAT));
            assert!(!encrypted.contains("Migration must survive interruption"));
            assert!(replace_sidecars(&store.path).unwrap().is_empty());
        }
    }

    #[test]
    fn fault_injected_atomic_new_retries_converge_across_sync_and_rename_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let before_rename = directory.path().join("before-rename.json");
        let fault = FailOnce::new(PersistenceBoundary::TemporaryFileSynced);
        assert!(write_private_atomic_new_with(&before_rename, b"one", &fault).is_err());
        assert!(!before_rename.exists());
        assert_eq!(sidecars(&before_rename, ".pending").unwrap().len(), 1);
        write_private_atomic_new(&before_rename, b"one").unwrap();
        assert_eq!(fs::read(&before_rename).unwrap(), b"one");
        assert!(sidecars(&before_rename, ".pending").unwrap().is_empty());

        let after_rename = directory.path().join("after-rename.json");
        let fault = FailOnce::new(PersistenceBoundary::DestinationRenamed);
        assert!(write_private_atomic_new_with(&after_rename, b"two", &fault).is_err());
        assert_eq!(fs::read(&after_rename).unwrap(), b"two");
        write_private_atomic_new(&after_rename, b"two").unwrap();
        assert!(sidecars(&after_rename, ".pending").unwrap().is_empty());
        assert!(write_private_atomic_new(&after_rename, b"different").is_err());
    }

    #[test]
    fn fault_injected_replacements_retry_without_losing_the_old_or_new_value() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        write_private_replace(&path, br#"{"version":1}"#).unwrap();

        let fault = FailOnce::new(PersistenceBoundary::TemporaryFileSynced);
        assert!(write_private_replace_with(&path, br#"{"version":2}"#, &fault).is_err());
        assert_eq!(fs::read(&path).unwrap(), br#"{"version":1}"#);
        assert_eq!(sidecars(&path, ".tmp").unwrap().len(), 1);
        write_private_replace(&path, br#"{"version":2}"#).unwrap();
        assert_eq!(fs::read(&path).unwrap(), br#"{"version":2}"#);
        assert!(sidecars(&path, ".tmp").unwrap().is_empty());

        let fault = FailOnce::new(PersistenceBoundary::DestinationRenamed);
        assert!(write_private_replace_with(&path, br#"{"version":3}"#, &fault).is_err());
        assert_eq!(fs::read(&path).unwrap(), br#"{"version":3}"#);
        write_private_replace(&path, br#"{"version":3}"#).unwrap();
        assert!(sidecars(&path, ".tmp").unwrap().is_empty());
    }

    #[test]
    fn storage_capacity_failures_preserve_the_committed_value_for_retry() {
        struct StorageFullOnce(std::cell::Cell<bool>);
        impl PersistenceFaultInjector for StorageFullOnce {
            fn checkpoint(&self, boundary: PersistenceBoundary) -> io::Result<()> {
                if boundary == PersistenceBoundary::TemporaryFileSynced && !self.0.replace(true) {
                    return Err(io::Error::new(
                        io::ErrorKind::StorageFull,
                        "injected local storage capacity failure",
                    ));
                }
                Ok(())
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        write_private_replace(&path, br#"{"version":1}"#).unwrap();
        let fault = StorageFullOnce(std::cell::Cell::new(false));
        let error = write_private_replace_with(&path, br#"{"version":2}"#, &fault)
            .expect_err("a capacity failure must fail closed");
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::StorageFull
        );
        assert_eq!(fs::read(&path).unwrap(), br#"{"version":1}"#);

        write_private_replace(&path, br#"{"version":2}"#).unwrap();
        assert_eq!(fs::read(&path).unwrap(), br#"{"version":2}"#);
        assert!(replace_sidecars(&path).unwrap().is_empty());
    }

    #[test]
    fn local_state_recovers_interrupted_quarantine_and_stale_sidecars() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let store = LocalStateFile::new(path.clone());
        fs::write(&path, b"not-json").unwrap();
        let temp = directory.path().join(".state.json.0000000001.tmp");
        write_private_new(
            &temp,
            &serde_json::to_vec(&Example {
                goal: "Recovered".into(),
            })
            .unwrap(),
        )
        .unwrap();

        let fault = FailOnce::new(PersistenceBoundary::QuarantineRenamed);
        assert!(store.load_with_fault::<Example>(&fault).is_err());
        assert!(!path.exists());
        assert!(store.exists(), "recoverable sidecars count as local state");
        assert_eq!(sidecars(&path, ".quarantined").unwrap().len(), 1);

        assert_eq!(
            store.load::<Example>().unwrap(),
            Example {
                goal: "Recovered".into()
            }
        );
        assert!(replace_sidecars(&path).unwrap().is_empty());

        fs::remove_file(&path).unwrap();
        let quarantine = directory.path().join(".state.json.0000000002.quarantined");
        write_private_new(
            &quarantine,
            &serde_json::to_vec(&Example {
                goal: "Quarantine recovery".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(store.load::<Example>().unwrap().goal, "Quarantine recovery");
        assert!(replace_sidecars(&path).unwrap().is_empty());
    }
}
