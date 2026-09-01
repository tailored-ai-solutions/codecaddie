use crate::{GoalAssessment, GoalVersion, Verdict};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct CoverageScore {
    pub coverage: Option<f64>,
    pub assessed_criteria: u32,
    pub unverified_criteria: u32,
}

pub fn criterion_value(verdict: Verdict) -> Option<f64> {
    match verdict {
        Verdict::Supported => Some(1.0),
        Verdict::Partial => Some(0.5),
        Verdict::Unsupported => Some(0.0),
        Verdict::Unverified => None,
    }
}

pub fn aggregate_goal(assessment: &GoalAssessment) -> Verdict {
    let assessed: Vec<Verdict> = assessment
        .criteria
        .iter()
        .map(|item| item.verdict)
        .filter(|item| *item != Verdict::Unverified)
        .collect();
    if assessed.is_empty() {
        Verdict::Unverified
    } else if assessed.iter().all(|item| *item == Verdict::Supported) {
        Verdict::Supported
    } else if assessed.iter().all(|item| *item == Verdict::Unsupported) {
        Verdict::Unsupported
    } else {
        Verdict::Partial
    }
}

pub fn score_report(goals: &[GoalVersion], assessments: &[GoalAssessment]) -> CoverageScore {
    let assessment_by_goal: BTreeMap<&str, &GoalAssessment> = assessments
        .iter()
        .map(|assessment| (assessment.goal_version_id.as_str(), assessment))
        .collect();

    let mut weighted_total = 0.0;
    let mut included_weight = 0.0;
    let mut assessed_criteria = 0;
    let mut unverified_criteria = 0;

    for goal in goals {
        let Some(assessment) = assessment_by_goal.get(goal.id.as_str()) else {
            unverified_criteria += goal.criteria.len() as u32;
            continue;
        };
        let values: Vec<f64> = assessment
            .criteria
            .iter()
            .filter_map(|criterion| {
                let value = criterion_value(criterion.verdict);
                if value.is_some() {
                    assessed_criteria += 1;
                } else {
                    unverified_criteria += 1;
                }
                value
            })
            .collect();
        if values.is_empty() {
            continue;
        }
        let goal_score = values.iter().sum::<f64>() / values.len() as f64;
        let weight = f64::from(goal.priority);
        weighted_total += goal_score * weight;
        included_weight += weight;
    }

    CoverageScore {
        coverage: (included_weight > 0.0).then_some(weighted_total / included_weight),
        assessed_criteria,
        unverified_criteria,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Criterion, CriterionAssessment};
    use time::OffsetDateTime;

    fn goal(id: &str, priority: u8, criteria: usize) -> GoalVersion {
        GoalVersion {
            id: id.into(),
            goal_id: format!("goal-{id}"),
            title: id.into(),
            business_outcome: "Outcome".into(),
            priority,
            position: 0,
            criteria: (0..criteria)
                .map(|index| Criterion {
                    id: format!("{id}-{index}"),
                    text: "Concrete criterion".into(),
                })
                .collect(),
            rubric_dimensions: vec![],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "owner".into(),
            supersedes: None,
        }
    }

    fn assessment(id: &str, verdicts: &[Verdict]) -> GoalAssessment {
        let mut item = GoalAssessment {
            architecture_narrative: String::new(),
            related_component_ids: vec![],
            goal_version_id: id.into(),
            verdict: Verdict::Unverified,
            summary: String::new(),
            criteria: verdicts
                .iter()
                .enumerate()
                .map(|(index, verdict)| CriterionAssessment {
                    criterion_id: format!("{id}-{index}"),
                    verdict: *verdict,
                    rationale: "Evidence-backed rationale".into(),
                    confidence: 0.9,
                    evidence: vec![],
                })
                .collect(),
        };
        item.verdict = aggregate_goal(&item);
        item
    }

    #[test]
    fn excludes_unverified_and_weights_goals_by_priority() {
        let goals = vec![goal("a", 5, 2), goal("b", 1, 2)];
        let assessments = vec![
            assessment("a", &[Verdict::Supported, Verdict::Unverified]),
            assessment("b", &[Verdict::Unsupported, Verdict::Unsupported]),
        ];
        let score = score_report(&goals, &assessments);
        assert_eq!(score.coverage, Some(5.0 / 6.0));
        assert_eq!(score.assessed_criteria, 3);
        assert_eq!(score.unverified_criteria, 1);
    }

    #[test]
    fn goal_verdict_uses_only_assessed_criteria() {
        assert_eq!(
            aggregate_goal(&assessment("a", &[Verdict::Unverified])),
            Verdict::Unverified
        );
        assert_eq!(
            aggregate_goal(&assessment("a", &[Verdict::Supported, Verdict::Unverified])),
            Verdict::Supported
        );
        assert_eq!(
            aggregate_goal(&assessment(
                "a",
                &[Verdict::Supported, Verdict::Unsupported]
            )),
            Verdict::Partial
        );
    }
}
