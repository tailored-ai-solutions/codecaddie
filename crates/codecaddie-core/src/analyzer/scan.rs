//! Scan orchestration: freezing disposable repository clones, batching
//! approved goals through the provider, and degrading unfinished batches
//! into an honest partial report.

use super::{
    RawAnalysis, RawEvidence,
    analysis_contract::{ANALYSIS_SCHEMA, analysis_prompt},
    assurance::repository_assurance_digest,
    materialize_report_for_repositories, provider_workspace_file_count,
    report_materialize::{
        SourceNarrativePolicy, materialize_report_for_repositories_with_source_policy,
    },
};
use crate::{
    provider::{ProgressSink, ProviderActivity, ProviderKind, ProviderRunner, display_file_count},
    repository::{LocalRepository, ProviderSnapshotWorkspace, SnapshotPurpose},
};
use codecaddie_domain::{CodebaseMap, GoalVersion, Report, ReportOrigin};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    ops::Range,
    sync::Arc,
    time::Duration,
};

const GOALS_PER_PROVIDER_BATCH: usize = 2;
const MAX_CONCURRENT_PROVIDER_BATCHES: usize = 2;
const MAX_RETRIED_PROVIDER_BATCHES: usize = 5;
const SOURCE_CLEAN_RETRY_INSTRUCTION: &str = "\n\nCORRECTION PASS: The first result was unusable because it omitted evidence or a Partial rationale copied repository wording. Reassess every criterion in this same batch. Read each criterion's inspectFirst paths before any broad search. For every Partial verdict, name the exact declared clause still missing or contradicted in original, source-clean language. If the routed evidence directly proves every declared clause, return Supported. Do not quote or paraphrase repository prose, do not carry forward the earlier verdict, and do not add requirements beyond the frozen criterion. Return the complete batch JSON again.";

fn provider_batch_ranges(goal_count: usize) -> Vec<Range<usize>> {
    (0..goal_count)
        .step_by(GOALS_PER_PROVIDER_BATCH)
        .map(|start| start..(start + GOALS_PER_PROVIDER_BATCH).min(goal_count))
        .collect()
}

async fn run_batch_attempts<T, E, Run, RunFuture>(
    batch_indices: Vec<usize>,
    concurrency: usize,
    attempt: u8,
    run: Run,
) -> Vec<(usize, Result<T, E>)>
where
    Run: Fn(usize, u8) -> RunFuture + Clone,
    RunFuture: Future<Output = Result<T, E>>,
{
    stream::iter(batch_indices.into_iter().map(|batch_index| {
        let run = run.clone();
        async move { (batch_index, run(batch_index, attempt).await) }
    }))
    .buffer_unordered(concurrency.max(1))
    .collect()
    .await
}

/// Runs every batch once, then retries at most `retry_limit` nonfatal failures
/// exactly once. Production attempts are individually wall-clock bounded by
/// `ProviderRunner`, so four batches at concurrency two plus two concurrent
/// retries have a hard upper bound of six provider windows for the maximum
/// nine-goal contract.
async fn run_batches_with_one_retry<T, E, Run, RunFuture, RetryableError, RetryableResult>(
    batch_count: usize,
    concurrency: usize,
    retry_limit: usize,
    run: Run,
    retryable_error: RetryableError,
    retryable_result: RetryableResult,
) -> Vec<(usize, Result<T, E>)>
where
    Run: Fn(usize, u8) -> RunFuture + Clone,
    RunFuture: Future<Output = Result<T, E>>,
    RetryableError: Fn(&E) -> bool,
    RetryableResult: Fn(&T) -> bool,
{
    let initial = run_batch_attempts((0..batch_count).collect(), concurrency, 1, run.clone()).await;
    let mut retry_indices = initial
        .iter()
        .filter_map(|(batch_index, result)| match result {
            Ok(value) if retryable_result(value) => Some(*batch_index),
            Err(error) if retryable_error(error) => Some(*batch_index),
            _ => None,
        })
        .collect::<Vec<_>>();
    retry_indices.sort_unstable();
    retry_indices.truncate(retry_limit);
    let retries = run_batch_attempts(retry_indices, concurrency, 2, run).await;
    let mut results = initial.into_iter().collect::<BTreeMap<_, _>>();
    for (batch_index, result) in retries {
        let initial_succeeded = results
            .get(&batch_index)
            .is_some_and(std::result::Result::is_ok);
        if result.is_ok() || !initial_succeeded {
            results.insert(batch_index, result);
        }
    }
    results.into_iter().collect()
}

fn provider_batch_result_needs_retry(
    value: &serde_json::Value,
    repositories: &[(LocalRepository, String)],
) -> bool {
    let Ok(batch) = serde_json::from_value::<RawAnalysis>(value.clone()) else {
        return true;
    };
    let criteria = batch
        .assessments
        .iter()
        .flat_map(|assessment| &assessment.criteria)
        .collect::<Vec<_>>();
    if batch.assessments.is_empty()
        || criteria.is_empty()
        || criteria
            .iter()
            .all(|criterion| criterion.evidence.is_empty())
    {
        return true;
    }
    let partial_rationales = criteria
        .iter()
        .filter(|criterion| criterion.verdict == codecaddie_domain::Verdict::Partial)
        .map(|criterion| criterion.rationale.clone())
        .collect::<Vec<_>>();
    !partial_rationales.is_empty()
        && repositories.iter().any(|(repository, commit)| {
            repository
                .narrative_fields_matching_source(commit, &partial_rationales)
                .is_ok_and(|matches| !matches.is_empty())
        })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    pub report_id: String,
    pub repositories: Vec<ScanRepository>,
    pub provider: ProviderKind,
    pub goals: Vec<GoalVersion>,
    #[serde(default)]
    pub product_brief: String,
    /// Regenerate the codebase map even when a valid cached map exists for
    /// this frozen repository set.
    #[serde(default)]
    pub refresh_map: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRepository {
    pub repository_id: String,
    pub repository_path: std::path::PathBuf,
    #[serde(default = "default_commit")]
    pub commit: String,
}

fn default_commit() -> String {
    "HEAD".into()
}

pub async fn run_scan(
    request: ScanRequest,
    progress: Option<ProgressSink>,
) -> anyhow::Result<Report> {
    run_scan_with_map(request, None, progress).await
}

/// Runs a scan whose goal batches are seeded with a validated codebase map:
/// the full map is written into the disposable workspace as
/// `codecaddie-map.json`, and each batch prompt carries a bounded digest
/// index pointing at it. The map is already-validated derived data
/// (coordinates and prose, never source), and the provider sees the full
/// source anyway, so placing it in the workspace adds zero exposure.
pub async fn run_scan_with_map(
    request: ScanRequest,
    codebase_map: Option<&CodebaseMap>,
    progress: Option<ProgressSink>,
) -> anyhow::Result<Report> {
    if request.goals.is_empty() {
        anyhow::bail!("a scan requires at least one approved goal version");
    }
    if request.repositories.is_empty() {
        anyhow::bail!("a scan requires at least one repository");
    }
    for goal in &request.goals {
        goal.validate().map_err(anyhow::Error::msg)?;
    }
    if let Some(sink) = &progress {
        sink("Preparing a disposable repository clone".to_string());
    }
    let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis)?;
    let mut frozen = Vec::with_capacity(request.repositories.len());
    let mut repository_locations = Vec::with_capacity(request.repositories.len());
    let mut repository_ids = BTreeSet::new();
    for (index, attachment) in request.repositories.iter().enumerate() {
        if attachment.repository_id.trim().is_empty()
            || !repository_ids.insert(attachment.repository_id.clone())
        {
            anyhow::bail!("repository IDs must be nonempty and unique");
        }
        let repository =
            LocalRepository::attach(&attachment.repository_id, &attachment.repository_path)?;
        let (directory_name, commit) =
            workspace.snapshot_repository(index, &repository, &attachment.commit)?;
        repository_locations.push((attachment.repository_id.clone(), directory_name));
        frozen.push((repository, commit));
    }
    let map_digest = if let Some(map) = codebase_map {
        std::fs::write(
            workspace.path().join("codecaddie-map.json"),
            serde_json::to_vec_pretty(map)?,
        )?;
        Some(map_prompt_digest(map))
    } else {
        None
    };
    let repository_file_total = provider_workspace_file_count(workspace.path())?;
    if let Some(sink) = &progress {
        sink(format!(
            "Repository snapshot ready: {} files available to the provider",
            display_file_count(repository_file_total)
        ));
    }
    let runner = Arc::new(ProviderRunner {
        timeout: Duration::from_secs(10 * 60),
    });
    let prepared = runner.prepare(request.provider).await?;
    let batch_ranges = provider_batch_ranges(request.goals.len());
    let batch_count = batch_ranges.len();
    let batch_prompts = batch_ranges
        .iter()
        .map(|range| {
            let goals = &request.goals[range.clone()];
            let assurance_digest =
                repository_assurance_digest(workspace.path(), &repository_locations, goals);
            analysis_prompt(
                goals,
                &repository_locations,
                &request.product_brief,
                request.provider,
                map_digest.as_deref(),
                assurance_digest.as_deref(),
            )
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let batch_prompts = Arc::new(batch_prompts);
    let run_batch = {
        let progress = progress.clone();
        let provider = request.provider;
        let runner = Arc::clone(&runner);
        let prepared = prepared.clone();
        let workspace_path = workspace.path().to_path_buf();
        move |batch_index: usize, attempt: u8| {
            let progress = progress.clone();
            let runner = Arc::clone(&runner);
            let prepared = prepared.clone();
            let workspace_path = workspace_path.clone();
            let mut prompt = batch_prompts[batch_index].clone();
            if attempt > 1 {
                prompt.push_str(SOURCE_CLEAN_RETRY_INSTRUCTION);
            }
            async move {
                if let Some(sink) = &progress {
                    if attempt == 1 {
                        sink(format!(
                            "Asking {} to assess goal batch {} of {}",
                            provider.executable(),
                            batch_index + 1,
                            batch_count
                        ));
                    } else {
                        sink(format!(
                            "Retrying {} goal batch {} of {} once after a recoverable provider result",
                            provider.executable(),
                            batch_index + 1,
                            batch_count
                        ));
                    }
                }
                runner
                    .run_structured_prepared_with_activity(
                        &prepared,
                        &workspace_path,
                        &prompt,
                        ANALYSIS_SCHEMA,
                        progress,
                        ProviderActivity {
                            phase: Some(if attempt == 1 {
                                format!("Goal batch {} of {}", batch_index + 1, batch_count)
                            } else {
                                format!("Goal batch {} of {} retry", batch_index + 1, batch_count)
                            }),
                            repository_file_total: Some(repository_file_total),
                        },
                    )
                    .await
            }
        }
    };
    let batch_runs = run_batches_with_one_retry(
        batch_count,
        MAX_CONCURRENT_PROVIDER_BATCHES,
        MAX_RETRIED_PROVIDER_BATCHES,
        run_batch,
        |error| !provider_batch_is_fatal(error),
        |value| provider_batch_result_needs_retry(value, &frozen),
    )
    .await;

    let mut raw = RawAnalysis {
        provider_version: String::new(),
        assessments: Vec::new(),
        architecture: Vec::new(),
        recommendations: Vec::new(),
    };
    let mut successful_batches = 0_usize;
    let mut degraded_batches = Vec::new();
    for (batch_index, result) in batch_runs {
        let value = match result {
            Ok(value) => value,
            Err(error) if !provider_batch_is_fatal(&error) => {
                degraded_batches.push(format!("batch {}: {error}", batch_index + 1));
                if let Some(sink) = &progress {
                    sink(format!(
                        "Goal batch {} could not finish; its unanswered items will remain unverified",
                        batch_index + 1
                    ));
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut batch: RawAnalysis = match serde_json::from_value(value) {
            Ok(batch) => batch,
            Err(error) => {
                degraded_batches.push(format!(
                    "batch {}: provider result did not match the analysis schema ({error})",
                    batch_index + 1
                ));
                if let Some(sink) = &progress {
                    sink(format!(
                        "Goal batch {} returned an invalid result; its items will remain unverified",
                        batch_index + 1
                    ));
                }
                continue;
            }
        };
        if batch.assessments.is_empty() {
            degraded_batches.push(format!(
                "batch {}: provider returned no goal assessments",
                batch_index + 1
            ));
            if let Some(sink) = &progress {
                sink(format!(
                    "Goal batch {} returned no assessments; its items will remain unverified",
                    batch_index + 1
                ));
            }
            continue;
        }
        let criterion_decisions = batch
            .assessments
            .iter()
            .map(|assessment| assessment.criteria.len())
            .sum::<usize>();
        let submitted_citations = batch
            .assessments
            .iter()
            .flat_map(|assessment| &assessment.criteria)
            .map(|criterion| criterion.evidence.len())
            .sum::<usize>();
        normalize_raw_evidence_paths(&mut batch, &repository_locations);
        let prefix = format!("batch-{}", batch_index + 1);
        for claim in &mut batch.architecture {
            claim.id = format!("{prefix}-{}", claim.id);
        }
        for recommendation in &mut batch.recommendations {
            recommendation.id = format!("{prefix}-{}", recommendation.id);
        }
        let batch_goals = &request.goals[batch_ranges[batch_index].clone()];
        let returned_goal_ids = batch
            .assessments
            .iter()
            .map(|assessment| assessment.goal_version_id.as_str())
            .collect::<BTreeSet<_>>();
        if batch_goals
            .iter()
            .any(|goal| !returned_goal_ids.contains(goal.id.as_str()))
        {
            degraded_batches.push(format!(
                "batch {}: provider omitted one or more requested goals",
                batch_index + 1
            ));
        }
        // Transient validation gate only: a Skip-policy batch report can
        // carry unscreened provider narrative and must never be persisted or
        // returned (see `SourceNarrativePolicy`). Only the raw batch fields
        // merge into the final report, which is materialized with `Redact`.
        let validated_batch = match materialize_report_for_repositories_with_source_policy(
            format!("{}-batch-{}", request.report_id, batch_index + 1),
            &frozen,
            format!("{:?}", request.provider).to_lowercase(),
            ReportOrigin::Scan,
            batch_goals,
            batch.clone(),
            SourceNarrativePolicy::Skip,
        ) {
            Ok(report) => report,
            Err(error) => {
                degraded_batches.push(format!(
                    "batch {}: provider result failed evidence and privacy validation ({error})",
                    batch_index + 1
                ));
                if let Some(sink) = &progress {
                    sink(format!(
                        "Goal batch {} returned evidence that could not be validated; its items will remain unverified",
                        batch_index + 1
                    ));
                }
                continue;
            }
        };
        if let Some(sink) = &progress {
            let bound = validated_batch
                .assessments
                .iter()
                .flat_map(|assessment| &assessment.criteria)
                .map(|criterion| criterion.evidence.len())
                .sum::<usize>();
            sink(format!(
                "Goal batch {} validated {} of {} citations across {} criteria",
                batch_index + 1,
                bound,
                submitted_citations,
                criterion_decisions
            ));
        }
        successful_batches += 1;
        if raw.provider_version.is_empty() {
            raw.provider_version = std::mem::take(&mut batch.provider_version);
        }
        raw.assessments.extend(batch.assessments);
        raw.architecture.extend(batch.architecture);
        raw.recommendations.extend(batch.recommendations);
    }
    if raw.provider_version.is_empty() {
        raw.provider_version = format!("{} bounded run", request.provider.executable());
    }
    let goal_priorities = request
        .goals
        .iter()
        .map(|goal| (goal.id.as_str(), goal.priority))
        .collect::<BTreeMap<_, _>>();
    raw.architecture.sort_by(|left, right| {
        max_goal_priority(&right.affected_goal_version_ids, &goal_priorities)
            .cmp(&max_goal_priority(
                &left.affected_goal_version_ids,
                &goal_priorities,
            ))
            .then_with(|| left.component.cmp(&right.component))
            .then_with(|| left.summary.cmp(&right.summary))
    });
    // Batches produce claims independently, so duplicates are usually not
    // adjacent after the priority sort; dedup against everything kept so far.
    let mut seen_claims = BTreeSet::new();
    raw.architecture.retain(|claim| {
        seen_claims.insert((claim.component.to_lowercase(), claim.summary.to_lowercase()))
    });
    let mut truncation_warnings = Vec::new();
    let stale_recommendations = retain_recommendations_for_goal_gaps(&mut raw, &request.goals);
    if stale_recommendations > 0 {
        truncation_warnings.push(format!(
            "{stale_recommendations} recommendations linked only to fully supported goals were discarded."
        ));
    }
    if raw.architecture.len() > MAX_REPORT_ARCHITECTURE_CLAIMS {
        truncation_warnings.push(format!(
            "{} architecture claims beyond the {MAX_REPORT_ARCHITECTURE_CLAIMS}-claim report limit were discarded, keeping the claims linked to the highest-priority goals.",
            raw.architecture.len() - MAX_REPORT_ARCHITECTURE_CLAIMS
        ));
    }
    raw.architecture.truncate(MAX_REPORT_ARCHITECTURE_CLAIMS);
    raw.recommendations.sort_by(|left, right| {
        max_goal_priority(&right.goal_version_ids, &goal_priorities)
            .cmp(&max_goal_priority(&left.goal_version_ids, &goal_priorities))
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| left.title.cmp(&right.title))
    });
    let mut seen_recommendations = BTreeSet::new();
    raw.recommendations.retain(|recommendation| {
        seen_recommendations.insert((
            recommendation.title.to_lowercase(),
            recommendation.rationale.to_lowercase(),
        ))
    });
    if raw.recommendations.len() > MAX_REPORT_RECOMMENDATIONS {
        truncation_warnings.push(format!(
            "{} recommendations beyond the {MAX_REPORT_RECOMMENDATIONS}-item report limit were discarded, keeping the items linked to the highest-priority goals.",
            raw.recommendations.len() - MAX_REPORT_RECOMMENDATIONS
        ));
    }
    raw.recommendations.truncate(MAX_REPORT_RECOMMENDATIONS);
    for (index, recommendation) in raw.recommendations.iter_mut().enumerate() {
        recommendation.rank = index as u32 + 1;
    }
    let mut report = materialize_report_for_repositories(
        request.report_id,
        &frozen,
        format!("{:?}", request.provider).to_lowercase(),
        ReportOrigin::Scan,
        &request.goals,
        raw,
    )?;
    report.analysis_warnings.extend(truncation_warnings);
    apply_batch_degradation(&mut report, successful_batches, &degraded_batches);
    if let Some(map) = codebase_map {
        report.codebase_map_id = Some(map.id.clone());
        report.codebase_map_hash = map.content_hash().ok();
        validate_component_references(&mut report, map);
    }
    Ok(report)
}

/// The bounded prompt index of a validated map: the system summary plus one
/// line per component (id, kind, name, owned paths). Deliberately an index,
/// not a document — an earlier regression came from stuffing whole
/// documents with their own output contracts into structured prompts.
pub(crate) fn map_prompt_digest(map: &CodebaseMap) -> String {
    const MAX_DIGEST_BYTES: usize = 4 * 1024;
    let mut digest = String::new();
    digest.push_str(&map.overview.system_summary);
    digest.push('\n');
    for component in &map.components {
        let line = format!(
            "{} [{}] {} — {}\n",
            component.id,
            serde_json::to_value(component.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default(),
            component.name,
            component.root_paths.join(", "),
        );
        if digest.len() + line.len() > MAX_DIGEST_BYTES {
            break;
        }
        digest.push_str(&line);
    }
    digest
}

/// Drops component references the seeding map does not declare, so a wrong
/// or hallucinated component id never reaches the ledger.
fn validate_component_references(report: &mut Report, map: &CodebaseMap) {
    let known: BTreeSet<&str> = map
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect();
    for assessment in &mut report.assessments {
        assessment
            .related_component_ids
            .retain(|id| known.contains(id.as_str()));
    }
    for claim in &mut report.architecture {
        if claim
            .component_id
            .as_deref()
            .is_some_and(|id| !known.contains(id))
        {
            claim.component_id = None;
        }
    }
}

/// Report-level caps applied after merging all goal batches. Each batch is
/// still schema-capped at five claims and five recommendations, but a
/// multi-batch scan may keep up to twelve distinct architecture claims —
/// truncating to five silently discarded most of the architectural signal a
/// nine-goal scan produces.
const MAX_REPORT_ARCHITECTURE_CLAIMS: usize = 12;
const MAX_REPORT_RECOMMENDATIONS: usize = 5;

fn apply_batch_degradation(
    report: &mut Report,
    successful_batches: usize,
    degraded_batches: &[String],
) {
    report.partial |= !degraded_batches.is_empty();
    report.analysis_warnings.extend(
        degraded_batches
            .iter()
            .filter_map(|warning| warning.split(':').next())
            .map(|batch| format!("{batch} did not return a complete provider result")),
    );
    if successful_batches == 0 && !degraded_batches.is_empty() {
        report.partial = true;
        report.analysis_warnings.push(
            "No goal batch returned a valid provider result; every goal remains unverified.".into(),
        );
    }
}

fn max_goal_priority(goal_ids: &[String], priorities: &BTreeMap<&str, u8>) -> u8 {
    goal_ids
        .iter()
        .filter_map(|id| priorities.get(id.as_str()).copied())
        .max()
        .unwrap_or_default()
}

fn retain_recommendations_for_goal_gaps(
    analysis: &mut RawAnalysis,
    goals: &[GoalVersion],
) -> usize {
    let goals_with_gaps = goals
        .iter()
        .filter_map(|goal| {
            let assessment = analysis
                .assessments
                .iter()
                .find(|assessment| assessment.goal_version_id == goal.id);
            let fully_supported = assessment.is_some_and(|assessment| {
                assessment.criteria.len() == goal.criteria.len()
                    && goal.criteria.iter().all(|criterion| {
                        assessment.criteria.iter().any(|decision| {
                            decision.criterion_id == criterion.id
                                && decision.verdict == codecaddie_domain::Verdict::Supported
                        })
                    })
            });
            (!fully_supported).then_some(goal.id.as_str())
        })
        .collect::<BTreeSet<_>>();
    let before = analysis.recommendations.len();
    analysis.recommendations.retain(|recommendation| {
        recommendation
            .goal_version_ids
            .iter()
            .any(|goal_id| goals_with_gaps.contains(goal_id.as_str()))
    });
    before - analysis.recommendations.len()
}

fn normalize_raw_evidence_paths(
    analysis: &mut RawAnalysis,
    repository_locations: &[(String, String)],
) {
    let normalize = |evidence: &mut RawEvidence| {
        let Some((_, directory)) = repository_locations
            .iter()
            .find(|(repository_id, _)| *repository_id == evidence.repository_id)
        else {
            return;
        };
        let prefix = format!("{directory}/");
        if let Some(relative) = evidence.path.strip_prefix(&prefix) {
            evidence.path = relative.to_string();
        }
    };
    for evidence in analysis
        .assessments
        .iter_mut()
        .flat_map(|assessment| &mut assessment.criteria)
        .flat_map(|criterion| &mut criterion.evidence)
        .chain(
            analysis
                .architecture
                .iter_mut()
                .flat_map(|claim| &mut claim.evidence),
        )
        .chain(
            analysis
                .recommendations
                .iter_mut()
                .flat_map(|recommendation| &mut recommendation.evidence),
        )
    {
        normalize(evidence);
    }
}

fn provider_batch_is_fatal(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_lowercase();
    [
        "is not installed",
        "could not be started",
        "needs authentication",
        "account or usage limit",
        "required read-only permissions",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_domain::{
        CodebaseMap, Component, ComponentKind, MAP_SCHEMA_VERSION, MapOverview, component_id,
    };

    #[test]
    fn provider_batches_bound_each_turn_to_two_goals() {
        assert_eq!(provider_batch_ranges(0), Vec::<Range<usize>>::new());
        assert_eq!(provider_batch_ranges(1), vec![0..1]);
        assert_eq!(provider_batch_ranges(2), vec![0..2]);
        assert_eq!(provider_batch_ranges(8), vec![0..2, 2..4, 4..6, 6..8]);
        assert!(
            provider_batch_ranges(9)
                .iter()
                .all(|range| range.len() <= GOALS_PER_PROVIDER_BATCH)
        );
        assert!(MAX_RETRIED_PROVIDER_BATCHES >= provider_batch_ranges(9).len());
    }

    #[test]
    fn source_clean_retry_contract_is_focused_and_cannot_carry_a_verdict() {
        assert!(SOURCE_CLEAN_RETRY_INSTRUCTION.contains("inspectFirst"));
        assert!(SOURCE_CLEAN_RETRY_INSTRUCTION.contains("exact declared clause"));
        assert!(SOURCE_CLEAN_RETRY_INSTRUCTION.contains("return Supported"));
        assert!(SOURCE_CLEAN_RETRY_INSTRUCTION.contains("do not carry forward"));
        assert!(!SOURCE_CLEAN_RETRY_INSTRUCTION.contains("repository source"));
        assert!(SOURCE_CLEAN_RETRY_INSTRUCTION.len() < 700);
    }

    #[test]
    fn recommendations_are_retained_only_for_real_goal_gaps() {
        let (_directory, _repository, _commit, goal) = fixture();
        let mut supported = raw();
        assert_eq!(
            retain_recommendations_for_goal_gaps(&mut supported, std::slice::from_ref(&goal)),
            1
        );
        assert!(supported.recommendations.is_empty());

        let mut partial = raw();
        partial.assessments[0].criteria[0].verdict = Verdict::Partial;
        assert_eq!(
            retain_recommendations_for_goal_gaps(&mut partial, std::slice::from_ref(&goal)),
            0
        );
        assert_eq!(partial.recommendations.len(), 1);

        let mut missing = raw();
        missing.assessments.clear();
        assert_eq!(
            retain_recommendations_for_goal_gaps(&mut missing, &[goal]),
            0
        );
        assert_eq!(missing.recommendations.len(), 1);
    }

    #[tokio::test]
    async fn recoverable_provider_batches_retry_once_without_retrying_fatal_failures() {
        use std::sync::Mutex;

        let attempts = Arc::new(Mutex::new(BTreeMap::<usize, u8>::new()));
        let observed = Arc::clone(&attempts);
        let results = run_batches_with_one_retry(
            4,
            2,
            2,
            move |batch_index, attempt| {
                let observed = Arc::clone(&observed);
                async move {
                    observed.lock().unwrap().insert(batch_index, attempt);
                    match (batch_index, attempt) {
                        (0, _) => Ok(batch_index),
                        (1, 1) | (3, _) => Err("retryable"),
                        (1, 2) => Ok(batch_index),
                        (2, _) => Err("fatal"),
                        _ => unreachable!(),
                    }
                }
            },
            |error| *error == "retryable",
            |_| false,
        )
        .await;

        assert_eq!(results[0], (0, Ok(0)));
        assert_eq!(results[1], (1, Ok(1)));
        assert_eq!(results[2], (2, Err("fatal")));
        assert_eq!(results[3], (3, Err("retryable")));
        assert_eq!(
            *attempts.lock().unwrap(),
            BTreeMap::from([(0, 1), (1, 2), (2, 1), (3, 2)])
        );
    }

    #[tokio::test]
    async fn semantically_empty_provider_batches_retry_once() {
        let attempts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = Arc::clone(&attempts);
        let results = run_batches_with_one_retry(
            2,
            2,
            2,
            move |batch_index, attempt| {
                let observed = Arc::clone(&observed);
                async move {
                    observed.lock().unwrap().push((batch_index, attempt));
                    if batch_index == 1 && attempt == 1 {
                        Ok("empty")
                    } else {
                        Ok("evidence")
                    }
                }
            },
            |_: &&str| false,
            |value| *value == "empty",
        )
        .await;

        assert_eq!(results, vec![(0, Ok("evidence")), (1, Ok("evidence"))]);
        assert_eq!(*attempts.lock().unwrap(), vec![(0, 1), (1, 1), (1, 2)]);
    }

    #[tokio::test]
    async fn failed_rewrite_preserves_the_usable_initial_result() {
        let results = run_batches_with_one_retry(
            1,
            1,
            1,
            |_batch_index, attempt| async move {
                if attempt == 1 {
                    Ok("source-matched partial")
                } else {
                    Err("provider timeout")
                }
            },
            |_: &&str| true,
            |value| *value == "source-matched partial",
        )
        .await;

        assert_eq!(results, vec![(0, Ok("source-matched partial"))]);
    }

    #[test]
    fn zero_citation_provider_result_is_recoverable_but_evidence_is_not() {
        let mut result = serde_json::json!({
            "providerVersion": "codex",
            "assessments": [{
                "goalVersionId": "goal-v1",
                "summary": "No repository evidence was returned.",
                "architectureNarrative": null,
                "relatedComponentIds": [],
                "criteria": [{
                    "criterionId": "criterion-1",
                    "verdict": "unverified",
                    "rationale": "The repository could not be assessed.",
                    "confidence": 0.1,
                    "evidence": []
                }]
            }],
            "architecture": [],
            "recommendations": []
        });

        assert!(provider_batch_result_needs_retry(&result, &[]));
        result["assessments"][0]["criteria"][0]["evidence"] = serde_json::json!([{
            "repositoryId": "repo",
            "path": "src/lib.rs",
            "startLine": 1,
            "endLine": 2,
            "kind": "implementation"
        }]);
        assert!(!provider_batch_result_needs_retry(&result, &[]));
    }

    #[test]
    fn source_matched_partial_rationale_gets_one_clean_rewrite_attempt() {
        let directory = tempfile::tempdir().unwrap();
        let run_git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        };
        run_git(&["init", "-q"]);
        run_git(&["config", "user.email", "scan-retry@invalid.example"]);
        run_git(&["config", "user.name", "Scan Retry"]);
        std::fs::write(
            directory.path().join("transport.txt"),
            "local transport remains encrypted across every boundary\n",
        )
        .unwrap();
        run_git(&["add", "transport.txt"]);
        run_git(&["commit", "-qm", "fixture"]);
        let commit = String::from_utf8(
            std::process::Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        let repository = LocalRepository::attach("repo", directory.path()).unwrap();
        let mut result = serde_json::json!({
            "providerVersion": "codex",
            "assessments": [{
                "goalVersionId": "goal-v1",
                "summary": "The repository has partial transport support.",
                "architectureNarrative": null,
                "relatedComponentIds": [],
                "criteria": [{
                    "criterionId": "criterion-1",
                    "verdict": "partial",
                    "rationale": "local transport remains encrypted across every boundary",
                    "confidence": 0.7,
                    "evidence": [{
                        "repositoryId": "repo",
                        "path": "transport.txt",
                        "startLine": 1,
                        "endLine": 1,
                        "kind": "test"
                    }]
                }]
            }],
            "architecture": [],
            "recommendations": []
        });

        assert!(provider_batch_result_needs_retry(
            &result,
            &[(repository.clone(), commit.clone())]
        ));
        result["assessments"][0]["criteria"][0]["rationale"] =
            serde_json::json!("The bounded review found one precise missing clause.");
        assert!(!provider_batch_result_needs_retry(
            &result,
            &[(repository, commit)]
        ));
    }

    fn seeded_map() -> CodebaseMap {
        CodebaseMap {
            id: "map-1".into(),
            schema_version: MAP_SCHEMA_VERSION,
            generated_at: time::OffsetDateTime::UNIX_EPOCH,
            repositories: vec![],
            provider: "codex".into(),
            provider_version: "test".into(),
            origin: ReportOrigin::Scan,
            overview: MapOverview {
                system_summary: "A billing service with one core component.".into(),
                architecture_style: "Modular".into(),
                technologies: vec![],
            },
            components: vec![Component {
                id: component_id("repo", "Billing"),
                name: "Billing".into(),
                kind: ComponentKind::Service,
                repository_id: "repo".into(),
                root_paths: vec!["tenant.rs".into()],
                responsibility: "Scopes invoices.".into(),
                key_interfaces: vec![],
                concerns: vec![],
                evidence: vec![],
            }],
            relationships: vec![],
            data_flows: vec![],
            entry_points: vec![],
            partial: false,
            analysis_warnings: vec![],
            supersedes: None,
        }
    }

    #[test]
    fn map_digests_are_bounded_component_indexes() {
        let digest = map_prompt_digest(&seeded_map());
        assert!(digest.contains("A billing service"));
        assert!(digest.contains(&component_id("repo", "Billing")));
        assert!(digest.contains("[service] Billing"));
        assert!(digest.len() <= 4 * 1024 + 800);
    }

    #[test]
    fn unknown_component_references_never_reach_the_ledger() {
        let (_directory, repository, commit, goal) = fixture();
        let mut output = raw();
        output.assessments[0].architecture_narrative =
            Some("The Billing component supports this goal.".into());
        output.assessments[0].related_component_ids = Some(vec![
            component_id("repo", "Billing"),
            "component-hallucinated-0000".into(),
        ]);
        output.architecture[0].component_id = Some("component-hallucinated-0000".into());
        let mut report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            output,
        )
        .unwrap();
        validate_component_references(&mut report, &seeded_map());
        assert_eq!(
            report.assessments[0].related_component_ids,
            vec![component_id("repo", "Billing")]
        );
        assert_eq!(report.architecture[0].component_id, None);
        assert_eq!(
            report.assessments[0].architecture_narrative,
            "The Billing component supports this goal."
        );
    }
    use crate::analyzer::{
        materialize_report,
        test_support::{fixture, raw},
    };
    use codecaddie_domain::{Criterion, Verdict};

    #[test]
    fn disposable_repository_prefixes_are_removed_before_evidence_binding() {
        let (_directory, repository, commit, goal) = fixture();
        let mut output = raw();
        for evidence in output
            .assessments
            .iter_mut()
            .flat_map(|assessment| &mut assessment.criteria)
            .flat_map(|criterion| &mut criterion.evidence)
            .chain(
                output
                    .architecture
                    .iter_mut()
                    .flat_map(|claim| &mut claim.evidence),
            )
            .chain(
                output
                    .recommendations
                    .iter_mut()
                    .flat_map(|recommendation| &mut recommendation.evidence),
            )
        {
            evidence.path = format!("repository-0/{}", evidence.path);
        }

        normalize_raw_evidence_paths(&mut output, &[("repo".into(), "repository-0".into())]);
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            output,
        )
        .unwrap();

        assert_eq!(report.assessments[0].verdict, Verdict::Supported);
        assert!(!report.assessments[0].criteria[0].evidence.is_empty());
        assert_eq!(
            report.assessments[0].criteria[0].evidence[0].path,
            "tenant.rs"
        );
    }

    #[test]
    fn bounded_provider_omissions_become_unverified_instead_of_losing_the_report() {
        let (_directory, repository, commit, mut goal) = fixture();
        goal.criteria.push(Criterion {
            id: "criterion-2".into(),
            text: "Cross-tenant requests fail closed".into(),
        });
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Grok,
            &[goal.clone()],
            raw(),
        )
        .unwrap();
        let missing = report.assessments[0]
            .criteria
            .iter()
            .find(|criterion| criterion.criterion_id == "criterion-2")
            .unwrap();
        assert_eq!(missing.verdict, Verdict::Unverified);
        assert!(missing.rationale.contains("bounded run ended"));

        let mut no_assessments = raw();
        no_assessments.assessments.clear();
        let mut report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Grok,
            &[goal],
            no_assessments,
        )
        .unwrap();
        assert_eq!(report.assessments[0].verdict, Verdict::Unverified);
        assert!(report.assessments[0].summary.contains("bounded run ended"));
        apply_batch_degradation(
            &mut report,
            0,
            &["batch 1: provider result did not match the analysis schema".into()],
        );
        assert!(report.partial);
        assert!(
            report
                .assessments
                .iter()
                .all(|item| item.verdict == Verdict::Unverified)
        );
        assert!(
            report.analysis_warnings.iter().any(|warning| {
                warning.contains("No goal batch returned a valid provider result")
            })
        );
    }
}
