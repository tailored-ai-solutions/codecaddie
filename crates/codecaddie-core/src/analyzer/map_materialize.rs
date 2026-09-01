//! Codebase-map materialization: turning raw survey and deep-dive provider
//! output into a validated `CodebaseMap` with bound immutable evidence,
//! resolved component references, hard caps, and the same credential and
//! source-excerpt defenses reports get. Every dropped item is named in the
//! map's warnings — provider work never vanishes silently.

use super::analysis_contract::{RawMapDeepDive, RawMapSurvey};
use super::report_materialize::bind_evidence;
use crate::repository::LocalRepository;
use codecaddie_domain::{
    CodebaseMap, Component, ComponentRelationship, DataFlow, DataFlowStep, EntryPoint, EvidenceRef,
    KeyInterface, MAP_SCHEMA_VERSION, MapConcern, MapOverview, ReportOrigin, TechnologyObservation,
    component_id,
};
use codecaddie_domain::{
    MAX_MAP_COMPONENTS, MAX_MAP_CONCERNS_PER_COMPONENT, MAX_MAP_DATA_FLOWS, MAX_MAP_ENTRY_POINTS,
    MAX_MAP_EVIDENCE_PER_ITEM, MAX_MAP_FLOW_STEPS, MAX_MAP_INTERFACES_PER_COMPONENT,
    MAX_MAP_RELATIONSHIPS, MAX_MAP_TECHNOLOGIES,
};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;

/// How map materialization treats narrative that matches repository source.
/// Maps have no `Skip`: every materialized map is persisted or returned.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapNarrativePolicy {
    /// Replace source-matching narrative fields with neutral wording and
    /// mark the map partial: the policy for scan-generated maps.
    Redact,
    /// Fail closed: the policy for agent-submitted maps, which can simply
    /// resubmit.
    Reject,
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn contains_credential_marker(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
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
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

pub(super) fn clipped(value: &str, maximum: usize) -> String {
    let line = one_line(value);
    if line.chars().count() <= maximum {
        return line;
    }
    // A silent mid-word cut reads as data loss; break at the last word
    // boundary inside the budget and say the text continues.
    let cut: String = line.chars().take(maximum.saturating_sub(1)).collect();
    let trimmed = match cut.rfind(' ') {
        Some(space) if space > maximum / 2 => &cut[..space],
        _ => cut.as_str(),
    };
    format!("{}…", trimmed.trim_end())
}

fn normalized_name(value: &str) -> String {
    value.trim().to_lowercase()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn materialize_codebase_map(
    map_id: String,
    repositories: &[(LocalRepository, String)],
    provider: String,
    provider_version: String,
    origin: ReportOrigin,
    survey: RawMapSurvey,
    deep_dives: Vec<RawMapDeepDive>,
    mut warnings: Vec<String>,
    mut partial: bool,
    supersedes: Option<String>,
    policy: MapNarrativePolicy,
    screened_out: Option<&mut Vec<(usize, String)>>,
) -> anyhow::Result<CodebaseMap> {
    if repositories.is_empty() {
        anyhow::bail!("map materialization requires a frozen repository");
    }
    let known_repositories: BTreeSet<&str> = repositories
        .iter()
        .map(|(repository, _)| repository.id.as_str())
        .collect();

    // Component skeleton: content-addressed ids, bound anchors, no
    // duplicates. The name index resolves every later by-name reference.
    let mut components: Vec<Component> = Vec::new();
    let mut names_to_ids: BTreeMap<String, String> = BTreeMap::new();
    for raw in survey.components.into_iter().take(MAX_MAP_COMPONENTS) {
        let name = clipped(&raw.name, 120);
        if name.is_empty() {
            warnings.push("A surveyed component had no name and was dropped.".into());
            continue;
        }
        if !known_repositories.contains(raw.repository_id.as_str()) {
            warnings.push(format!(
                "Component \"{name}\" was dropped: it named an unknown repositoryId."
            ));
            continue;
        }
        let id = component_id(&raw.repository_id, &name);
        if names_to_ids.contains_key(&normalized_name(&name)) {
            warnings.push(format!(
                "Component \"{name}\" was dropped: a component with that name already exists."
            ));
            continue;
        }
        let binding = bind_evidence(repositories, raw.evidence);
        if binding.evidence.is_empty() {
            let reason = binding
                .first_failure()
                .unwrap_or_else(|| "no citation was submitted".into());
            warnings.push(format!("Component \"{name}\" was dropped: {reason}."));
            continue;
        }
        let mut root_paths = raw
            .root_paths
            .into_iter()
            .map(|path| clipped(&path, 240))
            .filter(|path| {
                !path.is_empty()
                    && !path.starts_with('/')
                    && !path.split('/').any(|part| part == "..")
            })
            .collect::<Vec<_>>();
        root_paths.truncate(8);
        if root_paths.is_empty() {
            warnings.push(format!(
                "Component \"{name}\" was dropped: it had no valid repository-relative root paths."
            ));
            continue;
        }
        names_to_ids.insert(normalized_name(&name), id.clone());
        components.push(Component {
            id,
            name,
            kind: raw.kind,
            repository_id: raw.repository_id,
            root_paths,
            responsibility: clipped(&raw.responsibility, 480),
            key_interfaces: Vec::new(),
            concerns: Vec::new(),
            evidence: binding.evidence,
        });
    }
    if components.is_empty() {
        anyhow::bail!("the survey produced no component with bindable evidence");
    }

    let mut technologies = Vec::new();
    for raw in survey
        .overview
        .technologies
        .into_iter()
        .take(MAX_MAP_TECHNOLOGIES)
    {
        let binding = bind_evidence(repositories, raw.evidence);
        if binding.evidence.is_empty() {
            warnings.push(format!(
                "Technology \"{}\" was dropped: its citation could not be bound.",
                clipped(&raw.name, 60)
            ));
            continue;
        }
        technologies.push(TechnologyObservation {
            name: clipped(&raw.name, 60),
            role: clipped(&raw.role, 240),
            evidence: binding.evidence,
        });
    }

    let mut entry_points = Vec::new();
    for raw in survey.entry_points.into_iter().take(MAX_MAP_ENTRY_POINTS) {
        let name = clipped(&raw.name, 120);
        let Some(component) = names_to_ids.get(&normalized_name(&raw.component_name)) else {
            warnings.push(format!(
                "Entry point \"{name}\" was dropped: it named an unknown component."
            ));
            continue;
        };
        let binding = bind_evidence(repositories, raw.evidence);
        if binding.evidence.is_empty() {
            warnings.push(format!(
                "Entry point \"{name}\" was dropped: its citation could not be bound."
            ));
            continue;
        }
        entry_points.push(EntryPoint {
            id: format!("entry-{}", entry_points.len() + 1),
            name,
            kind: raw.kind,
            component_id: component.clone(),
            evidence: binding.evidence,
        });
    }

    // Deep-dive merge: interfaces, concerns, extra anchors, relationships,
    // and flows, all resolved through the surveyed name index.
    let mut relationships: Vec<ComponentRelationship> = Vec::new();
    let mut seen_relationships = BTreeSet::new();
    let mut data_flows: Vec<DataFlow> = Vec::new();
    for deep_dive in deep_dives {
        for detail in deep_dive.components {
            let Some(id) = names_to_ids.get(&normalized_name(&detail.name)) else {
                warnings.push(format!(
                    "A deep-dive detailed unknown component \"{}\"; it was dropped.",
                    clipped(&detail.name, 80)
                ));
                continue;
            };
            let component = components
                .iter_mut()
                .find(|component| &component.id == id)
                .expect("name index only holds surveyed components");
            for interface in detail.key_interfaces {
                if component.key_interfaces.len() >= MAX_MAP_INTERFACES_PER_COMPONENT {
                    break;
                }
                let binding = bind_evidence(repositories, interface.evidence);
                if binding.evidence.is_empty() {
                    warnings.push(format!(
                        "Interface \"{}\" of \"{}\" was dropped: its citation could not be bound.",
                        clipped(&interface.name, 80),
                        component.name
                    ));
                    continue;
                }
                component.key_interfaces.push(KeyInterface {
                    name: clipped(&interface.name, 120),
                    description: clipped(&interface.description, 240),
                    evidence: binding.evidence,
                });
            }
            for concern in detail.concerns {
                if component.concerns.len() >= MAX_MAP_CONCERNS_PER_COMPONENT {
                    break;
                }
                let binding = bind_evidence(repositories, concern.evidence);
                if binding.evidence.is_empty() {
                    warnings.push(format!(
                        "A concern of \"{}\" was dropped: its citation could not be bound.",
                        component.name
                    ));
                    continue;
                }
                component.concerns.push(MapConcern {
                    summary: clipped(&concern.summary, 240),
                    evidence: binding.evidence,
                });
            }
            let binding = bind_evidence(repositories, detail.additional_evidence);
            for reference in binding.evidence {
                if component.evidence.len() >= MAX_MAP_EVIDENCE_PER_ITEM {
                    break;
                }
                if !component.evidence.contains(&reference) {
                    component.evidence.push(reference);
                }
            }
        }
        for raw in deep_dive.relationships {
            if relationships.len() >= MAX_MAP_RELATIONSHIPS {
                break;
            }
            let (Some(from), Some(to)) = (
                names_to_ids.get(&normalized_name(&raw.from_component)),
                names_to_ids.get(&normalized_name(&raw.to_component)),
            ) else {
                warnings.push(format!(
                    "Relationship \"{} → {}\" was dropped: it named an unknown component.",
                    clipped(&raw.from_component, 60),
                    clipped(&raw.to_component, 60)
                ));
                continue;
            };
            if !seen_relationships.insert((from.clone(), to.clone(), raw.kind)) {
                continue;
            }
            let binding = bind_evidence(repositories, raw.evidence);
            if binding.evidence.is_empty() {
                warnings.push(format!(
                    "Relationship \"{} → {}\" was dropped: its citation could not be bound.",
                    clipped(&raw.from_component, 60),
                    clipped(&raw.to_component, 60)
                ));
                continue;
            }
            relationships.push(ComponentRelationship {
                id: format!("relationship-{}", relationships.len() + 1),
                from_component: from.clone(),
                to_component: to.clone(),
                kind: raw.kind,
                description: clipped(&raw.description, 240),
                evidence: binding.evidence,
            });
        }
        for raw in deep_dive.data_flows {
            if data_flows.len() >= MAX_MAP_DATA_FLOWS {
                break;
            }
            let name = clipped(&raw.name, 120);
            let mut steps = Vec::new();
            let mut bound_step_evidence = 0_usize;
            for step in raw.steps.into_iter().take(MAX_MAP_FLOW_STEPS) {
                let Some(component) = names_to_ids.get(&normalized_name(&step.component_name))
                else {
                    continue;
                };
                let binding = bind_evidence(repositories, step.evidence);
                bound_step_evidence += binding.evidence.len();
                steps.push(DataFlowStep {
                    component_id: component.clone(),
                    action: clipped(&step.action, 200),
                    evidence: binding.evidence,
                });
            }
            if steps.len() < 2 || bound_step_evidence == 0 {
                warnings.push(format!(
                    "Data flow \"{name}\" was dropped: it kept fewer than two resolvable steps or no bound evidence."
                ));
                continue;
            }
            data_flows.push(DataFlow {
                id: format!("flow-{}", data_flows.len() + 1),
                name,
                description: clipped(&raw.description, 480),
                steps,
            });
        }
    }

    let mut map = CodebaseMap {
        id: map_id,
        schema_version: MAP_SCHEMA_VERSION,
        generated_at: OffsetDateTime::now_utc(),
        repositories: repositories
            .iter()
            .map(|(repository, commit)| codecaddie_domain::FrozenRepository {
                repository_id: repository.id.clone(),
                commit_sha: commit.clone(),
            })
            .collect(),
        provider,
        provider_version: clipped(&provider_version, 120),
        origin,
        overview: MapOverview {
            system_summary: clipped(&survey.overview.system_summary, 700),
            architecture_style: clipped(&survey.overview.architecture_style, 240),
            technologies,
        },
        components,
        relationships,
        data_flows,
        entry_points,
        partial,
        analysis_warnings: Vec::new(),
        supersedes,
    };

    screen_map_narrative(
        &mut map,
        repositories,
        policy,
        &mut warnings,
        &mut partial,
        screened_out,
    )?;
    map.partial = partial;
    map.analysis_warnings = warnings;
    Ok(map)
}

/// Collects every derived-prose field of the map, in a stable order, with a
/// closure that can replace it.
fn map_narrative_fields(map: &CodebaseMap) -> Vec<String> {
    let mut fields = vec![
        map.overview.system_summary.clone(),
        map.overview.architecture_style.clone(),
    ];
    for technology in &map.overview.technologies {
        fields.push(technology.name.clone());
        fields.push(technology.role.clone());
    }
    for component in &map.components {
        fields.push(component.name.clone());
        fields.push(component.responsibility.clone());
        for interface in &component.key_interfaces {
            fields.push(interface.name.clone());
            fields.push(interface.description.clone());
        }
        for concern in &component.concerns {
            fields.push(concern.summary.clone());
        }
    }
    for relationship in &map.relationships {
        fields.push(relationship.description.clone());
    }
    for flow in &map.data_flows {
        fields.push(flow.name.clone());
        fields.push(flow.description.clone());
        for step in &flow.steps {
            fields.push(step.action.clone());
        }
    }
    for entry_point in &map.entry_points {
        fields.push(entry_point.name.clone());
    }
    fields
}

/// Deterministic neutral text for every narrative field position, derived
/// from the map's VALIDATED structure (kinds, roots, endpoints, counts) so a
/// screened field still tells the reader something true instead of a bare
/// placeholder. Order must mirror `map_narrative_fields` exactly.
fn neutral_map_replacements(map: &CodebaseMap) -> Vec<String> {
    fn kind_word(kind: codecaddie_domain::map::ComponentKind) -> &'static str {
        use codecaddie_domain::map::ComponentKind;
        match kind {
            ComponentKind::Service => "service",
            ComponentKind::Library => "library",
            ComponentKind::UiSurface => "UI surface",
            ComponentKind::DataStore => "data store",
            ComponentKind::Pipeline => "pipeline",
            ComponentKind::Infrastructure => "infrastructure",
            ComponentKind::ExternalInterface => "external interface",
            ComponentKind::TestSuite => "test suite",
            ComponentKind::BuildTooling => "build tooling",
        }
    }
    const WITHHELD: &str = "Text withheld — it matched repository source.";
    let mut fields = vec![
        format!(
            "Narrative withheld because it matched repository source. The validated structure remains: {} components, {} relationships, {} data flows, and {} entry points, each backed by verified references below.",
            map.components.len(),
            map.relationships.len(),
            map.data_flows.len(),
            map.entry_points.len()
        ),
        "Withheld — see the component groups and relationships below.".to_string(),
    ];
    for _technology in &map.overview.technologies {
        fields.push("(name withheld)".to_string());
        fields.push(WITHHELD.to_string());
    }
    for component in &map.components {
        fields.push("(component name withheld)".to_string());
        fields.push(format!(
            "Description withheld (matched repository source). Validated: a {} rooted at {} with {} verified reference(s).",
            kind_word(component.kind),
            if component.root_paths.is_empty() { "the repository root".to_string() } else { component.root_paths.join(", ") },
            component.evidence.len()
        ));
        for _interface in &component.key_interfaces {
            fields.push("(interface name withheld)".to_string());
            fields.push(WITHHELD.to_string());
        }
        for concern in &component.concerns {
            fields.push(format!(
                "Concern text withheld (matched repository source); {} verified reference(s) locate it.",
                concern.evidence.len()
            ));
        }
    }
    for relationship in &map.relationships {
        fields.push(format!(
            "Description withheld (matched repository source); the {:?} connection itself is validated.",
            relationship.kind
        ));
    }
    for flow in &map.data_flows {
        fields.push("(flow name withheld)".to_string());
        fields.push(WITHHELD.to_string());
        for _step in &flow.steps {
            fields.push("(step text withheld — matched repository source)".to_string());
        }
    }
    for _entry_point in &map.entry_points {
        fields.push("(entry point name withheld)".to_string());
    }
    fields
}

pub(super) fn replace_map_narrative_field(map: &mut CodebaseMap, index: usize, replacement: &str) {
    let mut cursor = 0_usize;
    let mut apply = |field: &mut String| {
        if cursor == index {
            *field = replacement.to_string();
        }
        cursor += 1;
    };
    apply(&mut map.overview.system_summary);
    apply(&mut map.overview.architecture_style);
    for technology in &mut map.overview.technologies {
        apply(&mut technology.name);
        apply(&mut technology.role);
    }
    for component in &mut map.components {
        apply(&mut component.name);
        apply(&mut component.responsibility);
        for interface in &mut component.key_interfaces {
            apply(&mut interface.name);
            apply(&mut interface.description);
        }
        for concern in &mut component.concerns {
            apply(&mut concern.summary);
        }
    }
    for relationship in &mut map.relationships {
        apply(&mut relationship.description);
    }
    for flow in &mut map.data_flows {
        apply(&mut flow.name);
        apply(&mut flow.description);
        for step in &mut flow.steps {
            apply(&mut step.action);
        }
    }
    for entry_point in &mut map.entry_points {
        apply(&mut entry_point.name);
    }
}

/// The same defenses reports get, at map-field granularity: credential
/// markers always fail closed; source-matching fields are replaced under
/// `Redact` and fail closed under `Reject`.
fn screen_map_narrative(
    map: &mut CodebaseMap,
    repositories: &[(LocalRepository, String)],
    policy: MapNarrativePolicy,
    warnings: &mut Vec<String>,
    partial: &mut bool,
    screened_out: Option<&mut Vec<(usize, String)>>,
) -> anyhow::Result<()> {
    let fields = map_narrative_fields(map);
    let joined = fields.join("\n");
    if contains_credential_marker(&joined) {
        anyhow::bail!("map narrative contained credential-shaped text");
    }

    let mut violations = BTreeSet::new();
    for (repository, commit) in repositories {
        violations.extend(repository.narrative_fields_matching_source(commit, &fields)?);
    }
    let mut checked = BTreeSet::new();
    let evidence = map
        .components
        .iter()
        .flat_map(|component| {
            component
                .evidence
                .iter()
                .chain(
                    component
                        .key_interfaces
                        .iter()
                        .flat_map(|interface| &interface.evidence),
                )
                .chain(
                    component
                        .concerns
                        .iter()
                        .flat_map(|concern| &concern.evidence),
                )
        })
        .chain(
            map.overview
                .technologies
                .iter()
                .flat_map(|technology| &technology.evidence),
        )
        .chain(
            map.relationships
                .iter()
                .flat_map(|relationship| &relationship.evidence),
        )
        .chain(
            map.data_flows
                .iter()
                .flat_map(|flow| flow.steps.iter().flat_map(|step| &step.evidence)),
        )
        .chain(map.entry_points.iter().flat_map(|entry| &entry.evidence))
        .cloned()
        .collect::<Vec<EvidenceRef>>();
    for reference in &evidence {
        let coordinate = (
            reference.repository_id.clone(),
            reference.blob_oid.clone(),
            reference.start_line,
            reference.end_line,
        );
        if !checked.insert(coordinate) {
            continue;
        }
        let Some((repository, _)) = repositories
            .iter()
            .find(|(repository, _)| repository.id == reference.repository_id)
        else {
            continue;
        };
        let excerpt = repository.read_evidence(reference)?;
        let excerpt = excerpt.trim();
        for (index, field) in fields.iter().enumerate() {
            if excerpt.len() >= 16 && field.contains(excerpt) {
                violations.insert(index);
                continue;
            }
            if excerpt
                .lines()
                .map(str::trim)
                .any(|line| line.len() >= 24 && field.contains(line))
            {
                violations.insert(index);
            }
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    if policy == MapNarrativePolicy::Reject {
        anyhow::bail!("map narrative matched repository source");
    }
    let count = violations.len();
    if let Some(out) = screened_out {
        for index in &violations {
            if let Some(original) = fields.get(*index) {
                out.push((*index, original.clone()));
            }
        }
    }
    let replacements = neutral_map_replacements(map);
    for index in violations {
        let replacement = replacements
            .get(index)
            .map(String::as_str)
            .unwrap_or("Text withheld — it matched repository source.")
            .to_string();
        replace_map_narrative_field(map, index, &replacement);
    }
    *partial = true;
    warnings.push(format!(
        "Map narrative matched repository source in {count} field(s) and was replaced with neutral structural wording; validated structure and evidence coordinates were retained."
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::analysis_contract::{
        RawEvidence, RawMapComponent, RawMapComponentDetail, RawMapConcern, RawMapDataFlow,
        RawMapEntryPoint, RawMapFlowStep, RawMapInterface, RawMapOverview, RawMapRelationship,
        RawMapTechnology,
    };
    use crate::analyzer::test_support::fixture;
    use codecaddie_domain::{ComponentKind, EntryPointKind, EvidenceKind, RelationshipKind};

    fn evidence(path: &str, start: u32, end: u32) -> RawEvidence {
        RawEvidence {
            repository_id: "repo".into(),
            path: path.into(),
            start_line: start,
            end_line: end,
            kind: EvidenceKind::Implementation,
        }
    }

    fn survey() -> RawMapSurvey {
        RawMapSurvey {
            provider_version: "test".into(),
            overview: RawMapOverview {
                system_summary: "A tenant-scoped billing service.".into(),
                architecture_style: "Modular library".into(),
                technologies: vec![
                    RawMapTechnology {
                        name: "Rust".into(),
                        role: "Core implementation language".into(),
                        evidence: vec![evidence("tenant.rs", 1, 1)],
                    },
                    RawMapTechnology {
                        name: "Ghost".into(),
                        role: "Names a missing file".into(),
                        evidence: vec![evidence("missing.rs", 1, 1)],
                    },
                ],
            },
            components: vec![
                RawMapComponent {
                    name: "Billing".into(),
                    kind: ComponentKind::Service,
                    repository_id: "repo".into(),
                    root_paths: vec!["tenant.rs".into()],
                    responsibility: "Scopes every invoice read to one tenant.".into(),
                    evidence: vec![evidence("tenant.rs", 1, 3)],
                },
                RawMapComponent {
                    name: "Phantom".into(),
                    kind: ComponentKind::Library,
                    repository_id: "repo".into(),
                    root_paths: vec!["phantom/".into()],
                    responsibility: "Cites a file that does not exist.".into(),
                    evidence: vec![evidence("missing.rs", 1, 1)],
                },
            ],
            entry_points: vec![
                RawMapEntryPoint {
                    name: "invoice".into(),
                    kind: EntryPointKind::Cli,
                    component_name: "billing".into(),
                    evidence: vec![evidence("tenant.rs", 1, 1)],
                },
                RawMapEntryPoint {
                    name: "orphan".into(),
                    kind: EntryPointKind::Cli,
                    component_name: "unknown-component".into(),
                    evidence: vec![evidence("tenant.rs", 1, 1)],
                },
            ],
        }
    }

    fn deep_dive() -> RawMapDeepDive {
        RawMapDeepDive {
            components: vec![RawMapComponentDetail {
                name: "Billing".into(),
                key_interfaces: vec![RawMapInterface {
                    name: "invoice lookup".into(),
                    description: "Tenant-scoped invoice read path.".into(),
                    evidence: vec![evidence("tenant.rs", 1, 2)],
                }],
                concerns: vec![RawMapConcern {
                    summary: "No cross-tenant denial test exists yet.".into(),
                    evidence: vec![evidence("tenant.rs", 2, 2)],
                }],
                additional_evidence: vec![],
            }],
            relationships: vec![RawMapRelationship {
                from_component: "Billing".into(),
                to_component: "Phantom".into(),
                kind: RelationshipKind::Calls,
                description: "Billing calls a dropped component.".into(),
                evidence: vec![evidence("tenant.rs", 1, 1)],
            }],
            data_flows: vec![RawMapDataFlow {
                name: "Invoice flow".into(),
                description: "One read path through billing.".into(),
                steps: vec![
                    RawMapFlowStep {
                        component_name: "Billing".into(),
                        action: "Receives the invoice request".into(),
                        evidence: vec![evidence("tenant.rs", 1, 1)],
                    },
                    RawMapFlowStep {
                        component_name: "Billing".into(),
                        action: "Scopes the lookup to the tenant".into(),
                        evidence: vec![],
                    },
                ],
            }],
        }
    }

    #[test]
    fn maps_bind_evidence_resolve_names_and_name_every_drop() {
        let (_directory, repository, commit, _goal) = fixture();
        let map = materialize_codebase_map(
            "map-test".into(),
            &[(repository, commit.clone())],
            "codex".into(),
            "test".into(),
            ReportOrigin::Scan,
            survey(),
            vec![deep_dive()],
            Vec::new(),
            false,
            None,
            MapNarrativePolicy::Redact,
            None,
        )
        .unwrap();

        // The bindable component survives with its deep-dive details; the
        // phantom component, its relationship, the orphan entry point, and
        // the unbindable technology are all dropped by name.
        assert_eq!(map.components.len(), 1);
        let billing = &map.components[0];
        assert_eq!(billing.name, "Billing");
        assert!(billing.id.starts_with("component-"));
        assert_eq!(billing.key_interfaces.len(), 1);
        assert_eq!(billing.concerns.len(), 1);
        assert_eq!(billing.evidence[0].commit_sha, commit);
        assert_eq!(map.overview.technologies.len(), 1);
        assert_eq!(map.entry_points.len(), 1);
        assert_eq!(map.entry_points[0].component_id, billing.id);
        assert!(map.relationships.is_empty());
        assert_eq!(map.data_flows.len(), 1);
        assert_eq!(map.data_flows[0].steps.len(), 2);
        for expected in [
            "Component \"Phantom\" was dropped",
            "Technology \"Ghost\" was dropped",
            "Entry point \"orphan\" was dropped",
            "Relationship \"Billing → Phantom\" was dropped",
        ] {
            assert!(
                map.analysis_warnings
                    .iter()
                    .any(|warning| warning.contains(expected)),
                "missing warning: {expected}; got {:?}",
                map.analysis_warnings
            );
        }
        // The map is content-addressable and descriptor-consistent.
        let descriptor = codecaddie_domain::CodebaseMapDescriptor::for_map(&map).unwrap();
        assert_eq!(descriptor.component_count, 1);
        assert_eq!(descriptor.content_hash.len(), 64);
        // No serialized source text.
        let encoded = serde_json::to_string(&map).unwrap();
        assert!(!encoded.contains("scoped(tenant)"));
    }

    #[test]
    fn map_narrative_matching_source_is_redacted_or_rejected_by_policy() {
        let (_directory, repository, commit, _goal) = fixture();
        let mut quoting = survey();
        // The fixture repo contains this exact line in uncited.txt.
        quoting.components[0].responsibility = "internal reconciliation token stays local".into();
        let map = materialize_codebase_map(
            "map-redact".into(),
            &[(repository.clone(), commit.clone())],
            "codex".into(),
            "test".into(),
            ReportOrigin::Scan,
            quoting.clone(),
            vec![],
            Vec::new(),
            false,
            None,
            MapNarrativePolicy::Redact,
            None,
        )
        .unwrap();
        assert!(map.partial);
        // The neutral replacement is structural: it names the validated
        // kind, roots, and reference count instead of a bare placeholder.
        assert!(
            map.components[0]
                .responsibility
                .starts_with("Description withheld")
        );
        assert!(
            map.components[0]
                .responsibility
                .contains("verified reference")
        );
        assert!(
            map.analysis_warnings
                .iter()
                .any(|warning| warning.contains("matched repository source"))
        );

        let rejected = materialize_codebase_map(
            "map-reject".into(),
            &[(repository, commit)],
            "codex".into(),
            "test".into(),
            ReportOrigin::AgentSession,
            quoting,
            vec![],
            Vec::new(),
            false,
            None,
            MapNarrativePolicy::Reject,
            None,
        );
        assert!(rejected.is_err());
    }
}
