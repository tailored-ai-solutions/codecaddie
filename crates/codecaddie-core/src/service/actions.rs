//! Handler for `actions.ready`: marking a recommendation as a ready action
//! inside the scoped workspace.

use super::{parsed_params, required_workspace};
use crate::{
    local_state::{LocalWorkspaceStore, ReadyActionRequest},
    protocol::{CoreRequest, CoreResponse},
};

pub(super) async fn ready(request: CoreRequest) -> CoreResponse {
    let workspace_id = match required_workspace(
        &request,
        "Create a local workspace before changing an action.",
    ) {
        Ok(workspace_id) => workspace_id,
        Err(failure) => return *failure,
    };
    let ready: ReadyActionRequest = match parsed_params(&request.id, request.params) {
        Ok(ready) => ready,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.ready_action(&workspace_id, ready))
    {
        Ok(action) => CoreResponse::success(request.id, serde_json::json!({ "action": action })),
        Err(error) => CoreResponse::failure(
            request.id,
            "action_transition_failed",
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
    async fn ready_requires_a_workspace_scope() {
        let response = ready(request("actions.ready", None, serde_json::json!({}))).await;
        let error = error_of(response);
        assert_eq!(error.code, "workspace_required");
        assert!(!error.retryable);
        assert_eq!(
            error.message,
            "Create a local workspace before changing an action."
        );
    }

    #[tokio::test]
    async fn ready_rejects_missing_params_once_scoped() {
        let response = ready(request(
            "actions.ready",
            Some("ws-1"),
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }
}
