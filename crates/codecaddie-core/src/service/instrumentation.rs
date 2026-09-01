//! Device-local, content-free decision-funnel recording. Authoritative
//! workspace creation, goal approval, and report completion events already
//! live in the signed ledger; this method accepts only the report-open action
//! initiated by the desktop UI.

use super::{parsed_params, required_workspace};
use crate::{
    local_state::LocalWorkspaceStore,
    protocol::{CoreRequest, CoreResponse},
};
use codecaddie_domain::ProductEventKind;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordRequest {
    event: UiDecisionEvent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UiDecisionEvent {
    ReportOpened,
    EvidenceOpened,
}

pub(super) async fn record(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Open a local workspace before recording a decision-funnel action.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let params: RecordRequest = match parsed_params(&request.id, request.params) {
        Ok(params) => params,
        Err(failure) => return *failure,
    };
    let kind = match params.event {
        UiDecisionEvent::ReportOpened => ProductEventKind::ReportRevisited,
        UiDecisionEvent::EvidenceOpened => ProductEventKind::EvidenceOpened,
    };
    let session_id = request.id.clone();
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.record_product_event(&workspace_id, kind, &session_id, None))
    {
        Ok(()) => CoreResponse::success(request.id, serde_json::json!({ "recorded": true })),
        Err(error) => CoreResponse::failure(
            request.id,
            "instrumentation_persistence_failed",
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
    async fn record_requires_a_workspace_and_accepts_only_known_actions() {
        let missing = record(request(
            "instrumentation.record",
            None,
            serde_json::json!({ "event": "report_opened" }),
        ))
        .await;
        assert_eq!(error_of(missing).code, "workspace_required");

        let unknown = record(request(
            "instrumentation.record",
            Some("ws"),
            serde_json::json!({ "event": "analysis_completed" }),
        ))
        .await;
        assert_eq!(error_of(unknown).code, "invalid_params");
    }
}
