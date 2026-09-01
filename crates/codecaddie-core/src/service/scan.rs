//! Handler for `scan.run`: loading the approved goal set for a scoped
//! workspace, running the provider-backed analysis, and persisting the
//! resulting report. Reports carry evidence coordinates and derived
//! claims, never repository source text.

use super::{parsed_params, serialized_success};
use crate::{
    analyzer::{self, ScanRequest},
    local_state::LocalWorkspaceStore,
    protocol::{CoreRequest, CoreResponse},
    provider::ProgressSink,
    repository::LocalRepository,
};
use codecaddie_domain::{CodebaseMap, FrozenRepository};
use uuid::Uuid;

/// Runs one scan. `progress` is the NDJSON progress sink when the host
/// opted into streaming, `None` for length-prefixed requests. Streaming
/// responses are a slim receipt (the report is persisted and re-read
/// through `workspace.recent`) so they always fit a line-framed transport;
/// non-streaming responses carry the full serialized report.
pub(super) async fn run(request: CoreRequest, progress: Option<ProgressSink>) -> CoreResponse {
    let mut scan: ScanRequest = match parsed_params(&request.id, request.params) {
        Ok(scan) => scan,
        Err(failure) => return *failure,
    };
    if let Some(workspace_id) = request.workspace_id.as_deref()
        && let Err(failure) = apply_approved_goals(&mut scan, workspace_id, &request.id)
    {
        return *failure;
    }
    if let Some(workspace_id) = request.workspace_id.as_deref()
        && let Err(error) = LocalWorkspaceStore::from_environment()
            .and_then(|store| store.record_analysis_started(workspace_id, &scan.report_id))
    {
        return CoreResponse::failure(
            request.id,
            "instrumentation_persistence_failed",
            error.to_string(),
            true,
        );
    }
    let persistence_repositories = scan.repositories.clone();
    let streaming = progress.is_some();
    // Phase 0: ensure a codebase map for this frozen repository set — reuse
    // a valid recorded map, otherwise generate one. Map failure degrades
    // the scan to mapless with a warning; it never fails the scan.
    let (codebase_map, map_warning) = match request.workspace_id.as_deref() {
        Some(workspace_id) => ensure_codebase_map(&scan, workspace_id, progress.clone()).await,
        None => (None, None),
    };
    match analyzer::run_scan_with_map(scan, codebase_map.as_ref(), progress).await {
        Ok(mut report) => {
            if let Some(warning) = map_warning {
                report.partial = true;
                report.analysis_warnings.push(warning);
            }
            let recorded = request.workspace_id.is_some();
            if let Some(workspace_id) = request.workspace_id.as_deref()
                && let Err(error) = persistence_repositories
                    .iter()
                    .map(|repository| {
                        LocalRepository::attach(
                            &repository.repository_id,
                            &repository.repository_path,
                        )
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
                    .and_then(|repositories| {
                        LocalWorkspaceStore::from_environment().and_then(|store| {
                            store.record_report_with_repositories(
                                workspace_id,
                                report.clone(),
                                &repositories,
                            )
                        })
                    })
            {
                return CoreResponse::failure(
                    request.id,
                    "report_persistence_failed",
                    error.to_string(),
                    true,
                );
            }
            if streaming {
                CoreResponse::success(
                    request.id,
                    serde_json::json!({
                        "reportId": report.id,
                        "recorded": recorded,
                        "partial": report.partial,
                        "warnings": report.analysis_warnings,
                    }),
                )
            } else {
                serialized_success(request.id, "analysis report", &report)
            }
        }
        Err(error) => super::provider_failure_response(&request.id, &error),
    }
}

/// Resolves the scan's frozen repository set, reuses the newest valid
/// recorded map for it, or generates and records a fresh one. Returns the
/// map to seed the scan with, plus a report warning when the map could not
/// be produced. Every failure path degrades — a scan is never blocked on
/// its map.
async fn ensure_codebase_map(
    scan: &ScanRequest,
    workspace_id: &str,
    progress: Option<ProgressSink>,
) -> (Option<CodebaseMap>, Option<String>) {
    const DEGRADED: &str =
        "The architecture map could not be generated; goal assessments ran without it.";
    let Ok(store) = LocalWorkspaceStore::from_environment() else {
        return (None, Some(DEGRADED.into()));
    };
    let mut frozen = Vec::with_capacity(scan.repositories.len());
    for repository in &scan.repositories {
        let resolved =
            LocalRepository::attach(&repository.repository_id, &repository.repository_path)
                .and_then(|local| local.resolve_commit(&repository.commit));
        match resolved {
            Ok(commit_sha) => frozen.push(FrozenRepository {
                repository_id: repository.repository_id.clone(),
                commit_sha,
            }),
            Err(_) => return (None, Some(DEGRADED.into())),
        }
    }
    let existing = store.load_codebase_map_for(workspace_id, &frozen);
    if !scan.refresh_map
        && let Ok(Some(_)) = &existing
    {
        if let Some(sink) = &progress {
            sink("Reusing the architecture map for this commit".into());
        }
        return (existing.ok().flatten(), None);
    }
    let supersedes = existing.ok().flatten().map(|map| map.id);
    let generation = analyzer::generate_codebase_map(
        analyzer::MapGenerationRequest {
            map_id: format!("map-{}", Uuid::now_v7()),
            repositories: scan.repositories.clone(),
            provider: scan.provider,
            supersedes,
        },
        progress.clone(),
    )
    .await;
    match generation {
        Ok(map) => {
            if store.record_codebase_map(workspace_id, &map).is_err()
                && let Some(sink) = &progress
            {
                sink("The architecture map could not be saved; this scan still uses it".into());
            }
            (Some(map), None)
        }
        Err(_) => {
            if let Some(sink) = &progress {
                sink("The architecture map could not be generated; continuing without it".into());
            }
            (None, Some(DEGRADED.into()))
        }
    }
}

/// Replaces the request's goals and product brief with the workspace's
/// approved goal set, failing when no goal version is approved, when the
/// goal set is not ready for analysis, or when the workspace cannot load.
fn apply_approved_goals(
    scan: &mut ScanRequest,
    workspace_id: &str,
    request_id: &str,
) -> Result<(), Box<CoreResponse>> {
    match LocalWorkspaceStore::from_environment().and_then(|store| {
        let goals = store.approved_goals(workspace_id)?;
        let brief = store.workspace_product_brief(workspace_id)?;
        analyzer::validate_approved_goal_set(&goals, &brief)?;
        Ok((goals, brief))
    }) {
        Ok((approved, brief)) if !approved.is_empty() => {
            scan.goals = approved;
            scan.product_brief = brief;
            Ok(())
        }
        Ok(_) => Err(Box::new(CoreResponse::failure(
            request_id,
            "approved_goals_required",
            "Approve at least one immutable goal version before scanning.",
            false,
        ))),
        Err(error)
            if error
                .to_string()
                .starts_with("goal set is not ready for analysis") =>
        {
            Err(Box::new(CoreResponse::failure(
                request_id,
                "goal_set_incomplete",
                error.to_string(),
                false,
            )))
        }
        Err(error) => Err(Box::new(CoreResponse::failure(
            request_id,
            "workspace_load_failed",
            error.to_string(),
            true,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{error_of, request};

    #[tokio::test]
    async fn run_rejects_missing_params_before_touching_a_repository() {
        let response = run(request("scan.run", None, serde_json::json!({})), None).await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn run_rejects_missing_params_identically_when_streaming() {
        let sink: ProgressSink = std::sync::Arc::new(|_message| {});
        let response = run(
            request("scan.run", None, serde_json::json!({ "stream": true })),
            Some(sink),
        )
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn run_rejects_params_of_the_wrong_shape() {
        let response = run(
            request(
                "scan.run",
                None,
                serde_json::json!({ "reportId": "r-1", "repositories": "not-a-list" }),
            ),
            None,
        )
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }
}
