//! Fail-closed validation at report persistence boundaries.
//!
//! Provider output is untrusted even after materialization. Before a report is
//! signed into the local ledger, every claim and prioritized action is tied to
//! a registered repository, a full frozen commit, and an evidence coordinate
//! that re-resolves to the same Git blob and excerpt hash. Validation returns
//! metadata-only errors and never exposes repository source.

use crate::repository::LocalRepository;
use codecaddie_domain::{
    EvidenceRef, Report, Verdict, WorkspaceProjection, aggregate_goal, score_report,
};
use std::collections::{BTreeMap, BTreeSet};

fn is_full_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_evidence_shape(evidence: &EvidenceRef) -> anyhow::Result<()> {
    if evidence.repository_id.trim().is_empty()
        || !is_full_object_id(&evidence.commit_sha)
        || !is_full_object_id(&evidence.blob_oid)
        || evidence.content_hash.len() != 64
        || !evidence
            .content_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || evidence.path.trim().is_empty()
        || evidence.start_line == 0
        || evidence.end_line < evidence.start_line
    {
        anyhow::bail!("report evidence metadata is incomplete");
    }
    Ok(())
}

pub(crate) fn validate_report_for_persistence(
    report: &Report,
    projection: &WorkspaceProjection,
    repositories: &[LocalRepository],
) -> anyhow::Result<()> {
    if report.id.trim().is_empty()
        || report.provider.trim().is_empty()
        || report.provider_version.trim().is_empty()
        || report.goal_set_hash.trim().is_empty()
        || report.repositories.is_empty()
        || report.goal_version_ids.is_empty()
    {
        anyhow::bail!("report persistence metadata is incomplete");
    }

    let mut repository_ids = BTreeSet::new();
    for frozen in &report.repositories {
        if !repository_ids.insert(frozen.repository_id.as_str())
            || !projection.repositories.contains_key(&frozen.repository_id)
            || !is_full_object_id(&frozen.commit_sha)
        {
            anyhow::bail!("report repositories are not uniquely frozen at full commits");
        }
    }
    let local_repositories = repositories
        .iter()
        .map(|repository| (repository.id.as_str(), repository))
        .collect::<BTreeMap<_, _>>();
    if local_repositories.len() != repositories.len()
        || local_repositories.len() != report.repositories.len()
    {
        anyhow::bail!("every frozen repository needs one local persistence verifier");
    }
    let frozen_repositories = report
        .repositories
        .iter()
        .map(|repository| (repository.repository_id.as_str(), repository))
        .collect::<BTreeMap<_, _>>();
    for frozen in &report.repositories {
        let repository = local_repositories
            .get(frozen.repository_id.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("a frozen repository has no local persistence verifier")
            })?;
        let resolved = repository.resolve_commit(&frozen.commit_sha).map_err(|_| {
            anyhow::anyhow!("an analyzed commit is unavailable for report verification")
        })?;
        if resolved != frozen.commit_sha {
            anyhow::bail!("the report must retain each full resolved analyzed commit");
        }
    }

    let report_goals = report
        .goal_version_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if report_goals.len() != report.goal_version_ids.len()
        || report_goals
            != projection
                .approved_goals
                .values()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
    {
        anyhow::bail!("report goal versions do not match the approved frozen set");
    }
    let frozen_goals = report
        .goal_version_ids
        .iter()
        .map(|version_id| {
            projection
                .goal_versions
                .get(version_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("report references an unknown goal version"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let expected_goal_set_hash = blake3::hash(&serde_json::to_vec(&frozen_goals)?)
        .to_hex()
        .to_string();
    if report.goal_set_hash != expected_goal_set_hash {
        anyhow::bail!("report goal-set hash does not match the frozen goal versions");
    }

    let mut assessed_goals = BTreeSet::new();
    for assessment in &report.assessments {
        if !assessed_goals.insert(assessment.goal_version_id.as_str())
            || !report_goals.contains(assessment.goal_version_id.as_str())
        {
            anyhow::bail!("report assessments do not uniquely cover the frozen goals");
        }
        let goal = projection
            .goal_versions
            .get(&assessment.goal_version_id)
            .ok_or_else(|| {
                anyhow::anyhow!("report assessment references an unknown goal version")
            })?;
        let expected_criteria = goal
            .criteria
            .iter()
            .map(|criterion| criterion.id.as_str())
            .collect::<BTreeSet<_>>();
        let actual_criteria = assessment
            .criteria
            .iter()
            .map(|criterion| criterion.criterion_id.as_str())
            .collect::<BTreeSet<_>>();
        if expected_criteria != actual_criteria
            || actual_criteria.len() != assessment.criteria.len()
        {
            anyhow::bail!("report criteria do not exactly cover the approved goal version");
        }
        if assessment.verdict != aggregate_goal(assessment) {
            anyhow::bail!("report goal verdict does not match its criterion results");
        }
        for criterion in &assessment.criteria {
            if matches!(criterion.verdict, Verdict::Supported | Verdict::Partial)
                && criterion.evidence.is_empty()
            {
                anyhow::bail!("supported and partial scorecard claims require immutable evidence");
            }
        }
    }
    if assessed_goals != report_goals {
        anyhow::bail!("report assessments do not cover every frozen goal");
    }

    let score = score_report(&frozen_goals, &report.assessments);
    if report.coverage != score.coverage || report.unverified_criteria != score.unverified_criteria
    {
        anyhow::bail!("report score does not match its frozen criteria");
    }

    let mut architecture_ids = BTreeSet::new();
    for claim in &report.architecture {
        if claim.id.trim().is_empty()
            || claim.component.trim().is_empty()
            || claim.summary.trim().is_empty()
            || claim.evidence.is_empty()
            || claim
                .affected_goal_version_ids
                .iter()
                .any(|goal| !report_goals.contains(goal.as_str()))
            || !architecture_ids.insert(claim.id.as_str())
        {
            anyhow::bail!(
                "architecture claims require unique ids, valid goal links, and immutable evidence"
            );
        }
    }
    let mut recommendation_ids = BTreeSet::new();
    let mut recommendation_ranks = BTreeSet::new();
    for recommendation in &report.recommendations {
        if recommendation.id.trim().is_empty()
            || recommendation.title.trim().is_empty()
            || recommendation.evidence.is_empty()
            || recommendation.goal_version_ids.is_empty()
            || recommendation
                .goal_version_ids
                .iter()
                .any(|goal| !report_goals.contains(goal.as_str()))
            || !recommendation_ids.insert(recommendation.id.as_str())
            || !recommendation_ranks.insert(recommendation.rank)
        {
            anyhow::bail!(
                "prioritized actions require unique ids, ranks, goals, and immutable evidence"
            );
        }
    }

    let all_evidence = report
        .assessments
        .iter()
        .flat_map(|assessment| assessment.criteria.iter())
        .flat_map(|criterion| criterion.evidence.iter())
        .chain(
            report
                .architecture
                .iter()
                .flat_map(|claim| claim.evidence.iter()),
        )
        .chain(
            report
                .recommendations
                .iter()
                .flat_map(|recommendation| recommendation.evidence.iter()),
        );
    for evidence in all_evidence {
        validate_evidence_shape(evidence)?;
        let frozen = frozen_repositories
            .get(evidence.repository_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("report evidence names an unfrozen repository"))?;
        let repository = local_repositories
            .get(evidence.repository_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("report evidence has no local persistence verifier"))?;
        if evidence.commit_sha != frozen.commit_sha {
            anyhow::bail!("report evidence is not bound to its frozen repository commit");
        }
        repository.verify_evidence(evidence).map_err(|_| {
            anyhow::anyhow!("report evidence cannot be resolved at the frozen commit")
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_domain::{
        ArchitectureClaim, Criterion, CriterionAssessment, EvidenceKind, FrozenRepository,
        GoalAssessment, GoalVersion, Recommendation, RepositoryRef, Verdict,
    };
    use std::{fs, process::Command};
    use time::OffsetDateTime;

    fn git(directory: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn fixture() -> (tempfile::TempDir, WorkspaceProjection, Report) {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-q"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(
            directory.path(),
            &["config", "user.name", "CodeCaddie Test"],
        );
        fs::write(directory.path().join("proof.txt"), "immutable proof\n").unwrap();
        git(directory.path(), &["add", "proof.txt"]);
        git(directory.path(), &["commit", "-qm", "proof"]);
        let repository = LocalRepository::attach("repository-1", directory.path()).unwrap();
        let commit = repository.head().unwrap();
        let evidence = repository
            .evidence(&commit, "proof.txt", 1, 1, EvidenceKind::Test)
            .unwrap();
        let goal = GoalVersion {
            id: "goal-version-1".into(),
            goal_id: "goal-1".into(),
            title: "Trust every saved claim".into(),
            business_outcome: "Keep decisions auditable".into(),
            priority: 5,
            position: 1,
            criteria: vec![Criterion {
                id: "criterion-1".into(),
                text: "Every saved claim resolves at its frozen commit".into(),
            }],
            rubric_dimensions: vec!["Trust".into()],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "actor".into(),
            supersedes: None,
        };
        let mut projection = WorkspaceProjection::default();
        projection.repositories.insert(
            "repository-1".into(),
            RepositoryRef {
                id: "repository-1".into(),
                display_name: "Repository".into(),
                remote_fingerprint: None,
            },
        );
        projection
            .goal_versions
            .insert(goal.id.clone(), goal.clone());
        projection
            .approved_goals
            .insert(goal.goal_id.clone(), goal.id.clone());
        let report = Report {
            id: "report-1".into(),
            completed_at: OffsetDateTime::UNIX_EPOCH,
            repositories: vec![FrozenRepository {
                repository_id: "repository-1".into(),
                commit_sha: commit,
            }],
            goal_version_ids: vec![goal.id.clone()],
            goal_set_hash: blake3::hash(&serde_json::to_vec(std::slice::from_ref(&goal)).unwrap())
                .to_hex()
                .to_string(),
            provider: "test".into(),
            provider_version: "test-1".into(),
            origin: Default::default(),
            assessments: vec![GoalAssessment {
                goal_version_id: goal.id.clone(),
                verdict: Verdict::Supported,
                summary: "The report is commit-resolvable.".into(),
                architecture_narrative: String::new(),
                related_component_ids: vec![],
                criteria: vec![CriterionAssessment {
                    criterion_id: "criterion-1".into(),
                    verdict: Verdict::Supported,
                    rationale: "The cited test proves the boundary.".into(),
                    confidence: 1.0,
                    evidence: vec![evidence.clone()],
                }],
            }],
            architecture: vec![ArchitectureClaim {
                id: "architecture-1".into(),
                component: "Persistence boundary".into(),
                relationship: None,
                summary: "Claims are checked before signing.".into(),
                affected_goal_version_ids: vec![goal.id.clone()],
                component_id: None,
                evidence: vec![evidence.clone()],
            }],
            recommendations: vec![Recommendation {
                id: "recommendation-1".into(),
                title: "Keep the gate".into(),
                rationale: "The persisted proof protects review.".into(),
                expected_business_impact: "Decisions remain auditable.".into(),
                goal_version_ids: vec![goal.id],
                evidence: vec![evidence],
                rank: 1,
            }],
            coverage: Some(1.0),
            unverified_criteria: 0,
            partial: false,
            analysis_warnings: vec![],
            codebase_map_id: None,
            codebase_map_hash: None,
        };
        (directory, projection, report)
    }

    #[test]
    fn persistence_accepts_only_commit_resolvable_claims_and_actions() {
        let (directory, projection, report) = fixture();
        let repository = LocalRepository::attach("repository-1", directory.path()).unwrap();
        validate_report_for_persistence(&report, &projection, std::slice::from_ref(&repository))
            .unwrap();

        let mut missing = report.clone();
        missing.assessments[0].criteria[0].evidence.clear();
        assert!(
            validate_report_for_persistence(
                &missing,
                &projection,
                std::slice::from_ref(&repository),
            )
            .unwrap_err()
            .to_string()
            .contains("require immutable evidence")
        );

        let mut stale = report;
        stale.recommendations[0].evidence[0].content_hash = "0".repeat(64);
        assert!(
            validate_report_for_persistence(&stale, &projection, &[repository])
                .unwrap_err()
                .to_string()
                .contains("cannot be resolved")
        );
    }

    #[test]
    fn persistence_rejects_missing_or_unresolvable_evidence_on_every_claim_and_action() {
        let (directory, projection, report) = fixture();
        let repository = LocalRepository::attach("repository-1", directory.path()).unwrap();

        let mut missing_criterion = report.clone();
        missing_criterion.assessments[0].criteria[0]
            .evidence
            .clear();
        assert!(
            validate_report_for_persistence(
                &missing_criterion,
                &projection,
                std::slice::from_ref(&repository),
            )
            .is_err()
        );

        let mut missing_architecture = report.clone();
        missing_architecture.architecture[0].evidence.clear();
        assert!(
            validate_report_for_persistence(
                &missing_architecture,
                &projection,
                std::slice::from_ref(&repository),
            )
            .is_err()
        );

        let mut missing_action = report.clone();
        missing_action.recommendations[0].evidence.clear();
        assert!(
            validate_report_for_persistence(
                &missing_action,
                &projection,
                std::slice::from_ref(&repository),
            )
            .is_err()
        );

        for unresolvable in [
            {
                let mut value = report.clone();
                value.assessments[0].criteria[0].evidence[0].content_hash = "0".repeat(64);
                value
            },
            {
                let mut value = report.clone();
                value.architecture[0].evidence[0].content_hash = "0".repeat(64);
                value
            },
            {
                let mut value = report.clone();
                value.recommendations[0].evidence[0].content_hash = "0".repeat(64);
                value
            },
        ] {
            assert!(
                validate_report_for_persistence(
                    &unresolvable,
                    &projection,
                    std::slice::from_ref(&repository),
                )
                .unwrap_err()
                .to_string()
                .contains("cannot be resolved")
            );
        }

        let mut wrong_commit = report;
        wrong_commit.assessments[0].criteria[0].evidence[0].commit_sha = "f".repeat(40);
        assert!(
            validate_report_for_persistence(&wrong_commit, &projection, &[repository])
                .unwrap_err()
                .to_string()
                .contains("not bound to its frozen repository commit")
        );
    }

    #[test]
    fn prioritized_action_reference_rejection_matrix_is_complete() {
        let (directory, projection, report) = fixture();
        let repository = LocalRepository::attach("repository-1", directory.path()).unwrap();
        validate_report_for_persistence(&report, &projection, std::slice::from_ref(&repository))
            .unwrap();

        type MutateAction = fn(&mut Recommendation);
        let cases: [(&str, MutateAction); 9] = [
            ("missing evidence", |action| action.evidence.clear()),
            ("missing repository identifier", |action| {
                action.evidence[0].repository_id.clear()
            }),
            ("truncated commit", |action| {
                action.evidence[0].commit_sha = "abc123".into()
            }),
            ("truncated blob", |action| {
                action.evidence[0].blob_oid = "abc123".into()
            }),
            ("missing path", |action| action.evidence[0].path.clear()),
            ("zero start line", |action| {
                action.evidence[0].start_line = 0
            }),
            ("reversed line range", |action| {
                action.evidence[0].start_line = 2;
                action.evidence[0].end_line = 1;
            }),
            ("wrong frozen commit", |action| {
                action.evidence[0].commit_sha = "f".repeat(40)
            }),
            ("unresolvable content hash", |action| {
                action.evidence[0].content_hash = "0".repeat(64)
            }),
        ];
        for (name, mutate) in cases {
            let mut invalid = report.clone();
            mutate(&mut invalid.recommendations[0]);
            assert!(
                validate_report_for_persistence(
                    &invalid,
                    &projection,
                    std::slice::from_ref(&repository),
                )
                .is_err(),
                "prioritized action case must fail closed: {name}"
            );
        }
    }
}
