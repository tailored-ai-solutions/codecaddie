//! Handlers for the `workspace.*` methods: creating, resuming, and opening
//! local workspaces, updating their project context, and exporting readable
//! local recovery material.

use super::{parsed_params, required_workspace};
use crate::{
    local_state::{
        CreateWorkspaceRequest, LocalWorkspaceStore, OpenWorkspaceRequest,
        UpdateWorkspaceContextRequest,
    },
    protocol::{CoreRequest, CoreResponse},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryExportRequest {
    destination: std::path::PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupExportRequest {
    destination: std::path::PathBuf,
    passphrase: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupImportRequest {
    source: std::path::PathBuf,
    repository_path: std::path::PathBuf,
    passphrase: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupScheduleEnableRequest {
    destination_directory: std::path::PathBuf,
    passphrase: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupScheduleRunRequest {
    #[serde(default)]
    force: bool,
}

pub(super) async fn create(request: CoreRequest) -> CoreResponse {
    let create: CreateWorkspaceRequest = match parsed_params(&request.id, request.params) {
        Ok(create) => create,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        let workspace = store.create_workspace(create)?;
        let context_files = store
            .workspace_project_context(&workspace.workspace_id)?
            .context_files;
        Ok((workspace, context_files))
    }) {
        Ok((workspace, context_files)) => CoreResponse::success(
            request.id,
            serde_json::json!({
                "workspaceId": workspace.workspace_id,
                "name": workspace.name,
                "encryptedAtRest": true,
                "storage": "local-encrypted-json",
                "role": "editor",
                "contextFiles": context_files
            }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "workspace_create_failed",
            error.to_string(),
            true,
        ),
    }
}

pub(super) async fn recent(request: CoreRequest) -> CoreResponse {
    match LocalWorkspaceStore::from_environment().and_then(|store| store.recent_workspace()) {
        Ok(workspace) => {
            CoreResponse::success(request.id, serde_json::json!({ "workspace": workspace }))
        }
        Err(error) => CoreResponse::failure(
            request.id,
            "workspace_resume_failed",
            error.to_string(),
            true,
        ),
    }
}

pub(super) async fn open(request: CoreRequest) -> CoreResponse {
    let open: OpenWorkspaceRequest = match parsed_params(&request.id, request.params) {
        Ok(open) => open,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.open_workspace(&open.workspace_id))
    {
        Ok(workspace) => {
            CoreResponse::success(request.id, serde_json::json!({ "workspace": workspace }))
        }
        Err(error) => {
            CoreResponse::failure(request.id, "workspace_open_failed", error.to_string(), true)
        }
    }
}

pub(super) async fn context_update(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create a local workspace before updating its context.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let update: UpdateWorkspaceContextRequest = match parsed_params(&request.id, request.params) {
        Ok(update) => update,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        store.update_workspace_context(&workspace_id, update)?;
        Ok(store
            .workspace_project_context(&workspace_id)?
            .context_files)
    }) {
        Ok(context_files) => CoreResponse::success(
            request.id,
            serde_json::json!({
                "workspaceId": workspace_id,
                "updated": true,
                "contextFiles": context_files
            }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "workspace_context_update_failed",
            error.to_string(),
            true,
        ),
    }
}

pub(super) async fn recovery_export(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create a local workspace before exporting recovery material.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let export: RecoveryExportRequest = match parsed_params(&request.id, request.params) {
        Ok(export) => export,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.export_recovery(&workspace_id, &export.destination))
    {
        Ok(()) => CoreResponse::success(
            request.id,
            serde_json::json!({ "destination": export.destination, "format": "plain-json" }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "recovery_export_failed",
            error.to_string(),
            true,
        ),
    }
}

pub(super) async fn backup_export(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create a local workspace before exporting a portable backup.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let export: BackupExportRequest = match parsed_params(&request.id, request.params) {
        Ok(export) => export,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        store.export_portable_backup(&workspace_id, &export.destination, &export.passphrase)
    }) {
        Ok(receipt) => super::serialized_success(request.id, "portable backup receipt", &receipt),
        Err(error) => {
            CoreResponse::failure(request.id, "backup_export_failed", error.to_string(), false)
        }
    }
}

pub(super) async fn backup_import(request: CoreRequest) -> CoreResponse {
    let import: BackupImportRequest = match parsed_params(&request.id, request.params) {
        Ok(import) => import,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        store.import_portable_backup(&import.source, &import.repository_path, &import.passphrase)
    }) {
        Ok(receipt) => super::serialized_success(request.id, "portable restore receipt", &receipt),
        Err(error) => {
            CoreResponse::failure(request.id, "backup_import_failed", error.to_string(), false)
        }
    }
}

pub(super) async fn backup_schedule_status(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create or restore a local workspace before checking its backup schedule.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.backup_schedule_status(&workspace_id))
    {
        Ok(status) => super::serialized_success(request.id, "backup schedule", &status),
        Err(error) => CoreResponse::failure(
            request.id,
            "backup_schedule_status_failed",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn backup_schedule_enable(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create or restore a local workspace before enabling scheduled backups.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let enable: BackupScheduleEnableRequest = match parsed_params(&request.id, request.params) {
        Ok(enable) => enable,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        store.enable_backup_schedule(
            &workspace_id,
            &enable.destination_directory,
            &enable.passphrase,
        )
    }) {
        Ok(receipt) => super::serialized_success(request.id, "scheduled backup receipt", &receipt),
        Err(error) => CoreResponse::failure(
            request.id,
            "backup_schedule_enable_failed",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn backup_schedule_disable(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create or restore a local workspace before disabling scheduled backups.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.disable_backup_schedule(&workspace_id))
    {
        Ok(status) => super::serialized_success(request.id, "backup schedule", &status),
        Err(error) => CoreResponse::failure(
            request.id,
            "backup_schedule_disable_failed",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn backup_schedule_run(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create or restore a local workspace before running scheduled backups.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let run: BackupScheduleRunRequest = match parsed_params(&request.id, request.params) {
        Ok(run) => run,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.run_scheduled_backup(&workspace_id, run.force))
    {
        Ok(receipt) => super::serialized_success(request.id, "scheduled backup receipt", &receipt),
        Err(error) => CoreResponse::failure(
            request.id,
            "backup_schedule_run_failed",
            error.to_string(),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{error_of, request};

    #[tokio::test]
    async fn create_rejects_missing_params() {
        let response = create(request("workspace.create", None, serde_json::json!({}))).await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn open_rejects_missing_params() {
        let response = open(request("workspace.open", None, serde_json::json!({}))).await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn context_update_requires_a_workspace_scope() {
        let response = context_update(request(
            "workspace.context.update",
            None,
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "workspace_required");
        assert!(!error.retryable);
        assert_eq!(
            error.message,
            "Create a local workspace before updating its context."
        );
    }

    #[tokio::test]
    async fn context_update_rejects_malformed_params_once_scoped() {
        let response = context_update(request(
            "workspace.context.update",
            Some("ws-1"),
            serde_json::json!({ "context": "not-an-object" }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }

    #[tokio::test]
    async fn recovery_export_requires_a_workspace_scope() {
        let response = recovery_export(request(
            "workspace.recovery.export",
            None,
            serde_json::json!({ "destination": "/tmp/recovery" }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "workspace_required");
        assert_eq!(
            error.message,
            "Create a local workspace before exporting recovery material."
        );
    }

    #[tokio::test]
    async fn recovery_export_rejects_missing_params() {
        let response = recovery_export(request(
            "workspace.recovery.export",
            Some("ws-1"),
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }

    #[tokio::test]
    async fn backup_export_requires_a_workspace_scope() {
        let response = backup_export(request(
            "workspace.backup.export",
            None,
            serde_json::json!({ "destination": "/tmp/backup", "passphrase": "correct horse battery staple" }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "workspace_required");
    }

    #[tokio::test]
    async fn backup_import_rejects_missing_secret_params_without_echoing_them() {
        let response = backup_import(request(
            "workspace.backup.import",
            None,
            serde_json::json!({ "passphrase": "private backup phrase" }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.message.contains("private backup phrase"));
    }

    #[tokio::test]
    async fn backup_schedule_methods_require_workspace_scope_and_hide_secrets() {
        for method in [
            "workspace.backup.schedule.status",
            "workspace.backup.schedule.enable",
            "workspace.backup.schedule.disable",
            "workspace.backup.schedule.run",
        ] {
            let response = match method {
                "workspace.backup.schedule.status" => {
                    backup_schedule_status(request(method, None, serde_json::json!({}))).await
                }
                "workspace.backup.schedule.enable" => {
                    backup_schedule_enable(request(
                        method,
                        None,
                        serde_json::json!({ "destinationDirectory": "/tmp", "passphrase": "private backup phrase" }),
                    ))
                    .await
                }
                "workspace.backup.schedule.disable" => {
                    backup_schedule_disable(request(method, None, serde_json::json!({}))).await
                }
                _ => backup_schedule_run(request(method, None, serde_json::json!({}))).await,
            };
            let error = error_of(response);
            assert_eq!(error.code, "workspace_required");
            assert!(!error.message.contains("private backup phrase"));
        }
    }
}
