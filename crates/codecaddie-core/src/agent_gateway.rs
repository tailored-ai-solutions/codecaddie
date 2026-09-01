//! Shared tool logic behind the MCP server and the one-shot agent CLI.
//!
//! The gateway owns everything the six agent tools do — workspace resolution,
//! goal serving, commit freezing, analysis re-validation, and action notes —
//! while its callers keep only their wire concerns (JSON-RPC framing for MCP,
//! argv/exit-code handling for the CLI). Analysis sessions are persisted as
//! encrypted metadata-only files under `<data_root>/agent-sessions` so a session begun
//! by one process (a long-lived MCP server or a one-shot CLI invocation) can
//! be submitted by another. Session files hold ids, commit pins, and a local
//! checkout path — never source text or key material.

use crate::{
    analyzer::{self, RawAnalysis, RawMapDeepDive, RawMapSurvey},
    local_state::{LocalWorkspaceStore, ReadyActionRequest},
    persistence::{read_encrypted_migrating, write_encrypted_atomic_new},
    repository::LocalRepository,
};
use codecaddie_domain::{FrozenRepository, GoalVersion, ReportOrigin, Role};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use time::OffsetDateTime;
use uuid::Uuid;

/// How long a begun analysis session stays submittable. Expired session
/// files are lazily garbage collected on any session operation.
const SESSION_TTL: time::Duration = time::Duration::hours(24);
const AGENT_SESSION_PURPOSE: &str = "agent-session-v1";

/// One repository a caller asks to freeze in `begin_analysis`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AnalysisAttachment {
    pub path: PathBuf,
    pub repository_id: Option<String>,
}

/// The durable form of an analysis session. Contains coordinates only: the
/// checkout `path` is a device-local directory name needed to re-open the git
/// object database at submit time, never repository content.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSession {
    session_id: String,
    workspace_id: String,
    repositories: Vec<StoredRepository>,
    goal_set_hash: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRepository {
    repository_id: String,
    commit_sha: String,
    path: PathBuf,
}

struct ClaimedSession {
    original: PathBuf,
    claimed: PathBuf,
    restore_on_drop: bool,
}

impl ClaimedSession {
    fn commit(mut self) {
        self.restore_on_drop = false;
        let _ = fs::remove_file(&self.claimed);
    }
}

impl Drop for ClaimedSession {
    fn drop(&mut self) {
        if self.restore_on_drop && !self.original.exists() {
            let _ = fs::rename(&self.claimed, &self.original);
        }
    }
}

pub(crate) struct AgentGateway {
    store: LocalWorkspaceStore,
    workspace_override: Option<String>,
}

impl AgentGateway {
    pub(crate) fn new(store: LocalWorkspaceStore, workspace_override: Option<String>) -> Self {
        Self {
            store,
            workspace_override,
        }
    }

    pub(crate) fn store(&self) -> &LocalWorkspaceStore {
        &self.store
    }

    pub(crate) fn resolve_workspace(&self) -> anyhow::Result<String> {
        if let Some(workspace_id) = &self.workspace_override {
            return Ok(workspace_id.clone());
        }
        self.store
            .recent_workspace()?
            .map(|workspace| workspace.workspace_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no CodeCaddie workspace exists on this device; create one in the CodeCaddie app first"
                )
            })
    }

    pub(crate) fn workspace_status(&self) -> anyhow::Result<Value> {
        let resolved = self.resolve_workspace().and_then(|workspace_id| {
            let (access, projection) = self.store.workspace_parts(&workspace_id)?;
            let goals = self.store.approved_goals(&workspace_id)?;
            Ok((workspace_id, access.role, projection.name, goals.len()))
        });
        Ok(match resolved {
            Ok((workspace_id, role, name, goal_count)) => json!({
                "attestedModeAvailable": goal_count > 0,
                "workspaceId": workspace_id,
                "workspaceName": name,
                "role": role,
                "canSubmit": role == Role::Editor,
                "approvedGoalCount": goal_count
            }),
            Err(error) => json!({
                "attestedModeAvailable": false,
                "reason": format!("{error:#}")
            }),
        })
    }

    pub(crate) fn approved_goals(&self) -> anyhow::Result<Value> {
        let workspace_id = self.resolve_workspace()?;
        let goals = self.store.approved_goals(&workspace_id)?;
        if goals.is_empty() {
            anyhow::bail!(
                "no approved goals in this workspace; approve an immutable goal version in the CodeCaddie app first"
            );
        }
        Ok(json!({ "goals": goals, "goalSetHash": goal_set_hash(&goals)? }))
    }

    pub(crate) fn begin_analysis(
        &self,
        attachments: Vec<AnalysisAttachment>,
    ) -> anyhow::Result<Value> {
        if attachments.is_empty() {
            anyhow::bail!("begin_analysis requires at least one repository path");
        }
        let workspace_id = self.resolve_workspace()?;
        let (access, projection) = self.store.workspace_parts(&workspace_id)?;
        let goals = self.store.approved_goals(&workspace_id)?;
        if goals.is_empty() {
            anyhow::bail!(
                "no approved goals in this workspace; approve an immutable goal version in the CodeCaddie app first"
            );
        }
        let registered: Vec<&str> = projection.repositories.keys().map(String::as_str).collect();
        if attachments.len() != 1 || registered.len() != 1 {
            anyhow::bail!(
                "local MCP analysis currently requires the workspace's single registered repository"
            );
        }
        if access.repository_path.trim().is_empty() {
            anyhow::bail!("this device has no registered local checkout for the workspace");
        }
        let registered_path = std::path::Path::new(&access.repository_path).canonicalize()?;
        let mut seen = BTreeSet::new();
        let mut repositories = Vec::with_capacity(attachments.len());
        for attachment in &attachments {
            let repository_id = match &attachment.repository_id {
                Some(id) => {
                    if !projection.repositories.contains_key(id) {
                        anyhow::bail!(
                            "repositoryId {id:?} is not registered in this workspace; registered: {registered:?}"
                        );
                    }
                    id.clone()
                }
                None if registered.len() == 1 && attachments.len() == 1 => {
                    registered[0].to_string()
                }
                None => anyhow::bail!(
                    "repositoryId is required when the workspace registers more than one repository; registered: {registered:?}"
                ),
            };
            if !seen.insert(repository_id.clone()) {
                anyhow::bail!("repository IDs must be unique");
            }
            let repository = LocalRepository::attach(&repository_id, &attachment.path)?;
            if repository.path != registered_path {
                anyhow::bail!(
                    "analysis path does not match the workspace's registered local checkout"
                );
            }
            let commit = repository.resolve_commit("HEAD")?;
            repositories.push((repository, commit));
        }
        self.open_session(workspace_id, repositories)
    }

    /// Persists a session file for already-frozen repositories and returns
    /// the begin-analysis payload. Shared by the MCP tool (which pins HEAD of
    /// caller-supplied paths) and the CLI (which pins explicit commits in the
    /// registered checkout).
    pub(crate) fn open_session(
        &self,
        workspace_id: String,
        repositories: Vec<(LocalRepository, String)>,
    ) -> anyhow::Result<Value> {
        self.collect_expired_sessions();
        let goals = self.store.approved_goals(&workspace_id)?;
        if goals.is_empty() {
            anyhow::bail!(
                "no approved goals in this workspace; approve an immutable goal version in the CodeCaddie app first"
            );
        }
        let goal_set_hash = goal_set_hash(&goals)?;
        let session_id = format!("analysis-{}", Uuid::now_v7());
        let created_at = OffsetDateTime::now_utc();
        let session = StoredSession {
            session_id: session_id.clone(),
            workspace_id: workspace_id.clone(),
            repositories: repositories
                .iter()
                .map(|(repository, commit)| StoredRepository {
                    repository_id: repository.id.clone(),
                    commit_sha: commit.clone(),
                    path: repository.path.clone(),
                })
                .collect(),
            goal_set_hash: goal_set_hash.clone(),
            created_at,
            expires_at: created_at + SESSION_TTL,
        };
        let session_path = self.session_path(&session_id)?;
        write_encrypted_atomic_new(
            &session_path,
            &serde_json::to_vec(&session)?,
            self.store.content_cipher(),
            AGENT_SESSION_PURPOSE,
        )?;
        if let Err(error) = self
            .store
            .record_analysis_started(&workspace_id, &session_id)
        {
            // A returned session is an analysis start. If the signed local
            // marker cannot be committed, retract the just-created session
            // so the two systems cannot silently disagree.
            let _ = fs::remove_file(&session_path);
            return Err(error.context("the local analysis-start marker could not be saved"));
        }
        let frozen: Vec<Value> = repositories
            .iter()
            .map(|(repository, commit)| {
                json!({ "repositoryId": repository.id, "commitSha": commit })
            })
            .collect();
        Ok(json!({
            "analysisSessionId": session_id,
            "workspaceId": workspace_id,
            "repositories": frozen,
            "goals": goals,
            "goalSetHash": goal_set_hash,
            "instructions": "Analyze the committed state at the frozen commits above. Assess every acceptance criterion of every goal exactly once. Every non-unverified verdict needs evidence citing the repositoryId shown here plus a repository-relative path and 1-based line range that exists at the frozen commit. Never include source excerpts in rationales. Submit the completed analysis with submit_analysis."
        }))
    }

    /// Confirms a session exists and has not expired, without consuming it.
    pub(crate) fn verify_session(&self, session_id: &str) -> anyhow::Result<()> {
        let _ = self.load_session(session_id)?;
        Ok(())
    }

    pub(crate) fn submit_analysis(
        &self,
        session_id: &str,
        analysis: Value,
        provider: String,
    ) -> anyhow::Result<Value> {
        let (session, claim) = self.claim_session(session_id)?;
        let workspace_id = session.workspace_id.clone();
        let (access, _) = self.store.workspace_parts(&workspace_id)?;
        if access.role != Role::Editor {
            anyhow::bail!(
                "this device has read-only access; an Editor device must submit analyses"
            );
        }
        let repositories = session
            .repositories
            .iter()
            .map(|stored| {
                Ok((
                    LocalRepository::attach(&stored.repository_id, &stored.path)?,
                    stored.commit_sha.clone(),
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let raw: RawAnalysis = serde_json::from_value(analysis)?;
        let goals = self.store.approved_goals(&workspace_id)?;
        if goal_set_hash(&goals)? != session.goal_set_hash {
            anyhow::bail!(
                "the approved goal set changed after this session began; call begin_analysis again"
            );
        }
        let report = analyzer::materialize_agent_report_for_repositories(
            format!("report-{}", Uuid::now_v7()),
            &repositories,
            provider,
            &goals,
            raw,
        )?;
        let persistence_repositories = repositories
            .iter()
            .map(|(repository, _)| repository.clone())
            .collect::<Vec<_>>();
        self.store.record_report_with_repositories(
            &workspace_id,
            report.clone(),
            &persistence_repositories,
        )?;
        claim.commit();
        let (_, projection) = self.store.workspace_parts(&workspace_id)?;
        let actions: Vec<Value> = projection
            .actions
            .values()
            .map(|action| {
                json!({
                    "actionId": action.id,
                    "title": action.title,
                    "status": action.status,
                    "lastNote": action.last_note
                })
            })
            .collect();
        Ok(json!({
            "reportId": report.id,
            "recorded": true,
            "origin": report.origin,
            "coverage": report.coverage,
            "unverifiedCriteria": report.unverified_criteria,
            "goalVerdicts": report
                .assessments
                .iter()
                .map(|assessment| json!({
                    "goalVersionId": assessment.goal_version_id,
                    "verdict": assessment.verdict
                }))
                .collect::<Vec<_>>(),
            "actions": actions
        }))
    }

    /// Serves the newest recorded codebase map whose frozen repository set
    /// matches the session's pinned commits (or the workspace's newest map
    /// when no session is given). Metadata-only, like every gateway
    /// payload: derived claims and coordinates, never source.
    pub(crate) fn get_codebase_map(&self, session_id: Option<&str>) -> anyhow::Result<Value> {
        let workspace_id = self.resolve_workspace()?;
        let map = match session_id {
            Some(session_id) => {
                let session = self.load_session(session_id)?;
                let frozen = session
                    .repositories
                    .iter()
                    .map(|stored| FrozenRepository {
                        repository_id: stored.repository_id.clone(),
                        commit_sha: stored.commit_sha.clone(),
                    })
                    .collect::<Vec<_>>();
                self.store.load_codebase_map_for(&workspace_id, &frozen)?
            }
            None => self
                .store
                .load_codebase_map(&workspace_id, None)?
                .map(|(_, map)| map),
        };
        match map {
            Some(map) => Ok(json!({ "available": true, "map": map })),
            None => Ok(json!({
                "available": false,
                "reason": "No recorded codebase map matches these frozen commits. Survey the codebase and submit one with submit_codebase_map, or proceed without a map.",
            })),
        }
    }

    /// Validates and records an agent-submitted codebase map against the
    /// session's pinned commits. Fails closed on any source-matching
    /// narrative — the agent can simply resubmit. The session is not
    /// consumed: it remains available for `submit_analysis`.
    pub(crate) fn submit_codebase_map(
        &self,
        session_id: &str,
        submission: Value,
        provider: String,
    ) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct MapSubmission {
            survey: RawMapSurvey,
            #[serde(default)]
            deep_dives: Vec<RawMapDeepDive>,
        }
        let session = self.load_session(session_id)?;
        let workspace_id = session.workspace_id.clone();
        let (access, _) = self.store.workspace_parts(&workspace_id)?;
        if access.role != Role::Editor {
            anyhow::bail!("this device has read-only access; an Editor device must submit maps");
        }
        let submission: MapSubmission = serde_json::from_value(submission)?;
        let repositories = session
            .repositories
            .iter()
            .map(|stored| {
                Ok((
                    LocalRepository::attach(&stored.repository_id, &stored.path)?,
                    stored.commit_sha.clone(),
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let supersedes = {
            let frozen = session
                .repositories
                .iter()
                .map(|stored| FrozenRepository {
                    repository_id: stored.repository_id.clone(),
                    commit_sha: stored.commit_sha.clone(),
                })
                .collect::<Vec<_>>();
            self.store
                .load_codebase_map_for(&workspace_id, &frozen)?
                .map(|map| map.id)
        };
        let provider_version = submission.survey.provider_version.clone();
        let map = analyzer::materialize_codebase_map(
            format!("map-{}", Uuid::now_v7()),
            &repositories,
            provider,
            provider_version,
            ReportOrigin::AgentSession,
            submission.survey,
            submission.deep_dives,
            Vec::new(),
            false,
            supersedes,
            analyzer::MapNarrativePolicy::Reject,
            None,
        )?;
        let descriptor = self.store.record_codebase_map(&workspace_id, &map)?;
        Ok(json!({
            "mapId": descriptor.map_id,
            "recorded": true,
            "componentCount": descriptor.component_count,
            "partial": descriptor.partial,
            "warnings": map.analysis_warnings,
        }))
    }

    pub(crate) fn action_backlog(&self) -> anyhow::Result<Value> {
        let workspace_id = self.resolve_workspace()?;
        let (_, projection) = self.store.workspace_parts(&workspace_id)?;
        let latest_report = projection
            .reports
            .values()
            .max_by_key(|report| report.completed_at);
        let actions: Vec<Value> = projection
            .actions
            .values()
            .map(|action| {
                json!({
                    "actionId": action.id,
                    "recommendationId": action.recommendation_id,
                    "title": action.title,
                    "status": action.status,
                    "lastNote": action.last_note
                })
            })
            .collect();
        let actioned: BTreeSet<&str> = projection
            .actions
            .values()
            .map(|action| action.recommendation_id.as_str())
            .collect();
        let recommendations: Vec<Value> = latest_report
            .map(|report| {
                report
                    .recommendations
                    .iter()
                    .map(|recommendation| {
                        json!({
                            "recommendationId": recommendation.id,
                            "title": recommendation.title,
                            "rank": recommendation.rank,
                            "goalVersionIds": recommendation.goal_version_ids,
                            "hasAction": actioned.contains(recommendation.id.as_str())
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({
            "actions": actions,
            "latestReportRecommendations": recommendations
        }))
    }

    pub(crate) fn record_action_note(&self, request: ReadyActionRequest) -> anyhow::Result<Value> {
        let workspace_id = self.resolve_workspace()?;
        let (access, _) = self.store.workspace_parts(&workspace_id)?;
        if access.role != Role::Editor {
            anyhow::bail!("this device has read-only access; an Editor device must update actions");
        }
        let action = self.store.ready_action(&workspace_id, request)?;
        Ok(json!({
            "action": action,
            "note": "The action is ready for verification. Only a subsequent scan can mark it Verified."
        }))
    }

    fn sessions_dir(&self) -> PathBuf {
        self.store.data_root().join("agent-sessions")
    }

    /// Session ids name files directly, so only the characters our own ids
    /// use are accepted; anything else cannot address a session file.
    fn session_path(&self, session_id: &str) -> anyhow::Result<PathBuf> {
        if session_id.is_empty()
            || !session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            anyhow::bail!("unknown or expired analysis session; call begin_analysis first");
        }
        Ok(self.sessions_dir().join(format!("{session_id}.json")))
    }

    fn load_session(&self, session_id: &str) -> anyhow::Result<StoredSession> {
        self.collect_expired_sessions();
        let path = self.session_path(session_id)?;
        let session =
            read_encrypted_migrating(&path, self.store.content_cipher(), AGENT_SESSION_PURPOSE)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<StoredSession>(&bytes).ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown or expired analysis session; call begin_analysis first"
                    )
                })?;
        if session.expires_at <= OffsetDateTime::now_utc() {
            let _ = fs::remove_file(&path);
            anyhow::bail!("unknown or expired analysis session; call begin_analysis first");
        }
        Ok(session)
    }

    fn claim_session(&self, session_id: &str) -> anyhow::Result<(StoredSession, ClaimedSession)> {
        self.collect_expired_sessions();
        let original = self.session_path(session_id)?;
        let claimed = self
            .sessions_dir()
            .join(format!(".claimed-{session_id}-{}.json", Uuid::now_v7()));
        fs::rename(&original, &claimed).map_err(|_| {
            anyhow::anyhow!("unknown or expired analysis session; call begin_analysis first")
        })?;
        let claim = ClaimedSession {
            original,
            claimed: claimed.clone(),
            restore_on_drop: true,
        };
        let session: StoredSession = serde_json::from_slice(&read_encrypted_migrating(
            &claimed,
            self.store.content_cipher(),
            AGENT_SESSION_PURPOSE,
        )?)?;
        if session.expires_at <= OffsetDateTime::now_utc() {
            let _ = fs::remove_file(&claimed);
            anyhow::bail!("unknown or expired analysis session; call begin_analysis first");
        }
        Ok((session, claim))
    }

    /// Lazy GC: removes expired or unreadable session files. Runs on every
    /// session operation so abandoned sessions never accumulate.
    fn collect_expired_sessions(&self) {
        let now = OffsetDateTime::now_utc();
        let Ok(entries) = fs::read_dir(self.sessions_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let expired =
                read_encrypted_migrating(&path, self.store.content_cipher(), AGENT_SESSION_PURPOSE)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<StoredSession>(&bytes).ok())
                    .map(|session| session.expires_at <= now)
                    // A concurrent process may still be publishing a file from an
                    // older build. Leave unreadable state for quarantine/recovery.
                    .unwrap_or(false);
            if expired {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

fn goal_set_hash(goals: &[GoalVersion]) -> anyhow::Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(goals)?)
        .to_hex()
        .to_string())
}
