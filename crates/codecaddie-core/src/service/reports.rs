//! Handler for `reports.export_word`: writing the latest persisted report
//! to a Word document. Reports carry paths, line ranges, hashes, and
//! derived claims — never repository source text.

use super::{parsed_params, required_workspace};
use crate::{
    local_state::LocalWorkspaceStore,
    protocol::{CoreRequest, CoreResponse},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportExportRequest {
    destination: std::path::PathBuf,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportHistoryRequest {
    before_event_id: Option<String>,
    #[serde(default = "default_history_limit")]
    limit: usize,
}

fn default_history_limit() -> usize {
    50
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportFindingRequest {
    report_event_id: String,
    goal_version_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportDeleteRequest {
    report_event_id: String,
}

pub(super) async fn history_list(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Open a local workspace before reading report history.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let query: ReportHistoryRequest = match parsed_params(&request.id, request.params) {
        Ok(query) => query,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        store.report_history_page(&workspace_id, query.before_event_id.as_deref(), query.limit)
    }) {
        Ok(page) => super::serialized_success(request.id, "report history", &page),
        Err(error) => CoreResponse::failure(
            request.id,
            "report_history_load_failed",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn finding_get(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Open a local workspace before reading a report finding.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let query: ReportFindingRequest = match parsed_params(&request.id, request.params) {
        Ok(query) => query,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        store.report_finding(
            &workspace_id,
            &query.report_event_id,
            &query.goal_version_id,
        )
    }) {
        Ok(finding) => CoreResponse::success(request.id, serde_json::json!({ "finding": finding })),
        Err(error) => CoreResponse::failure(
            request.id,
            "report_finding_load_failed",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn delete(request: CoreRequest) -> CoreResponse {
    let workspace_id =
        match required_workspace(&request, "Open a local workspace before removing a report.") {
            Ok(workspace_id) => workspace_id,
            Err(failure) => return *failure,
        };
    let deletion: ReportDeleteRequest = match parsed_params(&request.id, request.params) {
        Ok(deletion) => deletion,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.delete_report(&workspace_id, &deletion.report_event_id))
    {
        Ok(()) => CoreResponse::success(
            request.id,
            serde_json::json!({
                "reportEventId": deletion.report_event_id,
                "deleted": true
            }),
        ),
        Err(error) => {
            let message = error.to_string();
            let code = if message.contains("latest report") {
                "latest_report_protected"
            } else if message.contains("already removed") {
                "report_already_deleted"
            } else if message.contains("missing state") || message.contains("required") {
                "report_not_found"
            } else {
                "report_delete_failed"
            };
            CoreResponse::failure(request.id, code, message, false)
        }
    }
}

pub(super) async fn export_word(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Open a local workspace before downloading its report.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let export: ReportExportRequest = match parsed_params(&request.id, request.params) {
        Ok(export) => export,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.export_word_report(&workspace_id, &export.destination))
    {
        Ok(()) => CoreResponse::success(
            request.id,
            serde_json::json!({ "destination": export.destination, "format": "docx" }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "word_report_export_failed",
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
    async fn export_word_requires_a_workspace_scope() {
        let response = export_word(request(
            "reports.export_word",
            None,
            serde_json::json!({ "destination": "/tmp/report.docx" }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "workspace_required");
        assert!(!error.retryable);
        assert_eq!(
            error.message,
            "Open a local workspace before downloading its report."
        );
    }

    #[tokio::test]
    async fn export_word_rejects_missing_params() {
        let response = export_word(request(
            "reports.export_word",
            Some("ws-1"),
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }
}
