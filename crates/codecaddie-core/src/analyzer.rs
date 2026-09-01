//! Repository analysis, split along its seams: scan orchestration
//! (`scan`), goal drafting and validation (`goal_drafts`), report
//! materialization with evidence binding and leak defense
//! (`report_materialize`), and the provider prompts, schemas, and raw
//! output types they share (`analysis_contract`).

mod analysis_contract;
mod assurance;
mod goal_catalog;
mod goal_drafts;
mod map_generate;
mod map_materialize;
mod product_profile;
mod report_materialize;
mod scan;
#[cfg(test)]
mod test_support;

pub use analysis_contract::{
    ANALYSIS_SCHEMA, CODEBASE_MAP_DEEP_DIVE_SCHEMA, CODEBASE_MAP_SCHEMA,
    ENGINEERING_HEALTH_CHECKLIST, GOAL_GENERATION_RUBRIC, GOAL_GENERATION_SCHEMA,
    PRODUCT_FEATURE_FEEDBACK_SKILL, PRODUCT_KEY_MILESTONE_CHECKLIST, PRODUCT_PLAN_FEEDBACK_SKILL,
    RawAnalysis, RawArchitectureClaim, RawCriterionAssessment, RawEvidence, RawGoalAssessment,
    RawMapDeepDive, RawMapSurvey, RawRecommendation, goal_generation_prompt,
};
pub use goal_drafts::{
    ExistingGoalIdentity, GoalDraft, GoalGenerationRequest, GoalGenerationResult,
    generate_goal_drafts,
};
pub(crate) use goal_drafts::{
    validate_approved_goal_request, validate_approved_goal_set, validate_edited_goal_set,
};
pub use map_generate::{MapGenerationRequest, generate_codebase_map};
pub(crate) use map_materialize::{MapNarrativePolicy, materialize_codebase_map};
pub use report_materialize::materialize_report;
pub(crate) use report_materialize::{
    materialize_agent_report_for_repositories, materialize_report_for_repositories,
};
pub use scan::{ScanRepository, ScanRequest, run_scan, run_scan_with_map};

/// Counts regular files physically available in a disposable provider
/// workspace without following links. This is the denominator shown in the
/// activity feed; it is not a claim that the provider will read every file.
fn provider_workspace_file_count(root: &std::path::Path) -> anyhow::Result<usize> {
    let mut count = 0_usize;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                count = count.saturating_add(1);
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod workspace_file_count_tests {
    use super::*;

    #[test]
    fn counts_only_regular_files_available_in_the_provider_workspace() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("repository-0/src")).unwrap();
        std::fs::write(directory.path().join("repository-0/Cargo.toml"), "").unwrap();
        std::fs::write(directory.path().join("repository-0/src/lib.rs"), "").unwrap();
        assert_eq!(provider_workspace_file_count(directory.path()).unwrap(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_links_while_counting_provider_files() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("private.txt"), "secret").unwrap();
        symlink(outside.path(), directory.path().join("linked")).unwrap();
        assert_eq!(provider_workspace_file_count(directory.path()).unwrap(), 0);
    }
}
