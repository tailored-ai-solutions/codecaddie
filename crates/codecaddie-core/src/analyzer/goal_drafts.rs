//! Goal drafting: grounded provider generation with bounded revision calls,
//! fail-closed validation, and non-semantic stable-identity reconciliation.

use super::analysis_contract::{
    GOAL_GENERATION_SCHEMA, goal_generation_prompt_with_existing, goal_generation_revision_prompt,
};
use super::goal_catalog::{
    BUSINESS_GROUP, BriefSignal, CORE_OUTCOME_BUSINESS_OUTCOME, CORE_OUTCOME_CRITERIA,
    CORE_OUTCOME_KEY, CORE_OUTCOME_TITLE, COVERAGE_FAMILIES, FallbackGoalKind, FallbackGoalSpec,
    GOAL_GROUPS, GOAL_TEMPLATES, RepairPhase, fallback_goal_spec,
};
use super::product_profile::{ProductProfile, build_product_profile};
use crate::{
    context_documents::{ContextSourceMetadata, ExtractedContext},
    local_state::ApproveGoalRequest,
    provider::{ProgressSink, ProviderKind, ProviderRunner},
};
use codecaddie_domain::GoalVersion;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalDraft {
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub key: String,
    pub title: String,
    pub business_outcome: String,
    pub priority: u8,
    pub criteria: Vec<String>,
    pub rubric_dimensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grounding_fact_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalGenerationRequest {
    pub provider: ProviderKind,
    pub product_brief: String,
    #[serde(default)]
    pub existing_goals: Vec<ExistingGoalIdentity>,
    #[serde(skip)]
    pub extracted_context: Option<ExtractedContext>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalGenerationResult {
    pub goals: Vec<GoalDraft>,
    pub context_sources_used: Vec<ContextSourceMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingGoalIdentity {
    pub goal_id: String,
    pub key: String,
    pub title: String,
    pub business_outcome: String,
}

#[derive(Deserialize)]
struct GoalGenerationOutput {
    goals: Vec<GoalDraft>,
}

pub async fn generate_goal_drafts(
    request: GoalGenerationRequest,
    progress: Option<ProgressSink>,
) -> anyhow::Result<GoalGenerationResult> {
    match tokio::time::timeout(
        // One profile call, one initial draft, and two bounded revision calls
        // can each legitimately approach the provider's ten-minute ceiling.
        Duration::from_secs(42 * 60),
        generate_goal_drafts_inner(request, progress),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "goal generation reached its 42-minute limit; the current goals were left unchanged"
        ),
    }
}

async fn generate_goal_drafts_inner(
    request: GoalGenerationRequest,
    progress: Option<ProgressSink>,
) -> anyhow::Result<GoalGenerationResult> {
    if request.product_brief.trim().len() < 20 {
        anyhow::bail!("a meaningful product or strategy brief is required");
    }
    let directory = tempfile::tempdir()?;
    let runner = ProviderRunner {
        // Grounding and revision prompts can be sizeable even after bounded
        // extraction. Keep the same ceiling used by repository analysis so a
        // healthy local provider is not killed while correcting a draft.
        timeout: Duration::from_secs(10 * 60),
    };
    let prepared = runner.prepare(request.provider).await?;
    let profile = build_product_profile(
        &runner,
        &prepared,
        directory.path(),
        &request.product_brief,
        request.extracted_context.as_ref(),
        progress.clone(),
    )
    .await?;
    let mut grounded_context = profile.goal_context()?;
    let required_engineering = required_engineering_coverage_prompt(&profile);
    grounded_context.push_str(&required_engineering);
    let prompt = goal_generation_prompt_with_existing(&grounded_context, &request.existing_goals)?;
    if let Some(sink) = &progress {
        sink(format!(
            "Asking {} to draft goals from the product brief",
            request.provider.executable()
        ));
    }
    let mut value = runner
        .run_structured_prepared_without_repository_tools(
            &prepared,
            directory.path(),
            &prompt,
            GOAL_GENERATION_SCHEMA,
            progress.clone(),
        )
        .await?;
    const MAX_REVISIONS: usize = 2;
    for revision in 0..=MAX_REVISIONS {
        let (prior_goals, feedback) = match serde_json::from_value::<GoalGenerationOutput>(value) {
            Ok(mut output) => {
                match validate_grounded_provider_goal_drafts(
                    &output.goals,
                    &grounded_context,
                    &profile,
                ) {
                    Ok(()) => {
                        reconcile_goal_identities(&mut output.goals, &request.existing_goals)?;
                        return Ok(GoalGenerationResult {
                            goals: output.goals,
                            context_sources_used: request
                                .extracted_context
                                .as_ref()
                                .map(|context| context.sources.clone())
                                .unwrap_or_default(),
                        });
                    }
                    Err(error) => (output.goals, error.to_string()),
                }
            }
            Err(error) => (Vec::new(), goal_output_schema_feedback(&error)),
        };
        if revision == MAX_REVISIONS {
            anyhow::bail!(
                "goal set still failed the CTO and board materiality review after {MAX_REVISIONS} revisions: {feedback}"
            );
        }
        if let Some(sink) = &progress {
            sink(format!(
                "Goal draft needs another materiality or coverage repair; starting revision {} of {MAX_REVISIONS}",
                revision + 1
            ));
        }
        let revision_prompt =
            goal_generation_revision_prompt(&grounded_context, &prior_goals, &feedback)?;
        value = runner
            .run_structured_prepared_without_repository_tools(
                &prepared,
                directory.path(),
                &revision_prompt,
                GOAL_GENERATION_SCHEMA,
                progress.clone(),
            )
            .await?;
    }
    // The final loop iteration either returns the approved goal set or bails
    // with the review feedback, so falling through is a logic error. Fail as
    // a retryable typed error rather than aborting the core process.
    anyhow::bail!("goal generation ended without an approved goal set or final review feedback")
}

fn profile_capability_applies(profile: &ProductProfile, signal: BriefSignal) -> bool {
    match signal {
        BriefSignal::MultipleCustomerOrganizations => {
            profile.capabilities.multiple_customer_organizations.present
        }
        BriefSignal::Integrations => profile.capabilities.integrations.present,
        BriefSignal::Webhooks => profile.capabilities.webhooks.present,
        BriefSignal::ArtificialIntelligence => profile.capabilities.artificial_intelligence.present,
        BriefSignal::SensitiveData => profile.capabilities.sensitive_data.present,
        BriefSignal::ScaleOrCapacity => profile.capabilities.scale_or_capacity.present,
    }
}

fn required_engineering_coverage_prompt(profile: &ProductProfile) -> String {
    let mut text = String::from(
        "\n\nREQUIRED ENGINEERING COVERAGE\nEvery applicable item below must appear explicitly in a success check in one of the named goal groups. Tailor the surrounding goal title and outcome to this product, but preserve the concrete control and its key terms.\n",
    );
    for family in COVERAGE_FAMILIES {
        if family
            .signal
            .is_some_and(|signal| !profile_capability_applies(profile, signal))
        {
            continue;
        }
        text.push_str("- ");
        text.push_str(family.label);
        text.push_str(" [");
        text.push_str(&family.groups.join(" or "));
        text.push_str("]: ");
        if let Some(repair) = &family.repair {
            text.push_str(repair.criterion);
        } else {
            text.push_str("Include a concrete, repository-verifiable success check.");
        }
        text.push('\n');
    }
    text
}

fn goal_output_schema_feedback(error: &serde_json::Error) -> String {
    // Do not echo provider output into errors or progress. The only precise
    // detail retained is whether the required top-level collection was absent.
    if error.to_string().contains("missing field `goals`") {
        return "The prior response omitted the required top-level `goals` array. Return exactly one JSON object with a `goals` array containing 6–9 complete goal objects and no surrounding commentary.".into();
    }
    "The prior response did not match the required goal schema. Return exactly one JSON object with a `goals` array containing 6–9 complete goal objects; every object must include key, title, businessOutcome, priority, criteria, rubricDimensions, and groundingFactIds.".into()
}

#[allow(dead_code)]
fn cap_provider_goal_set(goals: &mut Vec<GoalDraft>) -> bool {
    // Reserve up to three slots for deterministic architecture, operations,
    // and supply-chain repairs so a schema-valid provider response can never
    // force the final editable set beyond the nine-goal UI limit.
    const MAX_PROVIDER_GOALS_BEFORE_REPAIR: usize = 6;
    if goals.len() <= MAX_PROVIDER_GOALS_BEFORE_REPAIR {
        return false;
    }
    let mut selected = BTreeSet::new();
    for group in GOAL_GROUPS {
        if let Some(index) = goals.iter().position(|goal| {
            goal.rubric_dimensions
                .first()
                .is_some_and(|candidate| candidate == group)
        }) {
            selected.insert(index);
        }
    }
    let mut candidates = goals
        .iter()
        .enumerate()
        .map(|(index, goal)| (Reverse(goal.priority), index))
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    for (_, index) in candidates {
        if selected.len() >= MAX_PROVIDER_GOALS_BEFORE_REPAIR {
            break;
        }
        selected.insert(index);
    }
    let compact = goals
        .iter()
        .enumerate()
        .filter(|(index, _)| selected.contains(index))
        .map(|(_, goal)| goal.clone())
        .collect();
    *goals = compact;
    true
}

#[allow(dead_code)]
fn consolidate_tactical_business_goals(goals: &mut Vec<GoalDraft>) -> bool {
    let mut retained = Vec::with_capacity(goals.len());
    let mut useful_criteria = Vec::new();
    let mut changed = false;
    for goal in goals.drain(..) {
        let is_business = goal
            .rubric_dimensions
            .first()
            .is_some_and(|group| group == BUSINESS_GROUP);
        let non_substantive = provider_goal_is_non_substantive(&goal);
        if is_business && (business_goal_is_tactical(&goal, false) || non_substantive) {
            changed = true;
            for criterion in if non_substantive {
                Vec::new()
            } else {
                goal.criteria
            } {
                let trimmed = criterion.trim();
                if !trimmed.is_empty()
                    && !trimmed.eq_ignore_ascii_case("placeholder")
                    && !useful_criteria.iter().any(|item: &String| item == trimmed)
                {
                    useful_criteria.push(trimmed.to_string());
                }
            }
        } else {
            retained.push(goal);
        }
    }
    if changed
        && !retained.iter().any(|goal| {
            goal.rubric_dimensions
                .first()
                .is_some_and(|group| group == BUSINESS_GROUP)
        })
    {
        let mut criteria = CORE_OUTCOME_CRITERIA
            .iter()
            .map(|criterion| (*criterion).to_string())
            .collect::<Vec<_>>();
        criteria.extend(useful_criteria.into_iter().take(3));
        let key = unique_repair_goal_key(&retained, CORE_OUTCOME_KEY);
        retained.push(GoalDraft {
            goal_id: None,
            key,
            title: CORE_OUTCOME_TITLE.into(),
            business_outcome: CORE_OUTCOME_BUSINESS_OUTCOME.into(),
            priority: 5,
            criteria,
            rubric_dimensions: vec![BUSINESS_GROUP.into()],
            grounding_fact_ids: Vec::new(),
        });
    }
    *goals = retained;
    changed
}

fn provider_goal_non_substantive_reason(goal: &GoalDraft) -> Option<String> {
    let is_placeholder = |value: &str| {
        let value = value.trim();
        value.eq_ignore_ascii_case("placeholder")
            || value.eq_ignore_ascii_case("tbd")
            || value.eq_ignore_ascii_case("sample")
    };
    let title = goal.title.trim().to_lowercase();
    let all_text = std::iter::once(goal.key.as_str())
        .chain(std::iter::once(goal.title.as_str()))
        .chain(std::iter::once(goal.business_outcome.as_str()))
        .chain(goal.criteria.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let drafting_terms = [
        "holding object",
        "schema-valid",
        "schema valid",
        "minimum array",
        "array length",
        "filler goal",
        "dummy goal",
        "drafting instruction",
    ];
    for (field, value) in [
        ("key", goal.key.as_str()),
        ("title", goal.title.as_str()),
        ("business outcome", goal.business_outcome.as_str()),
    ] {
        if is_placeholder(value) {
            return Some(format!(
                "the {field} is the reserved placeholder token {value:?}"
            ));
        }
    }
    if let Some((index, value)) = goal
        .criteria
        .iter()
        .enumerate()
        .find(|(_, item)| is_placeholder(item))
    {
        return Some(format!(
            "success check {} is the reserved placeholder token {value:?}",
            index + 1
        ));
    }
    if let Some(term) = drafting_terms.iter().find(|term| all_text.contains(**term)) {
        return Some(format!(
            "it contains the forbidden drafting phrase {term:?}"
        ));
    }
    if title.contains("product brief")
        && (title.contains("draft") || title.contains("candidate") || title.contains("inventory"))
    {
        return Some(
            "the title describes drafting the product brief instead of a product outcome".into(),
        );
    }
    None
}

fn provider_goal_is_non_substantive(goal: &GoalDraft) -> bool {
    provider_goal_non_substantive_reason(goal).is_some()
}

#[allow(dead_code)]
fn repair_duplicate_provider_keys(goals: &mut [GoalDraft]) -> bool {
    let mut seen = BTreeSet::new();
    let mut changed = false;
    for goal in goals {
        if seen.insert(goal.key.clone()) {
            continue;
        }
        let identity = format!("{}\0{}", goal.title.trim(), goal.business_outcome.trim());
        let base = format!(
            "generated-{}",
            &blake3::hash(identity.as_bytes()).to_hex()[..16]
        );
        let mut candidate = base.clone();
        for suffix in 2..=99 {
            if !seen.contains(&candidate) {
                break;
            }
            candidate = format!("{base}-{suffix}");
        }
        goal.key = candidate.clone();
        seen.insert(candidate);
        changed = true;
    }
    changed
}

fn normalized_goal_identity(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn reconcile_goal_identities(
    goals: &mut [GoalDraft],
    existing: &[ExistingGoalIdentity],
) -> anyhow::Result<()> {
    for goal in goals {
        let title = normalized_goal_identity(&goal.title);
        let outcome = normalized_goal_identity(&goal.business_outcome);
        let exact_key = existing.iter().find(|item| item.key == goal.key);
        if exact_key.is_some_and(|item| {
            normalized_goal_identity(&item.title) != title
                && normalized_goal_identity(&item.business_outcome) != outcome
        }) {
            let identity = format!("{title}\0{outcome}");
            goal.key = format!(
                "generated-{}",
                &blake3::hash(identity.as_bytes()).to_hex()[..16]
            );
            goal.goal_id = None;
        }
        let mut matches = existing
            .iter()
            .filter(|item| {
                item.key == goal.key
                    || normalized_goal_identity(&item.title) == title
                    || normalized_goal_identity(&item.business_outcome) == outcome
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| left.goal_id.cmp(&right.goal_id));
        matches.dedup_by(|left, right| left.goal_id == right.goal_id);
        if matches.len() > 1 {
            anyhow::bail!(
                "generated goal identity matches more than one current goal; edit the duplicate current goals before regenerating"
            );
        }
        if let Some(existing) = matches.first() {
            goal.key.clone_from(&existing.key);
            goal.goal_id = Some(existing.goal_id.clone());
        }
    }
    Ok(())
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}

fn normalized_criterion_text(value: &str) -> String {
    let words = value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    format!(" {words} ")
}

fn contains_criterion_phrase(normalized: &str, phrase: &str) -> bool {
    let phrase = normalized_criterion_text(phrase);
    let phrase = phrase.trim();
    if normalized.contains(&format!(" {phrase} ")) {
        return true;
    }
    let plural = if let Some(stem) = phrase.strip_suffix('y') {
        format!("{stem}ies")
    } else if phrase.ends_with(['s', 'x']) || phrase.ends_with("ch") || phrase.ends_with("sh") {
        format!("{phrase}es")
    } else {
        format!("{phrase}s")
    };
    normalized.contains(&format!(" {plural} "))
}

fn contains_any_criterion_phrase(normalized: &str, phrases: &[&str]) -> bool {
    phrases
        .iter()
        .any(|phrase| contains_criterion_phrase(normalized, phrase))
}

/// Returns actionable feedback when a success check cannot be decided from
/// immutable repository evidence. Business outcomes may name adoption,
/// satisfaction, revenue, or speed; criteria must instead name the code,
/// tests, configuration, instrumentation, or version-controlled control that
/// implements or measures that outcome.
pub(super) fn repository_verifiability_failure(criterion: &str) -> Option<&'static str> {
    let normalized = normalized_criterion_text(criterion);
    let repository_anchors = [
        "api",
        "application",
        "artifact",
        "audit log",
        "audit logging",
        "automated test",
        "backup",
        "build",
        "ci",
        "code",
        "command",
        "component",
        "configuration",
        "contract",
        "control",
        "dashboard",
        "data store",
        "database",
        "dependency",
        "documented",
        "documentation",
        "endpoint",
        "event",
        "evaluation",
        "export",
        "failure state",
        "feature flag",
        "implementation",
        "implemented",
        "instrumented",
        "instrumentation",
        "integration",
        "interface",
        "job",
        "log",
        "manifest",
        "metric",
        "migration",
        "module",
        "monitoring",
        "pipeline",
        "policy",
        "progress",
        "query",
        "queries",
        "record",
        "recovery path",
        "release gate",
        "repository",
        "restore",
        "rollback",
        "runbook",
        "schema",
        "screen",
        "script",
        "service",
        "setup",
        "slo",
        "source controlled",
        "static analysis",
        "store",
        "suite",
        "telemetry",
        "test",
        "tested",
        "ui",
        "version controlled",
        "workflow",
    ];
    if !contains_any_criterion_phrase(&normalized, &repository_anchors) {
        return Some(
            "it does not name a repository-verifiable implementation, test, configuration, schema, instrumentation signal, workflow, or version-controlled operational control",
        );
    }

    let already_achieved_outcomes = [
        "active organization",
        "active user",
        "adoption target",
        "conversion",
        "cycle time",
        "customer satisfaction",
        "customers report",
        "leaders report",
        "leaders say",
        "net revenue",
        "percent",
        "customer retention",
        "retention target",
        "revenue target",
        "satisfaction target",
        "surveyed",
        "time to value",
        "within 10 minutes",
        "within 30 days",
    ];
    let repository_measurement_controls = [
        "automated test",
        "ci",
        "code emits",
        "code records",
        "configured threshold",
        "configuration",
        "dashboard",
        "event",
        "instrumented",
        "instrumentation",
        "metric",
        "monitoring",
        "schema",
        "source controlled",
        "telemetry",
        "test",
        "version controlled",
    ];
    if (criterion.contains('%')
        || contains_any_criterion_phrase(&normalized, &already_achieved_outcomes))
        && !contains_any_criterion_phrase(&normalized, &repository_measurement_controls)
    {
        return Some(
            "it requires an already-achieved adoption, satisfaction, survey, revenue, retention, conversion, or cycle-time result without naming repository-verifiable instrumentation or controls that measure it",
        );
    }
    None
}

fn validate_goal_criteria_repository_contract(
    title: &str,
    criteria: &[String],
) -> anyhow::Result<()> {
    let failures = goal_criteria_repository_contract_failures(title, criteria);
    if !failures.is_empty() {
        anyhow::bail!(failures.join("; "));
    }
    Ok(())
}

fn goal_criteria_repository_contract_failures(title: &str, criteria: &[String]) -> Vec<String> {
    criteria
        .iter()
        .enumerate()
        .filter_map(|(index, criterion)| {
            repository_verifiability_failure(criterion).map(|reason| {
                format!(
                    "success check {} in goal {:?} is not verifiable from a frozen repository commit: {reason}; rewrite the check to require inspectable code, tests, configuration, schemas, instrumentation, workflows, or version-controlled operational material",
                    index + 1,
                    title.trim()
                )
            })
        })
        .collect()
}

fn business_goal_tactical_reason(
    goal: &GoalDraft,
    require_lexical_materiality: bool,
) -> Option<String> {
    let trimmed = goal.title.trim().to_lowercase();
    let title = format!(" {trimmed} ");
    let tactical_noun = [
        " screen ",
        " page ",
        " button ",
        " modal ",
        " form ",
        " wizard ",
        " persona ",
        " workflow step ",
        " user story ",
        " api endpoint ",
        " feature ",
    ]
    .iter()
    .any(|term| title.contains(term));
    let tactical_verb = [
        "configure ",
        "approve ",
        "create ",
        "edit ",
        "upload ",
        "export ",
        "send ",
        "schedule ",
        "produce ",
        "implement ",
        "add ",
        "update ",
        "delete ",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix));
    let persona_subject = [
        "end user",
        "operator",
        "analyst",
        "approver",
        "manager",
        "administrator",
        "admin",
        "user",
        "customer",
        "supervisor",
    ]
    .iter()
    .any(|subject| trimmed.starts_with(subject));
    let task_action = [
        " request ",
        " requests ",
        " review ",
        " reviews ",
        " approve ",
        " approves ",
        " submit ",
        " submits ",
        " balance ",
        " balances ",
        " configure ",
        " configures ",
        " edit ",
        " edits ",
        " upload ",
        " uploads ",
        " export ",
        " exports ",
        " schedule ",
        " schedules ",
        " enter ",
        " enters ",
    ]
    .iter()
    .any(|action| title.contains(action));
    let title_text = format!(" {} ", goal.title.trim().to_lowercase());
    let outcome_text = format!(" {} ", goal.business_outcome.trim().to_lowercase());
    let criterion_texts = goal
        .criteria
        .iter()
        .map(|criterion| format!(" {} ", criterion.trim().to_lowercase()))
        .collect::<Vec<_>>();
    let tactic_terms = [
        " screen ",
        " page ",
        " button ",
        " modal ",
        " form ",
        " wizard ",
        " workflow step ",
        " persona ",
        " user story ",
        " endpoint ",
        " report ",
        " summary ",
        " display ",
        " dashboard ",
        " color ",
        " colour ",
        " purple ",
        " font ",
        " typography ",
        " theme ",
        " styling ",
        " visual treatment ",
        " animation ",
        " meeting ",
        " meetings ",
        " documentation ",
        " naming ",
        " module ",
        " modules ",
        " label ",
        " labels ",
        " migrated ",
        " clicks ",
        " submits ",
        " renders ",
        " confirmation ",
    ];
    let hard_title_tactic_terms = [
        " screen ",
        " page ",
        " button ",
        " modal ",
        " form ",
        " wizard ",
        " workflow step ",
        " persona ",
        " user story ",
        " endpoint ",
        " display ",
        " color ",
        " colour ",
        " purple ",
        " font ",
        " typography ",
        " theme ",
        " styling ",
        " visual treatment ",
        " animation ",
        " clicks ",
        " renders ",
        " confirmation ",
    ];
    let title_contains_hard_tactic_terms = hard_title_tactic_terms
        .iter()
        .any(|term| title_text.contains(term));
    let body_describes_tactics = std::iter::once(&outcome_text)
        .chain(criterion_texts.iter())
        .any(|statement| tactic_terms.iter().any(|term| statement.contains(term)));
    let material_terms = [
        "revenue",
        "gross margin",
        "contribution margin",
        "operating margin",
        "profit margin",
        "subscription margin",
        "cash loss",
        "financial loss",
        "financial impact",
        "operating cost",
        "lose money",
        "overpayment",
        "underpayment",
        "churn",
        "fraud",
        "productivity",
        "efficiency",
        "retention",
        "renewal",
        "adoption",
        "customer trust",
        "compliance violation",
        "compliance error",
        "legal exposure",
        "security exposure",
        "cycle time",
        "time to first useful result",
        "time to value",
        "abandonment",
        "avoidable handoff",
        "avoidable contact",
        "self-service completion",
        "operating cost",
        "cost per",
        "operating leverage",
        "strategic capability",
        "customer outcome",
        "customer value",
        "product value",
        "accuracy",
        "error rate",
        "processing error",
        "failure rate",
        "conversion",
        "satisfaction",
        "availability",
        "reliability",
    ];
    let statement_has_material_consequence = |statement: &str| {
        let negated = [
            " not ",
            " never ",
            " without ",
            " no increase ",
            " no decrease ",
            " deferred ",
            " excluded ",
            " unavailable ",
            " unsupported ",
            " must not ",
            " should not ",
            " cannot ",
        ]
        .iter()
        .any(|term| statement.contains(term));
        !negated && material_terms.iter().any(|term| statement.contains(term))
    };
    let directional_terms = [
        "prevent ",
        "reduce ",
        "lower ",
        "increase ",
        "grow ",
        "improve ",
        "protect ",
        "preserve ",
        "accelerate ",
        "shorten ",
        "avoid ",
        "eliminate ",
    ];
    let threshold_terms = [
        " percent ",
        " percentage ",
        " target ",
        " threshold ",
        " below ",
        " above ",
        " at most ",
        " at least ",
    ];
    let statement_has_direction_or_threshold = |statement: &str| {
        directional_terms
            .iter()
            .chain(threshold_terms.iter())
            .any(|term| statement.contains(term))
    };
    let title_states_material_consequence = statement_has_material_consequence(&title_text);
    let outcome_states_material_consequence = statement_has_material_consequence(&outcome_text);
    let title_or_outcome_states_material_consequence =
        title_states_material_consequence || outcome_states_material_consequence;
    let criterion_states_material_result = criterion_texts.iter().any(|criterion| {
        statement_has_material_consequence(criterion)
            && statement_has_direction_or_threshold(criterion)
    });
    let executive_terms = [
        "baseline and target",
        "baselines and targets",
        "measurable target",
        "target and threshold",
        "executive owner",
        "accountable product executive",
        "decision triggered",
        "decision threshold",
        "decision rule",
        "review cadence",
    ];
    let states_executive_accountability_or_measure = std::iter::once(&title_text)
        .chain(std::iter::once(&outcome_text))
        .chain(criterion_texts.iter())
        .any(|statement| executive_terms.iter().any(|term| statement.contains(term)));
    if tactical_noun || tactical_verb || title_contains_hard_tactic_terms {
        return Some(
            "its title names a screen, UI artifact, implementation action, or workflow fragment instead of the durable outcome; consolidate tactics into success checks".into(),
        );
    }
    if persona_subject && task_action && !title_or_outcome_states_material_consequence {
        return Some(
            "its persona-and-task title lacks a material customer or business consequence".into(),
        );
    }
    if body_describes_tactics
        && !(title_states_material_consequence
            || (outcome_states_material_consequence && states_executive_accountability_or_measure))
    {
        return Some(
            "its outcome or success checks center implementation details without both a material consequence and an accountable measure".into(),
        );
    }
    if require_lexical_materiality
        && !(title_or_outcome_states_material_consequence || criterion_states_material_result)
    {
        return Some("it lacks a durable, measurable customer or business consequence".into());
    }
    None
}

fn business_goal_is_tactical(goal: &GoalDraft, require_lexical_materiality: bool) -> bool {
    business_goal_tactical_reason(goal, require_lexical_materiality).is_some()
}

fn statement_affirms_coverage(statement: &str, terms: &[&str]) -> bool {
    let statement = statement.to_lowercase();
    let negated = [
        "not ",
        "never",
        "deferred",
        "excluded",
        "without ",
        "missing",
        "planned",
        "future",
        "todo",
        "later",
        "doesn't",
        "does not",
        "no ",
        "absent",
        "unavailable",
        "unsupported",
        "aspirational",
        "manual only",
        "fails",
        "disabled",
        "unimplemented",
    ]
    .iter()
    .any(|term| statement.contains(term));
    !negated && contains_any(&statement, terms)
}

fn goals_affirm_coverage(goals: &[GoalDraft], groups: &[&str], terms: &[&str]) -> bool {
    goals
        .iter()
        .filter(|goal| {
            goal.rubric_dimensions
                .first()
                .is_some_and(|group| groups.contains(&group.as_str()))
        })
        .any(|goal| {
            goal.criteria
                .iter()
                .any(|statement| statement_affirms_coverage(statement, terms))
        })
}

fn goals_affirm_all_coverage(
    goals: &[GoalDraft],
    groups: &[&str],
    required_term_groups: &[&[&str]],
) -> bool {
    required_term_groups
        .iter()
        .all(|terms| goals_affirm_coverage(goals, groups, terms))
}

#[allow(dead_code)]
fn unique_repair_goal_key(goals: &[GoalDraft], base: &str) -> String {
    if goals.iter().all(|goal| goal.key != base) {
        return base.into();
    }
    for suffix in 2..=99 {
        let candidate = format!("{base}-{suffix}");
        if goals.iter().all(|goal| goal.key != candidate) {
            return candidate;
        }
    }
    format!("repair-{}", &blake3::hash(base.as_bytes()).to_hex()[..16])
}

#[allow(dead_code)]
fn place_repair_criterion(
    goals: &mut [GoalDraft],
    groups: &[&str],
    criterion: &str,
    target_terms: &[&str],
) -> bool {
    let target = goals
        .iter_mut()
        .filter(|goal| {
            goal.rubric_dimensions
                .first()
                .is_some_and(|group| groups.contains(&group.as_str()))
                && goal.criteria.len() < 6
        })
        .max_by_key(|goal| {
            let candidate = format!(
                "{} {} {}",
                goal.title,
                goal.business_outcome,
                goal.criteria.join(" ")
            )
            .to_lowercase();
            target_terms
                .iter()
                .filter(|term| candidate.contains(**term))
                .count()
        });
    if let Some(target) = target {
        target.criteria.push(criterion.into());
        true
    } else {
        false
    }
}

#[allow(dead_code)]
fn catalog_repairs<'a>(
    phase: RepairPhase,
    product_brief: &'a str,
) -> impl Iterator<
    Item = (
        &'static super::goal_catalog::CoverageFamily,
        &'static super::goal_catalog::RepairSpec,
    ),
> + 'a {
    COVERAGE_FAMILIES.iter().filter_map(move |family| {
        let repair = family.repair.as_ref()?;
        if repair.phase != phase {
            return None;
        }
        if family
            .signal
            .is_some_and(|signal| !signal.applies(product_brief))
        {
            return None;
        }
        Some((family, repair))
    })
}

#[allow(dead_code)]
fn unique_repair_goal_title(goals: &[GoalDraft], base: &str) -> String {
    let taken = |candidate: &str| {
        goals
            .iter()
            .any(|goal| goal.title.trim().eq_ignore_ascii_case(candidate))
    };
    if !taken(base) {
        return base.into();
    }
    for suffix in 2..=99 {
        let candidate = format!("{base} ({suffix})");
        if !taken(&candidate) {
            return candidate;
        }
    }
    base.into()
}

#[allow(dead_code)]
fn push_fallback_goal(
    goals: &mut Vec<GoalDraft>,
    spec: &FallbackGoalSpec,
    mut criteria: Vec<String>,
) {
    if let Some(lead) = spec.lead_criterion {
        criteria.insert(0, lead.into());
    }
    if criteria.len() == 1
        && let Some(pad) = spec.pad_criterion
    {
        criteria.push(pad.into());
    }
    let key = unique_repair_goal_key(goals, spec.key);
    // A provider draft may already carry a goal titled exactly like this
    // fallback (the catalog titles are published in the plugin reference);
    // validation rejects duplicate titles, so uniquify like the key.
    let title = unique_repair_goal_title(goals, spec.title);
    goals.push(GoalDraft {
        goal_id: None,
        key,
        title,
        business_outcome: spec.business_outcome.into(),
        priority: spec.priority,
        criteria,
        rubric_dimensions: vec![spec.group.into()],
        grounding_fact_ids: Vec::new(),
    });
}

/// Restores the catalog's required coverage deterministically. The
/// nine-goal guarantee assumes the set enters with at most six goals
/// (`cap_provider_goal_set` establishes that in the generation loop); with
/// seven or more full goals the `goals.len() < 9` guards drop unplaced
/// criteria and validation reports the missing families instead.
#[allow(dead_code)]
fn repair_required_engineering_baseline(goals: &mut Vec<GoalDraft>, product_brief: &str) -> bool {
    let mut changed = false;

    // Supply-chain families check and place sequentially; whatever cannot
    // ride in an existing engineering goal pools into the supply-chain
    // fallback goal behind its governance-record lead criterion.
    let mut unplaced_supply_chain = Vec::new();
    for (family, repair) in catalog_repairs(RepairPhase::SupplyChain, product_brief) {
        if goals_affirm_all_coverage(goals, family.groups, family.coverage) {
            continue;
        }
        if place_repair_criterion(goals, family.groups, repair.criterion, repair.target_terms) {
            changed = true;
        } else {
            unplaced_supply_chain.push(repair.criterion.to_string());
        }
    }
    if !unplaced_supply_chain.is_empty() && goals.len() < 9 {
        push_fallback_goal(
            goals,
            fallback_goal_spec(FallbackGoalKind::SupplyChain),
            unplaced_supply_chain,
        );
        changed = true;
    }

    // Core families precompute their missing flags before any placement so
    // one repair's wording can never mask another family's gap.
    let missing = catalog_repairs(RepairPhase::Core, product_brief)
        .filter(|(family, _)| !goals_affirm_all_coverage(goals, family.groups, family.coverage))
        .collect::<Vec<_>>();
    let mut unplaced_architecture = Vec::new();
    let mut unplaced_operations = Vec::new();
    for (family, repair) in missing {
        if place_repair_criterion(goals, family.groups, repair.criterion, repair.target_terms) {
            changed = true;
        } else if repair.fallback == FallbackGoalKind::Architecture {
            unplaced_architecture.push(repair.criterion.to_string());
        } else {
            unplaced_operations.push(repair.criterion.to_string());
        }
    }
    for (kind, criteria) in [
        (FallbackGoalKind::Architecture, unplaced_architecture),
        (FallbackGoalKind::Operations, unplaced_operations),
    ] {
        if criteria.is_empty() || goals.len() >= 9 {
            continue;
        }
        push_fallback_goal(goals, fallback_goal_spec(kind), criteria);
        changed = true;
    }

    // The webhook family runs last against the finished set so its criterion
    // can ride in a repair-created operations goal.
    for (family, repair) in catalog_repairs(RepairPhase::Webhook, product_brief) {
        if goals_affirm_all_coverage(goals, family.groups, family.coverage) {
            continue;
        }
        if place_repair_criterion(goals, family.groups, repair.criterion, repair.target_terms) {
            changed = true;
        } else if goals.len() < 9 {
            push_fallback_goal(
                goals,
                fallback_goal_spec(repair.fallback),
                vec![repair.criterion.to_string()],
            );
            changed = true;
        }
    }
    changed
}

#[allow(dead_code)]
fn validate_provider_goal_drafts(goals: &[GoalDraft], product_brief: &str) -> anyhow::Result<()> {
    validate_provider_goal_text(goals)?;
    validate_goal_drafts_with_materiality(goals, product_brief, false, true)
}

fn validate_grounded_provider_goal_drafts(
    goals: &[GoalDraft],
    product_brief: &str,
    profile: &ProductProfile,
) -> anyhow::Result<()> {
    validate_provider_goal_text(goals)?;
    validate_goal_drafts_with_materiality_and_profile(
        goals,
        product_brief,
        false,
        true,
        Some(profile),
    )?;
    validate_goal_grounding(goals, profile)
}

fn validate_provider_goal_text(goals: &[GoalDraft]) -> anyhow::Result<()> {
    // A provider that echoes an archetype-menu entry as a goal title has
    // ignored the tailoring instruction; route it back through the revision
    // loop instead of accepting an untailored draft. Goals reconciled to an
    // existing identity are exempt: reusing a stored title is instructed
    // behavior, and rejecting it would fail every regeneration of a
    // workspace whose goals legitimately carry such a title.
    for goal in goals {
        if goal.goal_id.is_some() {
            continue;
        }
        let normalized = normalized_goal_identity(&goal.title);
        if GOAL_TEMPLATES.iter().any(|template| {
            normalized_goal_identity(template.menu_topic) == normalized
                || normalized_goal_identity(template.title) == normalized
        }) {
            anyhow::bail!(
                "goal titles must be tailored to the product brief instead of echoing the archetype menu; rewrite: {}",
                goal.title.trim()
            );
        }
    }
    Ok(())
}

fn validate_goal_drafts(goals: &[GoalDraft], product_brief: &str) -> anyhow::Result<()> {
    validate_goal_drafts_with_materiality(goals, product_brief, false, false)
}

fn validate_goal_drafts_with_materiality(
    goals: &[GoalDraft],
    product_brief: &str,
    require_lexical_materiality: bool,
    generated_draft: bool,
) -> anyhow::Result<()> {
    validate_goal_drafts_with_materiality_and_profile(
        goals,
        product_brief,
        require_lexical_materiality,
        generated_draft,
        None,
    )
}

fn validate_goal_drafts_with_materiality_and_profile(
    goals: &[GoalDraft],
    product_brief: &str,
    require_lexical_materiality: bool,
    generated_draft: bool,
    profile: Option<&ProductProfile>,
) -> anyhow::Result<()> {
    let valid_count = if generated_draft { 6..=9 } else { 3..=9 };
    if !valid_count.contains(&goals.len()) {
        if generated_draft {
            anyhow::bail!("AI goal generation must return between 6 and 9 material goals");
        }
        anyhow::bail!("the goal set must contain between 3 and 9 material goals");
    }
    let mut group_counts = BTreeMap::new();
    let mut keys = BTreeSet::new();
    let mut goal_ids = BTreeSet::new();
    let mut titles = BTreeSet::new();
    let mut repository_contract_failures = Vec::new();
    for goal in goals {
        let key = goal.key.trim();
        let title = goal.title.trim();
        if let Some(reason) = provider_goal_non_substantive_reason(goal) {
            anyhow::bail!(
                "goal {:?} is not substantive: {reason}; replace it with product-specific outcome text",
                goal.title.trim()
            );
        }
        if key.len() < 3
            || key.len() > 64
            || !valid_goal_key(key)
            || title.is_empty()
            || title.chars().count() > 220
            || goal.business_outcome.trim().is_empty()
            || goal.business_outcome.chars().count() > 640
            || !(1..=5).contains(&goal.priority)
            || !(2..=6).contains(&goal.criteria.len())
            || goal
                .criteria
                .iter()
                .any(|item| item.trim().is_empty() || item.chars().count() > 280)
            || goal.rubric_dimensions.is_empty()
            || goal.rubric_dimensions.len() > 3
            || goal
                .rubric_dimensions
                .iter()
                .any(|item| item.trim().is_empty() || item.chars().count() > 100)
        {
            anyhow::bail!(
                "every goal must satisfy the schema's key, title, outcome, priority, criteria, and rubric-dimension limits"
            );
        }
        if !keys.insert(key.to_string()) {
            anyhow::bail!("every generated goal must have a unique key; duplicate key: {key}");
        }
        if !titles.insert(title.to_lowercase()) {
            anyhow::bail!("generated goals must have unique titles; merge or rename: {title}");
        }
        if let Some(goal_id) = &goal.goal_id
            && (goal_id.trim().is_empty() || !goal_ids.insert(goal_id.clone()))
        {
            anyhow::bail!("generated goal identities must be non-empty and unique");
        }
        let group = goal.rubric_dimensions[0].as_str();
        if !GOAL_GROUPS.contains(&group) {
            anyhow::bail!("every generated goal must use a recognized goal group");
        }
        if group == BUSINESS_GROUP
            && let Some(reason) = business_goal_tactical_reason(goal, require_lexical_materiality)
        {
            anyhow::bail!(
                "business goal {:?} is too tactical: {reason}; business goals must consolidate screen, persona, feature, and workflow-step tactics into durable outcomes; name a material consequence in the title or outcome, and when checks mention reports, dashboards, screens, or workflow steps, name the accountable owner, decision rule, or review cadence in the outcome",
                goal.title.trim()
            );
        }
        repository_contract_failures.extend(goal_criteria_repository_contract_failures(
            title,
            &goal.criteria,
        ));
        *group_counts.entry(group).or_insert(0_usize) += 1;
    }
    if !repository_contract_failures.is_empty() {
        anyhow::bail!(
            "goal success checks violate the frozen-commit verification contract: {}",
            repository_contract_failures.join("; ")
        );
    }
    if GOAL_GROUPS
        .iter()
        .any(|group| !group_counts.contains_key(group))
    {
        anyhow::bail!("goal generation must cover every goal group");
    }
    if generated_draft
        && group_counts
            .get(BUSINESS_GROUP)
            .copied()
            .unwrap_or_default()
            < 2
    {
        anyhow::bail!(
            "AI goal generation must include at least two grounded Business & product goals"
        );
    }

    // The required engineering baseline comes from the catalog's single
    // coverage-family list. Provider revisions must satisfy it; this path
    // never inserts, rewrites, or drops goals after validation.
    let mut missing_engineering_coverage = Vec::new();
    for family in COVERAGE_FAMILIES {
        if !generated_draft && !family.required_for_stored_sets {
            continue;
        }
        if let Some(signal) = family.signal {
            let applies = profile.map_or_else(
                || signal.applies(product_brief),
                |profile| profile_capability_applies(profile, signal),
            );
            if !applies {
                continue;
            }
        }
        if !goals_affirm_all_coverage(goals, family.groups, family.coverage) {
            let repair = family.repair.as_ref().map_or_else(
                || "add a concrete repository-verifiable success check".to_string(),
                |repair| format!("add this success check: {}", repair.criterion),
            );
            missing_engineering_coverage.push(format!(
                "{} [{}]: {}",
                family.label,
                family.groups.join(" or "),
                repair
            ));
        }
    }
    if !missing_engineering_coverage.is_empty() {
        anyhow::bail!(
            "goal set is missing engineering coverage: {}",
            missing_engineering_coverage.join("; ")
        );
    }
    Ok(())
}

fn validate_goal_grounding(goals: &[GoalDraft], profile: &ProductProfile) -> anyhow::Result<()> {
    let fact_ids = profile.fact_ids();
    let stop_terms = [
        "product",
        "customer",
        "customers",
        "software",
        "platform",
        "business",
    ];
    let terms = profile
        .product_terms
        .iter()
        .flat_map(|term| term.split(|character: char| !character.is_alphanumeric()))
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 4 && !stop_terms.contains(&term.as_str()))
        .collect::<BTreeSet<_>>();
    if terms.is_empty() {
        anyhow::bail!("the grounded product profile did not contain usable product-specific terms");
    }
    for goal in goals {
        if goal
            .grounding_fact_ids
            .iter()
            .any(|id| !fact_ids.contains(id.as_str()))
        {
            anyhow::bail!(
                "goal grounding references an unknown product fact: {}",
                goal.title
            );
        }
        if goal
            .rubric_dimensions
            .first()
            .is_some_and(|group| group == BUSINESS_GROUP)
        {
            if goal.grounding_fact_ids.is_empty() {
                anyhow::bail!(
                    "every Business & product goal must cite at least one grounded product fact"
                );
            }
            let narrative = format!("{} {}", goal.title, goal.business_outcome).to_lowercase();
            if !terms.iter().any(|term| narrative.contains(term)) {
                anyhow::bail!(
                    "Business & product goal is not written in the product's own terms: {}",
                    goal.title
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_edited_goal_set(
    goals: &[ApproveGoalRequest],
    product_brief: &str,
) -> anyhow::Result<()> {
    let drafts = goals
        .iter()
        .enumerate()
        .map(|(index, goal)| GoalDraft {
            goal_id: Some(goal.goal_id.clone()),
            key: format!("edited-goal-{}", index + 1),
            title: goal.title.clone(),
            business_outcome: goal.business_outcome.clone(),
            priority: goal.priority,
            criteria: goal.criteria.clone(),
            rubric_dimensions: goal.rubric_dimensions.clone(),
            grounding_fact_ids: Vec::new(),
        })
        .collect::<Vec<_>>();
    validate_goal_drafts(&drafts, product_brief).map_err(|error| {
        let message = error
            .to_string()
            .replace("goal generation", "the goal set")
            .replace(
                "provider returned invalid draft goals",
                "the goal set contains invalid goals",
            )
            .replace("every generated goal", "every goal");
        anyhow::anyhow!("goal set is not ready for analysis: {message}")
    })
}

pub(crate) fn validate_approved_goal_set(
    goals: &[GoalVersion],
    product_brief: &str,
) -> anyhow::Result<()> {
    let edited = goals
        .iter()
        .map(|goal| ApproveGoalRequest {
            goal_id: goal.goal_id.clone(),
            title: goal.title.clone(),
            business_outcome: goal.business_outcome.clone(),
            criteria: goal
                .criteria
                .iter()
                .map(|criterion| criterion.text.clone())
                .collect(),
            priority: goal.priority,
            position: goal.position,
            rubric_dimensions: goal.rubric_dimensions.clone(),
        })
        .collect::<Vec<_>>();
    validate_edited_goal_set(&edited, product_brief)
}

pub(crate) fn validate_approved_goal_request(goal: &ApproveGoalRequest) -> anyhow::Result<()> {
    validate_goal_criteria_repository_contract(&goal.title, &goal.criteria)
        .map_err(|error| anyhow::anyhow!("goal is not ready for approval: {error}"))
}

fn valid_goal_key(key: &str) -> bool {
    !key.starts_with('-')
        && !key.ends_with('-')
        && !key.contains("--")
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::test_support::{complete_generated_goal_set, generated_goal};

    fn example_leave_profile() -> ProductProfile {
        ProductProfile {
            product_name: "ExampleLeave".into(),
            product_terms: vec![
                "ExampleLeave".into(),
                "leave management".into(),
                "absence policy".into(),
                "manager approvals".into(),
            ],
            customers: vec!["HR teams and people managers".into()],
            core_jobs: vec!["Request, approve, and track employee leave".into()],
            desired_outcomes: vec![
                "Compliant leave decisions with less administrative work".into(),
            ],
            strategic_priorities: vec!["Enterprise readiness".into()],
            important_risks: vec!["Cross-customer data exposure".into()],
            facts: vec![
                super::super::product_profile::ProductFact {
                    id: "leave-workflow".into(),
                    statement:
                        "ExampleLeave is a fictional product that manages employee leave workflows"
                            .into(),
                    source_ids: vec!["file-1-slide-2".into()],
                },
                super::super::product_profile::ProductFact {
                    id: "enterprise-orgs".into(),
                    statement: "Multiple customer organizations use the service".into(),
                    source_ids: vec!["file-1-slide-4".into()],
                },
            ],
            capabilities: super::super::product_profile::ProductCapabilities {
                multiple_customer_organizations: super::super::product_profile::CapabilitySignal {
                    present: true,
                    source_ids: vec!["file-1-slide-4".into()],
                },
                integrations: super::super::product_profile::CapabilitySignal {
                    present: true,
                    source_ids: vec!["file-1-slide-5".into()],
                },
                ..Default::default()
            },
        }
    }

    fn grounded_example_leave_goals() -> Vec<GoalDraft> {
        let mut goals = complete_generated_goal_set();
        for (index, goal) in goals.iter_mut().take(3).enumerate() {
            goal.grounding_fact_ids = vec!["leave-workflow".into()];
            goal.title = match index {
                0 => "Employees complete ExampleLeave requests confidently",
                1 => "Managers approve leave without policy ambiguity",
                _ => "HR teams prove absence-policy outcomes",
            }
            .into();
            goal.business_outcome = format!(
                "ExampleLeave produces a measurable synthetic customer outcome for goal {}.",
                index + 1
            );
        }
        let engineering_titles = [
            "ExampleLeave tenants remain isolated",
            "ExampleLeave integrations preserve leave decisions",
            "ExampleLeave changes ship safely",
            "Leave operations surface problems before customers",
            "ExampleLeave records survive failures",
            "ExampleLeave releases remain secure",
        ];
        for (goal, title) in goals.iter_mut().skip(3).zip(engineering_titles) {
            goal.key = title
                .to_lowercase()
                .split(|character: char| !character.is_ascii_alphanumeric())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            goal.title = title.into();
            goal.business_outcome = format!(
                "{title} protects reliable leave planning for fictional ExampleLeave customers."
            );
        }
        goals
    }

    #[test]
    fn grounded_fictional_leave_fixture_passes_with_product_tenant_and_observability_goals() {
        let profile = example_leave_profile();
        let goals = grounded_example_leave_goals();
        validate_grounded_provider_goal_drafts(&goals, &profile.goal_context().unwrap(), &profile)
            .unwrap();
        assert!(goals.iter().any(|goal| goal.title.contains("ExampleLeave")));
        assert!(goals.iter().any(|goal| goal.title.contains("tenants")));
        assert!(goals.iter().any(|goal| {
            goal.title.contains("surface problems")
                && goal
                    .criteria
                    .iter()
                    .any(|criterion| criterion.contains("alerting"))
        }));
    }

    #[test]
    fn grounded_prompt_enumerates_exact_always_and_capability_required_controls() {
        let profile = example_leave_profile();
        let required = required_engineering_coverage_prompt(&profile);
        for expected in [
            "Structured logs, metrics, traces, and error tracking",
            "Automated tests, coverage thresholds, static analysis",
            "Every data access path enforces tenant isolation",
            "Versioned integration and API contracts",
            "Dependency licenses are inventoried",
            "documented developer bootstrap",
        ] {
            assert!(required.contains(expected), "missing {expected}");
        }
        assert!(!required.contains("Webhook delivery uses signed payloads"));
    }

    #[test]
    fn product_goal_with_financial_title_can_use_tactical_success_checks() {
        let mut goal = generated_goal(
            BUSINESS_GROUP,
            "Employers can calculate leave pay and see the operating cost before they lose money",
            &[
                "A leave-pay report reconciles to payroll fixtures",
                "The cost summary is available before approval",
            ],
        );
        goal.business_outcome =
            "Employers prevent financial loss by understanding leave costs before approval.".into();
        assert!(business_goal_tactical_reason(&goal, false).is_none());
    }

    #[test]
    fn screenshot_three_item_output_is_rejected_before_any_reconciliation() {
        let goals = grounded_example_leave_goals();
        let error = validate_grounded_provider_goal_drafts(
            &goals[..3],
            &example_leave_profile().goal_context().unwrap(),
            &example_leave_profile(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("between 6 and 9"), "{error}");
    }

    #[test]
    fn malformed_provider_output_becomes_sanitized_revision_feedback() {
        let error = serde_json::from_value::<GoalGenerationOutput>(serde_json::json!({
            "unexpected": "raw attachment text must never enter diagnostics"
        }))
        .err()
        .unwrap();
        let feedback = goal_output_schema_feedback(&error);
        assert!(feedback.contains("required top-level `goals` array"));
        assert!(!feedback.contains("raw attachment text"));
    }

    #[test]
    fn screenshot_schema_valid_holding_object_is_rejected_without_semantic_repair() {
        let profile = example_leave_profile();
        let brief = profile.goal_context().unwrap();
        let mut goals = grounded_example_leave_goals();
        goals[0].title = "Need a second schema-valid holding object".into();
        goals[0].business_outcome =
            "Satisfy the minimum array length while collecting real evidence".into();
        let before = goals.clone();
        let error = validate_grounded_provider_goal_drafts(&goals, &brief, &profile)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Need a second schema-valid holding object"),
            "{error}"
        );
        assert!(error.contains("schema-valid"), "{error}");
        reconcile_goal_identities(&mut goals, &[]).unwrap();
        assert_eq!(goals[0].title, before[0].title);
        assert_eq!(goals[0].business_outcome, before[0].business_outcome);
        assert!(validate_grounded_provider_goal_drafts(&goals, &brief, &profile).is_err());
    }

    #[test]
    fn exact_generic_catalog_fallback_titles_cannot_pass_generation() {
        let profile = example_leave_profile();
        let brief = profile.goal_context().unwrap();
        let mut goals = grounded_example_leave_goals();
        goals[6].title = "Operations stay observable and recoverable".into();
        let error = validate_grounded_provider_goal_drafts(&goals, &brief, &profile)
            .unwrap_err()
            .to_string();
        assert!(error.contains("tailored to the product brief"), "{error}");
    }

    #[test]
    fn generated_goals_enforce_broad_groups_and_engineering_coverage() {
        let goals = complete_generated_goal_set();
        validate_goal_drafts(&goals, "B2B SaaS for organizations").unwrap();

        let mut unequal_groups = goals.clone();
        unequal_groups[4].rubric_dimensions[0] = "Business & product".into();
        validate_goal_drafts(&unequal_groups, "B2B SaaS").unwrap();

        let mut missing_group = goals.clone();
        for goal in &mut missing_group[6..] {
            goal.rubric_dimensions[0] = "Business & product".into();
        }
        assert!(validate_goal_drafts(&missing_group, "B2B SaaS").is_err());

        let mut missing_observability = goals.clone();
        missing_observability[6].criteria = vec![
            "Operators review customer issues".into(),
            "Support follows documented steps".into(),
        ];
        assert!(validate_goal_drafts(&missing_observability, "B2B SaaS").is_err());

        let mut metrics_without_operational_alerting = goals.clone();
        metrics_without_operational_alerting[6].criteria = vec![
            "Product metrics are reviewed quarterly".into(),
            "A version-controlled record names the owner who reviews results".into(),
        ];
        let error = validate_goal_drafts(&metrics_without_operational_alerting, "B2B SaaS")
            .unwrap_err()
            .to_string();
        assert!(error.contains("observability and alerting"));

        let mut backups_without_tested_restore = goals.clone();
        backups_without_tested_restore[7].criteria[0] = "Backups run daily".into();
        let error = validate_goal_drafts(&backups_without_tested_restore, "B2B SaaS")
            .unwrap_err()
            .to_string();
        assert!(error.contains("tested backup and restore"));

        let mut security_without_auditability = goals.clone();
        security_without_auditability[8].criteria[0] =
            "Security controls are reviewed quarterly".into();
        let error = validate_goal_drafts(&security_without_auditability, "B2B SaaS")
            .unwrap_err()
            .to_string();
        assert!(error.contains("security and auditability"));
        assert!(error.contains("Least-privilege access controls and encryption"));
        assert!(error.contains("Architecture & platform or Operations & reliability"));

        let mut tests_without_ci_gates = goals.clone();
        tests_without_ci_gates[5].criteria[0] = "Automated tests run locally".into();
        tests_without_ci_gates[5].criteria[1] = "Static analysis produces a report".into();
        let error = validate_goal_drafts(&tests_without_ci_gates, "B2B SaaS")
            .unwrap_err()
            .to_string();
        assert!(error.contains("automated tests and CI"));

        let mut negated_engineering = goals.clone();
        negated_engineering[3].criteria = vec![
            "Multi-tenant isolation is not enforced".into(),
            "Cross-tenant tests are planned later".into(),
        ];
        negated_engineering[5].criteria = vec![
            "Automated tests are excluded".into(),
            "CI gates are deferred".into(),
        ];
        negated_engineering[6].criteria = vec![
            "Monitoring is deferred".into(),
            "Alerting is not implemented".into(),
        ];
        assert!(validate_goal_drafts(&negated_engineering, "B2B SaaS").is_err());

        let mut sparse_engineering = goals.clone();
        for index in [1, 3, 5, 6, 7, 8] {
            sparse_engineering[index].criteria = vec![
                "A version-controlled operational record names the executive owner".into(),
                "A repository dashboard configuration defines the quarterly measure".into(),
            ];
        }
        let error =
            validate_provider_goal_drafts(&sparse_engineering, "B2B SaaS for organizations")
                .unwrap_err()
                .to_string();
        for missing in [
            "observability and alerting",
            "automated tests and CI",
            "product usage metrics instrumentation",
            "multi-tenant isolation",
            "tested backup and restore",
            "security and auditability",
            "release rollback or incident recovery",
            "dependency vulnerability scanning",
            "automated dependency updates",
            "clean machine",
        ] {
            assert!(
                error.contains(missing),
                "missing aggregate feedback: {missing}"
            );
        }

        let mut missing_tenant_isolation = goals.clone();
        missing_tenant_isolation[3].criteria = vec![
            "Customer records use stable identifiers".into(),
            "Access tests fail closed".into(),
        ];
        assert!(
            validate_goal_drafts(&missing_tenant_isolation, "B2B SaaS for organizations").is_err()
        );
        validate_goal_drafts(&missing_tenant_isolation, "Single-user offline utility").unwrap();

        let explicitly_single_user_brief =
            "A single-user local desktop product, not a multi-tenant hosted service";
        let edited_without_tenant_isolation = missing_tenant_isolation
            .iter()
            .enumerate()
            .map(|(index, goal)| ApproveGoalRequest {
                goal_id: format!("single-user-{index}"),
                title: goal.title.clone(),
                business_outcome: goal.business_outcome.clone(),
                criteria: goal.criteria.clone(),
                priority: goal.priority,
                position: index as u32 + 1,
                rubric_dimensions: goal.rubric_dimensions.clone(),
            })
            .collect::<Vec<_>>();
        validate_edited_goal_set(
            &edited_without_tenant_isolation,
            explicitly_single_user_brief,
        )
        .unwrap();
        assert!(
            validate_edited_goal_set(
                &edited_without_tenant_isolation,
                "A multi-tenant SaaS product for customer organizations",
            )
            .unwrap_err()
            .to_string()
            .contains("multi-tenant isolation")
        );

        let conditional_brief =
            "The product requires AI quality controls and processes sensitive-data records";
        let mut missing_ai_and_privacy = complete_generated_goal_set();
        let error = validate_provider_goal_drafts(&missing_ai_and_privacy, conditional_brief)
            .unwrap_err()
            .to_string();
        assert!(error.contains("AI evaluation, provenance, and human-control safeguards"));
        assert!(error.contains("sensitive-data privacy lifecycle controls"));
        missing_ai_and_privacy[4].criteria.extend([
            "Versioned model evaluations and AI quality gates cover accuracy and failure modes before release".into(),
            "Every automated decision records model provenance and supports explainable human review or human override".into(),
            "Sensitive data privacy controls enforce retention, deletion, consent, and data minimization with automated tests".into(),
        ]);
        validate_provider_goal_drafts(&missing_ai_and_privacy, conditional_brief).unwrap();

        let mut duplicate_key = goals.clone();
        duplicate_key[1].key = duplicate_key[0].key.clone();
        assert!(validate_goal_drafts(&duplicate_key, "B2B SaaS").is_err());

        validate_goal_drafts(
            &[
                goals[0].clone(),
                goals[1].clone(),
                goals[3].clone(),
                goals[5].clone(),
                goals[6].clone(),
                goals[7].clone(),
                goals[8].clone(),
            ],
            "B2B SaaS",
        )
        .unwrap();

        let mut tactical_set = goals;
        for index in 0..15 {
            tactical_set.push(generated_goal(
                "Business & product",
                &format!("Billing settings screen {index}"),
                &["The screen renders", "The button saves"],
            ));
        }
        assert_eq!(tactical_set.len(), 24);
        assert!(validate_goal_drafts(&tactical_set, "B2B SaaS").is_err());

        let mut hidden_tactical_set = complete_generated_goal_set();
        hidden_tactical_set[0].title = "Configure discount rules".into();
        hidden_tactical_set[0].key = "configure-discount-rules".into();
        assert!(validate_goal_drafts(&hidden_tactical_set, "B2B SaaS").is_err());

        let mut persona_microgoals = complete_generated_goal_set();
        let patterns = [
            "Analysts upload forecasts",
            "Approvers review exceptions",
            "Operators balance connector queues",
        ];
        for index in 0..15 {
            persona_microgoals.push(generated_goal(
                "Business & product",
                &format!("{} {index}", patterns[index % patterns.len()]),
                &[
                    "The workflow step completes",
                    "The persona sees a confirmation",
                ],
            ));
        }
        assert_eq!(persona_microgoals.len(), 24);
        assert!(validate_goal_drafts(&persona_microgoals, "B2B SaaS").is_err());

        let mut positive_title_negative_criteria = complete_generated_goal_set();
        positive_title_negative_criteria[6].title = "Observability guides operations".into();
        positive_title_negative_criteria[6].criteria = vec![
            "Monitoring is unavailable".into(),
            "Alerts are unsupported and manual only".into(),
        ];
        assert!(validate_goal_drafts(&positive_title_negative_criteria, "B2B SaaS").is_err());

        let edited = complete_generated_goal_set()
            .into_iter()
            .enumerate()
            .map(|(index, goal)| ApproveGoalRequest {
                goal_id: format!("edited-{index}"),
                title: goal.title,
                business_outcome: goal.business_outcome,
                criteria: goal.criteria,
                priority: goal.priority,
                position: index as u32 + 1,
                rubric_dimensions: goal.rubric_dimensions,
            })
            .collect::<Vec<_>>();
        validate_edited_goal_set(&edited, "B2B SaaS for organizations").unwrap();
        let mut immaterial_edit = edited.clone();
        immaterial_edit[0] = ApproveGoalRequest {
            goal_id: "edited-purple-dashboard".into(),
            title: "Make the dashboard purple".into(),
            business_outcome: "The dashboard uses purple throughout".into(),
            criteria: vec![
                "Purple appears in every dashboard".into(),
                "An executive owner reviews purple monthly".into(),
            ],
            priority: 2,
            position: 1,
            rubric_dimensions: vec!["Business & product".into()],
        };
        let error = validate_edited_goal_set(&immaterial_edit, "B2B SaaS for organizations")
            .unwrap_err()
            .to_string();
        assert!(error.contains("durable outcomes"));

        let immaterial_cases = [
            (
                "edited-purple-unrelated-reliability",
                "Make the dashboard purple",
                "The dashboard uses purple throughout",
                "Purple appears in every dashboard",
                "An executive owner reviews system reliability monthly",
            ),
            (
                "edited-documentation-count",
                "Improve internal documentation in Q3",
                "Version 2 naming stays consistent",
                "Document 10 modules",
                "An owner reviews documentation monthly",
            ),
            (
                "edited-css-margin",
                "Increase dashboard margin to 20px",
                "The dashboard uses a larger margin",
                "Dashboard margin is 20px",
                "An executive owner reviews it monthly",
            ),
            (
                "edited-meeting-volume",
                "Increase internal meetings by week",
                "The team holds meetings",
                "Meeting cadence is recorded",
                "A facilitator is assigned",
            ),
            (
                "edited-animation-speed",
                "Make the dashboard animation faster",
                "The dashboard animation is faster",
                "Animation duration is 100ms",
                "An executive owner reviews it monthly",
            ),
            (
                "edited-regional-revenue-report",
                "Produce quarterly business summary",
                "Leaders see regional performance",
                "Revenue is grouped by region",
                "A leader reviews it monthly",
            ),
            (
                "edited-negated-revenue",
                "Preserve the current pricing display",
                "Revenue must not increase by design",
                "The existing price remains visible",
                "A leader reviews it monthly",
            ),
            (
                "edited-discount-rules-with-revenue",
                "Configure discount rules",
                "Revenue increases when pricing is consistent",
                "Revenue target exceeds plan",
                "An executive owner reviews it monthly",
            ),
            (
                "edited-purple-with-satisfaction",
                "Make the dashboard purple",
                "Customer satisfaction improves",
                "Purple appears in every dashboard",
                "An executive owner reviews it monthly",
            ),
        ];
        for (goal_id, title, outcome, first_criterion, second_criterion) in immaterial_cases {
            let mut candidate = edited.clone();
            candidate[0] = ApproveGoalRequest {
                goal_id: goal_id.into(),
                title: title.into(),
                business_outcome: outcome.into(),
                criteria: vec![first_criterion.into(), second_criterion.into()],
                priority: 2,
                position: 1,
                rubric_dimensions: vec!["Business & product".into()],
            };
            let result = validate_edited_goal_set(&candidate, "B2B SaaS for organizations");
            assert!(result.is_err(), "{title}: unexpectedly accepted");
            let error = result.unwrap_err().to_string();
            assert!(error.contains("durable outcomes"), "{title}: {error}");
        }

        let mut affirmed_material_edit = edited.clone();
        affirmed_material_edit[0].title = "Customer retention improves".into();
        affirmed_material_edit[0].business_outcome =
            "Customers renew because the core job reaches a reliable result".into();
        validate_edited_goal_set(&affirmed_material_edit, "B2B SaaS for organizations").unwrap();

        let mut broad_goal_with_tactical_criteria = affirmed_material_edit.clone();
        broad_goal_with_tactical_criteria[0].criteria = vec![
            "The request screen supports the retained-customer outcome".into(),
            "A version-controlled retention record names the executive owner and monthly review cadence".into(),
        ];
        validate_edited_goal_set(
            &broad_goal_with_tactical_criteria,
            "B2B SaaS for organizations",
        )
        .unwrap();

        let material_variants = [
            (
                "Prevent leave overpayments",
                "Payroll only pays employees for eligible time",
                "Payroll event instrumentation measures overpayments against a configured 0.1% threshold",
                "A version-controlled exception record names the finance owner and monthly review cadence",
            ),
            (
                "Protect subscription margin",
                "Contract pricing produces profitable revenue",
                "Version-controlled margin metrics measure results against a configured 70% threshold",
                "A repository dashboard configuration names the finance owner and monthly loss review",
            ),
            (
                "Reduce customer churn",
                "Customers continue using the service after renewal",
                "Retention event instrumentation measures annual churn against a configured 5% threshold",
                "A version-controlled retention record names the revenue owner and monthly review cadence",
            ),
            (
                "Prevent transaction fraud",
                "Customers complete legitimate purchases without financial loss",
                "Fraud event instrumentation measures loss against a configured 0.2% volume threshold",
                "A version-controlled exception record names the risk owner and weekly review cadence",
            ),
            (
                "Increase team productivity",
                "Teams complete more customer work with the same staffing",
                "Version-controlled productivity metrics measure results against a configured 20% threshold",
                "A repository dashboard configuration names the operations owner and monthly capacity review",
            ),
        ];
        for (index, (title, outcome, first_criterion, second_criterion)) in
            material_variants.into_iter().enumerate()
        {
            let mut candidate = edited.clone();
            candidate[0] = ApproveGoalRequest {
                goal_id: format!("edited-material-{index}"),
                title: title.into(),
                business_outcome: outcome.into(),
                criteria: vec![first_criterion.into(), second_criterion.into()],
                priority: 4,
                position: 1,
                rubric_dimensions: vec!["Business & product".into()],
            };
            validate_edited_goal_set(&candidate, "B2B SaaS for organizations").unwrap();
        }

        let mut incomplete_edit = edited.clone();
        incomplete_edit.retain(|goal| goal.rubric_dimensions[0] != "Operations & reliability");
        assert!(
            validate_edited_goal_set(&incomplete_edit, "B2B SaaS for organizations")
                .unwrap_err()
                .to_string()
                .contains("goal set is not ready for analysis")
        );

        let enterprise_brief = "Enterprise knowledge search with connected content sources, ERP and commerce integrations, APIs, webhooks, migration safety, idempotency, performance, and capacity targets.";
        validate_edited_goal_set(&edited, enterprise_brief).unwrap();
        let mut missing_integration_edit = edited.clone();
        missing_integration_edit.retain(|goal| goal.title != "Integrations remain dependable");
        let error = validate_edited_goal_set(&missing_integration_edit, enterprise_brief)
            .unwrap_err()
            .to_string();
        assert!(error.contains("integrations, APIs, or webhooks"));
        assert!(error.contains("performance and capacity targets"));

        let mut tactical_body = edited;
        tactical_body[0].title = "Improve the analyst experience".into();
        tactical_body[0].business_outcome =
            "Analysts use a request form and see a confirmation page to reduce risk.".into();
        tactical_body[0].criteria = vec![
            "The request screen renders".into(),
            "The submit button opens a confirmation modal".into(),
        ];
        let error = validate_edited_goal_set(&tactical_body, "B2B SaaS for organizations")
            .unwrap_err()
            .to_string();
        assert!(error.contains("durable outcomes"));

        tactical_body[0].business_outcome =
            "Analysts use a request form and see a confirmation page to improve customer trust."
                .into();
        let error = validate_edited_goal_set(&tactical_body, "B2B SaaS for organizations")
            .unwrap_err()
            .to_string();
        assert!(error.contains("durable outcomes"));

        let mut incomplete_webhook_controls = complete_generated_goal_set();
        let webhook_goal = incomplete_webhook_controls
            .iter_mut()
            .find(|goal| goal.title == "Releases remain secure and supportable")
            .unwrap();
        webhook_goal.criteria[2] = "Webhook delivery is documented".into();
        let error = validate_goal_drafts(
            &incomplete_webhook_controls,
            "B2B SaaS with webhook delivery",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("webhook signing, retries, replay, and dead-letter recovery"));

        assert!(repair_required_engineering_baseline(
            &mut incomplete_webhook_controls,
            "B2B SaaS with webhook delivery"
        ));
        validate_goal_drafts(
            &incomplete_webhook_controls,
            "B2B SaaS with webhook delivery",
        )
        .unwrap();
    }

    #[test]
    fn outcome_only_criteria_are_rejected_across_generic_product_briefs() {
        let cases = [
            (
                "Collaborative project planning for customer organizations",
                "The onboarding workflow achieves 70 percent adoption within 30 days",
            ),
            (
                "Workforce scheduling software for distributed service teams",
                "The service-team workflow achieves at least 80% satisfaction among surveyed users, with an executive owner reviewing quarterly",
            ),
            (
                "Wholesale billing and reconciliation for regional distributors",
                "The billing workflow increases net revenue by 15%, with the finance owner reviewing monthly",
            ),
        ];
        for (brief, outcome_only) in cases {
            let mut edited = complete_generated_goal_set()
                .into_iter()
                .enumerate()
                .map(|(index, goal)| ApproveGoalRequest {
                    goal_id: format!("generic-{index}"),
                    title: goal.title,
                    business_outcome: goal.business_outcome,
                    criteria: goal.criteria,
                    priority: goal.priority,
                    position: index as u32 + 1,
                    rubric_dimensions: goal.rubric_dimensions,
                })
                .collect::<Vec<_>>();
            edited[0].criteria[0] = outcome_only.into();
            let error = validate_edited_goal_set(&edited, brief)
                .unwrap_err()
                .to_string();
            assert!(error.contains("already-achieved"), "{brief}: {error}");
            assert!(
                error.contains("instrumentation or controls"),
                "{brief}: {error}"
            );
        }
    }

    #[test]
    fn provider_validation_reports_every_repository_contract_failure_at_once() {
        let mut goals = complete_generated_goal_set();
        let first_title = goals[0].title.clone();
        let second_title = goals[1].title.clone();
        goals[0].criteria[0] = "Leaders say the product improves decisions".into();
        goals[1].criteria[1] = "80 percent of teams finish within 30 days".into();

        let error = validate_goal_drafts(&goals, "B2B SaaS")
            .unwrap_err()
            .to_string();
        assert!(error.contains("goal success checks violate"));
        assert!(error.contains(&first_title));
        assert!(error.contains(&second_title));
        assert!(error.contains("success check 1"));
        assert!(error.contains("success check 2"));
    }

    #[test]
    fn repository_measurement_controls_are_accepted_across_generic_product_briefs() {
        let cases = [
            (
                "Collaborative project planning for customer organizations",
                "Version-controlled onboarding instrumentation emits first-result events, and automated tests verify the configured activation threshold",
            ),
            (
                "Workforce scheduling software for distributed service teams",
                "A survey-response event schema and local aggregation code measure satisfaction, with automated privacy tests and no free text",
            ),
            (
                "Wholesale billing and reconciliation for regional distributors",
                "Billing events and version-controlled metric configuration measure revenue attribution against a configured threshold",
            ),
        ];
        for (brief, repository_control) in cases {
            let mut edited = complete_generated_goal_set()
                .into_iter()
                .enumerate()
                .map(|(index, goal)| ApproveGoalRequest {
                    goal_id: format!("generic-{index}"),
                    title: goal.title,
                    business_outcome: goal.business_outcome,
                    criteria: goal.criteria,
                    priority: goal.priority,
                    position: index as u32 + 1,
                    rubric_dimensions: goal.rubric_dimensions,
                })
                .collect::<Vec<_>>();
            edited[0].criteria[0] = repository_control.into();
            validate_edited_goal_set(&edited, brief).unwrap();
        }
    }

    #[test]
    fn standardized_catalog_criteria_all_satisfy_the_repository_contract() {
        for template in GOAL_TEMPLATES {
            for criterion in template.criteria {
                assert!(
                    repository_verifiability_failure(criterion).is_none(),
                    "{}: {criterion}",
                    template.key
                );
            }
        }
    }

    #[test]
    fn provider_and_final_drafts_accept_non_tactical_material_outcomes_without_keyword_gates() {
        let mut goals = complete_generated_goal_set();
        goals[0].title = "Keep source analysis private and useful".into();
        goals[0].business_outcome =
            "Engineering teams keep proprietary code on device while receiving actionable architecture findings"
                .into();
        goals[0].criteria = vec![
            "Instrumentation code records completed scans and actionable findings, with a configured 95 percent threshold".into(),
            "Automated IPC privacy tests prove no source text crosses the desktop IPC boundary".into(),
        ];

        validate_goal_drafts(&goals, "Single-user local analysis utility").unwrap();
        validate_goal_drafts(&goals, "Single-user local analysis utility").unwrap();
    }

    #[test]
    fn deterministic_baseline_repair_completes_dependency_and_bootstrap_criteria() {
        assert!(statement_affirms_coverage(
            "Data migrations have a safe recovery path, and retryable writes are idempotent, preserving single-write semantics and customer-state integrity through interruption.",
            &["idempotent"]
        ));
        let mut goals = complete_generated_goal_set();
        goals[5].criteria.truncate(2);

        assert!(repair_required_engineering_baseline(
            &mut goals,
            "B2B SaaS with webhooks"
        ));
        validate_goal_drafts(&goals, "B2B SaaS for organizations").unwrap();
        let repaired = goals
            .iter()
            .flat_map(|goal| goal.criteria.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        for required in [
            "dependency vulnerabilities",
            "automated dependency updates",
            "dependency licenses",
            "documented developer bootstrap",
            "clean machine",
            "webhook delivery",
        ] {
            assert!(repaired.contains(required), "repair omitted: {required}");
        }
        assert!(!repair_required_engineering_baseline(
            &mut goals,
            "B2B SaaS with webhooks"
        ));

        let mut missing_capacity = complete_generated_goal_set();
        missing_capacity[4].criteria[2] =
            "Enterprise integrations preserve their versioned API contracts".into();
        assert!(repair_required_engineering_baseline(
            &mut missing_capacity,
            "A large enterprise platform that must scale across many services"
        ));
        validate_goal_drafts(
            &missing_capacity,
            "A large enterprise platform that must scale across many services",
        )
        .unwrap();

        let mut missing_integrations = complete_generated_goal_set();
        missing_integrations.retain(|goal| goal.title != "Integrations remain dependable");
        assert!(repair_required_engineering_baseline(
            &mut missing_integrations,
            "ExampleLeave is a fictional multi-service leave platform with APIs and integrations"
        ));
        validate_goal_drafts(
            &missing_integrations,
            "ExampleLeave is a fictional multi-service leave platform with APIs and integrations",
        )
        .unwrap();
        assert!(missing_integrations.iter().any(|goal| {
            goal.rubric_dimensions
                .first()
                .is_some_and(|group| group == GOAL_GROUPS[1])
                && goal
                    .criteria
                    .iter()
                    .any(|criterion| criterion.contains("Versioned integration and API contracts"))
        }));

        let mut sparse_core = complete_generated_goal_set();
        for index in [3, 5, 6, 7, 8] {
            sparse_core[index].criteria = vec![
                "A version-controlled operational record names the executive owner".into(),
                "A repository dashboard configuration defines the quarterly measure".into(),
            ];
        }
        assert!(repair_required_engineering_baseline(
            &mut sparse_core,
            "Single-user local analysis utility"
        ));
        validate_goal_drafts(&sparse_core, "Single-user local analysis utility").unwrap();

        let mut malformed = goals.clone();
        malformed[0].rubric_dimensions.clear();
        repair_required_engineering_baseline(&mut malformed, "B2B SaaS");
        assert!(validate_goal_drafts(&malformed, "B2B SaaS").is_err());
    }

    #[test]
    fn oversized_provider_drafts_are_capped_before_baseline_repair() {
        let mut goals = complete_generated_goal_set();
        for index in 0..15 {
            goals.push(generated_goal(
                "Business & product",
                &format!("Additional durable outcome {index}"),
                &[
                    "Customer retention improves above the quarterly target",
                    "An executive owner reviews the outcome monthly",
                ],
            ));
        }
        assert_eq!(goals.len(), 24);
        assert!(cap_provider_goal_set(&mut goals));
        assert_eq!(goals.len(), 6);
        for group in GOAL_GROUPS {
            assert!(goals.iter().any(|goal| goal.rubric_dimensions[0] == group));
        }
        repair_required_engineering_baseline(&mut goals, "Single-user local analysis utility");
        validate_goal_drafts(&goals, "Single-user local analysis utility").unwrap();
    }

    #[test]
    fn tactical_provider_goals_are_consolidated_without_another_model_call() {
        let mut goals = complete_generated_goal_set();
        for goal in goals.iter_mut().filter(|goal| {
            goal.rubric_dimensions
                .first()
                .is_some_and(|group| group == GOAL_GROUPS[0])
        }) {
            goal.title = "Configure the results dashboard".into();
            goal.business_outcome = "The dashboard displays analysis results".into();
            goal.criteria = vec![
                "The results screen renders".into(),
                "The export button opens a confirmation modal".into(),
            ];
        }

        cap_provider_goal_set(&mut goals);
        assert!(consolidate_tactical_business_goals(&mut goals));
        assert_eq!(
            goals
                .iter()
                .filter(|goal| goal.rubric_dimensions[0] == GOAL_GROUPS[0])
                .count(),
            1
        );
        repair_required_engineering_baseline(&mut goals, "Single-user local analysis utility");
        validate_goal_drafts(&goals, "Single-user local analysis utility").unwrap();
    }

    #[test]
    fn obsolete_semantic_repair_helper_is_not_part_of_generated_validation() {
        let mut goals = complete_generated_goal_set();
        for goal in goals.iter_mut().filter(|goal| {
            goal.rubric_dimensions
                .first()
                .is_some_and(|group| group == GOAL_GROUPS[0])
        }) {
            goal.key = "placeholder".into();
            goal.title = "Inventory product brief candidates before drafting".into();
            goal.business_outcome = "Placeholder".into();
            goal.criteria = vec!["List outcomes".into(), "Merge shared outcomes".into()];
        }

        assert!(
            validate_provider_goal_drafts(&goals, "Single-user local analysis utility").is_err()
        );
    }

    #[test]
    fn duplicate_provider_keys_are_repaired_without_another_model_call() {
        let mut goals = complete_generated_goal_set();
        goals[1].key = goals[0].key.clone();
        assert!(repair_duplicate_provider_keys(&mut goals));
        assert_ne!(goals[0].key, goals[1].key);
        assert!(goals[1].key.starts_with("generated-"));
        validate_provider_goal_drafts(&goals, "B2B SaaS for organizations").unwrap();
    }

    #[test]
    fn baseline_repair_adds_material_goals_when_every_existing_goal_is_full() {
        let mut goals = complete_generated_goal_set();
        for goal in &mut goals {
            while goal.criteria.len() < 6 {
                goal.criteria.push(format!(
                    "A version-controlled operational record preserves material criterion {}",
                    goal.criteria.len() + 1
                ));
            }
        }
        goals[5].criteria[4] = "Release documentation is current".into();
        goals[8].criteria[2] = "Webhook delivery is documented".into();

        assert!(cap_provider_goal_set(&mut goals));
        assert!(repair_required_engineering_baseline(
            &mut goals,
            "B2B SaaS with webhook delivery"
        ));
        assert_eq!(goals.len(), 9);
        validate_goal_drafts(&goals, "B2B SaaS with webhook delivery").unwrap();
        assert!(goals.iter().any(|goal| {
            goal.title == "Software supply-chain risk stays within policy"
                && goal
                    .criteria
                    .iter()
                    .any(|criterion| criterion.contains("Dependency licenses"))
        }));
        assert!(
            goals
                .iter()
                .any(|goal| goal.criteria.iter().any(|criterion| {
                    criterion.contains("signed payloads")
                        && criterion.contains("bounded retries")
                        && criterion.contains("replay controls")
                        && criterion.contains("dead-letter")
                }))
        );
    }

    #[test]
    fn regeneration_reuses_existing_logical_goal_ids() {
        let mut goals = complete_generated_goal_set();
        goals[0].key = "new-provider-key".into();
        let existing = ExistingGoalIdentity {
            goal_id: "manual-customer-outcome".into(),
            key: "durable-customer-outcome".into(),
            title: goals[0].title.clone(),
            business_outcome: goals[0].business_outcome.clone(),
        };
        reconcile_goal_identities(&mut goals, &[existing]).unwrap();
        assert_eq!(goals[0].key, "durable-customer-outcome");
        assert_eq!(goals[0].goal_id.as_deref(), Some("manual-customer-outcome"));
    }

    #[test]
    fn regeneration_rekeys_a_changed_outcome_instead_of_corrupting_history() {
        let mut goals = complete_generated_goal_set();
        goals[0].key = "durable-customer-outcome".into();
        goals[0].goal_id = Some("provider-supplied-id".into());
        let existing = ExistingGoalIdentity {
            goal_id: "existing-goal-id".into(),
            key: "durable-customer-outcome".into(),
            title: "A different durable goal".into(),
            business_outcome: "A different measurable outcome".into(),
        };
        reconcile_goal_identities(&mut goals, &[existing]).unwrap();
        assert!(goals[0].key.starts_with("generated-"));
        assert_ne!(goals[0].key, "durable-customer-outcome");
        assert_eq!(goals[0].goal_id, None);
    }

    #[test]
    fn catalog_templates_pass_the_tactical_and_placeholder_validators_as_authored() {
        for template in GOAL_TEMPLATES {
            let goal = GoalDraft {
                goal_id: None,
                key: template.key.into(),
                title: template.title.into(),
                business_outcome: template.business_outcome.into(),
                priority: template.priority,
                criteria: template
                    .criteria
                    .iter()
                    .map(|criterion| (*criterion).to_string())
                    .collect(),
                rubric_dimensions: vec![template.group.into()],
                grounding_fact_ids: Vec::new(),
            };
            assert!(
                !provider_goal_is_non_substantive(&goal),
                "catalog template {} reads as a placeholder",
                template.key
            );
            assert!(
                valid_goal_key(&goal.key),
                "catalog template key is invalid: {}",
                template.key
            );
            if template.group == BUSINESS_GROUP {
                assert!(
                    !business_goal_is_tactical(&goal, false),
                    "catalog template {} would be consolidated away as tactical",
                    template.key
                );
            }
        }
    }

    #[test]
    fn catalog_repair_criteria_satisfy_their_own_coverage_terms() {
        for family in COVERAGE_FAMILIES {
            let Some(repair) = &family.repair else {
                continue;
            };
            for terms in family.coverage {
                assert!(
                    statement_affirms_coverage(repair.criterion, terms),
                    "repair criterion for {} does not satisfy its own coverage terms {terms:?}",
                    family.label
                );
            }
        }
    }

    #[test]
    fn worst_case_sparse_provider_set_repairs_within_nine_goals() {
        // Six full goals with zero engineering coverage, on a brief that
        // trips every conditional family: the reserved repair slots must be
        // enough to restore the entire baseline without another model call.
        let brief = "Enterprise B2B SaaS for customer organizations with ERP integrations, webhooks, and high-volume performance and capacity demands";
        let mut goals = vec![
            generated_goal(
                "Business & product",
                "Customer retention outcome one",
                &[
                    "Version-controlled retention instrumentation records progress against the quarterly target",
                ],
            ),
            generated_goal(
                "Business & product",
                "Customer retention outcome two",
                &[
                    "Version-controlled adoption instrumentation records progress against the quarterly target",
                ],
            ),
            generated_goal(
                "Architecture & platform",
                "Platform outcome one",
                &["Automated platform contract tests keep the outcome durable"],
            ),
            generated_goal(
                "Architecture & platform",
                "Platform outcome two",
                &["A version-controlled platform record reviews the durable outcome each quarter"],
            ),
            generated_goal(
                "Operations & reliability",
                "Operational outcome one",
                &["Automated operations tests keep the outcome durable"],
            ),
            generated_goal(
                "Operations & reliability",
                "Operational outcome two",
                &[
                    "A version-controlled operations record reviews the durable outcome each quarter",
                ],
            ),
        ];
        for goal in &mut goals {
            while goal.criteria.len() < 6 {
                goal.criteria.push(format!(
                    "A version-controlled operational record preserves durable criterion {}",
                    goal.criteria.len() + 1
                ));
            }
        }
        assert!(repair_required_engineering_baseline(&mut goals, brief));
        assert!(
            goals.len() <= 9,
            "repair overflowed the nine-goal limit: {}",
            goals.len()
        );
        validate_goal_drafts(&goals, brief).unwrap();
    }

    #[test]
    fn provider_titles_that_echo_the_archetype_menu_are_rejected_for_revision() {
        let mut goals = complete_generated_goal_set();
        validate_provider_goal_drafts(&goals, "B2B SaaS for organizations").unwrap();
        goals[0].title = "Product metrics instrumentation".into();
        let error = validate_provider_goal_drafts(&goals, "B2B SaaS for organizations")
            .unwrap_err()
            .to_string();
        assert!(error.contains("echoing the archetype menu"), "{error}");

        // Deterministic repair goals must never trip the echo check: their
        // titles are full goal statements, not menu topics.
        let mut sparse = complete_generated_goal_set();
        for index in [3, 5, 6, 7, 8] {
            sparse[index].criteria = vec![
                "A version-controlled operational record names the executive owner".into(),
                "A repository dashboard configuration defines the quarterly measure".into(),
            ];
        }
        repair_required_engineering_baseline(&mut sparse, "B2B SaaS with webhook delivery");
        validate_provider_goal_drafts(&sparse, "B2B SaaS with webhook delivery").unwrap();
    }

    #[test]
    fn stored_goal_sets_approved_before_the_metrics_family_still_validate() {
        let mut legacy = complete_generated_goal_set();
        let host = legacy
            .iter_mut()
            .find(|goal| goal.title == "New customers reach value quickly")
            .unwrap();
        host.criteria.retain(|item| !item.contains("usage metrics"));
        // Stored-set validation (the scan and edit gates) must keep accepting
        // sets approved before the metrics family existed...
        validate_goal_drafts(&legacy, "B2B SaaS for organizations").unwrap();
        // ...while freshly generated drafts still demand it.
        let error = validate_provider_goal_drafts(&legacy, "B2B SaaS for organizations")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("product usage metrics instrumentation"),
            "{error}"
        );
    }

    #[test]
    fn stored_goal_sets_approved_before_the_ai_assisted_verification_family_still_validate() {
        let mut legacy = complete_generated_goal_set();
        let host = legacy
            .iter_mut()
            .find(|goal| goal.title == "Customer data stays isolated")
            .unwrap();
        host.criteria
            .retain(|item| !item.contains("agent contract"));
        // Stored-set validation (the scan and edit gates) must keep accepting
        // sets approved before the family existed...
        validate_goal_drafts(&legacy, "B2B SaaS for organizations").unwrap();
        // ...while freshly generated drafts still demand it in an
        // Architecture & platform goal.
        let error = validate_provider_goal_drafts(&legacy, "B2B SaaS for organizations")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("AI-assisted engineering verification"),
            "{error}"
        );
        assert!(error.contains("[Architecture & platform]"), "{error}");
        assert!(error.contains("verification harness"), "{error}");
    }

    #[test]
    fn baseline_repair_restores_ai_assisted_verification_coverage() {
        let mut goals = complete_generated_goal_set();
        goals[3]
            .criteria
            .retain(|item| !item.contains("agent contract"));
        assert!(repair_required_engineering_baseline(
            &mut goals,
            "B2B SaaS for organizations"
        ));
        validate_provider_goal_drafts(&goals, "B2B SaaS for organizations").unwrap();
        assert!(goals.iter().any(|goal| {
            goal.rubric_dimensions[0] == GOAL_GROUPS[1]
                && goal
                    .criteria
                    .iter()
                    .any(|criterion| criterion.contains("verification harness"))
        }));
        assert!(!repair_required_engineering_baseline(
            &mut goals,
            "B2B SaaS for organizations"
        ));
    }

    #[test]
    fn reconciled_existing_titles_matching_menu_topics_are_not_rejected() {
        // A stored goal legitimately titled like a menu topic is fed back
        // through regeneration as an existing identity; reusing it is
        // instructed behavior, so only unreconciled echoes are rejected.
        let mut goals = complete_generated_goal_set();
        goals[3].title = "Tenant isolation".into();
        goals[3].goal_id = Some("existing-tenant-goal".into());
        validate_provider_goal_drafts(&goals, "B2B SaaS for organizations").unwrap();
    }

    #[test]
    fn provider_goals_titled_like_fallback_goals_do_not_break_repair() {
        // A full provider goal already titled exactly like the operations
        // fallback, with observability coverage missing everywhere: repair
        // must create the fallback under a uniquified title instead of
        // producing a duplicate-title validation failure.
        let mut goals = complete_generated_goal_set();
        for index in [6, 7, 8] {
            goals[index].criteria = vec![
                "A version-controlled operations record names the executive owner".into(),
                "A repository dashboard configuration defines the quarterly measure".into(),
            ];
            while goals[index].criteria.len() < 6 {
                let next = goals[index].criteria.len() + 1;
                goals[index].criteria.push(format!(
                    "A version-controlled operational record preserves durable criterion {next}"
                ));
            }
        }
        goals[6].title = "Operations stay observable and recoverable".into();
        assert!(cap_provider_goal_set(&mut goals));
        assert!(repair_required_engineering_baseline(
            &mut goals,
            "B2B SaaS for organizations"
        ));
        validate_goal_drafts(&goals, "B2B SaaS for organizations").unwrap();
        assert!(
            goals
                .iter()
                .any(|goal| goal.title == "Operations stay observable and recoverable (2)"),
            "colliding fallback title was not uniquified"
        );
    }
}
