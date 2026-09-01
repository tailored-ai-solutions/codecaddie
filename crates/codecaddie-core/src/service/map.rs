//! Handlers for `map.generate` and `map.get`: pre-warming or refreshing
//! the codebase architecture map for a scoped workspace, and reading a
//! recorded map back with its hash-verified body. Maps carry evidence
//! coordinates and derived claims, never repository source text.

use super::{parsed_params, serialized_success};
use crate::{
    analyzer::{self, ScanRepository},
    local_state::LocalWorkspaceStore,
    protocol::{CoreRequest, CoreResponse},
    provider::{ProgressSink, ProviderKind},
    repository::LocalRepository,
};
use codecaddie_domain::FrozenRepository;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MapGenerateParams {
    repositories: Vec<ScanRepository>,
    provider: ProviderKind,
    #[serde(default)]
    refresh: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MapGetParams {
    #[serde(default)]
    map_id: Option<String>,
}

/// Generates (or reuses) the codebase map for the given repositories at
/// their current commits and records it in the workspace ledger. Streaming
/// responses return a slim receipt; non-streaming responses carry the full
/// map.
pub(super) async fn generate(request: CoreRequest, progress: Option<ProgressSink>) -> CoreResponse {
    let params: MapGenerateParams = match parsed_params(&request.id, request.params) {
        Ok(params) => params,
        Err(failure) => return *failure,
    };
    let Some(workspace_id) = request.workspace_id.as_deref() else {
        return CoreResponse::failure(
            request.id,
            "workspace_required",
            "map.generate requires a workspaceId.",
            false,
        );
    };
    let store = match LocalWorkspaceStore::from_environment() {
        Ok(store) => store,
        Err(error) => {
            return CoreResponse::failure(
                request.id,
                "workspace_load_failed",
                error.to_string(),
                true,
            );
        }
    };
    let mut frozen = Vec::with_capacity(params.repositories.len());
    for repository in &params.repositories {
        let resolved =
            LocalRepository::attach(&repository.repository_id, &repository.repository_path)
                .and_then(|local| local.resolve_commit(&repository.commit));
        match resolved {
            Ok(commit_sha) => frozen.push(FrozenRepository {
                repository_id: repository.repository_id.clone(),
                commit_sha,
            }),
            Err(error) => {
                return CoreResponse::failure(
                    request.id,
                    "repository_unavailable",
                    error.to_string(),
                    true,
                );
            }
        }
    }
    let existing = store.load_codebase_map_for(workspace_id, &frozen);
    if !params.refresh
        && let Ok(Some(map)) = existing
    {
        return finish(request.id, map, false, progress.is_some());
    }
    let supersedes = store
        .load_codebase_map_for(workspace_id, &frozen)
        .ok()
        .flatten()
        .map(|map| map.id);
    let generation = analyzer::generate_codebase_map(
        analyzer::MapGenerationRequest {
            map_id: format!("map-{}", Uuid::now_v7()),
            repositories: params.repositories,
            provider: params.provider,
            supersedes,
        },
        progress.clone(),
    )
    .await;
    match generation {
        Ok(map) => {
            if let Err(error) = store.record_codebase_map(workspace_id, &map) {
                return CoreResponse::failure(
                    request.id,
                    "map_persistence_failed",
                    error.to_string(),
                    true,
                );
            }
            finish(request.id, map, true, progress.is_some())
        }
        Err(error) => {
            CoreResponse::failure(request.id, "map_generation_failed", error.to_string(), true)
        }
    }
}

fn finish(
    request_id: String,
    map: codecaddie_domain::CodebaseMap,
    generated: bool,
    streaming: bool,
) -> CoreResponse {
    if streaming {
        CoreResponse::success(
            request_id,
            serde_json::json!({
                "mapId": map.id,
                "generated": generated,
                "partial": map.partial,
                "componentCount": map.components.len(),
                "warnings": map.analysis_warnings,
            }),
        )
    } else {
        serialized_success(request_id, "codebase map", &map)
    }
}

/// Returns a recorded map (the newest, or one named by `mapId`) with its
/// descriptor, after re-verifying the body's content hash.
pub(super) async fn get(request: CoreRequest) -> CoreResponse {
    let params: MapGetParams = match parsed_params(&request.id, request.params) {
        Ok(params) => params,
        Err(failure) => return *failure,
    };
    let Some(workspace_id) = request.workspace_id.as_deref() else {
        return CoreResponse::failure(
            request.id,
            "workspace_required",
            "map.get requires a workspaceId.",
            false,
        );
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.load_codebase_map(workspace_id, params.map_id.as_deref()))
    {
        Ok(Some((descriptor, map))) => serialized_success(
            request.id,
            "codebase map",
            &serde_json::json!({ "descriptor": descriptor, "map": map }),
        ),
        Ok(None) => CoreResponse::failure(
            request.id,
            "map_not_found",
            "No recorded codebase map matches this workspace.",
            false,
        ),
        Err(error) => {
            CoreResponse::failure(request.id, "workspace_load_failed", error.to_string(), true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{error_of, request};

    #[tokio::test]
    async fn generate_requires_params_and_a_workspace() {
        let response = generate(request("map.generate", None, serde_json::json!({})), None).await;
        assert_eq!(error_of(response).code, "invalid_params");

        let response = generate(
            request(
                "map.generate",
                None,
                serde_json::json!({
                    "repositories": [{
                        "repositoryId": "attached-repository",
                        "repositoryPath": "/tmp/does-not-matter",
                    }],
                    "provider": "codex",
                }),
            ),
            None,
        )
        .await;
        assert_eq!(error_of(response).code, "workspace_required");
    }

    #[tokio::test]
    async fn get_requires_a_workspace() {
        let response = get(request("map.get", None, serde_json::json!({}))).await;
        assert_eq!(error_of(response).code, "workspace_required");
    }
}
