//! Report materialization: turning raw provider output into a validated
//! report with bound immutable evidence, narrative limits, and the
//! source-excerpt and credential leak defenses.

use super::{RawAnalysis, RawEvidence};
use crate::{provider::ProviderKind, repository::LocalRepository};
use codecaddie_domain::{
    ArchitectureClaim, CriterionAssessment, EvidenceRef, FrozenRepository, GoalAssessment,
    GoalVersion, Recommendation, Report, ReportOrigin, Verdict, aggregate_goal, score_report,
};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn one_sentence(value: &str) -> String {
    let normalized = one_line(value);
    let mut output = String::with_capacity(normalized.len());
    let mut chars = normalized.chars().peekable();
    while let Some(character) = chars.next() {
        if matches!(character, '.' | '!' | '?') && chars.peek() == Some(&' ') {
            output.push(';');
            let _ = chars.next();
            output.push(' ');
            continue;
        }
        output.push(character);
    }
    output
}

fn direct_summary(assessment: &GoalAssessment, body: &str) -> String {
    let has_evidence = assessment
        .criteria
        .iter()
        .any(|criterion| !criterion.evidence.is_empty());
    let prefix = match assessment.verdict {
        Verdict::Supported => "Yes",
        Verdict::Partial => "Partly",
        Verdict::Unsupported if has_evidence => "No",
        Verdict::Unsupported => "Could not find evidence",
        Verdict::Unverified => "Could not verify",
    };
    format!("{prefix} — {body}")
}

pub fn materialize_report(
    report_id: String,
    repository: &LocalRepository,
    commit: &str,
    provider: ProviderKind,
    goals: &[GoalVersion],
    raw: RawAnalysis,
) -> anyhow::Result<Report> {
    materialize_report_for_repositories(
        report_id,
        &[(repository.clone(), commit.into())],
        format!("{provider:?}").to_lowercase(),
        ReportOrigin::Scan,
        goals,
        raw,
    )
}

pub(crate) fn materialize_report_for_repositories(
    report_id: String,
    repositories: &[(LocalRepository, String)],
    provider: String,
    origin: ReportOrigin,
    goals: &[GoalVersion],
    raw: RawAnalysis,
) -> anyhow::Result<Report> {
    materialize_report_for_repositories_with_source_policy(
        report_id,
        repositories,
        provider,
        origin,
        goals,
        raw,
        SourceNarrativePolicy::Redact,
    )
}

pub(crate) fn materialize_agent_report_for_repositories(
    report_id: String,
    repositories: &[(LocalRepository, String)],
    provider: String,
    goals: &[GoalVersion],
    raw: RawAnalysis,
) -> anyhow::Result<Report> {
    materialize_report_for_repositories_with_source_policy(
        report_id,
        repositories,
        provider,
        ReportOrigin::AgentSession,
        goals,
        raw,
        SourceNarrativePolicy::Reject,
    )
}

/// How report materialization treats provider narrative that matches
/// repository source text (see [`reject_source_excerpts`]).
///
/// Scan-batch invariant: [`run_scan`](super::run_scan) materializes every
/// intermediate goal batch with `Skip`, purely as an evidence-binding
/// validation gate, so a batch report may still carry provider narrative
/// verbatim — including quoted source. Batch reports must therefore never
/// leave `run_scan` and must never be persisted. Only the raw analysis
/// fields of a validated batch are merged into the aggregate, and that
/// aggregate is materialized again through
/// [`materialize_report_for_repositories`], which always applies `Redact`
/// before the final report reaches `LocalWorkspaceStore::record_report`.
/// The regression test `intermediate_scan_batch_reports_are_never_persisted`
/// locks this invariant in.
///
/// Redact versus fail closed: a scan aggregates several provider batches, so
/// failing closed on one overquoting narrative would discard every validated
/// verdict and evidence binding from the whole run. Persisted scan reports
/// therefore redact — the offending narrative is replaced with neutral
/// validated wording while verdicts, evidence coordinates, and scores are
/// retained, and the report is marked partial with a warning. Interactive
/// agent-session reports use `Reject` and fail closed instead, because a
/// session can rerun its analysis immediately and nothing is lost by
/// refusing the report outright.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceNarrativePolicy {
    /// No source-narrative screening. Only valid for the transient per-batch
    /// validation reports inside [`run_scan`](super::run_scan); never for a
    /// report that is returned to a caller or persisted.
    Skip,
    /// Replace source-matching narrative with neutral validated wording and
    /// mark the report partial. The policy for every persisted scan report.
    Redact,
    /// Fail closed on source-matching narrative. The policy for
    /// agent-session reports.
    Reject,
}

pub(super) fn materialize_report_for_repositories_with_source_policy(
    report_id: String,
    repositories: &[(LocalRepository, String)],
    provider: String,
    origin: ReportOrigin,
    goals: &[GoalVersion],
    raw: RawAnalysis,
    source_narrative_policy: SourceNarrativePolicy,
) -> anyhow::Result<Report> {
    if repositories.is_empty() {
        anyhow::bail!("report materialization requires a frozen repository");
    }
    validate_raw_narrative_limits(&raw)?;
    let mut report_has_gaps = goals.iter().any(|goal| {
        raw.assessments
            .iter()
            .find(|assessment| assessment.goal_version_id == goal.id)
            .is_none_or(|assessment| {
                goal.criteria.iter().any(|criterion| {
                    !assessment
                        .criteria
                        .iter()
                        .any(|returned| returned.criterion_id == criterion.id)
                })
            })
    });
    let goal_ids: BTreeSet<String> = goals.iter().map(|goal| goal.id.clone()).collect();
    let criteria: BTreeMap<&str, BTreeSet<String>> = goals
        .iter()
        .map(|goal| {
            (
                goal.id.as_str(),
                goal.criteria.iter().map(|item| item.id.clone()).collect(),
            )
        })
        .collect();
    let mut seen_goals = BTreeSet::new();
    let mut assessments = Vec::with_capacity(raw.assessments.len());
    for goal in raw.assessments {
        let summary = one_sentence(&goal.summary);
        if summary.is_empty() || summary.chars().count() > 240 {
            anyhow::bail!("goal assessment summary must be 1 to 240 characters");
        }
        let allowed_criteria = criteria
            .get(goal.goal_version_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("provider returned an unknown goal version"))?;
        if !seen_goals.insert(goal.goal_version_id.clone()) {
            anyhow::bail!("provider returned a duplicate goal assessment");
        }
        let mut seen_criteria = BTreeSet::new();
        let mut items = Vec::with_capacity(goal.criteria.len());
        let mut invalid_evidence = false;
        for criterion in goal.criteria {
            if !allowed_criteria.contains(criterion.criterion_id.as_str())
                || !seen_criteria.insert(criterion.criterion_id.clone())
            {
                anyhow::bail!("provider returned an unknown or duplicate criterion");
            }
            if !(0.0..=1.0).contains(&criterion.confidence) || criterion.rationale.trim().is_empty()
            {
                anyhow::bail!("criterion rationale and confidence are invalid");
            }
            let submitted_evidence = !criterion.evidence.is_empty();
            let binding = bind_evidence(repositories, criterion.evidence);
            let evidence_required =
                matches!(criterion.verdict, Verdict::Supported | Verdict::Partial);
            let evidence_unavailable = (submitted_evidence && binding.evidence.is_empty())
                || (evidence_required && binding.evidence.is_empty());
            invalid_evidence |= evidence_unavailable;
            let rationale = if evidence_unavailable {
                // Say why the citation failed: an oversized range, a stale
                // path, and a hallucinated file are different failures, and
                // the reader cannot act on an undifferentiated message.
                match binding.first_failure() {
                    Some(reason) => format!(
                        "CodeCaddie could not bind the provider's citation to this frozen commit ({reason}), so this criterion remains unverified."
                    ),
                    None => "The provider returned this verdict without any citation, so this criterion remains unverified.".into(),
                }
            } else {
                criterion.rationale
            };
            items.push(CriterionAssessment {
                criterion_id: criterion.criterion_id,
                verdict: if evidence_unavailable {
                    Verdict::Unverified
                } else {
                    criterion.verdict
                },
                rationale,
                confidence: if evidence_unavailable {
                    0.0
                } else {
                    criterion.confidence
                },
                evidence: binding.evidence,
            });
        }
        if seen_criteria != *allowed_criteria {
            invalid_evidence = true;
            for criterion_id in allowed_criteria.difference(&seen_criteria) {
                items.push(CriterionAssessment {
                    criterion_id: criterion_id.clone(),
                    verdict: Verdict::Unverified,
                    rationale: "The provider did not return an assessment for this criterion before the bounded run ended.".into(),
                    confidence: 0.0,
                    evidence: Vec::new(),
                });
            }
        }
        report_has_gaps |= invalid_evidence;
        let mut related_component_ids = goal.related_component_ids.unwrap_or_default();
        related_component_ids.retain(|id| !id.trim().is_empty());
        related_component_ids.dedup();
        related_component_ids.truncate(4);
        let mut assessment = GoalAssessment {
            goal_version_id: goal.goal_version_id,
            verdict: Verdict::Unverified,
            summary: String::new(),
            architecture_narrative: one_sentence(
                goal.architecture_narrative.as_deref().unwrap_or(""),
            ),
            related_component_ids,
            criteria: items,
        };
        assessment.verdict = aggregate_goal(&assessment);
        let summary = if invalid_evidence {
            if assessment
                .criteria
                .iter()
                .any(|criterion| !criterion.evidence.is_empty())
            {
                "Some repository evidence was validated, but one or more submitted citations could not be bound to this frozen commit."
            } else {
                "CodeCaddie could not bind the provider's submitted citations to this frozen commit."
            }
        } else {
            &summary
        };
        assessment.summary = direct_summary(&assessment, summary);
        assessments.push(assessment);
    }
    for goal in goals {
        if seen_goals.contains(&goal.id) {
            continue;
        }
        report_has_gaps = true;
        assessments.push(GoalAssessment {
            goal_version_id: goal.id.clone(),
            verdict: Verdict::Unverified,
            summary: "Could not verify — The provider did not return an assessment for this goal before the bounded run ended.".into(),
            architecture_narrative: String::new(),
            related_component_ids: Vec::new(),
            criteria: goal
                .criteria
                .iter()
                .map(|criterion| CriterionAssessment {
                    criterion_id: criterion.id.clone(),
                    verdict: Verdict::Unverified,
                    rationale: "The provider did not return an assessment for this criterion before the bounded run ended.".into(),
                    confidence: 0.0,
                    evidence: Vec::new(),
                })
                .collect(),
        });
    }
    let goal_order = goals
        .iter()
        .enumerate()
        .map(|(index, goal)| (goal.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    assessments.sort_by_key(|assessment| {
        goal_order
            .get(assessment.goal_version_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });

    // A claim or recommendation that loses its evidence is dropped, but
    // never silently: each drop is named in the report warnings so validated
    // provider work cannot vanish without a trace.
    let mut decision_warnings: Vec<String> = Vec::new();
    let architecture = raw
        .architecture
        .into_iter()
        .map(|claim| {
            validate_goal_ids(&goal_ids, &claim.affected_goal_version_ids)?;
            let binding = bind_evidence(repositories, claim.evidence);
            if claim.summary.trim().is_empty() {
                decision_warnings.push(format!(
                    "Architecture claim \"{}\" was dropped: it had no summary.",
                    clipped_one_line(&claim.component, 80)
                ));
                return Ok(None);
            }
            if binding.evidence.is_empty() {
                let reason = binding
                    .first_failure()
                    .unwrap_or_else(|| "no citation was submitted".into());
                decision_warnings.push(format!(
                    "Architecture claim \"{}\" was dropped: {reason}.",
                    clipped_one_line(&claim.component, 80)
                ));
                return Ok(None);
            }
            Ok(Some(ArchitectureClaim {
                id: claim.id,
                component: claim.component,
                relationship: claim.relationship,
                summary: claim.summary,
                affected_goal_version_ids: claim.affected_goal_version_ids,
                component_id: claim
                    .component_id
                    .filter(|component| !component.trim().is_empty()),
                evidence: binding.evidence,
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let recommendations = raw
        .recommendations
        .into_iter()
        .map(|recommendation| {
            validate_goal_ids(&goal_ids, &recommendation.goal_version_ids)?;
            let binding = bind_evidence(repositories, recommendation.evidence);
            if recommendation.rank == 0 || recommendation.expected_business_impact.trim().is_empty()
            {
                decision_warnings.push(format!(
                    "Recommendation \"{}\" was dropped: it was missing a rank or expected impact.",
                    clipped_one_line(&recommendation.title, 80)
                ));
                return Ok(None);
            }
            if binding.evidence.is_empty() {
                let reason = binding
                    .first_failure()
                    .unwrap_or_else(|| "no citation was submitted".into());
                decision_warnings.push(format!(
                    "Recommendation \"{}\" was dropped: {reason}.",
                    clipped_one_line(&recommendation.title, 80)
                ));
                return Ok(None);
            }
            Ok(Some(Recommendation {
                id: recommendation.id,
                title: recommendation.title,
                rationale: recommendation.rationale,
                expected_business_impact: recommendation.expected_business_impact,
                goal_version_ids: recommendation.goal_version_ids,
                evidence: binding.evidence,
                rank: recommendation.rank,
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    let score = score_report(goals, &assessments);
    let goal_version_ids = goals.iter().map(|goal| goal.id.clone()).collect::<Vec<_>>();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&serde_json::to_vec(goals)?);
    let mut report = Report {
        id: report_id,
        completed_at: OffsetDateTime::now_utc(),
        repositories: repositories
            .iter()
            .map(|(repository, commit)| FrozenRepository {
                repository_id: repository.id.clone(),
                commit_sha: commit.clone(),
            })
            .collect(),
        goal_version_ids,
        goal_set_hash: hasher.finalize().to_hex().to_string(),
        provider,
        provider_version: raw.provider_version,
        origin,
        assessments,
        architecture,
        recommendations,
        coverage: score.coverage,
        unverified_criteria: score.unverified_criteria,
        partial: report_has_gaps,
        analysis_warnings: report_has_gaps
            .then(|| {
                "The provider omitted or failed validation for one or more goal criteria.".into()
            })
            .into_iter()
            .chain(decision_warnings)
            .collect(),
        codebase_map_id: None,
        codebase_map_hash: None,
    };
    if source_narrative_policy != SourceNarrativePolicy::Skip {
        match screen_provider_narrative(&report, repositories)? {
            NarrativeScreening::Clean => {}
            NarrativeScreening::Credential => {
                if source_narrative_policy == SourceNarrativePolicy::Reject {
                    return Err(ProviderNarrativeViolation::Credential.into());
                }
                redact_provider_narrative(&mut report, goals);
            }
            NarrativeScreening::Source(violations) => {
                if source_narrative_policy == SourceNarrativePolicy::Reject {
                    return Err(ProviderNarrativeViolation::Source.into());
                }
                let fields = narrative_fields(&report);
                // A handful of matches is a quoting slip in otherwise valid
                // narrative; redact only those fields. Systemic quoting
                // (over a quarter of all fields) forfeits the whole
                // narrative, exactly as before.
                if violations.len() * 4 > fields.len() {
                    redact_provider_narrative(&mut report, goals);
                } else {
                    let targets = violations
                        .iter()
                        .filter_map(|index| fields.get(*index).map(|(field, _)| *field))
                        .collect::<Vec<_>>();
                    redact_narrative_fields(&mut report, goals, &targets);
                }
            }
        }
    }
    Ok(report)
}

fn validate_raw_narrative_limits(raw: &RawAnalysis) -> anyhow::Result<()> {
    let within = |value: &str, maximum: usize| value.chars().count() <= maximum;
    if !within(&raw.provider_version, 120)
        || raw.assessments.iter().any(|assessment| {
            !within(&assessment.summary, 240)
                || !within(
                    assessment.architecture_narrative.as_deref().unwrap_or(""),
                    700,
                )
                || assessment
                    .criteria
                    .iter()
                    .any(|criterion| !within(&criterion.rationale, 900))
        })
        || raw.architecture.iter().any(|claim| {
            !within(&claim.component, 160)
                || claim
                    .relationship
                    .as_deref()
                    .is_some_and(|value| !within(value, 240))
                || !within(&claim.summary, 480)
        })
        || raw.recommendations.iter().any(|recommendation| {
            !within(&recommendation.title, 220)
                || !within(&recommendation.rationale, 900)
                || !within(&recommendation.expected_business_impact, 640)
        })
    {
        anyhow::bail!("provider narrative exceeded a report field limit");
    }
    Ok(())
}

/// Reports may retain conclusions and coordinates, but never code. This
/// catches a provider attempting to quote its cited lines inside a rationale,
/// architecture summary, or recommendation even though the schema has no
/// source-excerpt field.
#[derive(Debug, thiserror::Error)]
enum ProviderNarrativeViolation {
    #[error("provider response contained credential-shaped text")]
    Credential,
    #[error("provider response contained repository source")]
    Source,
}

fn clipped_one_line(value: &str, maximum: usize) -> String {
    one_line(value).chars().take(maximum).collect()
}

fn linked_goal<'a>(goal_ids: &[String], goals: &'a [GoalVersion]) -> Option<&'a GoalVersion> {
    goal_ids
        .iter()
        .find_map(|goal_id| goals.iter().find(|goal| goal.id == *goal_id))
}

fn merge_unique_evidence(target: &mut Vec<EvidenceRef>, additional: Vec<EvidenceRef>) {
    for evidence in additional {
        if !target.contains(&evidence) {
            target.push(evidence);
        }
    }
}

fn consolidate_redacted_decisions(report: &mut Report, goals: &[GoalVersion]) {
    let mut architecture = Vec::<ArchitectureClaim>::new();
    for mut claim in report.architecture.drain(..) {
        let linked_goal_id =
            linked_goal(&claim.affected_goal_version_ids, goals).map(|goal| goal.id.as_str());
        let existing = linked_goal_id.and_then(|goal_id| {
            architecture.iter_mut().find(|candidate| {
                candidate
                    .affected_goal_version_ids
                    .iter()
                    .any(|candidate_id| candidate_id == goal_id)
            })
        });
        if let Some(existing) = existing {
            merge_unique_evidence(&mut existing.evidence, claim.evidence);
            for goal_id in claim.affected_goal_version_ids.drain(..) {
                if !existing.affected_goal_version_ids.contains(&goal_id) {
                    existing.affected_goal_version_ids.push(goal_id);
                }
            }
        } else {
            architecture.push(claim);
        }
    }
    report.architecture = architecture;

    report.recommendations.sort_by_key(|item| item.rank);
    let mut recommendations = Vec::<Recommendation>::new();
    for mut recommendation in report.recommendations.drain(..) {
        let linked_goal_id =
            linked_goal(&recommendation.goal_version_ids, goals).map(|goal| goal.id.as_str());
        let existing = linked_goal_id.and_then(|goal_id| {
            recommendations.iter_mut().find(|candidate| {
                candidate
                    .goal_version_ids
                    .iter()
                    .any(|candidate_id| candidate_id == goal_id)
            })
        });
        if let Some(existing) = existing {
            merge_unique_evidence(&mut existing.evidence, recommendation.evidence);
            for goal_id in recommendation.goal_version_ids.drain(..) {
                if !existing.goal_version_ids.contains(&goal_id) {
                    existing.goal_version_ids.push(goal_id);
                }
            }
        } else {
            recommendations.push(recommendation);
        }
    }
    report.recommendations = recommendations;
}

fn neutral_criterion_rationale(criterion: &CriterionAssessment) -> &'static str {
    match criterion.verdict {
        Verdict::Supported => "Validated repository evidence supports this criterion.",
        Verdict::Partial => {
            "Validated repository evidence supports part of this criterion; material coverage remains incomplete."
        }
        Verdict::Unsupported if criterion.evidence.is_empty() => {
            "The bounded review did not find repository evidence for this criterion."
        }
        Verdict::Unsupported => {
            "Validated repository evidence shows this criterion is not satisfied."
        }
        Verdict::Unverified => {
            "CodeCaddie could not validate enough repository evidence for this criterion."
        }
    }
}

fn neutral_assessment_summary(assessment: &GoalAssessment) -> &'static str {
    match assessment.verdict {
        Verdict::Supported => "Validated repository evidence supports the assessed goal.",
        Verdict::Partial => {
            "Validated repository evidence supports part of the goal; material coverage remains incomplete."
        }
        Verdict::Unsupported => {
            "The bounded review did not validate enough implementation evidence for the goal."
        }
        Verdict::Unverified => {
            "CodeCaddie could not validate enough repository evidence for the goal."
        }
    }
}

fn redact_architecture_claim(claim: &mut ArchitectureClaim, index: usize, goals: &[GoalVersion]) {
    if let Some(goal) = linked_goal(&claim.affected_goal_version_ids, goals) {
        claim.component = format!(
            "Architecture support for {}",
            clipped_one_line(&goal.title, 130)
        );
        claim.relationship = Some(format!(
            "Supports the approved outcome: {}",
            clipped_one_line(&goal.business_outcome, 200)
        ));
        claim.summary = "The cited implementation, configuration, and test references show where the frozen repository supports this goal.".into();
    } else {
        claim.component = format!("Evidence-backed architecture area {}", index + 1);
        claim.relationship = None;
        claim.summary = "The cited implementation, configuration, and test references show a validated architecture relationship in the frozen repository.".into();
    }
}

fn redact_recommendation_narrative(
    recommendation: &mut Recommendation,
    index: usize,
    goals: &[GoalVersion],
) {
    if let Some(goal) = linked_goal(&recommendation.goal_version_ids, goals) {
        recommendation.title = format!(
            "Strengthen repository support for {}",
            clipped_one_line(&goal.title, 170)
        );
        recommendation.rationale = "The latest assessment leaves one or more linked checks partial or unsupported; use the cited files to close the highest-impact gap.".into();
        recommendation.expected_business_impact = format!(
            "Advances the approved outcome: {}",
            clipped_one_line(&goal.business_outcome, 560)
        );
    } else {
        recommendation.title = format!("Close validated evidence gap {}", index + 1);
        recommendation.rationale =
            "The latest assessment linked this improvement to cited evidence and an approved goal."
                .into();
        recommendation.expected_business_impact =
            "Closing the cited gap improves delivery confidence for the linked business outcomes."
                .into();
    }
}

fn redact_provider_narrative(report: &mut Report, goals: &[GoalVersion]) {
    report.provider_version = format!("{} (version omitted)", report.provider);
    for assessment in &mut report.assessments {
        for criterion in &mut assessment.criteria {
            criterion.rationale = neutral_criterion_rationale(criterion).into();
        }
        assessment.summary = direct_summary(assessment, neutral_assessment_summary(assessment));
        if !assessment.architecture_narrative.is_empty() {
            assessment.architecture_narrative =
                "The cited architecture references show where the repository supports this goal."
                    .into();
        }
    }
    consolidate_redacted_decisions(report, goals);
    for (index, claim) in report.architecture.iter_mut().enumerate() {
        claim.id = format!("validated-architecture-{}", index + 1);
        redact_architecture_claim(claim, index, goals);
    }
    for (index, recommendation) in report.recommendations.iter_mut().enumerate() {
        recommendation.id = format!("validated-recommendation-{}", index + 1);
        recommendation.rank = (index + 1) as u32;
        redact_recommendation_narrative(recommendation, index, goals);
    }
    report.partial = true;
    let warning = "Provider narrative matched repository source and was omitted; validated verdicts, evidence coordinates, and scores were retained.";
    if !report
        .analysis_warnings
        .iter()
        .any(|existing| existing == warning)
    {
        report.analysis_warnings.push(warning.into());
    }
}

/// Replaces only the named narrative fields with neutral validated wording,
/// leaving every other field, verdict, coordinate, and score untouched. A
/// violating architecture claim or recommendation has its whole narrative
/// replaced (its fields describe one decision together), but keeps its
/// identity, rank, goal links, and evidence.
fn redact_narrative_fields(report: &mut Report, goals: &[GoalVersion], targets: &[NarrativeField]) {
    let mut redacted_claims = BTreeSet::new();
    let mut redacted_recommendations = BTreeSet::new();
    let mut redacted_fields = 0_usize;
    for target in targets {
        redacted_fields += 1;
        match *target {
            NarrativeField::ProviderVersion => {
                report.provider_version = format!("{} (version omitted)", report.provider);
            }
            NarrativeField::AssessmentSummary(index) => {
                if let Some(assessment) = report.assessments.get_mut(index) {
                    assessment.summary =
                        direct_summary(assessment, neutral_assessment_summary(assessment));
                }
            }
            NarrativeField::AssessmentNarrative(index) => {
                if let Some(assessment) = report.assessments.get_mut(index) {
                    assessment.architecture_narrative =
                        "The cited architecture references show where the repository supports this goal.".into();
                }
            }
            NarrativeField::CriterionRationale(assessment_index, criterion_index) => {
                if let Some(criterion) = report
                    .assessments
                    .get_mut(assessment_index)
                    .and_then(|assessment| assessment.criteria.get_mut(criterion_index))
                {
                    criterion.rationale = neutral_criterion_rationale(criterion).into();
                }
            }
            NarrativeField::ArchitectureComponent(index)
            | NarrativeField::ArchitectureRelationship(index)
            | NarrativeField::ArchitectureSummary(index) => {
                redacted_claims.insert(index);
            }
            NarrativeField::RecommendationTitle(index)
            | NarrativeField::RecommendationRationale(index)
            | NarrativeField::RecommendationImpact(index) => {
                redacted_recommendations.insert(index);
            }
        }
    }
    for index in redacted_claims {
        if let Some(claim) = report.architecture.get_mut(index) {
            redact_architecture_claim(claim, index, goals);
        }
    }
    for index in redacted_recommendations {
        if let Some(recommendation) = report.recommendations.get_mut(index) {
            redact_recommendation_narrative(recommendation, index, goals);
        }
    }
    report.partial = true;
    report.analysis_warnings.push(format!(
        "Provider narrative matched repository source in {redacted_fields} field(s) and was replaced with neutral wording; validated verdicts, evidence coordinates, and scores were retained."
    ));
}

/// One redactable narrative field of a report, addressed structurally so a
/// violation can be replaced without touching any other field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NarrativeField {
    ProviderVersion,
    AssessmentSummary(usize),
    AssessmentNarrative(usize),
    CriterionRationale(usize, usize),
    ArchitectureComponent(usize),
    ArchitectureRelationship(usize),
    ArchitectureSummary(usize),
    RecommendationTitle(usize),
    RecommendationRationale(usize),
    RecommendationImpact(usize),
}

/// Every narrative field of the report in a stable order, paired with its
/// current text. The index into this list is the shared coordinate between
/// screening and redaction.
fn narrative_fields(report: &Report) -> Vec<(NarrativeField, String)> {
    let mut fields = vec![(
        NarrativeField::ProviderVersion,
        report.provider_version.clone(),
    )];
    for (assessment_index, assessment) in report.assessments.iter().enumerate() {
        fields.push((
            NarrativeField::AssessmentSummary(assessment_index),
            assessment.summary.clone(),
        ));
        if !assessment.architecture_narrative.is_empty() {
            fields.push((
                NarrativeField::AssessmentNarrative(assessment_index),
                assessment.architecture_narrative.clone(),
            ));
        }
        for (criterion_index, criterion) in assessment.criteria.iter().enumerate() {
            fields.push((
                NarrativeField::CriterionRationale(assessment_index, criterion_index),
                criterion.rationale.clone(),
            ));
        }
    }
    for (index, claim) in report.architecture.iter().enumerate() {
        fields.push((
            NarrativeField::ArchitectureComponent(index),
            claim.component.clone(),
        ));
        if let Some(relationship) = &claim.relationship {
            fields.push((
                NarrativeField::ArchitectureRelationship(index),
                relationship.clone(),
            ));
        }
        fields.push((
            NarrativeField::ArchitectureSummary(index),
            claim.summary.clone(),
        ));
    }
    for (index, recommendation) in report.recommendations.iter().enumerate() {
        fields.push((
            NarrativeField::RecommendationTitle(index),
            recommendation.title.clone(),
        ));
        fields.push((
            NarrativeField::RecommendationRationale(index),
            recommendation.rationale.clone(),
        ));
        fields.push((
            NarrativeField::RecommendationImpact(index),
            recommendation.expected_business_impact.clone(),
        ));
    }
    fields
}

/// The screening verdict for a report's narrative: clean, credential-shaped
/// text anywhere (always the most severe outcome), or repository source
/// matched in the identified fields.
enum NarrativeScreening {
    Clean,
    Credential,
    Source(BTreeSet<usize>),
}

fn screen_provider_narrative(
    report: &Report,
    repositories: &[(LocalRepository, String)],
) -> anyhow::Result<NarrativeScreening> {
    let fields = narrative_fields(report);
    let lower_narrative = fields
        .iter()
        .map(|(_, text)| text.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    for marker in [
        "begin private key",
        "begin rsa private key",
        "begin openssh private key",
        "github_pat_",
        "ghp_",
        "glpat-",
        "akia",
        "asia",
        "aiza",
        "ya29.",
        "xoxb-",
        "xoxp-",
        "xapp-",
        "sk-live-",
        "sk-proj-",
        "sk-ant-",
        "rk_live_",
        "whsec_",
        "npm_",
        "pypi-",
    ] {
        if lower_narrative.contains(marker) {
            return Ok(NarrativeScreening::Credential);
        }
    }

    let field_texts = fields
        .iter()
        .map(|(_, text)| text.clone())
        .collect::<Vec<_>>();
    let mut violations = BTreeSet::new();
    for (repository, commit) in repositories {
        violations.extend(repository.narrative_fields_matching_source(commit, &field_texts)?);
    }

    let evidence = report
        .assessments
        .iter()
        .flat_map(|assessment| &assessment.criteria)
        .flat_map(|criterion| &criterion.evidence)
        .chain(report.architecture.iter().flat_map(|claim| &claim.evidence))
        .chain(
            report
                .recommendations
                .iter()
                .flat_map(|recommendation| &recommendation.evidence),
        );
    let mut checked = BTreeSet::new();
    for reference in evidence {
        let coordinate = (
            reference.repository_id.clone(),
            reference.blob_oid.clone(),
            reference.start_line,
            reference.end_line,
        );
        if !checked.insert(coordinate) {
            continue;
        }
        let (repository, _) = repositories
            .iter()
            .find(|(repository, _)| repository.id == reference.repository_id)
            .ok_or_else(|| anyhow::anyhow!("evidence references an unknown repository"))?;
        let excerpt = repository.read_evidence(reference)?;
        let excerpt = excerpt.trim();
        for (index, (_, text)) in fields.iter().enumerate() {
            if excerpt.len() >= 16 && text.contains(excerpt) {
                violations.insert(index);
                continue;
            }
            if excerpt
                .lines()
                .map(str::trim)
                .any(|line| line.len() >= 24 && text.contains(line))
            {
                violations.insert(index);
            }
        }
    }
    if violations.is_empty() {
        Ok(NarrativeScreening::Clean)
    } else {
        Ok(NarrativeScreening::Source(violations))
    }
}

fn validate_goal_ids(allowed: &BTreeSet<String>, values: &[String]) -> anyhow::Result<()> {
    if values.iter().any(|value| !allowed.contains(value)) {
        anyhow::bail!("provider referenced an unknown goal version");
    }
    Ok(())
}

/// The outcome of binding one submission's citations: the evidence that
/// bound, plus a human-readable reason for every citation that did not.
/// Failure reasons carry only sanitized coordinate and git-status text —
/// never source — so they are safe in rationales and report warnings.
pub(super) struct EvidenceBinding {
    pub(super) evidence: Vec<EvidenceRef>,
    pub(super) failures: Vec<String>,
}

impl EvidenceBinding {
    pub(super) fn first_failure(&self) -> Option<String> {
        self.failures
            .first()
            .map(|failure| clipped_one_line(failure, 200))
    }
}

pub(super) fn bind_evidence(
    repositories: &[(LocalRepository, String)],
    raw: Vec<RawEvidence>,
) -> EvidenceBinding {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    for item in raw {
        let coordinate = format!("{}:{}-{}", item.path, item.start_line, item.end_line);
        let Some((repository, commit)) = repositories
            .iter()
            .find(|(repository, _)| repository.id == item.repository_id)
        else {
            failures.push(format!(
                "{coordinate} names an unknown repositoryId \"{}\"",
                item.repository_id
            ));
            continue;
        };
        match repository.evidence(
            commit,
            &item.path,
            item.start_line,
            item.end_line,
            item.kind,
        ) {
            Ok(reference) => evidence.push(reference),
            Err(error) => failures.push(format!("{coordinate}: {error}")),
        }
    }
    EvidenceBinding { evidence, failures }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::test_support::{fixture, raw};
    use codecaddie_domain::EvidenceKind;

    #[test]
    fn direct_summaries_are_collapsed_to_one_sentence_without_losing_unicode() {
        assert_eq!(
            one_sentence("Datadog is instrumented. PostHog was not found."),
            "Datadog is instrumented; PostHog was not found."
        );
        assert_eq!(
            one_sentence("Partly — Datadog is installed, not instrumented."),
            "Partly — Datadog is installed, not instrumented."
        );
    }

    #[test]
    fn provider_coordinates_become_immutable_evidence_without_source_text() {
        let (_directory, repository, commit, goal) = fixture();
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            raw(),
        )
        .unwrap();
        assert_eq!(report.coverage, Some(1.0));
        assert_eq!(
            report.assessments[0].summary,
            "Yes — Tenant-scoped invoice lookup is implemented in the billing repository."
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("scoped(tenant)"));
        assert_eq!(
            report.assessments[0].criteria[0].evidence[0].commit_sha,
            commit
        );
    }

    #[test]
    fn assessed_claims_without_bindable_evidence_become_unverified() {
        let (_directory, repository, commit, goal) = fixture();
        let mut output = raw();
        let invalid = RawEvidence {
            repository_id: "repo".into(),
            path: "missing.rs".into(),
            start_line: 1,
            end_line: 1,
            kind: EvidenceKind::Implementation,
        };
        output.assessments[0].criteria[0].evidence = vec![invalid.clone()];
        output.architecture[0].evidence = vec![invalid.clone()];
        output.recommendations[0].evidence = vec![invalid];
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            output,
        )
        .unwrap();
        let criterion = &report.assessments[0].criteria[0];
        assert_eq!(criterion.verdict, Verdict::Unverified);
        assert_eq!(criterion.confidence, 0.0);
        assert!(criterion.evidence.is_empty());
        assert!(criterion.rationale.contains("frozen commit"));
        assert_eq!(
            report.assessments[0].summary,
            "Could not verify — CodeCaddie could not bind the provider's submitted citations to this frozen commit."
        );
        assert!(report.architecture.is_empty());
        assert!(report.recommendations.is_empty());
    }

    #[test]
    fn unsupported_without_submitted_evidence_is_an_honest_not_found_result() {
        let (_directory, repository, commit, goal) = fixture();
        let mut output = raw();
        output.assessments[0].summary =
            "No product analytics SDK, initialization, or event capture usage was found.".into();
        output.assessments[0].criteria[0].verdict = Verdict::Unsupported;
        output.assessments[0].criteria[0].rationale =
            "Could not find evidence of a product analytics integration in the scanned commit."
                .into();
        output.assessments[0].criteria[0].evidence.clear();
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            output,
        )
        .unwrap();
        let assessment = &report.assessments[0];
        assert_eq!(assessment.verdict, Verdict::Unsupported);
        assert!(assessment.criteria[0].evidence.is_empty());
        assert_eq!(
            assessment.summary,
            "Could not find evidence — No product analytics SDK, initialization, or event capture usage was found."
        );
    }

    #[test]
    fn historical_goal_assessments_deserialize_without_a_summary() {
        let json = r#"{"goalVersionId":"goal-v1","verdict":"unverified","criteria":[]}"#;
        let assessment: GoalAssessment = serde_json::from_str(json).unwrap();
        assert!(assessment.summary.is_empty());
    }

    #[test]
    fn reports_freeze_and_validate_every_participating_repository() {
        let (directory, repository, commit, goal) = fixture();
        let second = LocalRepository::attach("repo-2", directory.path()).unwrap();
        let mut output = raw();
        output.recommendations[0].evidence[0].repository_id = "repo-2".into();
        let report = materialize_report_for_repositories(
            "report".into(),
            &[(repository, commit.clone()), (second, commit.clone())],
            "codex".into(),
            ReportOrigin::Scan,
            &[goal],
            output,
        )
        .unwrap();
        assert_eq!(report.repositories.len(), 2);
        assert_eq!(
            report.recommendations[0].evidence[0].repository_id,
            "repo-2"
        );
        assert!(
            report
                .repositories
                .iter()
                .all(|frozen| frozen.commit_sha == commit)
        );
    }

    #[test]
    fn provider_source_quotes_are_redacted_field_by_field_before_persistence() {
        let (_directory, repository, commit, goal) = fixture();
        let mut output = raw();
        output.assessments[0].criteria[0].rationale =
            "The provider quoted fn invoice(tenant: Id) { directly.".into();
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            output,
        )
        .unwrap();
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("fn invoice"));
        assert!(report.partial);
        // Only the violating rationale is replaced; every clean field —
        // including the goal summary and the architecture and recommendation
        // narrative — survives redaction untouched.
        assert_eq!(
            report.assessments[0].criteria[0].rationale,
            "Validated repository evidence supports this criterion."
        );
        assert_eq!(
            report.assessments[0].summary,
            "Yes — Tenant-scoped invoice lookup is implemented in the billing repository."
        );
        assert_eq!(report.architecture[0].component, "billing");
        assert_eq!(report.architecture[0].summary, "Billing scopes invoices");
        assert_eq!(report.recommendations[0].title, "Keep the denial test");
        assert!(
            report
                .analysis_warnings
                .iter()
                .any(|warning| warning.contains("narrative matched"))
        );
    }

    #[test]
    fn systemic_source_quoting_forfeits_the_whole_narrative() {
        let (_directory, repository, commit, goal) = fixture();
        let mut output = raw();
        // Quote cited source into most narrative fields: the rationale, the
        // architecture summary, and the recommendation rationale all carry
        // the same 24-character cited line.
        let quoted = "Quoting fn invoice(tenant: Id) { verbatim.";
        output.assessments[0].summary = quoted.into();
        output.assessments[0].criteria[0].rationale = quoted.into();
        output.architecture[0].summary = quoted.into();
        output.recommendations[0].rationale = quoted.into();
        output.recommendations[0].title = quoted.into();
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            output,
        )
        .unwrap();
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("fn invoice"));
        assert!(report.partial);
        // Over a quarter of the fields violated, so the full-narrative
        // redaction path runs, including decision consolidation.
        assert_eq!(
            report.architecture[0].component,
            "Architecture support for Tenant isolation"
        );
        assert_eq!(
            report.recommendations[0].title,
            "Strengthen repository support for Tenant isolation"
        );
        assert!(
            report
                .analysis_warnings
                .iter()
                .any(|warning| warning.contains("narrative matched"))
        );
    }

    #[test]
    fn privacy_adversarial_report_redacts_repository_payload_and_hostile_direction() {
        let (_directory, repository, commit, goal) = fixture();
        let mut output = raw();
        output.assessments[0].criteria[0].rationale = format!(
            "The provider copied {} directly.",
            crate::privacy_test_support::REPOSITORY_SOURCE_LINE
        );
        output.recommendations[0].title = crate::privacy_test_support::INJECTION_TEXT.into();
        output.recommendations[0].rationale = format!(
            "{} {}",
            crate::privacy_test_support::REPOSITORY_SOURCE_LINE,
            crate::privacy_test_support::INJECTION_TEXT
        );
        let report = materialize_report(
            "adversarial-report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            output,
        )
        .unwrap();
        let encoded = serde_json::to_vec(&report).unwrap();
        crate::privacy_test_support::assert_private_payload_absent(&encoded);
        assert!(
            !String::from_utf8_lossy(&encoded)
                .contains(crate::privacy_test_support::INJECTION_TEXT),
            "a repository-authored instruction must be treated as source, not report prose"
        );
        assert!(report.partial);
        assert!(
            report
                .analysis_warnings
                .iter()
                .any(|warning| warning.contains("narrative matched"))
        );
    }

    #[test]
    fn intermediate_scan_batch_reports_are_never_persisted() {
        let (_directory, repository, commit, goal) = fixture();
        let mut quoted = raw();
        quoted.assessments[0].criteria[0].rationale =
            "The provider quoted fn invoice(tenant: Id) { directly.".into();

        // Batch validation inside `run_scan` uses `Skip`, so an intermediate
        // batch report can still carry provider narrative verbatim. That is
        // exactly why batch reports are transient validation artifacts:
        // persisting one would leak repository source.
        let batch_report = materialize_report_for_repositories_with_source_policy(
            "report-batch-1".into(),
            &[(repository.clone(), commit.clone())],
            "codex".into(),
            ReportOrigin::Scan,
            std::slice::from_ref(&goal),
            quoted.clone(),
            SourceNarrativePolicy::Skip,
        )
        .unwrap();
        assert!(
            serde_json::to_string(&batch_report)
                .unwrap()
                .contains("fn invoice"),
            "a Skip-policy batch report retains unscreened narrative and must stay transient",
        );

        // The only report `run_scan` returns for persistence through
        // `LocalWorkspaceStore::record_report` is the final materialization
        // of the merged raw analysis, which always applies `Redact`.
        let final_report = materialize_report_for_repositories(
            "report".into(),
            &[(repository, commit)],
            "codex".into(),
            ReportOrigin::Scan,
            &[goal],
            quoted,
        )
        .unwrap();
        assert!(
            !final_report.id.contains("-batch-"),
            "batch-suffixed report IDs never reach persistence",
        );
        let encoded = serde_json::to_string(&final_report).unwrap();
        assert!(
            !encoded.contains("fn invoice"),
            "the persisted scan report never carries source-matching narrative",
        );
        assert!(final_report.partial);
        assert!(
            final_report
                .analysis_warnings
                .iter()
                .any(|warning| warning.contains("narrative matched"))
        );
    }

    #[test]
    fn uncited_source_and_credential_shaped_text_are_redacted() {
        let (_directory, repository, commit, goal) = fixture();
        let mut uncited = raw();
        uncited.recommendations[0].rationale = "internal reconciliation token stays local".into();
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            std::slice::from_ref(&goal),
            uncited,
        )
        .unwrap();
        assert_eq!(report.recommendations.len(), 1);
        assert_eq!(
            report.recommendations[0].title,
            "Strengthen repository support for Tenant isolation"
        );
        assert!(!report.recommendations[0].evidence.is_empty());
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("internal reconciliation token")
        );

        let mut credential = raw();
        credential.provider_version = ["gh", "p_", &"a".repeat(40)].concat();
        let report = materialize_report(
            "report".into(),
            &repository,
            &commit,
            ProviderKind::Codex,
            &[goal],
            credential,
        )
        .unwrap();
        assert_eq!(report.provider_version, "codex (version omitted)");
        assert!(!serde_json::to_string(&report).unwrap().contains("ghp_"));
    }
}
