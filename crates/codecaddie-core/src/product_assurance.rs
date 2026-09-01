//! Compact executable journeys for CodeCaddie's repository-owned product
//! assurance. These tests deliberately cross the analyzer, immutable-evidence,
//! signed-persistence, history, action, and local-runtime boundaries instead of
//! checking that configuration merely names them.

use crate::{
    analyzer::{
        RawAnalysis, RawCriterionAssessment, RawEvidence, RawGoalAssessment, RawRecommendation,
        materialize_agent_report_for_repositories,
    },
    local_state::{
        ApproveGoalRequest, CreateWorkspaceRequest, LocalWorkspaceStore, ProjectContext,
    },
    reliability,
    repository::LocalRepository,
};
use codecaddie_domain::{EvidenceKind, ProductEventKind, ReliabilityOutcome, Verdict};
use std::{fs, path::Path, process::Command, time::Instant};

fn checked(command: &mut Command) {
    assert!(command.status().unwrap().success());
}

fn repository(root: &Path) -> (LocalRepository, String) {
    let path = root.join("repository");
    fs::create_dir_all(&path).unwrap();
    checked(Command::new("git").args(["init", "--quiet"]).arg(&path));
    checked(Command::new("git").args(["-C"]).arg(&path).args([
        "config",
        "user.email",
        "assurance@local",
    ]));
    checked(Command::new("git").args(["-C"]).arg(&path).args([
        "config",
        "user.name",
        "CodeCaddie assurance",
    ]));
    fs::write(
        path.join("journey.rs"),
        "pub fn save_report() -> bool { true }\n",
    )
    .unwrap();
    checked(
        Command::new("git")
            .args(["-C"])
            .arg(&path)
            .args(["add", "."]),
    );
    checked(Command::new("git").args(["-C"]).arg(&path).args([
        "commit",
        "--quiet",
        "-m",
        "first report proof",
    ]));
    let repository = LocalRepository::attach("attached-repository", &path).unwrap();
    let commit = repository.head().unwrap();
    (repository, commit)
}

fn supported_analysis(goal_version_id: &str, criterion_id: &str, suffix: &str) -> RawAnalysis {
    let evidence = RawEvidence {
        repository_id: "attached-repository".into(),
        path: "journey.rs".into(),
        start_line: 1,
        end_line: 1,
        kind: EvidenceKind::Test,
    };
    RawAnalysis {
        provider_version: "assurance-v1".into(),
        assessments: vec![RawGoalAssessment {
            goal_version_id: goal_version_id.into(),
            summary: format!("The saved-report journey is executable {suffix}."),
            architecture_narrative: None,
            related_component_ids: None,
            criteria: vec![RawCriterionAssessment {
                criterion_id: criterion_id.into(),
                verdict: Verdict::Supported,
                rationale: "The committed journey proof resolves at the frozen commit.".into(),
                confidence: 1.0,
                evidence: vec![evidence.clone()],
            }],
        }],
        architecture: vec![],
        recommendations: vec![RawRecommendation {
            id: format!("next-action-{suffix}"),
            title: "Keep the saved-report journey executable".into(),
            rationale: "A ranked action keeps the decision tied to tested evidence.".into(),
            expected_business_impact: "Preserves first-session decision confidence.".into(),
            goal_version_ids: vec![goal_version_id.into()],
            evidence: vec![evidence],
            rank: 1,
        }],
    }
}

pub(crate) struct FirstReportJourneyProof {
    pub workspace_creations: u32,
    pub goal_approvals: u32,
    pub exact_commit_saved: bool,
    pub supported_scorecard_saved: bool,
    pub prioritized_action_saved: bool,
    pub analysis_starts: u32,
    pub scorecards_generated: u32,
    pub reports_saved: u32,
    pub time_to_first_report_recorded: bool,
}

pub(crate) fn exercise_first_report_journey() -> FirstReportJourneyProof {
    let directory = tempfile::tempdir().unwrap();
    let (repository, commit) = repository(directory.path());
    let data_root = directory.path().join("data");
    let store = LocalWorkspaceStore::new(data_root.clone()).unwrap();
    let workspace = store
        .create_workspace(CreateWorkspaceRequest {
            name: "First report assurance".into(),
            repository_display_name: "repository".into(),
            repository_path: repository.path.to_string_lossy().into_owned(),
            product_brief: "Turn approved goals into a saved repository scorecard.".into(),
            context: ProjectContext::default(),
        })
        .unwrap();
    assert!(
        store
            .recent_workspace()
            .unwrap()
            .unwrap()
            .latest_report
            .is_none()
    );

    let goal = store
        .approve_goal(
            &workspace.workspace_id,
            ApproveGoalRequest {
                goal_id: "first-report".into(),
                title: "The first report supports a decision".into(),
                business_outcome: "A product owner can act on repository evidence.".into(),
                criteria: vec![
                    "A tested repository journey persists a scorecard and ranked action.".into(),
                ],
                priority: 5,
                position: 1,
                rubric_dimensions: vec!["Business & product".into()],
            },
        )
        .unwrap();
    store
        .record_analysis_started(&workspace.workspace_id, "first-report-session")
        .unwrap();
    let report = materialize_agent_report_for_repositories(
        "first-report-scorecard".into(),
        &[(repository.clone(), commit.clone())],
        "assurance-provider".into(),
        std::slice::from_ref(&goal),
        supported_analysis(&goal.id, &goal.criteria[0].id, "first"),
    )
    .unwrap();
    assert_eq!(report.coverage, Some(1.0));
    assert_eq!(report.recommendations.len(), 1);
    assert_eq!(report.recommendations[0].rank, 1);
    store
        .record_report_with_repositories(
            &workspace.workspace_id,
            report,
            std::slice::from_ref(&repository),
        )
        .unwrap();
    drop(store);

    let reopened = LocalWorkspaceStore::new(data_root).unwrap();
    let recent = reopened.recent_workspace().unwrap().unwrap();
    let saved = recent.latest_report.unwrap();
    assert_eq!(saved.id, "first-report-scorecard");
    assert_eq!(saved.repositories[0].commit_sha, commit);
    assert_eq!(saved.assessments[0].verdict, Verdict::Supported);
    assert_eq!(saved.recommendations[0].id, "next-action-first");
    assert_eq!(recent.decision_funnel.analysis_starts, 1);
    assert_eq!(recent.decision_funnel.scorecards_generated, 1);
    assert_eq!(recent.decision_funnel.reports_saved, 1);
    FirstReportJourneyProof {
        workspace_creations: recent.decision_funnel.workspace_creations,
        goal_approvals: recent.decision_funnel.goal_approvals,
        exact_commit_saved: saved.repositories[0].commit_sha == commit,
        supported_scorecard_saved: saved.assessments[0].verdict == Verdict::Supported,
        prioritized_action_saved: saved.recommendations[0].rank == 1,
        analysis_starts: recent.decision_funnel.analysis_starts,
        scorecards_generated: recent.decision_funnel.scorecards_generated,
        reports_saved: recent.decision_funnel.reports_saved,
        time_to_first_report_recorded: recent
            .decision_funnel
            .time_to_first_report_seconds
            .is_some(),
    }
}

#[test]
fn repeatable_first_report_load_stays_within_versioned_p95_budget() {
    const SAMPLE_COUNT: usize = 12;
    let policy: serde_json::Value =
        serde_json::from_str(include_str!("../../../config/reliability-gates.json")).unwrap();
    let budget_milliseconds = policy["performance"]["firstSavedReportP95Seconds"]
        .as_u64()
        .unwrap()
        * 1_000;
    let mut latencies = Vec::with_capacity(SAMPLE_COUNT);

    for _ in 0..SAMPLE_COUNT {
        let started = Instant::now();
        let proof = exercise_first_report_journey();
        assert!(proof.exact_commit_saved);
        assert!(proof.supported_scorecard_saved);
        assert!(proof.prioritized_action_saved);
        latencies.push(started.elapsed().as_millis());
    }

    latencies.sort_unstable();
    let p95_index = ((latencies.len() * 95).div_ceil(100)).saturating_sub(1);
    let p95 = latencies[p95_index];
    assert!(
        p95 <= u128::from(budget_milliseconds),
        "repeatable first-report p95 {p95}ms exceeded the versioned {budget_milliseconds}ms target"
    );
}

pub(crate) struct SavedEvidenceProjectionProof {
    pub checkout_changed: bool,
    pub report_commit_preserved: bool,
    pub displayed_reference_preserved: bool,
    pub original_blob_reopened: bool,
}

pub(crate) fn exercise_saved_evidence_after_checkout_switch() -> SavedEvidenceProjectionProof {
    let directory = tempfile::tempdir().unwrap();
    let (repository, analyzed_commit) = repository(directory.path());
    let data_root = directory.path().join("data");
    let store = LocalWorkspaceStore::new(data_root.clone()).unwrap();
    let workspace = store
        .create_workspace(CreateWorkspaceRequest {
            name: "Saved evidence projection assurance".into(),
            repository_display_name: "repository".into(),
            repository_path: repository.path.to_string_lossy().into_owned(),
            product_brief: "Keep the saved report pinned after checkout changes.".into(),
            context: ProjectContext::default(),
        })
        .unwrap();
    let goal = store
        .approve_goal(
            &workspace.workspace_id,
            ApproveGoalRequest {
                goal_id: "saved-evidence-projection".into(),
                title: "Saved evidence stays reviewable".into(),
                business_outcome: "A later checkout cannot rewrite a prior decision.".into(),
                criteria: vec![
                    "The displayed evidence remains pinned to the analyzed commit.".into(),
                ],
                priority: 5,
                position: 1,
                rubric_dimensions: vec!["Decision confidence".into()],
            },
        )
        .unwrap();
    let report = materialize_agent_report_for_repositories(
        "saved-report-before-checkout-switch".into(),
        &[(repository.clone(), analyzed_commit.clone())],
        "assurance-provider".into(),
        std::slice::from_ref(&goal),
        supported_analysis(&goal.id, &goal.criteria[0].id, "saved"),
    )
    .unwrap();
    let saved_evidence = report.assessments[0].criteria[0].evidence[0].clone();
    store
        .record_report_with_repositories(
            &workspace.workspace_id,
            report,
            std::slice::from_ref(&repository),
        )
        .unwrap();

    fs::write(
        repository.path.join("journey.rs"),
        "pub fn save_report() -> bool { false }\npub fn rewrite_history() -> bool { true }\n",
    )
    .unwrap();
    checked(
        Command::new("git")
            .args(["-C"])
            .arg(&repository.path)
            .args(["checkout", "-qb", "changed-after-analysis"]),
    );
    checked(
        Command::new("git")
            .args(["-C"])
            .arg(&repository.path)
            .args(["add", "journey.rs"]),
    );
    checked(
        Command::new("git")
            .args(["-C"])
            .arg(&repository.path)
            .args(["commit", "--quiet", "-m", "change checkout after analysis"]),
    );
    let current_commit = repository.head().unwrap();
    assert_ne!(current_commit, analyzed_commit);
    drop(store);

    let reopened = LocalWorkspaceStore::new(data_root).unwrap();
    let recent = reopened.recent_workspace().unwrap().unwrap();
    let saved = recent.latest_report.as_ref().unwrap();
    assert_eq!(saved.repositories[0].commit_sha, analyzed_commit);
    assert_eq!(saved.assessments[0].criteria[0].evidence[0], saved_evidence);
    let displayed = &recent.report_heatmap[0].cells[0].criteria[0].evidence[0];
    assert_eq!(displayed.commit_sha, analyzed_commit);
    assert_eq!(displayed.path, "journey.rs");
    assert_eq!(displayed.start_line, 1);
    assert_eq!(displayed.end_line, 1);

    let reopened_repository =
        LocalRepository::attach("attached-repository", &repository.path).unwrap();
    assert!(
        fs::read_to_string(repository.path.join("journey.rs"))
            .unwrap()
            .contains("rewrite_history")
    );
    reopened_repository
        .verify_evidence(&saved_evidence)
        .unwrap();
    let reopened_source = reopened_repository.read_evidence(&saved_evidence).unwrap();
    assert_eq!(reopened_source, "pub fn save_report() -> bool { true }");
    SavedEvidenceProjectionProof {
        checkout_changed: current_commit != analyzed_commit,
        report_commit_preserved: saved.repositories[0].commit_sha == analyzed_commit,
        displayed_reference_preserved: displayed.commit_sha == analyzed_commit
            && displayed.path == "journey.rs"
            && displayed.start_line == 1
            && displayed.end_line == 1,
        original_blob_reopened: reopened_source == "pub fn save_report() -> bool { true }",
    }
}

pub(crate) struct RepeatAnalysisComparisonProof {
    pub retained_reports: usize,
    pub distinct_commit_identities: bool,
    pub previous_evidence_preserved: bool,
    pub repeat_analyses: u32,
    pub report_opens: u32,
    pub evidence_opens: u32,
    pub comparisons_generated: u32,
    pub repeat_review_opens: u32,
}

pub(crate) fn exercise_repeat_analysis_comparison() -> RepeatAnalysisComparisonProof {
    let directory = tempfile::tempdir().unwrap();
    let (repository, first_commit) = repository(directory.path());
    let store = LocalWorkspaceStore::new(directory.path().join("data")).unwrap();
    let workspace = store
        .create_workspace(CreateWorkspaceRequest {
            name: "Immutable history assurance".into(),
            repository_display_name: "repository".into(),
            repository_path: repository.path.to_string_lossy().into_owned(),
            product_brief: "Keep every analysis tied to immutable evidence.".into(),
            context: ProjectContext::default(),
        })
        .unwrap();
    let goal = store
        .approve_goal(
            &workspace.workspace_id,
            ApproveGoalRequest {
                goal_id: "immutable-history".into(),
                title: "Repeat reports retain immutable proof".into(),
                business_outcome: "Reviewers can trust what changed between decisions.".into(),
                criteria: vec!["Every saved claim resolves at its recorded Git commit.".into()],
                priority: 5,
                position: 1,
                rubric_dimensions: vec!["Architecture & platform".into()],
            },
        )
        .unwrap();

    let first = materialize_agent_report_for_repositories(
        "immutable-report-1".into(),
        &[(repository.clone(), first_commit.clone())],
        "assurance-provider".into(),
        std::slice::from_ref(&goal),
        supported_analysis(&goal.id, &goal.criteria[0].id, "one"),
    )
    .unwrap();
    let mut missing_reference = first.clone();
    missing_reference.assessments[0].criteria[0]
        .evidence
        .clear();
    assert!(
        store
            .record_report(&workspace.workspace_id, missing_reference)
            .is_err()
    );
    let mut wrong_commit = first.clone();
    wrong_commit.repositories[0].commit_sha = "0".repeat(40);
    assert!(
        store
            .record_report(&workspace.workspace_id, wrong_commit)
            .is_err()
    );

    store
        .record_analysis_started(&workspace.workspace_id, "immutable-session-1")
        .unwrap();
    store
        .record_report_with_repositories(
            &workspace.workspace_id,
            first,
            std::slice::from_ref(&repository),
        )
        .unwrap();
    fs::write(
        repository.path.join("journey.rs"),
        "pub fn save_report() -> bool { true }\npub fn compare_reports() -> bool { true }\n",
    )
    .unwrap();
    checked(
        Command::new("git")
            .args(["-C"])
            .arg(&repository.path)
            .args(["add", "journey.rs"]),
    );
    checked(
        Command::new("git")
            .args(["-C"])
            .arg(&repository.path)
            .args(["commit", "--quiet", "-m", "comparison proof"]),
    );
    let second_commit = repository.head().unwrap();
    store
        .record_analysis_started(&workspace.workspace_id, "immutable-session-2")
        .unwrap();
    let second = materialize_agent_report_for_repositories(
        "immutable-report-2".into(),
        &[(repository.clone(), second_commit.clone())],
        "assurance-provider".into(),
        std::slice::from_ref(&goal),
        supported_analysis(&goal.id, &goal.criteria[0].id, "two"),
    )
    .unwrap();
    store
        .record_report_with_repositories(
            &workspace.workspace_id,
            second,
            std::slice::from_ref(&repository),
        )
        .unwrap();
    store
        .record_product_event(
            &workspace.workspace_id,
            ProductEventKind::ReportRevisited,
            "review-session",
            Some("immutable-report-2".into()),
        )
        .unwrap();
    store
        .record_product_event(
            &workspace.workspace_id,
            ProductEventKind::EvidenceOpened,
            "review-session",
            Some("immutable-report-2".into()),
        )
        .unwrap();

    drop(store);
    let reopened = LocalWorkspaceStore::new(directory.path().join("data")).unwrap();
    let recent = reopened.recent_workspace().unwrap().unwrap();
    assert_eq!(recent.report_heatmap.len(), 2);
    assert_eq!(recent.report_heatmap[0].report_id, "immutable-report-1");
    assert_eq!(recent.report_heatmap[1].report_id, "immutable-report-2");
    assert!(recent.report_heatmap[0].repositories[0].contains(&first_commit));
    assert!(recent.report_heatmap[1].repositories[0].contains(&second_commit));
    let first_check = &recent.report_heatmap[0].cells[0].criteria[0];
    let second_check = &recent.report_heatmap[1].cells[0].criteria[0];
    assert_eq!(first_check.change_kind, "first");
    assert_eq!(second_check.change_kind, "evidence_changed");
    assert_eq!(second_check.previous_verdict.as_deref(), Some("supported"));
    assert_eq!(second_check.previous_evidence, first_check.evidence);
    assert_eq!(second_check.previous_evidence[0].commit_sha, first_commit);
    assert_eq!(second_check.evidence[0].commit_sha, second_commit);
    assert_eq!(recent.decision_funnel.repeat_analyses, 1);
    assert_eq!(recent.decision_funnel.report_opens, 1);
    assert_eq!(recent.decision_funnel.evidence_opens, 1);
    assert_eq!(recent.decision_funnel.comparisons_generated, 1);
    assert_eq!(recent.decision_funnel.repeat_review_opens, 1);
    RepeatAnalysisComparisonProof {
        retained_reports: recent.report_heatmap.len(),
        distinct_commit_identities: first_commit != second_commit,
        previous_evidence_preserved: second_check.previous_evidence == first_check.evidence,
        repeat_analyses: recent.decision_funnel.repeat_analyses,
        report_opens: recent.decision_funnel.report_opens,
        evidence_opens: recent.decision_funnel.evidence_opens,
        comparisons_generated: recent.decision_funnel.comparisons_generated,
        repeat_review_opens: recent.decision_funnel.repeat_review_opens,
    }
}

#[test]
fn security_relevant_operations_append_signed_content_free_audit_records() {
    let directory = tempfile::tempdir().unwrap();
    let (repository, _) = repository(directory.path());
    let data_root = directory.path().join("data");
    let store = LocalWorkspaceStore::new(data_root.clone()).unwrap();
    let workspace = store
        .create_workspace(CreateWorkspaceRequest {
            name: "Security audit assurance".into(),
            repository_display_name: "repository".into(),
            repository_path: repository.path.to_string_lossy().into_owned(),
            product_brief: "Audit local privileged operations without product content.".into(),
            context: ProjectContext::default(),
        })
        .unwrap();
    let controls: serde_json::Value =
        serde_json::from_str(include_str!("../../../config/security-audit-controls.json")).unwrap();
    assert_eq!(controls["localOnly"], true);
    let operations = controls["controls"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|control| control["operationCodes"].as_array().unwrap())
        .map(|operation| operation.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(!operations.is_empty());
    for operation in &operations {
        let record = reliability::operation_record(
            reliability::new_correlation_id(),
            operation,
            ReliabilityOutcome::Succeeded,
            None,
            false,
            1,
        );
        let serialized = serde_json::to_string(&record).unwrap();
        for forbidden in [
            "repositorySource",
            "attachmentContent",
            "goalText",
            "freeText",
            "credential",
        ] {
            assert!(!serialized.contains(forbidden));
        }
        store
            .record_reliability_operation(&workspace.workspace_id, record)
            .unwrap();
    }
    drop(store);

    let reopened = LocalWorkspaceStore::new(data_root).unwrap();
    let summary = reopened.recent_workspace().unwrap().unwrap().reliability;
    assert_eq!(summary.operation_samples as usize, operations.len());
    assert_eq!(summary.operation_failures, 0);
    assert_eq!(summary.operation_cancellations, 0);
}
