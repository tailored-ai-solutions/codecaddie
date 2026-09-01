//! Small executable entrypoints for the complete first-report and saved-report
//! decision journeys. The reusable harnesses exercise production persistence,
//! report materialization, immutable Git evidence, restart, and projection.

#[test]
fn approved_goals_and_repository_selection_produce_a_persisted_scorecard_and_ranked_action() {
    let proof = crate::product_assurance::exercise_first_report_journey();
    assert_eq!(proof.workspace_creations, 1);
    assert_eq!(proof.goal_approvals, 1);
    assert!(proof.exact_commit_saved);
    assert!(proof.supported_scorecard_saved);
    assert!(proof.prioritized_action_saved);
    assert_eq!(proof.analysis_starts, 1);
    assert_eq!(proof.scorecards_generated, 1);
    assert_eq!(proof.reports_saved, 1);
    assert!(proof.time_to_first_report_recorded);
}

#[test]
fn repeat_analysis_retains_both_commits_and_emits_review_and_comparison_lifecycle_events() {
    let proof = crate::product_assurance::exercise_repeat_analysis_comparison();
    assert_eq!(proof.retained_reports, 2);
    assert!(proof.distinct_commit_identities);
    assert!(proof.previous_evidence_preserved);
    assert_eq!(proof.repeat_analyses, 1);
    assert_eq!(proof.report_opens, 1);
    assert_eq!(proof.evidence_opens, 1);
    assert_eq!(proof.comparisons_generated, 1);
    assert_eq!(proof.repeat_review_opens, 1);
}

#[test]
fn switching_the_working_tree_cannot_change_saved_or_displayed_evidence() {
    let proof = crate::product_assurance::exercise_saved_evidence_after_checkout_switch();
    assert!(proof.checkout_changed);
    assert!(proof.report_commit_preserved);
    assert!(proof.displayed_reference_preserved);
    assert!(proof.original_blob_reopened);
}
