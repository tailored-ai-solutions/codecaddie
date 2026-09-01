//! Handlers for the `goals.*` methods: approving immutable goal versions,
//! replacing an edited goal set after validation, and generating drafts
//! through the selected installed provider.

use super::{parsed_params, required_workspace};
use crate::{
    analyzer::{self, GoalGenerationRequest},
    context_documents,
    local_state::{ApproveGoalRequest, LocalWorkspaceStore, ReplaceGoalsRequest},
    protocol::{CoreRequest, CoreResponse},
    provider::ProgressSink,
};

pub(super) async fn approve(request: CoreRequest) -> CoreResponse {
    let workspace_id =
        match required_workspace(&request, "Create a local workspace before approving goals.") {
            Ok(workspace_id) => workspace_id,
            Err(failure) => return *failure,
        };
    let approval: ApproveGoalRequest = match parsed_params(&request.id, request.params) {
        Ok(approval) => approval,
        Err(failure) => return *failure,
    };
    match (|| {
        analyzer::validate_approved_goal_request(&approval)?;
        LocalWorkspaceStore::from_environment()?.approve_goal(&workspace_id, approval)
    })() {
        Ok(version) => CoreResponse::success(
            request.id,
            serde_json::json!({ "goalVersion": version, "status": "approved" }),
        ),
        Err(error) => {
            CoreResponse::failure(request.id, "goal_approval_failed", error.to_string(), true)
        }
    }
}

pub(super) async fn replace(request: CoreRequest) -> CoreResponse {
    let workspace_id =
        match required_workspace(&request, "Create a local workspace before saving goals.") {
            Ok(workspace_id) => workspace_id,
            Err(failure) => return *failure,
        };
    let replacement: ReplaceGoalsRequest = match parsed_params(&request.id, request.params) {
        Ok(replacement) => replacement,
        Err(failure) => return *failure,
    };
    match (|| {
        let store = LocalWorkspaceStore::from_environment()?;
        let product_brief = store.workspace_product_brief(&workspace_id)?;
        analyzer::validate_edited_goal_set(&replacement.goals, &product_brief)?;
        store.replace_goals(&workspace_id, replacement)
    })() {
        Ok(goals) => CoreResponse::success(
            request.id,
            serde_json::json!({ "goals": goals, "status": "approved" }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "goal_replacement_failed",
            error.to_string(),
            true,
        ),
    }
}

/// Generates goal drafts through the selected provider. `progress` is the
/// NDJSON progress sink when the host opted into streaming, `None` for
/// length-prefixed requests; the terminal response is identical either way.
pub(super) async fn generate(request: CoreRequest, progress: Option<ProgressSink>) -> CoreResponse {
    let workspace_id = request.workspace_id.clone();
    let mut generation: GoalGenerationRequest = match parsed_params(&request.id, request.params) {
        Ok(generation) => generation,
        Err(failure) => return *failure,
    };
    if let Some(workspace_id) = workspace_id {
        let context = match (|| {
            let store = LocalWorkspaceStore::from_environment()?;
            let product_brief = store.workspace_product_brief(&workspace_id)?;
            let project_context = store.workspace_project_context(&workspace_id)?;
            if !project_context.context_file_names.is_empty()
                && project_context.context_files.is_empty()
            {
                anyhow::bail!(
                    "legacy project-context files were saved by name only; reattach them before generating goals"
                );
            }
            let extracted = if project_context.context_files.is_empty() {
                None
            } else {
                Some(context_documents::extract_references(
                    &project_context.context_files,
                )?)
            };
            Ok::<_, anyhow::Error>((product_brief, extracted))
        })() {
            Ok(context) => context,
            Err(error) => {
                return CoreResponse::failure(
                    request.id,
                    "goal_context_unavailable",
                    error.to_string(),
                    true,
                );
            }
        };
        generation.product_brief = context.0;
        generation.extracted_context = context.1;
    }
    match analyzer::generate_goal_drafts(generation, progress).await {
        Ok(result) => CoreResponse::success(
            request.id,
            serde_json::json!({
                "goals": result.goals,
                "contextSourcesUsed": result.context_sources_used,
                "status": "draft"
            }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "goal_generation_failed",
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
    async fn approve_requires_a_workspace_scope() {
        let response = approve(request("goals.approve", None, serde_json::json!({}))).await;
        let error = error_of(response);
        assert_eq!(error.code, "workspace_required");
        assert!(!error.retryable);
        assert_eq!(
            error.message,
            "Create a local workspace before approving goals."
        );
    }

    #[tokio::test]
    async fn approve_rejects_missing_params_once_scoped() {
        let response = approve(request(
            "goals.approve",
            Some("ws-1"),
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }

    #[tokio::test]
    async fn approve_rejects_an_outcome_only_success_check_before_storage() {
        let response = approve(request(
            "goals.approve",
            Some("generic-workspace"),
            serde_json::json!({
                "goalId": "goal-activation",
                "title": "Customers reach value",
                "businessOutcome": "Adoption grows as customers reach a useful result",
                "criteria": ["The onboarding workflow achieves 70% adoption within 30 days"],
                "priority": 4,
                "position": 1,
                "rubricDimensions": ["Business & product"]
            }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "goal_approval_failed");
        assert!(
            error.message.contains("already-achieved"),
            "{}",
            error.message
        );
        assert!(error.message.contains("instrumentation or controls"));
    }

    #[tokio::test]
    async fn replace_requires_a_workspace_scope() {
        let response = replace(request("goals.replace", None, serde_json::json!({}))).await;
        let error = error_of(response);
        assert_eq!(error.code, "workspace_required");
        assert_eq!(
            error.message,
            "Create a local workspace before saving goals."
        );
    }

    #[tokio::test]
    async fn replace_rejects_missing_params_once_scoped() {
        let response = replace(request(
            "goals.replace",
            Some("ws-1"),
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }

    #[tokio::test]
    async fn generate_rejects_missing_params_before_calling_a_provider() {
        let response = generate(request("goals.generate", None, serde_json::json!({})), None).await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn generate_rejects_missing_params_identically_when_streaming() {
        let sink: ProgressSink = std::sync::Arc::new(|_message| {});
        let response = generate(
            request(
                "goals.generate",
                None,
                serde_json::json!({ "stream": true }),
            ),
            Some(sink),
        )
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }
}
