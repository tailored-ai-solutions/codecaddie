use super::{parsed_params, required_workspace};
use crate::{
    local_state::LocalWorkspaceStore,
    protocol::{CoreRequest, CoreResponse},
};
use codecaddie_domain::ReliabilityEventKind;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClientReliabilityKind {
    SessionStarted,
    SessionEnded,
    OperationCancelled,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientReliabilityRequest {
    kind: ClientReliabilityKind,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    operation: String,
}

pub(super) async fn record(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Reliability events require a local workspace scope.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let params: ClientReliabilityRequest = match parsed_params(&request.id, request.params) {
        Ok(params) => params,
        Err(failure) => return *failure,
    };
    if matches!(params.kind, ClientReliabilityKind::OperationCancelled)
        && !matches!(
            params.operation.as_str(),
            "scan.run" | "goals.generate" | "map.generate" | "workspace.create"
        )
    {
        return CoreResponse::failure(
            request.id,
            "reliability_event_rejected",
            "Client cancellation recording is limited to core-owned operations.",
            false,
        );
    }
    let session_id = if params.session_id.trim().is_empty() {
        format!("desktop-{}", crate::reliability::new_correlation_id())
    } else {
        params.session_id.clone()
    };
    let result = LocalWorkspaceStore::from_environment().and_then(|store| match params.kind {
        ClientReliabilityKind::SessionStarted => store
            .record_desktop_session(
                &workspace_id,
                ReliabilityEventKind::DesktopSessionStarted,
                &session_id,
            )
            .map(|(crash_detected, correlation_id)| {
                serde_json::json!({
                    "correlationId": correlation_id,
                    "crashDetected": crash_detected,
                    "sessionId": session_id,
                })
            }),
        ClientReliabilityKind::SessionEnded => store
            .record_desktop_session(
                &workspace_id,
                ReliabilityEventKind::DesktopSessionEnded,
                &session_id,
            )
            .map(|(_, correlation_id)| {
                serde_json::json!({
                    "correlationId": correlation_id,
                    "crashDetected": false,
                    "sessionId": session_id,
                })
            }),
        ClientReliabilityKind::OperationCancelled => store
            .record_client_cancellation(&workspace_id, &params.operation)
            .map(|correlation_id| {
                serde_json::json!({
                    "correlationId": correlation_id,
                    "crashDetected": false,
                    "sessionId": session_id,
                })
            }),
    });
    match result {
        Ok(result) => CoreResponse::success(request.id, result),
        Err(error) => CoreResponse::failure(
            request.id,
            "reliability_persistence_failed",
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
    async fn recording_requires_a_workspace_and_allowlisted_event_shape() {
        let response = record(request(
            "reliability.record",
            None,
            serde_json::json!({"kind":"operation_cancelled","operation":"scan.run"}),
        ))
        .await;
        assert_eq!(error_of(response).code, "workspace_required");

        let response = record(request(
            "reliability.record",
            Some("workspace"),
            serde_json::json!({"kind":"operation_cancelled","operation":"arbitrary.user.text"}),
        ))
        .await;
        assert_eq!(error_of(response).code, "reliability_event_rejected");
    }
}
