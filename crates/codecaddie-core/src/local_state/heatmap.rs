//! Presentation of saved reports as a per-goal heatmap: verdict categories,
//! bounded one-line summaries, and sorted immutable evidence references.
//! Everything here is derived from the workspace projection and
//! contains report metadata only, never repository source text.

use codecaddie_domain::{
    ArchitectureClaim, EvidenceKind, EvidenceRef, GoalAssessment, GoalVersion, Report,
    ReportOrigin, Verdict, WorkspaceProjection,
};
use serde::Serialize;
use time::OffsetDateTime;

/// The desktop history keeps a bounded year-like window without changing
/// the event ledger: older signed reports remain stored and exports or future
/// projections can still rebuild from them.
pub(super) const REPORT_HISTORY_LIMIT: usize = 12;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapCriterion {
    pub criterion_id: String,
    pub text: String,
    pub verdict: String,
    /// Field-level delta against the preceding saved report for this logical
    /// goal and criterion id. Prior evidence remains structured so the UI can
    /// show exactly which immutable proof set changed.
    pub change_kind: String,
    pub change: String,
    pub previous_verdict: Option<String>,
    pub previous_evidence: Vec<EvidenceRef>,
    pub rationale: String,
    pub confidence: f32,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapCell {
    pub goal_title: String,
    pub goal_id: String,
    pub goal_version_id: String,
    pub verdict: String,
    pub summary: String,
    pub rationale: String,
    /// The per-goal architecture narrative from the seeding codebase map;
    /// empty for reports that predate it.
    pub architecture_narrative: String,
    pub change: String,
    pub checks: Vec<String>,
    pub references: Vec<String>,
    pub criteria: Vec<HeatmapCriterion>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapWeek {
    pub week_start: String,
    pub label: String,
    pub report_id: String,
    pub report_event_id: String,
    pub run_number: u32,
    pub origin: ReportOrigin,
    pub provider: String,
    pub provider_version: String,
    pub repositories: Vec<String>,
    pub unverified_criteria: u32,
    pub partial: bool,
    pub analysis_warnings: Vec<String>,
    /// The report's priority-weighted coverage score in [0, 1], when any
    /// criterion was assessed.
    pub coverage: Option<f64>,
    /// The report's validated architecture claims — ids, coordinates, and
    /// derived prose only, exactly as persisted. The desktop joins these to
    /// goals through `affectedGoalVersionIds` at render time.
    pub architecture: Vec<ArchitectureClaim>,
    pub cells: Vec<HeatmapCell>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportHistoryCell {
    pub goal_title: String,
    pub goal_id: String,
    pub goal_version_id: String,
    pub verdict: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportHistoryRun {
    pub week_start: String,
    pub label: String,
    pub report_id: String,
    pub report_event_id: String,
    pub run_number: u32,
    pub origin: ReportOrigin,
    pub provider: String,
    pub provider_version: String,
    pub repositories: Vec<String>,
    pub unverified_criteria: u32,
    pub coverage: Option<f64>,
    pub partial: bool,
    pub analysis_warnings: Vec<String>,
    pub cells: Vec<ReportHistoryCell>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportHistoryPage {
    pub runs: Vec<ReportHistoryRun>,
    pub total_active_runs: usize,
    pub has_older: bool,
    pub next_before: Option<String>,
}

#[derive(Clone)]
struct ActiveReportEntry {
    event_id: String,
    run_number: u32,
    report: Report,
}

fn assessment_category(assessment: &GoalAssessment) -> &'static str {
    let has_evidence = assessment
        .criteria
        .iter()
        .any(|criterion| !criterion.evidence.is_empty());
    let every_supported = !assessment.criteria.is_empty()
        && assessment
            .criteria
            .iter()
            .all(|criterion| criterion.verdict == Verdict::Supported);
    let every_supported_with_evidence = every_supported
        && assessment
            .criteria
            .iter()
            .all(|criterion| !criterion.evidence.is_empty());
    let has_supported_or_partial = assessment
        .criteria
        .iter()
        .any(|criterion| matches!(criterion.verdict, Verdict::Supported | Verdict::Partial));
    match assessment.verdict {
        Verdict::Supported if every_supported_with_evidence => "strong",
        Verdict::Supported => "functional",
        Verdict::Partial | Verdict::Unverified => "incomplete",
        Verdict::Unsupported if has_evidence || has_supported_or_partial => "broken",
        Verdict::Unsupported => "missing",
    }
}

fn category_rank(category: &str) -> Option<u8> {
    match category {
        "missing" => Some(0),
        "broken" => Some(1),
        "incomplete" => Some(2),
        "functional" => Some(3),
        "strong" => Some(4),
        _ => None,
    }
}

fn category_change(previous: Option<&str>, current: &str) -> String {
    let Some(current_rank) = category_rank(current) else {
        return "Not applicable".into();
    };
    let Some(previous) = previous else {
        return "First assessment for this goal".into();
    };
    let Some(previous_rank) = category_rank(previous) else {
        return "First assessment for this goal".into();
    };
    match current_rank.cmp(&previous_rank) {
        std::cmp::Ordering::Greater => format!("Improved from {}", title_case_category(previous)),
        std::cmp::Ordering::Less => format!("Declined from {}", title_case_category(previous)),
        std::cmp::Ordering::Equal => format!("Unchanged from {}", title_case_category(current)),
    }
}

fn title_case_category(category: &str) -> &str {
    match category {
        "missing" => "Missing",
        "broken" => "Broken",
        "incomplete" => "Incomplete",
        "functional" => "Functional",
        "strong" => "Strong",
        _ => "N/A",
    }
}

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

fn trim_sentence_end(value: &str) -> &str {
    value.trim().trim_end_matches(['.', '!', '?'])
}

fn clamp_summary(value: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 280;
    if value.chars().count() <= MAX_SUMMARY_CHARS {
        return value.to_string();
    }
    let mut clipped = value
        .chars()
        .take(MAX_SUMMARY_CHARS - 1)
        .collect::<String>();
    if let Some(last_space) = clipped.rfind(' ') {
        clipped.truncate(last_space);
    }
    clipped.push('…');
    clipped
}

fn assessment_prefix(assessment: &GoalAssessment) -> &'static str {
    let has_evidence = assessment
        .criteria
        .iter()
        .any(|criterion| !criterion.evidence.is_empty());
    match assessment.verdict {
        Verdict::Supported => "Yes",
        Verdict::Partial => "Partly",
        Verdict::Unsupported if has_evidence => "No",
        Verdict::Unsupported => "Could not find evidence",
        Verdict::Unverified => "Could not verify",
    }
}

fn assessment_summary(assessment: &GoalAssessment) -> String {
    let saved = one_sentence(&assessment.summary);
    if !saved.is_empty() {
        return clamp_summary(&saved);
    }

    let positive = assessment
        .criteria
        .iter()
        .find(|criterion| matches!(criterion.verdict, Verdict::Supported | Verdict::Partial));
    let gap = assessment.criteria.iter().find(|criterion| {
        matches!(
            criterion.verdict,
            Verdict::Unsupported | Verdict::Unverified
        )
    });
    let mut parts = Vec::new();
    if let Some(criterion) = positive {
        parts.push(trim_sentence_end(&criterion.rationale));
    }
    if let Some(criterion) = gap
        && positive.map(|item| item.criterion_id.as_str()) != Some(criterion.criterion_id.as_str())
    {
        parts.push(trim_sentence_end(&criterion.rationale));
    }
    let body = if parts.is_empty() {
        "The saved analysis did not include a usable conclusion".to_string()
    } else {
        parts.join("; ")
    };
    clamp_summary(&one_sentence(&format!(
        "{} — {body}",
        assessment_prefix(assessment)
    )))
}

fn criterion_verdict(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Supported => "supported",
        Verdict::Partial => "partial",
        Verdict::Unsupported => "unsupported",
        Verdict::Unverified => "unverified",
    }
}

fn criterion_rank(verdict: &str) -> Option<u8> {
    match verdict {
        "unverified" => Some(0),
        "unsupported" => Some(1),
        "partial" => Some(2),
        "supported" => Some(3),
        _ => None,
    }
}

fn title_case_verdict(verdict: &str) -> &str {
    match verdict {
        "supported" => "Supported",
        "partial" => "Partial",
        "unsupported" => "Unsupported",
        "unverified" => "Unverified",
        _ => "Unknown",
    }
}

fn evidence_commit_label(evidence: &[EvidenceRef]) -> String {
    let mut commits = evidence
        .iter()
        .map(|item| {
            item.commit_sha
                .get(..12)
                .unwrap_or(&item.commit_sha)
                .to_string()
        })
        .collect::<Vec<_>>();
    commits.sort();
    commits.dedup();
    if commits.is_empty() {
        "no verified references".into()
    } else {
        format!(
            "{} verified {} at {}",
            evidence.len(),
            if evidence.len() == 1 {
                "reference"
            } else {
                "references"
            },
            commits.join(", ")
        )
    }
}

fn criterion_comparison(
    previous: Option<&HeatmapCriterion>,
    current_verdict: &str,
    current_evidence: &[EvidenceRef],
) -> (String, String, Option<String>, Vec<EvidenceRef>) {
    let Some(previous) = previous else {
        return (
            "first".into(),
            format!(
                "First saved assessment: {} with {}.",
                title_case_verdict(current_verdict),
                evidence_commit_label(current_evidence)
            ),
            None,
            Vec::new(),
        );
    };
    let previous_evidence = previous.evidence.clone();
    let evidence_changed = previous_evidence != current_evidence;
    let previous_rank = criterion_rank(&previous.verdict);
    let current_rank = criterion_rank(current_verdict);
    let change_kind = match (previous_rank, current_rank) {
        (Some(before), Some(after)) if after > before => "improved",
        (Some(before), Some(after)) if after < before => "declined",
        _ if evidence_changed => "evidence_changed",
        _ => "unchanged",
    };
    let direction = match change_kind {
        "improved" => "Improved",
        "declined" => "Declined",
        "evidence_changed" => "Evidence changed",
        _ => "Unchanged",
    };
    let change = format!(
        "{direction}: {} with {} → {} with {}.",
        title_case_verdict(&previous.verdict),
        evidence_commit_label(&previous_evidence),
        title_case_verdict(current_verdict),
        evidence_commit_label(current_evidence)
    );
    (
        change_kind.into(),
        change,
        Some(previous.verdict.clone()),
        previous_evidence,
    )
}

fn field_change_summary(base: String, criteria: &[HeatmapCriterion]) -> String {
    if criteria.is_empty() || criteria.iter().all(|item| item.change_kind == "first") {
        return base;
    }
    let improved = criteria
        .iter()
        .filter(|item| item.change_kind == "improved")
        .count();
    let declined = criteria
        .iter()
        .filter(|item| item.change_kind == "declined")
        .count();
    let evidence_changed = criteria
        .iter()
        .filter(|item| item.change_kind == "evidence_changed")
        .count();
    let unchanged = criteria
        .iter()
        .filter(|item| item.change_kind == "unchanged")
        .count();
    format!(
        "{base} · field-level: {improved} improved, {declined} declined, {evidence_changed} evidence-changed, {unchanged} unchanged"
    )
}

fn evidence_kind_rank(kind: EvidenceKind) -> u8 {
    match kind {
        EvidenceKind::Implementation => 0,
        EvidenceKind::Configuration => 1,
        EvidenceKind::Test => 2,
        EvidenceKind::Architecture => 3,
        EvidenceKind::Documentation => 4,
    }
}

fn sorted_evidence(values: &[EvidenceRef]) -> Vec<EvidenceRef> {
    let mut values = values.to_vec();
    values.sort_by(|left, right| {
        evidence_kind_rank(left.kind)
            .cmp(&evidence_kind_rank(right.kind))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });
    values.dedup();
    values
}

fn analysis_label(moment: OffsetDateTime) -> String {
    let month = moment.month().to_string();
    format!("{} {}", month.get(..3).unwrap_or("Run"), moment.day())
}

fn goal_version_for_logical_goal<'a>(
    projection: &'a WorkspaceProjection,
    logical_goal_id: &str,
    report: &Report,
) -> Option<&'a GoalVersion> {
    report.goal_version_ids.iter().find_map(|version_id| {
        projection
            .goal_versions
            .get(version_id)
            .filter(|version| version.goal_id == logical_goal_id)
    })
}

fn active_report_entries(projection: &WorkspaceProjection) -> Vec<ActiveReportEntry> {
    let mut entries = projection
        .reports
        .iter()
        .map(|(storage_key, report)| {
            let event_id = projection
                .report_event_ids
                .get(storage_key)
                .cloned()
                .unwrap_or_else(|| report.id.clone());
            let run_number = projection
                .report_ordinals
                .get(&event_id)
                .copied()
                .unwrap_or(0);
            ActiveReportEntry {
                event_id,
                run_number,
                report: report.clone(),
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        (left.report.completed_at, left.run_number, &left.event_id).cmp(&(
            right.report.completed_at,
            right.run_number,
            &right.event_id,
        ))
    });
    for (index, entry) in entries.iter_mut().enumerate() {
        if entry.run_number == 0 {
            entry.run_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        }
    }
    entries
}

fn current_goals(projection: &WorkspaceProjection) -> Vec<(String, GoalVersion)> {
    let mut goals = projection
        .approved_goals
        .iter()
        .filter_map(|(goal_id, version_id)| {
            projection
                .goal_versions
                .get(version_id)
                .map(|goal| (goal_id.clone(), goal.clone()))
        })
        .collect::<Vec<_>>();
    goals.sort_by(|left, right| match (left.1.position, right.1.position) {
        (0, 0) => right
            .1
            .priority
            .cmp(&left.1.priority)
            .then_with(|| left.1.title.cmp(&right.1.title)),
        (0, _) => std::cmp::Ordering::Greater,
        (_, 0) => std::cmp::Ordering::Less,
        _ => left.1.position.cmp(&right.1.position),
    });
    goals
}

fn history_cell(
    projection: &WorkspaceProjection,
    report: &Report,
    goal_id: &str,
    current_goal: &GoalVersion,
) -> ReportHistoryCell {
    let first_created_at = projection
        .goal_versions
        .values()
        .filter(|version| version.goal_id == goal_id)
        .map(|version| version.created_at)
        .min()
        .unwrap_or(current_goal.created_at);
    let assessed_version = goal_version_for_logical_goal(projection, goal_id, report);
    let assessment = assessed_version.and_then(|version| {
        report
            .assessments
            .iter()
            .find(|assessment| assessment.goal_version_id == version.id)
    });
    let (verdict, summary, goal_version_id) = if report.completed_at < first_created_at {
        (
            "not_applicable".to_string(),
            "Not applicable — This goal did not exist when this analysis ran.".to_string(),
            String::new(),
        )
    } else if let (Some(version), Some(assessment)) = (assessed_version, assessment) {
        (
            assessment_category(assessment).to_string(),
            assessment_summary(assessment),
            version.id.clone(),
        )
    } else {
        (
            "missing".to_string(),
            "Could not verify — No assessment was recorded for this goal in the selected analysis."
                .to_string(),
            String::new(),
        )
    };
    ReportHistoryCell {
        goal_title: current_goal.title.clone(),
        goal_id: goal_id.to_string(),
        goal_version_id,
        verdict,
        summary,
    }
}

pub(super) fn build_report_history_page(
    projection: &WorkspaceProjection,
    before_event_id: Option<&str>,
    limit: usize,
) -> anyhow::Result<ReportHistoryPage> {
    let entries = active_report_entries(projection);
    let end = if let Some(before) = before_event_id {
        entries
            .iter()
            .position(|entry| entry.event_id == before)
            .ok_or_else(|| anyhow::anyhow!("the history cursor is no longer available"))?
    } else {
        entries.len()
    };
    let start = end.saturating_sub(limit.clamp(1, 100));
    let goals = current_goals(projection);
    let runs = entries[start..end]
        .iter()
        .map(|entry| {
            let report = &entry.report;
            ReportHistoryRun {
                week_start: report
                    .completed_at
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
                label: format!(
                    "{} · Run {}",
                    analysis_label(report.completed_at),
                    entry.run_number
                ),
                report_id: report.id.clone(),
                report_event_id: entry.event_id.clone(),
                run_number: entry.run_number,
                origin: report.origin,
                provider: report.provider.clone(),
                provider_version: report.provider_version.clone(),
                repositories: report
                    .repositories
                    .iter()
                    .map(|repository| {
                        format!("{} @ {}", repository.repository_id, repository.commit_sha)
                    })
                    .collect(),
                unverified_criteria: report.unverified_criteria,
                coverage: report.coverage,
                partial: report.partial,
                analysis_warnings: report.analysis_warnings.clone(),
                cells: goals
                    .iter()
                    .map(|(goal_id, goal)| history_cell(projection, report, goal_id, goal))
                    .collect(),
            }
        })
        .collect();
    Ok(ReportHistoryPage {
        runs,
        total_active_runs: entries.len(),
        has_older: start > 0,
        next_before: (start > 0).then(|| entries[start].event_id.clone()),
    })
}

pub(super) fn build_report_finding(
    projection: &WorkspaceProjection,
    report_event_id: &str,
    goal_version_id: &str,
) -> anyhow::Result<HeatmapWeek> {
    let entries = active_report_entries(projection);
    let index = entries
        .iter()
        .position(|entry| entry.event_id == report_event_id)
        .ok_or_else(|| anyhow::anyhow!("the selected report is not in active history"))?;
    let logical_goal_id = projection
        .goal_versions
        .get(goal_version_id)
        .map(|goal| goal.goal_id.as_str())
        .ok_or_else(|| anyhow::anyhow!("the selected goal version is unavailable"))?;

    let mut scoped = projection.clone();
    scoped.reports.clear();
    scoped.report_event_ids.clear();
    for entry in entries[index.saturating_sub(1)..=index].iter() {
        let storage_key = format!("\0detail:{}", entry.event_id);
        scoped
            .reports
            .insert(storage_key.clone(), entry.report.clone());
        scoped
            .report_event_ids
            .insert(storage_key, entry.event_id.clone());
    }
    let mut finding = build_report_heatmap(&scoped, 2)
        .into_iter()
        .find(|analysis| analysis.report_event_id == report_event_id)
        .ok_or_else(|| anyhow::anyhow!("the selected report finding is unavailable"))?;
    finding.cells.retain(|cell| cell.goal_id == logical_goal_id);
    if finding.cells.is_empty() {
        anyhow::bail!("the selected goal finding is unavailable");
    }
    Ok(finding)
}

pub(super) fn build_report_heatmap(
    projection: &WorkspaceProjection,
    analyses: usize,
) -> Vec<HeatmapWeek> {
    let mut reports = active_report_entries(projection);
    if reports.len() > analyses {
        reports.drain(0..reports.len() - analyses);
    }
    let goals = current_goals(projection);
    let mut analyses_out: Vec<HeatmapWeek> = Vec::with_capacity(reports.len());
    for entry in &reports {
        let report = &entry.report;
        let label = format!(
            "{} · Run {}",
            analysis_label(report.completed_at),
            entry.run_number
        );
        let mut cells = Vec::with_capacity(goals.len());
        for (goal_id, current_goal) in &goals {
            let previous_cell = analyses_out
                .last()
                .and_then(|analysis| analysis.cells.iter().find(|cell| cell.goal_id == *goal_id));
            let first_created_at = projection
                .goal_versions
                .values()
                .filter(|version| version.goal_id == *goal_id)
                .map(|version| version.created_at)
                .min()
                .unwrap_or(current_goal.created_at);
            let assessed_version = goal_version_for_logical_goal(projection, goal_id, report);
            let assessment = assessed_version.and_then(|version| {
                report
                    .assessments
                    .iter()
                    .find(|assessment| assessment.goal_version_id == version.id)
            });
            let (
                verdict,
                summary,
                rationale,
                narrative,
                checks,
                references,
                criteria,
                assessed_version_id,
            ) = if report.completed_at < first_created_at {
                (
                    "not_applicable".to_string(),
                    "Not applicable — This goal did not exist when this analysis ran.".to_string(),
                    "This goal did not exist when this analysis ran.".to_string(),
                    String::new(),
                    Vec::new(),
                    vec!["Goal history · created after this analysis".into()],
                    Vec::new(),
                    String::new(),
                )
            } else if let (Some(version), Some(assessment)) = (assessed_version, assessment) {
                let verdict = assessment_category(assessment).to_string();
                let summary = assessment_summary(assessment);
                let rationale = summary.clone();
                let checks = version
                    .criteria
                    .iter()
                    .map(|criterion| criterion.text.clone())
                    .collect::<Vec<_>>();
                let criteria = version
                    .criteria
                    .iter()
                    .map(|criterion| {
                        assessment
                            .criteria
                            .iter()
                            .find(|item| item.criterion_id == criterion.id)
                            .map(|item| {
                                let verdict = criterion_verdict(item.verdict).to_string();
                                let evidence = sorted_evidence(&item.evidence);
                                let previous = previous_cell.and_then(|cell| {
                                    cell.criteria
                                        .iter()
                                        .find(|previous| previous.criterion_id == criterion.id)
                                });
                                let (change_kind, change, previous_verdict, previous_evidence) =
                                    criterion_comparison(previous, &verdict, &evidence);
                                HeatmapCriterion {
                                    criterion_id: criterion.id.clone(),
                                    text: criterion.text.clone(),
                                    verdict,
                                    change_kind,
                                    change,
                                    previous_verdict,
                                    previous_evidence,
                                    rationale: one_line(&item.rationale),
                                    confidence: item.confidence,
                                    evidence,
                                }
                            })
                            .unwrap_or_else(|| HeatmapCriterion {
                                criterion_id: criterion.id.clone(),
                                text: criterion.text.clone(),
                                verdict: "unverified".into(),
                                change_kind: "first".into(),
                                change:
                                    "No comparable saved assessment was recorded for this check."
                                        .into(),
                                previous_verdict: None,
                                previous_evidence: Vec::new(),
                                rationale:
                                    "CodeCaddie could not match this check to the saved assessment."
                                        .into(),
                                confidence: 0.0,
                                evidence: Vec::new(),
                            })
                    })
                    .collect::<Vec<_>>();
                let mut references = criteria
                    .iter()
                    .flat_map(|criterion| criterion.evidence.iter())
                    .map(|evidence| {
                        format!(
                            "{}:{}-{} @ {}",
                            evidence.path,
                            evidence.start_line,
                            evidence.end_line,
                            evidence
                                .commit_sha
                                .get(..12)
                                .unwrap_or(&evidence.commit_sha)
                        )
                    })
                    .collect::<Vec<_>>();
                references.sort();
                references.dedup();
                if references.is_empty() {
                    references.push(
                        "No immutable repository reference was validated for this finding.".into(),
                    );
                }
                (
                    verdict,
                    summary,
                    rationale,
                    one_line(&assessment.architecture_narrative),
                    checks,
                    references,
                    criteria,
                    version.id.clone(),
                )
            } else {
                let criteria = current_goal
                    .criteria
                    .iter()
                    .map(|criterion| HeatmapCriterion {
                        criterion_id: criterion.id.clone(),
                        text: criterion.text.clone(),
                        verdict: "unverified".into(),
                        change_kind: "first".into(),
                        change: "No comparable saved assessment was recorded for this check."
                            .into(),
                        previous_verdict: None,
                        previous_evidence: Vec::new(),
                        rationale:
                            "No assessment was recorded for this check in the selected analysis."
                                .into(),
                        confidence: 0.0,
                        evidence: Vec::new(),
                    })
                    .collect::<Vec<_>>();
                (
                        "missing".to_string(),
                        "Could not verify — No assessment was recorded for this goal in the selected analysis."
                            .to_string(),
                        "No assessment was recorded for this goal in the selected analysis."
                            .to_string(),
                        String::new(),
                        current_goal
                            .criteria
                            .iter()
                            .map(|criterion| criterion.text.clone())
                            .collect(),
                        vec!["Saved local analysis · no matching goal assessment".into()],
                        criteria,
                        String::new(),
                    )
            };
            let previous = previous_cell.map(|cell| cell.verdict.as_str());
            let change = field_change_summary(category_change(previous, &verdict), &criteria);
            cells.push(HeatmapCell {
                goal_title: current_goal.title.clone(),
                goal_id: goal_id.clone(),
                goal_version_id: assessed_version_id,
                change,
                verdict,
                summary,
                rationale,
                architecture_narrative: narrative,
                checks,
                references,
                criteria,
            });
        }
        analyses_out.push(HeatmapWeek {
            week_start: report
                .completed_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
            label,
            report_id: report.id.clone(),
            report_event_id: entry.event_id.clone(),
            run_number: entry.run_number,
            origin: report.origin,
            provider: report.provider.clone(),
            provider_version: report.provider_version.clone(),
            repositories: report
                .repositories
                .iter()
                .map(|repository| {
                    format!("{} @ {}", repository.repository_id, repository.commit_sha)
                })
                .collect(),
            unverified_criteria: report.unverified_criteria,
            partial: report.partial,
            analysis_warnings: report.analysis_warnings.clone(),
            coverage: report.coverage,
            architecture: report.architecture.clone(),
            cells,
        });
    }
    analyses_out
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_domain::{Criterion, FrozenRepository};

    #[test]
    fn historical_assessment_fallback_is_one_direct_sentence() {
        let assessment = GoalAssessment {
            goal_version_id: "goal-observability".into(),
            verdict: Verdict::Partial,
            summary: String::new(),
            architecture_narrative: String::new(),
            related_component_ids: vec![],
            criteria: vec![
                codecaddie_domain::CriterionAssessment {
                    criterion_id: "telemetry".into(),
                    verdict: Verdict::Supported,
                    rationale: "Datadog tracing is initialized. Runtime spans are emitted.".into(),
                    confidence: 0.9,
                    evidence: vec![],
                },
                codecaddie_domain::CriterionAssessment {
                    criterion_id: "analytics".into(),
                    verdict: Verdict::Unsupported,
                    rationale: "Could not find evidence of PostHog event capture.".into(),
                    confidence: 0.8,
                    evidence: vec![],
                },
            ],
        };

        assert_eq!(
            assessment_summary(&assessment),
            "Partly — Datadog tracing is initialized; Runtime spans are emitted; Could not find evidence of PostHog event capture"
        );
    }

    #[test]
    fn later_goals_are_na_only_before_they_exist() {
        let first_goal = GoalVersion {
            id: "goal-version-private".into(),
            goal_id: "private".into(),
            title: "Keep analysis private".into(),
            business_outcome: "Repository source stays local".into(),
            priority: 5,
            position: 1,
            criteria: vec![Criterion {
                id: "criterion-private".into(),
                text: "No source text crosses the provider boundary".into(),
            }],
            rubric_dimensions: vec!["Trust".into()],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "actor-test".into(),
            supersedes: None,
        };
        let later_goal = GoalVersion {
            id: "goal-version-history".into(),
            goal_id: "history".into(),
            title: "Keep progress history trustworthy".into(),
            business_outcome: "Later goals do not invent past results".into(),
            priority: 4,
            position: 2,
            criteria: vec![Criterion {
                id: "criterion-history".into(),
                text: "Earlier analyses show N/A".into(),
            }],
            rubric_dimensions: vec!["Trust".into()],
            created_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(2),
            created_by: "actor-test".into(),
            supersedes: None,
        };
        let assessment = |goal: &GoalVersion| codecaddie_domain::GoalAssessment {
            goal_version_id: goal.id.clone(),
            verdict: Verdict::Unverified,
            summary: String::new(),
            architecture_narrative: String::new(),
            related_component_ids: vec![],
            criteria: vec![codecaddie_domain::CriterionAssessment {
                criterion_id: goal.criteria[0].id.clone(),
                verdict: Verdict::Unverified,
                rationale: "Not yet verified".into(),
                confidence: 0.5,
                evidence: vec![],
            }],
        };
        let mut projection = WorkspaceProjection::default();
        projection
            .goal_versions
            .insert(first_goal.id.clone(), first_goal.clone());
        projection
            .goal_versions
            .insert(later_goal.id.clone(), later_goal.clone());
        projection
            .approved_goals
            .insert(first_goal.goal_id.clone(), first_goal.id.clone());
        projection
            .approved_goals
            .insert(later_goal.goal_id.clone(), later_goal.id.clone());
        projection.reports.insert(
            "report-1".into(),
            Report {
                id: "report-1".into(),
                completed_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
                repositories: vec![],
                goal_version_ids: vec![first_goal.id.clone()],
                goal_set_hash: "first".into(),
                provider: "test".into(),
                provider_version: "test".into(),
                origin: codecaddie_domain::ReportOrigin::Scan,
                assessments: vec![assessment(&first_goal)],
                architecture: vec![],
                recommendations: vec![],
                coverage: None,
                unverified_criteria: 1,
                partial: false,
                analysis_warnings: vec![],
                codebase_map_id: None,
                codebase_map_hash: None,
            },
        );
        projection.reports.insert(
            "report-2".into(),
            Report {
                id: "report-2".into(),
                completed_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(3),
                repositories: vec![],
                goal_version_ids: vec![first_goal.id.clone(), later_goal.id.clone()],
                goal_set_hash: "second".into(),
                provider: "test".into(),
                provider_version: "test".into(),
                origin: codecaddie_domain::ReportOrigin::Scan,
                assessments: vec![assessment(&first_goal), assessment(&later_goal)],
                architecture: vec![],
                recommendations: vec![],
                coverage: None,
                unverified_criteria: 2,
                partial: false,
                analysis_warnings: vec![],
                codebase_map_id: None,
                codebase_map_hash: None,
            },
        );

        let heatmap = build_report_heatmap(&projection, 4);
        assert_eq!(heatmap.len(), 2);
        assert_eq!(heatmap[0].label, "Jan 1 · Run 1");
        assert_eq!(heatmap[1].label, "Jan 1 · Run 2");
        assert_eq!(heatmap[0].cells[0].verdict, "incomplete");
        assert_eq!(heatmap[1].cells[0].verdict, "incomplete");
        assert_eq!(heatmap[0].cells[1].verdict, "not_applicable");
        assert_eq!(heatmap[1].cells[1].verdict, "incomplete");
    }

    #[test]
    fn evidence_first_projection_orders_criteria_and_prefers_implementation() {
        let criteria = vec![
            Criterion {
                id: "instrumentation".into(),
                text: "User behavior is instrumented with a concrete tool".into(),
            },
            Criterion {
                id: "analytics".into(),
                text: "Product analytics capture activation".into(),
            },
            Criterion {
                id: "invalid".into(),
                text: "Every claim has immutable evidence".into(),
            },
        ];
        let goal = GoalVersion {
            id: "goal-version-observability".into(),
            goal_id: "observability".into(),
            title: "Observe user behavior".into(),
            business_outcome: "Know whether the product wedge works".into(),
            priority: 5,
            position: 1,
            criteria: criteria.clone(),
            rubric_dimensions: vec!["Evidence".into()],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "actor-test".into(),
            supersedes: None,
        };
        let evidence = |kind, path: &str| EvidenceRef {
            repository_id: "repo-test".into(),
            commit_sha: "0123456789abcdef0123456789abcdef01234567".into(),
            blob_oid: "abcdef".into(),
            path: path.into(),
            start_line: 10,
            end_line: 18,
            content_hash: "a".repeat(64),
            kind,
        };
        let assessment = GoalAssessment {
            goal_version_id: goal.id.clone(),
            verdict: Verdict::Partial,
            summary: "Partly — Datadog is instrumented for system telemetry, but product analytics such as PostHog were not found.".into(),
            architecture_narrative: String::new(),
            related_component_ids: vec![],
            criteria: vec![
                codecaddie_domain::CriterionAssessment {
                    criterion_id: "analytics".into(),
                    verdict: Verdict::Unsupported,
                    rationale: "Could not find a product analytics SDK or event capture call.".into(),
                    confidence: 0.9,
                    evidence: vec![],
                },
                codecaddie_domain::CriterionAssessment {
                    criterion_id: "instrumentation".into(),
                    verdict: Verdict::Supported,
                    rationale: "Datadog tracing is initialized and used at runtime.".into(),
                    confidence: 0.95,
                    evidence: vec![
                        evidence(EvidenceKind::Documentation, "README.md"),
                        evidence(EvidenceKind::Configuration, "datadog.yaml"),
                        evidence(EvidenceKind::Implementation, "src/telemetry.rs"),
                    ],
                },
                codecaddie_domain::CriterionAssessment {
                    criterion_id: "invalid".into(),
                    verdict: Verdict::Unverified,
                    rationale: "The submitted citation could not be validated.".into(),
                    confidence: 0.0,
                    evidence: vec![],
                },
            ],
        };
        let report = Report {
            id: "report-observability".into(),
            completed_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
            repositories: vec![],
            goal_version_ids: vec![goal.id.clone()],
            goal_set_hash: "goals".into(),
            provider: "test".into(),
            provider_version: "1".into(),
            origin: codecaddie_domain::ReportOrigin::Scan,
            assessments: vec![assessment],
            architecture: vec![],
            recommendations: vec![],
            coverage: None,
            unverified_criteria: 1,
            partial: false,
            analysis_warnings: vec![],
            codebase_map_id: None,
            codebase_map_hash: None,
        };
        let mut projection = WorkspaceProjection::default();
        projection
            .goal_versions
            .insert(goal.id.clone(), goal.clone());
        projection
            .approved_goals
            .insert(goal.goal_id.clone(), goal.id.clone());
        projection.reports.insert(report.id.clone(), report);

        let heatmap = build_report_heatmap(&projection, 4);
        let cell = &heatmap[0].cells[0];
        assert_eq!(
            cell.summary,
            "Partly — Datadog is instrumented for system telemetry, but product analytics such as PostHog were not found."
        );
        assert_eq!(cell.change, "First assessment for this goal");
        assert_eq!(cell.criteria[0].criterion_id, "instrumentation");
        assert_eq!(cell.criteria[1].criterion_id, "analytics");
        assert_eq!(cell.criteria[2].criterion_id, "invalid");
        assert_eq!(
            cell.criteria[0].evidence[0].kind,
            EvidenceKind::Implementation
        );
        assert_eq!(
            cell.criteria[0].evidence[1].kind,
            EvidenceKind::Configuration
        );
        assert_eq!(
            cell.criteria[0].evidence[2].kind,
            EvidenceKind::Documentation
        );
        let serialized = serde_json::to_string(&heatmap).unwrap();
        assert!(!serialized.contains("sourceText"));
        assert!(!serialized.contains("Datadog.init("));
    }

    #[test]
    fn history_projection_retains_the_latest_twelve_reports() {
        let goal = GoalVersion {
            id: "goal-version-history".into(),
            goal_id: "history".into(),
            title: "Keep progress visible".into(),
            business_outcome: "Leaders can compare repository decisions over time".into(),
            priority: 5,
            position: 1,
            criteria: vec![Criterion {
                id: "history-check".into(),
                text: "Automated tests verify retained analysis history".into(),
            }],
            rubric_dimensions: vec!["Business & product".into()],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "actor-test".into(),
            supersedes: None,
        };
        let mut projection = WorkspaceProjection::default();
        projection
            .goal_versions
            .insert(goal.id.clone(), goal.clone());
        projection
            .approved_goals
            .insert(goal.goal_id.clone(), goal.id.clone());
        for run in 1..=13 {
            let report_id = format!("report-{run}");
            let report_commit = format!("{run:040x}");
            projection
                .report_event_ids
                .insert(report_id.clone(), report_id.clone());
            projection
                .report_ordinals
                .insert(report_id.clone(), u32::try_from(run).unwrap());
            projection.reports.insert(
                report_id.clone(),
                Report {
                    id: report_id,
                    completed_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(run),
                    repositories: vec![FrozenRepository {
                        repository_id: "repository-1".into(),
                        commit_sha: report_commit.clone(),
                    }],
                    goal_version_ids: vec![goal.id.clone()],
                    goal_set_hash: "goal-set".into(),
                    provider: "test".into(),
                    provider_version: "1".into(),
                    origin: codecaddie_domain::ReportOrigin::Scan,
                    assessments: vec![GoalAssessment {
                        goal_version_id: goal.id.clone(),
                        verdict: Verdict::Partial,
                        summary: format!("Run {run} assessment"),
                        architecture_narrative: String::new(),
                        related_component_ids: vec![],
                        criteria: vec![codecaddie_domain::CriterionAssessment {
                            criterion_id: "history-check".into(),
                            verdict: Verdict::Partial,
                            rationale: format!("Run {run} evidence"),
                            confidence: 0.8,
                            evidence: vec![EvidenceRef {
                                repository_id: "repository-1".into(),
                                commit_sha: report_commit,
                                blob_oid: format!("{:040x}", run + 100),
                                path: "tests/history.rs".into(),
                                start_line: 10,
                                end_line: 20,
                                content_hash: format!("{:064x}", run + 200),
                                kind: EvidenceKind::Test,
                            }],
                        }],
                    }],
                    architecture: vec![],
                    recommendations: vec![],
                    coverage: Some(run as f64 / 13.0),
                    unverified_criteria: 0,
                    partial: false,
                    analysis_warnings: vec![],
                    codebase_map_id: None,
                    codebase_map_hash: None,
                },
            );
        }
        projection.report_completion_count = 13;

        let heatmap = build_report_heatmap(&projection, REPORT_HISTORY_LIMIT);
        assert_eq!(heatmap.len(), 12);
        assert_eq!(heatmap.first().unwrap().report_id, "report-2");
        assert_eq!(heatmap.last().unwrap().report_id, "report-13");
        assert_eq!(
            heatmap.first().unwrap().repositories,
            vec![format!("repository-1 @ {:040x}", 2)]
        );
        assert_eq!(
            heatmap.last().unwrap().repositories,
            vec![format!("repository-1 @ {:040x}", 13)]
        );
        assert_eq!(
            heatmap.first().unwrap().cells[0].change,
            "First assessment for this goal"
        );
        assert_eq!(
            heatmap.last().unwrap().cells[0].change,
            "Unchanged from Incomplete · field-level: 0 improved, 0 declined, 1 evidence-changed, 0 unchanged"
        );
        let latest = &heatmap.last().unwrap().cells[0].criteria[0];
        assert_eq!(latest.change_kind, "evidence_changed");
        assert_eq!(latest.previous_verdict.as_deref(), Some("partial"));
        assert_eq!(
            latest.previous_evidence[0].commit_sha,
            format!("{:040x}", 12)
        );
        assert_eq!(latest.evidence[0].commit_sha, format!("{:040x}", 13));
        assert_eq!(projection.reports.len(), 13);
        assert_eq!(
            projection.reports["report-12"].repositories[0].commit_sha,
            format!("{:040x}", 12)
        );
        assert_eq!(
            projection.reports["report-13"].repositories[0].commit_sha,
            format!("{:040x}", 13)
        );

        let newest = build_report_history_page(&projection, None, 5).unwrap();
        assert_eq!(newest.total_active_runs, 13);
        assert!(newest.has_older);
        assert_eq!(newest.next_before.as_deref(), Some("report-9"));
        assert_eq!(newest.runs.first().unwrap().run_number, 9);
        assert_eq!(newest.runs.last().unwrap().run_number, 13);
        assert!(newest.runs[0].cells[0].summary.contains("Run 9"));
        let earlier =
            build_report_history_page(&projection, newest.next_before.as_deref(), 5).unwrap();
        assert_eq!(earlier.runs.first().unwrap().run_number, 4);
        assert_eq!(earlier.runs.last().unwrap().run_number, 8);

        let finding = build_report_finding(&projection, "report-13", &goal.id).unwrap();
        assert_eq!(finding.report_id, "report-13");
        assert_eq!(finding.cells.len(), 1);
        assert_eq!(
            finding.cells[0].criteria[0].evidence[0].commit_sha,
            format!("{:040x}", 13)
        );

        projection.reports.remove("report-12");
        projection.report_event_ids.remove("report-12");
        projection
            .deleted_report_event_ids
            .insert("report-12".into());
        let after_deletion = build_report_history_page(&projection, None, 20).unwrap();
        assert_eq!(after_deletion.total_active_runs, 12);
        assert_eq!(after_deletion.runs.last().unwrap().run_number, 13);
        assert!(!after_deletion.runs.iter().any(|run| run.run_number == 12));
        let recomputed = build_report_finding(&projection, "report-13", &goal.id).unwrap();
        assert_eq!(
            recomputed.cells[0].criteria[0].previous_evidence[0].commit_sha,
            format!("{:040x}", 11)
        );
    }
}
