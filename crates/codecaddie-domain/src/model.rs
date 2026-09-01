use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type WorkspaceId = String;
pub type DeviceId = String;
pub type ActorId = String;
pub type RepositoryId = String;
pub type GoalId = String;
pub type GoalVersionId = String;
pub type ReportId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Editor,
    Viewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    pub actor_id: ActorId,
    pub device_id: DeviceId,
    pub signing_public_key: String,
    pub label: String,
}

impl DeviceIdentity {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.actor_id.trim().is_empty()
            || self.device_id.trim().is_empty()
            || self.label.trim().is_empty()
        {
            return Err("device identity fields are required");
        }
        if self.signing_public_key.len() != 64 || hex::decode(&self.signing_public_key).is_err() {
            return Err("device signing key must be a 32-byte hexadecimal value");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAccess {
    pub identity: DeviceIdentity,
    pub role: Role,
    pub grant_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeState {
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Criterion {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalVersion {
    pub id: GoalVersionId,
    pub goal_id: GoalId,
    pub title: String,
    pub business_outcome: String,
    pub priority: u8,
    /// One-based portfolio order. Zero denotes a legacy version whose order
    /// falls back to priority and title.
    #[serde(default)]
    pub position: u32,
    pub criteria: Vec<Criterion>,
    pub rubric_dimensions: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub created_by: ActorId,
    pub supersedes: Option<GoalVersionId>,
}

impl GoalVersion {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.title.trim().is_empty() {
            return Err("goal title is required");
        }
        if self.business_outcome.trim().is_empty() {
            return Err("business outcome is required");
        }
        if !(1..=5).contains(&self.priority) {
            return Err("goal priority must be between 1 and 5");
        }
        if self.position > 10_000 {
            return Err("goal position is outside the supported range");
        }
        if self.criteria.is_empty() {
            return Err("at least one acceptance criterion is required");
        }
        if self.criteria.iter().any(|item| item.text.trim().is_empty()) {
            return Err("acceptance criteria cannot be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRef {
    pub id: RepositoryId,
    pub display_name: String,
    pub remote_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenRepository {
    pub repository_id: RepositoryId,
    pub commit_sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Supported,
    Partial,
    Unsupported,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub repository_id: RepositoryId,
    pub commit_sha: String,
    pub blob_oid: String,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub content_hash: String,
    pub kind: EvidenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Implementation,
    Test,
    Configuration,
    Documentation,
    Architecture,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CriterionAssessment {
    pub criterion_id: String,
    pub verdict: Verdict,
    pub rationale: String,
    pub confidence: f32,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalAssessment {
    pub goal_version_id: GoalVersionId,
    pub verdict: Verdict,
    /// A short, user-facing conclusion for the goal. Older reports
    /// predate evidence-first summaries, so the report projection derives a
    /// fallback when this field is empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// How the codebase architecture supports or fails this goal, written
    /// against the codebase map that seeded the analysis. Empty for reports
    /// that predate the map (the UI falls back to joined architecture
    /// claims).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub architecture_narrative: String,
    /// Codebase-map component ids the narrative names, validated against
    /// the seeding map at materialization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_component_ids: Vec<String>,
    pub criteria: Vec<CriterionAssessment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureClaim {
    pub id: String,
    pub component: String,
    pub relationship: Option<String>,
    pub summary: String,
    pub affected_goal_version_ids: Vec<GoalVersionId>,
    /// The codebase-map component this claim describes, when the analysis
    /// was seeded with a map and named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Open,
    Planned,
    InProgress,
    ReadyForVerification,
    Verified,
    Reopened,
    Deferred,
    Dismissed,
}

impl ActionStatus {
    pub fn may_transition_to(self, next: Self) -> bool {
        use ActionStatus::*;
        matches!(
            (self, next),
            (Open, Planned | Deferred | Dismissed)
                | (Planned, InProgress | Deferred | Dismissed)
                | (InProgress, ReadyForVerification | Deferred)
                | (ReadyForVerification, Verified | Reopened)
                | (Verified, Reopened)
                | (Reopened, Planned | InProgress | Deferred | Dismissed)
                | (Deferred, Open | Planned | Dismissed)
                | (Dismissed, Reopened)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub id: String,
    pub title: String,
    pub rationale: String,
    pub expected_business_impact: String,
    pub goal_version_ids: Vec<GoalVersionId>,
    pub evidence: Vec<EvidenceRef>,
    pub rank: u32,
}

/// How a report entered the ledger. Both paths run the same validation; the
/// origin records whether the analysis was driven by an app-initiated scan or
/// submitted by a coding agent through the local MCP server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportOrigin {
    #[default]
    Scan,
    AgentSession,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: ReportId,
    #[serde(with = "time::serde::rfc3339")]
    pub completed_at: OffsetDateTime,
    pub repositories: Vec<FrozenRepository>,
    pub goal_version_ids: Vec<GoalVersionId>,
    pub goal_set_hash: String,
    pub provider: String,
    pub provider_version: String,
    #[serde(default)]
    pub origin: ReportOrigin,
    pub assessments: Vec<GoalAssessment>,
    pub architecture: Vec<ArchitectureClaim>,
    pub recommendations: Vec<Recommendation>,
    pub coverage: Option<f64>,
    pub unverified_criteria: u32,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub analysis_warnings: Vec<String>,
    /// The codebase map that seeded this analysis, when one was available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codebase_map_id: Option<String>,
    /// BLAKE3 content hash of the seeding map's canonical JSON body,
    /// binding this report to the exact map content it was seeded with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codebase_map_hash: Option<String>,
}
