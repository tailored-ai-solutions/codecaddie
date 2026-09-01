//! Cross-process advisory write locks serializing mutations of the
//! workspace event logs and the device-wide local state file.

use std::{
    fs::{self, OpenOptions},
    path::Path,
};

/// Held while a process mutates a workspace's event log. The OS releases the
/// advisory lock when the file handle closes, including after a process crash.
pub(crate) struct WorkspaceWriteGuard {
    _file: fs::File,
}

impl WorkspaceWriteGuard {
    pub(super) fn acquire(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            _file: acquire_write_lock(path, "workspace")?,
        })
    }
}

/// Serializes read-modify-write updates to the device-wide local state file.
/// Workspace locks cannot protect this file because concurrent processes may
/// be updating different workspaces in the same document.
pub(super) struct LocalStateWriteGuard {
    _file: fs::File,
}

impl LocalStateWriteGuard {
    pub(super) fn acquire(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            _file: acquire_write_lock(path, "local state")?,
        })
    }
}

fn acquire_write_lock(path: &Path, resource: &str) -> anyhow::Result<fs::File> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("the {resource} lock path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let started = std::time::Instant::now();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() > std::time::Duration::from_secs(10) {
                    anyhow::bail!(
                        "another CodeCaddie process is writing to the {resource}; try again"
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_path_without_a_parent_directory_is_a_typed_error_not_a_panic() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let error = acquire_write_lock(Path::new(root), "test local_state")
            .expect_err("a filesystem root cannot hold a lock file");
        assert!(error.to_string().contains("no parent directory"));
    }
}
