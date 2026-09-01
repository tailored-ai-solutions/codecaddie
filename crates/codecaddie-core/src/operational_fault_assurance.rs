//! One executable journey through every fault named by the local operational
//! matrix. Faults cross their real provider or persistence boundary, then use
//! the same response finalization and signed reliability store as the app.

use crate::{
    local_state::{CreateWorkspaceRequest, LocalWorkspaceStore, ProjectContext},
    persistence::{
        FailOnce, PersistenceBoundary, PersistenceFaultInjector, write_private_replace,
        write_private_replace_with,
    },
    protocol::{CoreRequest, CoreResponse, PROTOCOL_VERSION},
    provider::{PreparedProvider, ProviderKind, ProviderRunner},
    service::{finalize_response_with, provider_failure_response},
};
use std::{cell::Cell, io, path::Path, time::Duration};

#[cfg(unix)]
fn executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
async fn provider_fault(body: &str, timeout: Duration) -> anyhow::Error {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grok");
    executable(&path, body);
    let prepared = PreparedProvider {
        kind: ProviderKind::Grok,
        executable: path,
        claude_streams: false,
        grok_help:
            "--disable-web-search --no-subagents --tools --max-turns --sandbox --disallowed-tools"
                .into(),
    };
    ProviderRunner { timeout }
        .run_structured_prepared(&prepared, directory.path(), "prompt", "{}", None)
        .await
        .unwrap_err()
}

fn request(method: &str, workspace_id: &str) -> CoreRequest {
    CoreRequest {
        id: format!("fault-{method}"),
        protocol_version: PROTOCOL_VERSION,
        workspace_id: Some(workspace_id.into()),
        method: method.into(),
        params: Default::default(),
    }
}

fn record_failure(
    store: &LocalWorkspaceStore,
    workspace_id: &str,
    method: &str,
    response: CoreResponse,
) -> CoreResponse {
    finalize_response_with(
        &request(method, workspace_id),
        response,
        Duration::from_millis(25),
        |workspace_id, record| store.record_reliability_operation(workspace_id, record),
    )
}

struct StorageFullOnce(Cell<bool>);

impl PersistenceFaultInjector for StorageFullOnce {
    fn checkpoint(&self, boundary: PersistenceBoundary) -> io::Result<()> {
        if boundary == PersistenceBoundary::TemporaryFileSynced && !self.0.replace(true) {
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "injected storage exhaustion",
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn operational_fault_matrix_proves_metrics_errors_and_alerts_end_to_end() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("repository");
    std::fs::create_dir(&repository).unwrap();
    let data_root = directory.path().join("data");
    let store = LocalWorkspaceStore::new(data_root.clone()).unwrap();
    let workspace = store
        .create_workspace(CreateWorkspaceRequest {
            name: "Fault assurance".into(),
            repository_display_name: "repository".into(),
            repository_path: repository.to_string_lossy().into_owned(),
            product_brief: "Exercise content-free local failure records.".into(),
            context: ProjectContext::default(),
        })
        .unwrap();

    let timeout = provider_fault("sleep 2", Duration::from_millis(25)).await;
    let timeout = record_failure(
        &store,
        &workspace.workspace_id,
        "scan.run",
        provider_failure_response("provider-timeout", &timeout),
    );
    assert_eq!(timeout.error.unwrap().code, "provider_timeout");

    // This case proves malformed-output classification, not scheduling speed.
    // Keep its responsive-fixture budget distinct from the intentionally
    // timed-out process above so a loaded full suite cannot change the fault.
    let malformed = provider_fault("printf 'not-json'", Duration::from_secs(10)).await;
    let malformed = record_failure(
        &store,
        &workspace.workspace_id,
        "scan.run",
        provider_failure_response("provider-malformed", &malformed),
    );
    assert_eq!(malformed.error.unwrap().code, "provider_response_invalid");

    let state_path = directory.path().join("customer-state.json");
    write_private_replace(&state_path, br#"{"version":1}"#).unwrap();
    let disk_full = StorageFullOnce(Cell::new(false));
    assert!(write_private_replace_with(&state_path, br#"{"version":2}"#, &disk_full).is_err());
    assert_eq!(std::fs::read(&state_path).unwrap(), br#"{"version":1}"#);
    let disk = record_failure(
        &store,
        &workspace.workspace_id,
        "workspace.recent",
        CoreResponse::failure(
            "disk-exhaustion",
            "storage_write_failed",
            "The local state could not be saved.",
            true,
        ),
    );
    assert_eq!(disk.error.unwrap().code, "storage_write_failed");

    let interrupted = FailOnce::new(PersistenceBoundary::TemporaryFileSynced);
    assert!(write_private_replace_with(&state_path, br#"{"version":3}"#, &interrupted,).is_err());
    assert_eq!(std::fs::read(&state_path).unwrap(), br#"{"version":1}"#);
    let interrupted = record_failure(
        &store,
        &workspace.workspace_id,
        "workspace.recent",
        CoreResponse::failure(
            "interrupted-write",
            "persistence_interrupted",
            "The interrupted local write is safe to retry.",
            true,
        ),
    );
    assert_eq!(interrupted.error.unwrap().code, "persistence_interrupted");

    let telemetry_outage = finalize_response_with(
        &request("workspace.recent", &workspace.workspace_id),
        CoreResponse::success(
            "telemetry-outage",
            serde_json::json!({"workspaceId": workspace.workspace_id}),
        ),
        Duration::from_millis(5),
        |_workspace_id, _record| anyhow::bail!("injected local ledger outage"),
    );
    assert!(telemetry_outage.ok);
    assert_eq!(
        telemetry_outage.result.unwrap()["reliabilityWarning"]["code"],
        "local_reliability_unavailable"
    );

    drop(store);
    let reopened = LocalWorkspaceStore::new(data_root).unwrap();
    let reliability = reopened.recent_workspace().unwrap().unwrap().reliability;
    assert_eq!(reliability.operation_samples, 4);
    assert_eq!(reliability.operation_failures, 4);
    assert_eq!(reliability.alerts_raised, 4);
    assert_eq!(reliability.provider_operation_samples, 2);
    assert_eq!(reliability.provider_operation_failures, 2);
    assert_eq!(reliability.provider_alerts_raised, 2);
    assert_eq!(reliability.availability_percent, Some(0.0));

    let serialized = serde_json::to_string(&reliability).unwrap();
    for forbidden in [
        "repositorySource",
        "attachmentContent",
        "goalText",
        "prompt",
        "credential",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}
