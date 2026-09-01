// One executable snapshot matrix for every declared lifecycle exit.

use super::*;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn snapshot_lifecycle_matrix_proves_every_declared_cleanup_path() {
    let (_directory, repository) = repository();
    let frozen_commit = repository.head().unwrap();
    let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis).unwrap();
    let workspace_path = workspace.path().to_path_buf();
    let (directory_name, resolved) = workspace
        .snapshot_repository(0, &repository, &frozen_commit)
        .unwrap();
    let snapshot = workspace.path().join(directory_name);
    let source = snapshot.join("tenant.rs");
    assert_eq!(resolved, frozen_commit);
    assert!(!snapshot.join(".git").exists());
    assert!(fs::metadata(&source).unwrap().permissions().readonly());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&source)
            .is_err()
    );
    assert!(!workspace_path.starts_with(&repository.path));
    drop(workspace);
    assert!(!workspace_path.exists());

    let mut failure_path = None;
    let failure = (|| -> anyhow::Result<()> {
        let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis)?;
        failure_path = Some(workspace.path().to_path_buf());
        anyhow::bail!("simulated provider failure")
    })();
    assert!(failure.is_err());
    assert!(!failure_path.unwrap().exists());

    let map = ProviderSnapshotWorkspace::new(SnapshotPurpose::Map).unwrap();
    let map_path = map.path().to_path_buf();
    fs::write(map.path().join("codecaddie-map.json"), b"{}").unwrap();
    drop(map);
    assert!(!map_path.exists());

    let parent = tempfile::tempdir().unwrap();
    let crashed = [
        "codecaddie-multi-repository-scan-crashed-process",
        "codecaddie-map-crashed-process",
    ];
    for name in crashed {
        let path = parent.path().join(name);
        fs::create_dir(&path).unwrap();
        File::create(path.join(SNAPSHOT_LEASE_FILE)).unwrap();
    }
    let restarted =
        ProviderSnapshotWorkspace::new_in(SnapshotPurpose::Analysis, parent.path()).unwrap();
    for name in crashed {
        assert!(!parent.path().join(name).exists());
    }
    let restarted_path = restarted.path().to_path_buf();
    drop(restarted);
    assert!(!restarted_path.exists());
    let retry = ProviderSnapshotWorkspace::new_in(SnapshotPurpose::Analysis, parent.path()).unwrap();
    let retry_path = retry.path().to_path_buf();
    drop(retry);
    assert!(!retry_path.exists());

    let timeout_path = Arc::new(Mutex::new(None));
    let recorded_timeout_path = Arc::clone(&timeout_path);
    let timed = tokio::time::timeout(std::time::Duration::from_millis(20), async move {
        let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis).unwrap();
        *recorded_timeout_path.lock().unwrap() = Some(workspace.path().to_path_buf());
        std::future::pending::<()>().await;
        drop(workspace);
    })
    .await;
    assert!(timed.is_err());
    assert!(!timeout_path.lock().unwrap().clone().unwrap().exists());

    let cancel_path = Arc::new(Mutex::new(None));
    let recorded_cancel_path = Arc::clone(&cancel_path);
    let (ready, prepared) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis).unwrap();
        *recorded_cancel_path.lock().unwrap() = Some(workspace.path().to_path_buf());
        let _ = ready.send(());
        std::future::pending::<()>().await;
        drop(workspace);
    });
    prepared.await.unwrap();
    let path = cancel_path.lock().unwrap().clone().unwrap();
    assert!(path.exists());
    task.abort();
    let _ = task.await;
    assert!(!path.exists());
}
