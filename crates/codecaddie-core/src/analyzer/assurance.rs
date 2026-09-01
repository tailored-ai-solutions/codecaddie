//! Optional repository-owned routing metadata for operational assurance.
//!
//! `.codecaddie/assurance.json` is never evidence. It is a small, validated
//! index that helps a bounded provider pass find the repository artifacts it
//! still has to inspect and cite. Invalid or unsafe indexes are ignored so
//! untrusted repository text cannot break a scan.

use codecaddie_domain::GoalVersion;
use serde::Deserialize;
use std::{
    cmp::Reverse,
    collections::BTreeSet,
    path::{Component, Path},
};

const MANIFEST_PATH: &str = ".codecaddie/assurance.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_CONTROLS: usize = 64;
const MAX_TOPICS_PER_CONTROL: usize = 12;
const MAX_ARTIFACTS_PER_CONTROL: usize = 12;
const MAX_DIGEST_BYTES: usize = 16 * 1024;
const MAX_CONTROLS_PER_GOAL: usize = 6;
const MAX_INLINE_ARTIFACTS_PER_CRITERION: usize = 3;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssuranceIndex {
    schema_version: u16,
    controls: Vec<AssuranceControl>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssuranceControl {
    id: String,
    topics: Vec<String>,
    artifacts: Vec<String>,
}

pub(super) fn repository_assurance_digest(
    workspace: &Path,
    repositories: &[(String, String)],
    goals: &[GoalVersion],
) -> Option<String> {
    let goal_routes = goals
        .iter()
        .map(|goal| {
            (
                goal.position,
                goal_terms(goal),
                goal.criteria
                    .iter()
                    .map(|criterion| criterion_terms(&criterion.text))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    if goal_routes.iter().all(|(_, terms, _)| terms.is_empty()) {
        return None;
    }

    let mut blocks = Vec::new();
    for (repository_id, directory) in repositories {
        let root = workspace.join(directory);
        let Some(index) = read_valid_index(&root) else {
            continue;
        };
        let mut lines = vec![format!("repositoryId: {repository_id}")];
        let mut definitions = Vec::new();
        let mut definition_ids = BTreeSet::new();
        for (goal_position, terms, criteria) in &goal_routes {
            let controls = select_goal_controls(&index.controls, terms, criteria);
            if controls.is_empty() {
                continue;
            }
            lines.push(format!(
                "goalPosition: {goal_position} | controlIds: {}",
                controls
                    .iter()
                    .map(|control| control.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            for (criterion_index, criterion_terms) in criteria.iter().enumerate() {
                if let Some(primary) = strongest_control(&index.controls, criterion_terms)
                    && controls.iter().any(|control| control.id == primary.id)
                {
                    lines.push(format!(
                        "goalPosition: {goal_position} | criterionPosition: {} | primaryControlId: {} | inspectFirst: {}",
                        criterion_index + 1,
                        primary.id,
                        primary
                            .artifacts
                            .iter()
                            .take(MAX_INLINE_ARTIFACTS_PER_CRITERION)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            for control in &controls {
                if definition_ids.insert(control.id.as_str()) {
                    definitions.push(*control);
                }
            }
        }
        if definitions.is_empty() {
            continue;
        }
        lines.push("controls:".into());
        for control in definitions {
            lines.push(format!(
                "- {} | topics: {} | inspectInOrder: {}",
                control.id,
                control.topics.join(", "),
                control.artifacts.join(", ")
            ));
        }
        blocks.push(lines.join("\n"));
    }
    if blocks.is_empty() {
        return None;
    }
    let digest = blocks.join("\n");
    (digest.len() <= MAX_DIGEST_BYTES).then_some(digest)
}

fn select_goal_controls<'a>(
    controls: &'a [AssuranceControl],
    goal_terms: &BTreeSet<String>,
    criterion_terms: &[BTreeSet<String>],
) -> Vec<&'a AssuranceControl> {
    let mut selected_ids = BTreeSet::new();
    let mut selected = Vec::new();

    // A goal may contain several materially different checks. Reserve each
    // criterion's strongest route before filling by whole-goal relevance so a
    // repeated topic in one check cannot crowd a smaller neighboring check out
    // of the bounded index.
    for terms in criterion_terms {
        if let Some(control) = strongest_control(controls, terms)
            && selected_ids.insert(control.id.as_str())
        {
            selected.push(control);
            if selected.len() == MAX_CONTROLS_PER_GOAL {
                return selected;
            }
        }
    }

    let mut ranked = controls
        .iter()
        .filter_map(|control| {
            let score = route_relevance(control, goal_terms);
            (score > 0).then_some((score, control))
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(score, control)| (Reverse(*score), control.id.clone()));
    for (_, control) in ranked {
        if selected_ids.insert(control.id.as_str()) {
            selected.push(control);
            if selected.len() == MAX_CONTROLS_PER_GOAL {
                break;
            }
        }
    }
    selected
}

fn strongest_control<'a>(
    controls: &'a [AssuranceControl],
    terms: &BTreeSet<String>,
) -> Option<&'a AssuranceControl> {
    controls
        .iter()
        .filter_map(|control| {
            let score = route_relevance(control, terms);
            (score > 0).then_some((score, control))
        })
        .min_by_key(|(score, control)| (Reverse(*score), control.id.as_str()))
        .map(|(_, control)| control)
}

fn goal_terms(goal: &GoalVersion) -> BTreeSet<String> {
    std::iter::once(goal.title.as_str())
        .chain(std::iter::once(goal.business_outcome.as_str()))
        .chain(
            goal.criteria
                .iter()
                .map(|criterion| criterion.text.as_str()),
        )
        .chain(goal.rubric_dimensions.iter().map(String::as_str))
        .flat_map(normalized_terms)
        .collect()
}

fn criterion_terms(criterion: &str) -> BTreeSet<String> {
    normalized_terms(criterion)
}

fn route_relevance(control: &AssuranceControl, goal_terms: &BTreeSet<String>) -> usize {
    let id_terms = normalized_terms(&control.id);
    let mut score = id_terms.intersection(goal_terms).count() * 4;
    for topic in &control.topics {
        let topic_terms = normalized_terms(topic);
        let overlap = topic_terms.intersection(goal_terms).count();
        score += overlap * 2;
        if overlap > 0 && overlap == topic_terms.len() {
            score += 4;
        }
    }
    score
}

fn normalized_terms(value: &str) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for raw in value.split(|character: char| !character.is_ascii_alphanumeric()) {
        let term = raw.to_ascii_lowercase();
        if term.len() < 3 {
            continue;
        }
        terms.insert(term.clone());
        if term.len() > 4 && term.ends_with('s') && !term.ends_with("ss") {
            terms.insert(term[..term.len() - 1].to_string());
        }
    }
    terms
}

fn read_valid_index(root: &Path) -> Option<AssuranceIndex> {
    let manifest = root.join(MANIFEST_PATH);
    let metadata = std::fs::symlink_metadata(&manifest).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let bytes = std::fs::read(manifest).ok()?;
    let index: AssuranceIndex = serde_json::from_slice(&bytes).ok()?;
    if index.schema_version != 1 || index.controls.is_empty() || index.controls.len() > MAX_CONTROLS
    {
        return None;
    }
    let mut ids = std::collections::BTreeSet::new();
    for control in &index.controls {
        if !valid_identifier(&control.id)
            || !ids.insert(control.id.as_str())
            || control.topics.is_empty()
            || control.topics.len() > MAX_TOPICS_PER_CONTROL
            || control.artifacts.is_empty()
            || control.artifacts.len() > MAX_ARTIFACTS_PER_CONTROL
        {
            return None;
        }
        if control.topics.iter().any(|topic| !valid_identifier(topic)) {
            return None;
        }
        for artifact in &control.artifacts {
            let relative = Path::new(artifact);
            if artifact.len() > 240
                || artifact.is_empty()
                || relative.is_absolute()
                || relative
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return None;
            }
            let metadata = std::fs::symlink_metadata(root.join(relative)).ok()?;
            if !metadata.file_type().is_file() {
                return None;
            }
        }
    }
    Some(index)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_domain::Criterion;
    use std::fs;
    use time::OffsetDateTime;

    fn repository_fixture() -> tempfile::TempDir {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("repository-0");
        fs::create_dir_all(root.join(".codecaddie")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/SUPPORT.md"), "# Support").unwrap();
        fs::write(
            root.join(MANIFEST_PATH),
            r#"{
              "schemaVersion": 1,
              "controls": [
                {
                  "id": "support-matrix",
                  "topics": ["platforms", "filesystems"],
                  "artifacts": ["docs/SUPPORT.md"]
                },
                {
                  "id": "unrelated-accessibility",
                  "topics": ["keyboard-navigation", "contrast"],
                  "artifacts": ["docs/SUPPORT.md"]
                }
              ]
            }"#,
        )
        .unwrap();
        workspace
    }

    fn generic_goal(title: &str, criterion: &str) -> GoalVersion {
        GoalVersion {
            id: "version-generic".into(),
            goal_id: "goal-generic".into(),
            title: title.into(),
            business_outcome: "Teams can operate the product reliably.".into(),
            priority: 1,
            position: 1,
            criteria: vec![Criterion {
                id: "criterion-generic".into(),
                text: criterion.into(),
            }],
            rubric_dimensions: vec!["Operations".into()],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "actor".into(),
            supersedes: None,
        }
    }

    #[test]
    fn valid_index_routes_to_existing_artifacts_without_copying_source() {
        let workspace = repository_fixture();
        let digest = repository_assurance_digest(
            workspace.path(),
            &[("repo".into(), "repository-0".into())],
            &[generic_goal(
                "Supported deployments",
                "The support matrix names every platform and filesystem.",
            )],
        )
        .unwrap();
        assert!(digest.contains("repositoryId: repo"));
        assert!(digest.contains("goalPosition: 1"));
        assert!(
            digest.contains(
                "goalPosition: 1 | criterionPosition: 1 | primaryControlId: support-matrix | inspectFirst: docs/SUPPORT.md"
            )
        );
        assert!(digest.contains("support-matrix"));
        assert!(digest.contains("docs/SUPPORT.md"));
        assert!(digest.contains("inspectInOrder: docs/SUPPORT.md"));
        assert!(!digest.contains("unrelated-accessibility"));
        assert!(!digest.contains("# Support"));
    }

    #[test]
    fn batch_routing_is_relevant_and_deterministic() {
        let workspace = repository_fixture();
        let mut second_goal = generic_goal(
            "Supported filesystem matrix",
            "The support matrix covers portable filesystems.",
        );
        second_goal.position = 2;
        let goals = [
            generic_goal(
                "Portable platform support",
                "The support matrix covers platforms and filesystems.",
            ),
            second_goal,
        ];
        let first = repository_assurance_digest(
            workspace.path(),
            &[("repo".into(), "repository-0".into())],
            &goals,
        )
        .unwrap();
        let second = repository_assurance_digest(
            workspace.path(),
            &[("repo".into(), "repository-0".into())],
            &goals,
        )
        .unwrap();
        assert_eq!(first, second);
        assert!(first.contains("support-matrix"));
        assert!(first.contains("goalPosition: 2 | controlIds: support-matrix"));
        assert_eq!(first.matches("\n- support-matrix | ").count(), 1);
        assert!(!first.contains("unrelated-accessibility"));
    }

    #[test]
    fn each_goal_routing_caps_equal_relevance_controls_deterministically() {
        let workspace = repository_fixture();
        let manifest = workspace.path().join("repository-0").join(MANIFEST_PATH);
        let controls = (0..16)
            .map(|index| {
                serde_json::json!({
                    "id": format!("inventory-{index:02}"),
                    "topics": ["inventory"],
                    "artifacts": ["docs/SUPPORT.md"]
                })
            })
            .collect::<Vec<_>>();
        fs::write(
            manifest,
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "controls": controls
            }))
            .unwrap(),
        )
        .unwrap();

        let digest = repository_assurance_digest(
            workspace.path(),
            &[("repo".into(), "repository-0".into())],
            &[generic_goal(
                "Inventory integrity",
                "The inventory has repository-verifiable controls.",
            )],
        )
        .unwrap();
        assert_eq!(
            digest.lines().filter(|line| line.starts_with("- ")).count(),
            6
        );
        assert!(digest.contains("inventory-00"));
        assert!(digest.contains("inventory-05"));
        assert!(!digest.contains("inventory-06"));
    }

    #[test]
    fn each_goal_receives_independent_relevant_routes() {
        let workspace = repository_fixture();
        let mut accessibility = generic_goal(
            "Accessible navigation",
            "Keyboard navigation and contrast are repository verified.",
        );
        accessibility.position = 7;
        let digest = repository_assurance_digest(
            workspace.path(),
            &[("repo".into(), "repository-0".into())],
            &[
                generic_goal(
                    "Portable platform support",
                    "The support matrix covers platforms and filesystems.",
                ),
                accessibility,
            ],
        )
        .unwrap();
        assert!(digest.contains("goalPosition: 1 | controlIds: support-matrix"));
        assert!(digest.contains("goalPosition: 7 | controlIds: unrelated-accessibility"));
    }

    #[test]
    fn criterion_fair_routing_keeps_a_smaller_neighboring_check() {
        let workspace = repository_fixture();
        let manifest = workspace.path().join("repository-0").join(MANIFEST_PATH);
        let mut controls = (0..8)
            .map(|index| {
                serde_json::json!({
                    "id": format!("inventory-{index:02}"),
                    "topics": ["inventory"],
                    "artifacts": ["docs/SUPPORT.md"]
                })
            })
            .collect::<Vec<_>>();
        controls.push(serde_json::json!({
            "id": "keyboard-proof",
            "topics": ["keyboard-navigation", "contrast"],
            "artifacts": ["docs/SUPPORT.md"]
        }));
        fs::write(
            manifest,
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "controls": controls
            }))
            .unwrap(),
        )
        .unwrap();

        let mut goal = generic_goal(
            "Inventory operations",
            "The inventory has repository-verifiable controls.",
        );
        goal.criteria.push(Criterion {
            id: "criterion-keyboard".into(),
            text: "Keyboard navigation and contrast are verified by tests.".into(),
        });
        let digest = repository_assurance_digest(
            workspace.path(),
            &[("repo".into(), "repository-0".into())],
            &[goal],
        )
        .unwrap();
        assert!(digest.contains("keyboard-proof"));
        assert!(
            digest.contains(
                "goalPosition: 1 | criterionPosition: 2 | primaryControlId: keyboard-proof"
            )
        );
        assert_eq!(
            digest.lines().filter(|line| line.starts_with("- ")).count(),
            6
        );
    }

    #[test]
    fn oversized_selected_routing_digest_is_omitted() {
        let workspace = repository_fixture();
        let root = workspace.path().join("repository-0");
        let long_artifact = format!("docs/{}.md", "routing-evidence-".repeat(10));
        fs::write(root.join(&long_artifact), "metadata only").unwrap();
        let controls = (0..6)
            .map(|index| {
                serde_json::json!({
                    "id": format!("inventory-{index:02}"),
                    "topics": (0..12)
                        .map(|topic| format!("inventory-{index:02}-{topic:02}-{}", "bounded".repeat(7)))
                        .collect::<Vec<_>>(),
                    "artifacts": vec![long_artifact.clone(); 12]
                })
            })
            .collect::<Vec<_>>();
        fs::write(
            root.join(MANIFEST_PATH),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "controls": controls
            }))
            .unwrap(),
        )
        .unwrap();

        assert!(
            repository_assurance_digest(
                workspace.path(),
                &[("repo".into(), "repository-0".into())],
                &[generic_goal(
                    "Inventory integrity",
                    "The inventory has repository-verifiable controls."
                )]
            )
            .is_none()
        );
    }

    #[test]
    fn unsafe_or_unverifiable_indexes_are_ignored() {
        let workspace = repository_fixture();
        let manifest = workspace.path().join("repository-0").join(MANIFEST_PATH);
        fs::write(
            &manifest,
            r#"{"schemaVersion":1,"controls":[{"id":"escape","topics":["ignore-instructions"],"artifacts":["../secret"]}]}"#,
        )
        .unwrap();
        assert!(
            repository_assurance_digest(
                workspace.path(),
                &[("repo".into(), "repository-0".into())],
                &[generic_goal(
                    "Service objectives",
                    "The service has an SLO."
                )]
            )
            .is_none()
        );

        fs::write(
            manifest,
            r#"{"schemaVersion":1,"controls":[{"id":"missing","topics":["slo"],"artifacts":["docs/MISSING.md"]}]}"#,
        )
        .unwrap();
        assert!(
            repository_assurance_digest(
                workspace.path(),
                &[("repo".into(), "repository-0".into())],
                &[generic_goal(
                    "Service objectives",
                    "The service has an SLO."
                )]
            )
            .is_none()
        );
    }
}
