//! Shared fixtures for the analyzer submodule tests: a committed throwaway
//! repository, a bindable raw provider analysis, and a complete generated
//! goal set that passes draft validation.

use super::{
    GoalDraft, RawAnalysis, RawArchitectureClaim, RawCriterionAssessment, RawEvidence,
    RawGoalAssessment, RawRecommendation,
};
use crate::repository::LocalRepository;
use codecaddie_domain::{Criterion, EvidenceKind, GoalVersion, Verdict};
use std::{fs, process::Command};
use tempfile::TempDir;
use time::OffsetDateTime;

pub(super) fn fixture() -> (TempDir, LocalRepository, String, GoalVersion) {
    let directory = tempfile::tempdir().unwrap();
    Command::new("git")
        .args(["init", "--quiet"])
        .arg(directory.path())
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(directory.path())
        .args(["config", "user.email", "test@local"])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(directory.path())
        .args(["config", "user.name", "Test"])
        .status()
        .unwrap();
    fs::write(
        directory.path().join("tenant.rs"),
        "fn invoice(tenant: Id) {\n    scoped(tenant);\n}\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("uncited.txt"),
        "internal reconciliation token stays local\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("adversarial_repository.rs"),
        crate::privacy_test_support::REPOSITORY_FIXTURE,
    )
    .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(directory.path())
        .args(["add", "."])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(directory.path())
        .args(["commit", "--quiet", "-m", "fixture"])
        .status()
        .unwrap();
    let repository = LocalRepository::attach("repo", directory.path()).unwrap();
    let commit = repository.head().unwrap();
    let goal = GoalVersion {
        id: "goal-v1".into(),
        goal_id: "goal".into(),
        title: "Tenant isolation".into(),
        business_outcome: "Customers cannot cross boundaries".into(),
        priority: 5,
        position: 1,
        criteria: vec![Criterion {
            id: "criterion-1".into(),
            text: "Every invoice read is tenant scoped".into(),
        }],
        rubric_dimensions: vec!["security".into()],
        created_at: OffsetDateTime::UNIX_EPOCH,
        created_by: "owner".into(),
        supersedes: None,
    };
    (directory, repository, commit, goal)
}

pub(super) fn raw() -> RawAnalysis {
    let evidence = RawEvidence {
        repository_id: "repo".into(),
        path: "tenant.rs".into(),
        start_line: 1,
        end_line: 2,
        kind: EvidenceKind::Implementation,
    };
    RawAnalysis {
        provider_version: "test".into(),
        assessments: vec![RawGoalAssessment {
            goal_version_id: "goal-v1".into(),
            summary: "Tenant-scoped invoice lookup is implemented in the billing repository."
                .into(),
            architecture_narrative: None,
            related_component_ids: None,
            criteria: vec![RawCriterionAssessment {
                criterion_id: "criterion-1".into(),
                verdict: Verdict::Supported,
                rationale: "Scoped lookup".into(),
                confidence: 0.9,
                evidence: vec![evidence.clone()],
            }],
        }],
        architecture: vec![RawArchitectureClaim {
            id: "claim".into(),
            component: "billing".into(),
            relationship: None,
            summary: "Billing scopes invoices".into(),
            affected_goal_version_ids: vec!["goal-v1".into()],
            component_id: None,
            evidence: vec![evidence.clone()],
        }],
        recommendations: vec![RawRecommendation {
            id: "rec".into(),
            title: "Keep the denial test".into(),
            rationale: "Protect the boundary".into(),
            expected_business_impact: "Prevents data exposure".into(),
            goal_version_ids: vec!["goal-v1".into()],
            evidence: vec![evidence],
            rank: 1,
        }],
    }
}

pub(super) fn generated_goal(group: &str, title: &str, criteria: &[&str]) -> GoalDraft {
    GoalDraft {
        goal_id: None,
        key: title
            .to_lowercase()
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-"),
        title: title.into(),
        business_outcome: format!("{title} protects a durable customer outcome."),
        priority: 4,
        criteria: criteria.iter().map(|item| (*item).to_string()).collect(),
        rubric_dimensions: vec![group.into()],
        grounding_fact_ids: Vec::new(),
    }
}

pub(super) fn complete_generated_goal_set() -> Vec<GoalDraft> {
    vec![
        generated_goal(
            "Business & product",
            "Customers complete the core job",
            &[
                "The core workflow reaches a result through implemented code paths",
                "The delivered result is rendered in the customer UI",
            ],
        ),
        generated_goal(
            "Business & product",
            "New customers reach value quickly",
            &[
                "The setup workflow reaches first value through implemented code paths",
                "Setup failure states have implemented recovery paths",
                "Product usage metrics are instrumented with cohort analysis",
            ],
        ),
        generated_goal(
            "Business & product",
            "Teams can prove product value",
            &[
                "Product outcome metrics are emitted by instrumentation code",
                "A shareable result export is implemented and tested",
            ],
        ),
        generated_goal(
            "Architecture & platform",
            "Customer data stays isolated",
            &[
                "Multi-tenant queries enforce tenant isolation",
                "Cross-tenant access tests fail closed",
                "A version-controlled agent contract and repository-local verification harness prove changes at runtime before merge",
            ],
        ),
        generated_goal(
            "Architecture & platform",
            "Integrations remain dependable",
            &[
                "Versioned APIs preserve contracts",
                "Integration retries are idempotent",
                "Capacity tests prove enterprise throughput and latency targets",
            ],
        ),
        generated_goal(
            "Architecture & platform",
            "Changes reach customers safely",
            &[
                "Automated tests run in CI gates",
                "Static analysis blocks known bug classes",
                "Dependency vulnerabilities are scanned in CI and daily",
                "Automated dependency updates keep critical patches small",
                "Dependency licenses are inventoried for commercial distribution",
                "A documented bootstrap is verified from a clean machine",
            ],
        ),
        generated_goal(
            "Operations & reliability",
            "Teams see problems before customers",
            &[
                "Monitoring and error tracking cover paid flows",
                "Metrics and alerting map to SLOs",
            ],
        ),
        generated_goal(
            "Operations & reliability",
            "Customer data survives failures",
            &[
                "Backup restore is tested",
                "Migrations have a safe recovery path",
            ],
        ),
        generated_goal(
            "Operations & reliability",
            "Releases remain secure and supportable",
            &[
                "Audit logging covers security changes",
                "Runbooks define incident recovery",
                "Webhook delivery uses signed payloads, bounded retries, replay controls, and dead-letter recovery",
            ],
        ),
    ]
}
