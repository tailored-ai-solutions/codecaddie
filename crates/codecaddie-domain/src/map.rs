//! The codebase architecture map: a typed, evidence-bound component graph
//! generated from a frozen repository set before goals are evaluated.
//! Every item carries immutable evidence coordinates and derived prose
//! only — never repository source text — so a map is legal in IPC, the
//! event ledger, and exports by construction.

use crate::{EvidenceRef, FrozenRepository, ReportOrigin, RepositoryId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type CodebaseMapId = String;
pub type ComponentId = String;

pub const MAP_SCHEMA_VERSION: u32 = 1;

pub const MAX_MAP_COMPONENTS: usize = 24;
pub const MAX_MAP_RELATIONSHIPS: usize = 48;
pub const MAX_MAP_DATA_FLOWS: usize = 8;
pub const MAX_MAP_FLOW_STEPS: usize = 10;
pub const MAX_MAP_ENTRY_POINTS: usize = 24;
pub const MAX_MAP_INTERFACES_PER_COMPONENT: usize = 6;
pub const MAX_MAP_CONCERNS_PER_COMPONENT: usize = 3;
pub const MAX_MAP_EVIDENCE_PER_ITEM: usize = 6;
pub const MAX_MAP_TECHNOLOGIES: usize = 16;

/// Content-addresses a component id from its repository and name so the id
/// is stable across regenerations of the same architecture.
pub fn component_id(repository_id: &str, name: &str) -> ComponentId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(repository_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(name.trim().to_lowercase().as_bytes());
    let digest = hasher.finalize().to_hex();
    format!("component-{}", &digest.as_str()[..20])
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebaseMap {
    pub id: CodebaseMapId,
    pub schema_version: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    /// The frozen coordinates this map describes; the invalidation key.
    pub repositories: Vec<FrozenRepository>,
    pub provider: String,
    pub provider_version: String,
    pub origin: ReportOrigin,
    pub overview: MapOverview,
    pub components: Vec<Component>,
    pub relationships: Vec<ComponentRelationship>,
    pub data_flows: Vec<DataFlow>,
    pub entry_points: Vec<EntryPoint>,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub analysis_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<CodebaseMapId>,
}

impl CodebaseMap {
    /// BLAKE3 of the canonical JSON encoding; the content address the
    /// descriptor pins and every reader re-verifies before use.
    pub fn content_hash(&self) -> Result<String, serde_json::Error> {
        Ok(blake3::hash(&serde_json::to_vec(self)?)
            .to_hex()
            .to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapOverview {
    pub system_summary: String,
    pub architecture_style: String,
    pub technologies: Vec<TechnologyObservation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TechnologyObservation {
    pub name: String,
    pub role: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Service,
    Library,
    UiSurface,
    DataStore,
    Pipeline,
    Infrastructure,
    ExternalInterface,
    TestSuite,
    BuildTooling,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: ComponentId,
    pub name: String,
    pub kind: ComponentKind,
    pub repository_id: RepositoryId,
    /// Repository-relative directory or file prefixes owned by this
    /// component — allowed report vocabulary, like every evidence path.
    pub root_paths: Vec<String>,
    pub responsibility: String,
    pub key_interfaces: Vec<KeyInterface>,
    pub concerns: Vec<MapConcern>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInterface {
    pub name: String,
    pub description: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapConcern {
    pub summary: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    Calls,
    Spawns,
    Reads,
    Writes,
    Validates,
    DependsOn,
    Builds,
    SerializesTo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentRelationship {
    pub id: String,
    pub from_component: ComponentId,
    pub to_component: ComponentId,
    pub kind: RelationshipKind,
    pub description: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFlow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps: Vec<DataFlowStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFlowStep {
    pub component_id: ComponentId,
    pub action: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryPointKind {
    Cli,
    IpcMethod,
    HttpRoute,
    UiScreen,
    McpTool,
    Scheduled,
    BuildTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPoint {
    pub id: String,
    pub name: String,
    pub kind: EntryPointKind,
    pub component_id: ComponentId,
    pub evidence: Vec<EvidenceRef>,
}

/// The slim, signed record of a map: everything the ledger needs for
/// provenance and invalidation, while the content-addressed body lives as
/// a prunable file beside the event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebaseMapDescriptor {
    pub map_id: CodebaseMapId,
    pub schema_version: u32,
    pub repositories: Vec<FrozenRepository>,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    pub provider: String,
    pub origin: ReportOrigin,
    /// BLAKE3 of the canonical map JSON body.
    pub content_hash: String,
    pub component_count: u32,
    #[serde(default)]
    pub partial: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<CodebaseMapId>,
}

impl CodebaseMapDescriptor {
    pub fn for_map(map: &CodebaseMap) -> Result<Self, serde_json::Error> {
        Ok(Self {
            map_id: map.id.clone(),
            schema_version: map.schema_version,
            repositories: map.repositories.clone(),
            generated_at: map.generated_at,
            provider: map.provider.clone(),
            origin: map.origin,
            content_hash: map.content_hash()?,
            component_count: map.components.len() as u32,
            partial: map.partial,
            supersedes: map.supersedes.clone(),
        })
    }

    /// Whether this map describes exactly the given frozen repository set.
    pub fn matches_repositories(&self, frozen: &[FrozenRepository]) -> bool {
        if self.repositories.len() != frozen.len() {
            return false;
        }
        let mut ours = self.repositories.clone();
        let mut theirs = frozen.to_vec();
        let key = |repository: &FrozenRepository| {
            (
                repository.repository_id.clone(),
                repository.commit_sha.clone(),
            )
        };
        ours.sort_by_key(key);
        theirs.sort_by_key(key);
        ours == theirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_ids_are_stable_and_name_insensitive_to_case() {
        let first = component_id("repo", "Core Service");
        let second = component_id("repo", "core service");
        assert_eq!(first, second);
        assert!(first.starts_with("component-"));
        assert_ne!(first, component_id("other", "Core Service"));
        assert_ne!(first, component_id("repo", "Other Service"));
    }

    #[test]
    fn descriptors_match_exact_frozen_sets_regardless_of_order() {
        let repository = |id: &str, sha: &str| FrozenRepository {
            repository_id: id.into(),
            commit_sha: sha.into(),
        };
        let map = CodebaseMap {
            id: "map-test".into(),
            schema_version: MAP_SCHEMA_VERSION,
            generated_at: OffsetDateTime::UNIX_EPOCH,
            repositories: vec![repository("a", "1"), repository("b", "2")],
            provider: "codex".into(),
            provider_version: "1".into(),
            origin: ReportOrigin::Scan,
            overview: MapOverview {
                system_summary: "One system".into(),
                architecture_style: "Modular".into(),
                technologies: vec![],
            },
            components: vec![],
            relationships: vec![],
            data_flows: vec![],
            entry_points: vec![],
            partial: false,
            analysis_warnings: vec![],
            supersedes: None,
        };
        let descriptor = CodebaseMapDescriptor::for_map(&map).unwrap();
        assert!(descriptor.matches_repositories(&[repository("b", "2"), repository("a", "1")]));
        assert!(!descriptor.matches_repositories(&[repository("a", "1")]));
        assert!(!descriptor.matches_repositories(&[repository("a", "1"), repository("b", "3")]));
        assert_eq!(descriptor.content_hash, map.content_hash().unwrap());
    }
}
