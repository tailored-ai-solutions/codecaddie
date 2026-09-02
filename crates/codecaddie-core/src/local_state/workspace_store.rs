//! Workspace CRUD over the encrypted event log: creation, project context,
//! goal approval, reports, actions, and recovery export. Every read and write
//! enforces the role and workspace fingerprint recorded in local state.

use super::{
    heatmap::{
        HeatmapWeek, REPORT_HISTORY_LIMIT, ReportHistoryPage, build_report_finding,
        build_report_heatmap, build_report_history_page,
    },
    identity::{
        LocalDeviceSecret, LocalState, LocalWorkspaceAccess, ProjectContext, RecoveryPayload,
    },
    locks::{LocalStateWriteGuard, WorkspaceWriteGuard},
    portable_backup::{self, MAX_BACKUP_BYTES, PortableBackupPayload},
};
use crate::{
    at_rest::ContentCipher,
    persistence::{
        LocalStateFile, protect_file_at_rest, read_encrypted_migrating, write_encrypted_atomic_new,
        write_encrypted_replace, write_private_new,
    },
    repository::LocalRepository,
    runtime_channel::RuntimeChannel,
    storage::LocalEventLog,
};
use codecaddie_domain::{
    ActionProjection, ActionStatus, CodebaseMap, CodebaseMapDescriptor, Criterion,
    DecisionFunnelEventKind, DomainEvent, EventEnvelope, FrozenRepository, GoalVersion,
    ProductEventKind, ProductEventRecord, ReliabilityEventKind, ReliabilityEventRecord,
    ReliabilityOutcome, Report, RepositoryRef, Role, Verdict, WorkspaceProjection,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};
use time::OffsetDateTime;
use uuid::Uuid;

const INITIAL_EPOCH: u64 = 1;
const RECENT_WORKSPACE_PURPOSE: &str = "recent-workspace-v1";
const PROVIDER_PREFERENCE_PURPOSE: &str = "provider-preference-v1";
const CODEBASE_MAP_PURPOSE: &str = "codebase-map-v1";
const AGENT_SESSION_PURPOSE: &str = "agent-session-v1";
const AT_REST_MIGRATION_PURPOSE: &str = "at-rest-migration-v1";
const AT_REST_MIGRATION_MARKER: &str = "at-rest-migration-v1.complete";
const AT_REST_MIGRATION_VALUE: &[u8] = b"managed-private-surfaces-encrypted-v1";
const BACKUP_SCHEDULE_PURPOSE: &str = "backup-schedule-v1";
const BACKUP_SCHEDULE_FORMAT: &str = "codecaddie-backup-schedule-v1";
const BACKUP_CADENCE_SECONDS: i64 = 24 * 60 * 60;
const BACKUP_RETENTION_COUNT: usize = 14;
const BACKUP_RTO_MINUTES: u32 = 30;
const PRODUCT_EVENT_SCHEMA_VERSION: u16 = 2;

fn product_event_record(
    workspace_id: &str,
    kind: ProductEventKind,
    session_id: impl Into<String>,
    report_id: Option<String>,
    elapsed_milliseconds: Option<u64>,
) -> ProductEventRecord {
    let product_version = env!("CARGO_PKG_VERSION").to_string();
    ProductEventRecord {
        schema_version: PRODUCT_EVENT_SCHEMA_VERSION,
        kind,
        workspace_id: workspace_id.to_string(),
        session_id: session_id.into(),
        cohort: format!("desktop-{product_version}"),
        product_version,
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        report_id,
        elapsed_milliseconds,
    }
}

fn normalize_project_context(context: &mut ProjectContext) -> anyhow::Result<()> {
    let paths = if !context.context_file_paths.is_empty() {
        context.context_file_paths.clone()
    } else if !context.context_files.is_empty() {
        context
            .context_files
            .iter()
            .map(|file| file.path.clone())
            .collect()
    } else {
        return Ok(());
    };
    let references = crate::context_documents::inspect_paths(&paths)?;
    context.context_file_names = references
        .iter()
        .map(|file| file.display_name.clone())
        .collect();
    context.context_files = references;
    context.context_file_paths.clear();
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub repository_display_name: String,
    pub repository_path: String,
    pub product_brief: String,
    #[serde(default)]
    pub context: ProjectContext,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkspaceContextRequest {
    pub name: String,
    #[serde(default)]
    pub repository_path: String,
    pub product_brief: String,
    #[serde(default)]
    pub context: ProjectContext,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveGoalRequest {
    #[serde(default = "default_goal_id")]
    pub goal_id: String,
    pub title: String,
    pub business_outcome: String,
    pub criteria: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub rubric_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceGoalsRequest {
    pub goals: Vec<ApproveGoalRequest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyActionRequest {
    pub recommendation_id: String,
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentWorkspace {
    pub workspace_id: String,
    pub name: String,
    pub repository_path: String,
    pub product_brief: String,
    pub context: ProjectContext,
    pub approved_goal: Option<GoalVersion>,
    pub approved_goals: Vec<GoalVersion>,
    pub latest_report: Option<Report>,
    pub report_heatmap: Vec<HeatmapWeek>,
    pub decision_funnel: DecisionFunnelSummary,
    pub reliability: ReliabilitySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableBackupReceipt {
    pub workspace_id: String,
    pub event_count: usize,
    pub manifest_blake3: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableRestoreReceipt {
    pub workspace_id: String,
    pub event_count: usize,
    pub manifest_blake3: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupScheduleStatus {
    pub enabled: bool,
    pub destination_directory: Option<String>,
    pub cadence_hours: u32,
    pub retention_count: usize,
    pub recovery_point_objective_hours: u32,
    pub recovery_time_objective_minutes: u32,
    pub last_successful_at_unix: Option<i64>,
    pub next_due_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledBackupRunReceipt {
    pub status: String,
    pub schedule: BackupScheduleStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_blake3: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackupScheduleConfig {
    format: String,
    workspace_id: String,
    destination_directory: String,
    passphrase: String,
    last_successful_at_unix: Option<i64>,
}

impl BackupScheduleConfig {
    fn status(&self) -> BackupScheduleStatus {
        BackupScheduleStatus {
            enabled: true,
            destination_directory: Some(self.destination_directory.clone()),
            cadence_hours: 24,
            retention_count: BACKUP_RETENTION_COUNT,
            recovery_point_objective_hours: 24,
            recovery_time_objective_minutes: BACKUP_RTO_MINUTES,
            last_successful_at_unix: self.last_successful_at_unix,
            next_due_at_unix: self
                .last_successful_at_unix
                .and_then(|value| value.checked_add(BACKUP_CADENCE_SECONDS)),
        }
    }

    fn validate(&self, workspace_id: &str) -> anyhow::Result<()> {
        if self.format != BACKUP_SCHEDULE_FORMAT || self.workspace_id != workspace_id {
            anyhow::bail!("portable backup schedule identity is invalid");
        }
        portable_backup::validate_passphrase(&self.passphrase)?;
        if self.destination_directory.trim().is_empty() {
            anyhow::bail!("portable backup schedule destination is invalid");
        }
        Ok(())
    }
}

fn disabled_backup_schedule() -> BackupScheduleStatus {
    BackupScheduleStatus {
        enabled: false,
        destination_directory: None,
        cadence_hours: 24,
        retention_count: BACKUP_RETENTION_COUNT,
        recovery_point_objective_hours: 24,
        recovery_time_objective_minutes: BACKUP_RTO_MINUTES,
        last_successful_at_unix: None,
        next_due_at_unix: None,
    }
}

fn prune_scheduled_backups(directory: &Path, prefix: &str, retain: usize) -> anyhow::Result<()> {
    let mut backups = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(prefix)
                && name.ends_with(".codecaddie-backup")
                && entry.file_type().is_ok_and(|kind| kind.is_file())
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    backups.sort();
    let remove_count = backups.len().saturating_sub(retain);
    for stale in backups.into_iter().take(remove_count) {
        fs::remove_file(stale)?;
    }
    if remove_count > 0 {
        crate::persistence::sync_parent(directory)?;
    }
    Ok(())
}

/// Aggregate product-usage measurements derived entirely from the signed,
/// device-local workspace ledger. Only counts and elapsed seconds leave the
/// core; event payloads contain no source, attachment contents, or goal text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionFunnelSummary {
    pub workspace_creations: u32,
    pub goal_approvals: u32,
    pub analysis_starts: u32,
    pub analysis_completions: u32,
    pub report_opens: u32,
    pub prompt_copies: u32,
    pub repeat_analyses: u32,
    pub repeat_review_opens: u32,
    pub scorecards_generated: u32,
    pub reports_saved: u32,
    pub evidence_opens: u32,
    pub comparisons_generated: u32,
    pub time_to_first_report_seconds: Option<i64>,
    pub decision_cycle_average_seconds: Option<i64>,
    pub decision_cycles: u32,
}

/// Device-local reliability measurements derived from signed, content-free
/// ledger events. Percentages are returned for the product UI; no raw
/// repository or user-authored material enters this projection.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilitySummary {
    pub operation_samples: u32,
    pub trace_spans_recorded: u32,
    pub operation_failures: u32,
    pub operation_cancellations: u32,
    pub provider_operation_samples: u32,
    pub provider_operation_failures: u32,
    pub provider_alerts_raised: u32,
    pub alerts_raised: u32,
    pub desktop_sessions_started: u32,
    pub desktop_sessions_ended: u32,
    pub desktop_crashes_detected: u32,
    pub average_latency_milliseconds: Option<u64>,
    pub availability_percent: Option<f64>,
    pub crash_free_sessions_percent: Option<f64>,
}

fn elapsed_seconds(earlier: OffsetDateTime, later: OffsetDateTime) -> Option<i64> {
    let seconds = (later - earlier).whole_seconds();
    (seconds >= 0).then_some(seconds)
}

fn build_decision_funnel_summary(
    events: &[EventEnvelope],
    projection: &WorkspaceProjection,
) -> DecisionFunnelSummary {
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (left.lamport, left.event_id, &left.device_id).cmp(&(
            right.lamport,
            right.event_id,
            &right.device_id,
        ))
    });
    let mut summary = DecisionFunnelSummary::default();
    let mut created_at = None;
    let mut pending_goal_approval = None;
    let mut completed_reports = 0_u32;
    let mut decision_cycle_seconds = 0_i128;
    for envelope in ordered {
        if !projection
            .applied_event_ids
            .contains(&envelope.event_id.to_string())
        {
            continue;
        }
        match &envelope.event {
            DomainEvent::WorkspaceCreated { .. } => {
                summary.workspace_creations = summary.workspace_creations.saturating_add(1);
                created_at.get_or_insert(envelope.occurred_at);
            }
            DomainEvent::GoalVersionApproved { .. } | DomainEvent::GoalSetReplaced { .. } => {
                summary.goal_approvals = summary.goal_approvals.saturating_add(1);
                pending_goal_approval = Some(envelope.occurred_at);
            }
            DomainEvent::ReportCompleted { .. } => {
                summary.analysis_completions = summary.analysis_completions.saturating_add(1);
                completed_reports = completed_reports.saturating_add(1);
                if summary.time_to_first_report_seconds.is_none()
                    && let Some(created) = created_at
                {
                    summary.time_to_first_report_seconds =
                        elapsed_seconds(created, envelope.occurred_at);
                }
                if let Some(approved_at) = pending_goal_approval.take()
                    && let Some(seconds) = elapsed_seconds(approved_at, envelope.occurred_at)
                {
                    decision_cycle_seconds += i128::from(seconds);
                    summary.decision_cycles = summary.decision_cycles.saturating_add(1);
                }
            }
            DomainEvent::DecisionFunnelEventRecorded { kind } => match kind {
                DecisionFunnelEventKind::AnalysisStarted => {
                    summary.analysis_starts = summary.analysis_starts.saturating_add(1);
                }
                DecisionFunnelEventKind::RepeatAnalysis => {
                    summary.repeat_analyses = summary.repeat_analyses.saturating_add(1);
                }
                DecisionFunnelEventKind::ReportOpened => {
                    summary.report_opens = summary.report_opens.saturating_add(1);
                    if completed_reports >= 2 {
                        summary.repeat_review_opens = summary.repeat_review_opens.saturating_add(1);
                    }
                }
                DecisionFunnelEventKind::PromptCopied => {
                    summary.prompt_copies = summary.prompt_copies.saturating_add(1);
                }
            },
            DomainEvent::ProductEventRecorded { record } => match record.kind {
                ProductEventKind::AnalysisStarted => {
                    summary.analysis_starts = summary.analysis_starts.saturating_add(1);
                }
                ProductEventKind::RepeatAnalysisStarted => {
                    summary.repeat_analyses = summary.repeat_analyses.saturating_add(1);
                }
                ProductEventKind::ReportRevisited => {
                    summary.report_opens = summary.report_opens.saturating_add(1);
                    if completed_reports >= 2 {
                        summary.repeat_review_opens = summary.repeat_review_opens.saturating_add(1);
                    }
                }
                ProductEventKind::PromptCopied => {
                    summary.prompt_copies = summary.prompt_copies.saturating_add(1);
                }
                ProductEventKind::ScorecardGenerated => {
                    summary.scorecards_generated = summary.scorecards_generated.saturating_add(1);
                }
                ProductEventKind::ReportSaved => {
                    summary.reports_saved = summary.reports_saved.saturating_add(1);
                }
                ProductEventKind::EvidenceOpened => {
                    summary.evidence_opens = summary.evidence_opens.saturating_add(1);
                }
                ProductEventKind::ComparisonGenerated => {
                    summary.comparisons_generated = summary.comparisons_generated.saturating_add(1);
                }
                ProductEventKind::WorkspaceCreated
                | ProductEventKind::GoalApproved
                | ProductEventKind::TimeToFirstSavedReport => {}
            },
            _ => {}
        }
    }
    if summary.decision_cycles > 0 {
        summary.decision_cycle_average_seconds =
            Some((decision_cycle_seconds / i128::from(summary.decision_cycles)) as i64);
    }
    summary
}

fn build_reliability_summary(
    events: &[EventEnvelope],
    projection: &WorkspaceProjection,
) -> ReliabilitySummary {
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (left.lamport, left.event_id, &left.device_id).cmp(&(
            right.lamport,
            right.event_id,
            &right.device_id,
        ))
    });
    let mut summary = ReliabilitySummary::default();
    let mut latency_total = 0_u128;
    let mut latency_samples = 0_u32;
    // Earlier builds wrote a second `trace_span_completed` record. New builds
    // reuse `operation_completed` as the span so supported prior binaries can
    // still parse and verify new history. Count either representation once.
    let legacy_trace_correlations = ordered
        .iter()
        .filter_map(|envelope| match &envelope.event {
            DomainEvent::ReliabilityEventRecorded { record }
                if record.kind == ReliabilityEventKind::TraceSpanCompleted =>
            {
                Some(record.correlation_id.clone())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    for envelope in ordered {
        if !projection
            .applied_event_ids
            .contains(&envelope.event_id.to_string())
        {
            continue;
        }
        let DomainEvent::ReliabilityEventRecorded { record } = &envelope.event else {
            continue;
        };
        match record.kind {
            ReliabilityEventKind::OperationCompleted => {
                summary.operation_samples = summary.operation_samples.saturating_add(1);
                if !legacy_trace_correlations.contains(record.correlation_id.as_str()) {
                    summary.trace_spans_recorded = summary.trace_spans_recorded.saturating_add(1);
                }
                let provider_operation = record
                    .operation
                    .as_deref()
                    .is_some_and(crate::reliability::is_provider_operation);
                if provider_operation {
                    summary.provider_operation_samples =
                        summary.provider_operation_samples.saturating_add(1);
                }
                match record.outcome {
                    Some(ReliabilityOutcome::Failed) => {
                        summary.operation_failures = summary.operation_failures.saturating_add(1);
                        if provider_operation
                            || record.error_category
                                == Some(codecaddie_domain::ReliabilityErrorCategory::Provider)
                        {
                            summary.provider_operation_failures =
                                summary.provider_operation_failures.saturating_add(1);
                        }
                    }
                    Some(ReliabilityOutcome::Cancelled) => {
                        summary.operation_cancellations =
                            summary.operation_cancellations.saturating_add(1);
                    }
                    Some(ReliabilityOutcome::Succeeded) | None => {}
                }
                if let Some(elapsed) = record.elapsed_milliseconds {
                    latency_total = latency_total.saturating_add(u128::from(elapsed));
                    latency_samples = latency_samples.saturating_add(1);
                }
            }
            ReliabilityEventKind::TraceSpanCompleted => {
                summary.trace_spans_recorded = summary.trace_spans_recorded.saturating_add(1);
            }
            ReliabilityEventKind::DesktopSessionStarted => {
                summary.desktop_sessions_started =
                    summary.desktop_sessions_started.saturating_add(1);
            }
            ReliabilityEventKind::DesktopSessionEnded => {
                summary.desktop_sessions_ended = summary.desktop_sessions_ended.saturating_add(1);
            }
            ReliabilityEventKind::DesktopCrashDetected => {
                // Schema-1 builds briefly inferred crashes from unmatched
                // session starts. Only an actual Native SDK panic marker is
                // authoritative; retaining but ignoring older inferred events
                // preserves signed history without polluting the aggregate.
                if record.error_code.as_deref() == Some("native_panic_detected") {
                    summary.desktop_crashes_detected =
                        summary.desktop_crashes_detected.saturating_add(1);
                }
            }
            ReliabilityEventKind::SloAlertRaised => {
                // Preserve signed schema-1 history while excluding the alert
                // paired with its now-ignored inferred crash. New native-panic
                // alerts carry the authoritative marker code.
                let legacy_inferred_crash_alert = record.alert_code.as_deref()
                    == Some("desktop_crash_detected")
                    && record.error_code.as_deref() != Some("native_panic_detected");
                if !legacy_inferred_crash_alert {
                    summary.alerts_raised = summary.alerts_raised.saturating_add(1);
                    if record.error_category
                        == Some(codecaddie_domain::ReliabilityErrorCategory::Provider)
                    {
                        summary.provider_alerts_raised =
                            summary.provider_alerts_raised.saturating_add(1);
                    }
                }
            }
        }
    }
    if latency_samples > 0 {
        summary.average_latency_milliseconds =
            Some((latency_total / u128::from(latency_samples)) as u64);
    }
    if summary.operation_samples > 0 {
        let succeeded = summary
            .operation_samples
            .saturating_sub(summary.operation_failures)
            .saturating_sub(summary.operation_cancellations);
        summary.availability_percent =
            Some(f64::from(succeeded) * 100.0 / f64::from(summary.operation_samples));
    }
    if summary.desktop_sessions_started > 0 {
        let crash_free = summary
            .desktop_sessions_started
            .saturating_sub(summary.desktop_crashes_detected);
        summary.crash_free_sessions_percent =
            Some(f64::from(crash_free) * 100.0 / f64::from(summary.desktop_sessions_started));
    }
    summary
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWorkspaceRequest {
    pub workspace_id: String,
}

fn default_priority() -> u8 {
    5
}

fn sort_goals(goals: &mut [GoalVersion]) {
    goals.sort_by(|left, right| match (left.position, right.position) {
        (0, 0) => right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.title.cmp(&right.title)),
        (0, _) => std::cmp::Ordering::Greater,
        (_, 0) => std::cmp::Ordering::Less,
        _ => left.position.cmp(&right.position),
    });
}

fn default_goal_id() -> String {
    "primary-goal".into()
}

pub struct LocalWorkspaceStore {
    root: PathBuf,
    local_state: LocalStateFile,
    content_cipher: ContentCipher,
}

fn protect_directory_files(
    directory: &Path,
    extension: &str,
    cipher: &ContentCipher,
    purpose: &str,
) -> anyhow::Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.file_type().is_dir() {
        anyhow::bail!("managed local-state directory is not a directory");
    }
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some(extension) {
            continue;
        }
        protect_file_at_rest(&path, cipher, purpose)?;
    }
    Ok(())
}

/// A core process is short-lived, so the marker prevents every request from
/// rereading all workspaces. The lock serializes the first post-upgrade sweep
/// across concurrently spawned cores. New writes already use encryption, and
/// encrypted local state is intentionally unreadable to builds old enough to
/// create new plaintext files after the marker exists.
fn migrate_active_state_at_rest(
    root: &Path,
    local_state: &LocalStateFile,
    cipher: &ContentCipher,
) -> anyhow::Result<()> {
    let lock_path = root.join("locks-v1").join("at-rest-migration-v1.lock");
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options.open(&lock_path)?;
    #[cfg(unix)]
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))?;
    lock.lock_exclusive()?;

    let result = (|| {
        let marker = root.join(AT_REST_MIGRATION_MARKER);
        if marker.exists() {
            let decoded = cipher.open_or_plain(AT_REST_MIGRATION_PURPOSE, &fs::read(&marker)?)?;
            if decoded.encrypted && decoded.plaintext == AT_REST_MIGRATION_VALUE {
                return Ok(());
            }
        }

        if local_state.exists() {
            let state: LocalState = local_state.load()?;
            state.validate()?;
        }
        LocalEventLog::open(root.join("events-v2"), cipher.clone())?.protect_all_at_rest()?;
        for (path, purpose) in [
            (root.join("recent-workspace-v1"), RECENT_WORKSPACE_PURPOSE),
            (
                root.join("provider-preference-v1"),
                PROVIDER_PREFERENCE_PURPOSE,
            ),
        ] {
            if path.exists() {
                protect_file_at_rest(&path, cipher, purpose)?;
            }
        }
        protect_directory_files(
            &root.join("codebase-maps-v1"),
            "json",
            cipher,
            CODEBASE_MAP_PURPOSE,
        )?;
        protect_directory_files(
            &root.join("agent-sessions"),
            "json",
            cipher,
            AGENT_SESSION_PURPOSE,
        )?;
        protect_directory_files(
            &root.join("backup-schedules-v1"),
            "json",
            cipher,
            BACKUP_SCHEDULE_PURPOSE,
        )?;
        write_encrypted_replace(
            &marker,
            AT_REST_MIGRATION_VALUE,
            cipher,
            AT_REST_MIGRATION_PURPOSE,
        )
    })();

    FileExt::unlock(&lock)?;
    result
}

fn claim_native_panic_marker(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    let pending = path.with_file_name("last-panic.pending");
    let candidate = if pending.exists() {
        pending.clone()
    } else {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::rename(path, &pending)?;
                pending.clone()
            }
            Ok(_) => anyhow::bail!("native panic marker is not a regular file"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    };
    let metadata = fs::symlink_metadata(&candidate)?;
    if !metadata.file_type().is_file() {
        anyhow::bail!("claimed native panic marker is not a regular file");
    }
    Ok(Some(candidate))
}

impl LocalWorkspaceStore {
    pub fn from_environment() -> anyhow::Result<Self> {
        let channel = RuntimeChannel::detect();
        let root = channel.data_root()?;
        let content_cipher = ContentCipher::from_local_key_file(&root)?;
        Self::from_root(root, content_cipher)
    }

    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> anyhow::Result<Self> {
        Self::from_root(root, ContentCipher::for_tests())
    }

    fn from_root(root: PathBuf, content_cipher: ContentCipher) -> anyhow::Result<Self> {
        let local_state = LocalStateFile::for_data_root(&root, content_cipher.clone())?;
        for directory in [
            root.clone(),
            root.join("events-v2"),
            root.join("locks-v1"),
            root.join("backup-schedules-v1"),
        ] {
            fs::create_dir_all(&directory)?;
            #[cfg(unix)]
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(unix)]
        for file in [
            root.join("recent-workspace-v1"),
            root.join("provider-preference-v1"),
        ] {
            if file.exists() {
                fs::set_permissions(file, fs::Permissions::from_mode(0o600))?;
            }
        }
        migrate_active_state_at_rest(&root, &local_state, &content_cipher)?;
        Ok(Self {
            root,
            local_state,
            content_cipher,
        })
    }

    fn load_local_state(&self) -> anyhow::Result<LocalState> {
        if !self.local_state.exists() {
            anyhow::bail!("local state is unavailable; start a new local workspace");
        }
        let local_state: LocalState = self.local_state.load()?;
        local_state.validate()?;
        Ok(local_state)
    }

    fn event_log(&self) -> anyhow::Result<LocalEventLog> {
        LocalEventLog::open(self.root.join("events-v2"), self.content_cipher.clone())
    }

    fn recent_path(&self) -> PathBuf {
        self.root.join("recent-workspace-v1")
    }

    fn provider_preference_path(&self) -> PathBuf {
        self.root.join("provider-preference-v1")
    }

    fn lock_path(&self, workspace_id: &str) -> PathBuf {
        self.root.join("locks-v1").join(format!(
            "{}.lock",
            blake3::hash(workspace_id.as_bytes()).to_hex()
        ))
    }

    fn local_state_lock_path(&self) -> PathBuf {
        self.root.join("locks-v1").join("local-state-v2.lock")
    }

    /// The desktop app spawns a fresh core process per request and an agent
    /// session keeps a long-lived MCP process, so two writers can race the
    /// load-projection/append sequence and derive the same lamport clock.
    /// Mutating entry points take this advisory lock for their full duration.
    pub(crate) fn write_lock(&self, workspace_id: &str) -> anyhow::Result<WorkspaceWriteGuard> {
        WorkspaceWriteGuard::acquire(&self.lock_path(workspace_id))
    }

    fn local_state_write_lock(&self) -> anyhow::Result<LocalStateWriteGuard> {
        LocalStateWriteGuard::acquire(&self.local_state_lock_path())
    }

    fn load_parts(
        &self,
        workspace_id: &str,
    ) -> anyhow::Result<(LocalState, LocalWorkspaceAccess, WorkspaceProjection)> {
        let local_state = self.load_local_state()?;
        let access = local_state
            .workspaces
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("workspace capability is unavailable on this device"))?;
        let events = self.event_log()?.load(workspace_id)?;
        let projection = WorkspaceProjection::rebuild(&events)?;
        if projection.workspace_fingerprint != access.workspace_fingerprint {
            anyhow::bail!("workspace fingerprint does not match the recorded capability");
        }
        Ok((local_state, access, projection))
    }

    fn append(
        &self,
        workspace_id: &str,
        access: &LocalWorkspaceAccess,
        device: &LocalDeviceSecret,
        projection: &mut WorkspaceProjection,
        event: DomainEvent,
    ) -> anyhow::Result<()> {
        if access.role != Role::Editor {
            anyhow::bail!("this device has read-only access");
        }
        let envelope = EventEnvelope::sign(
            workspace_id.into(),
            projection.applied_event_ids.len() as u64 + 1,
            OffsetDateTime::now_utc(),
            device.device_id.clone(),
            device.actor_id.clone(),
            projection.team_epoch().max(INITIAL_EPOCH),
            event,
            &device.signing_key()?,
        )?;
        projection.apply(&envelope)?;
        self.event_log()?.append(workspace_id, &envelope)
    }

    pub fn create_workspace(
        &self,
        mut request: CreateWorkspaceRequest,
    ) -> anyhow::Result<WorkspaceProjection> {
        let _local_state_guard = self.local_state_write_lock()?;
        if request.name.trim().is_empty()
            || request.repository_display_name.trim().is_empty()
            || request.repository_path.trim().is_empty()
            || request.product_brief.trim().is_empty()
        {
            anyhow::bail!("workspace name, repository, and product brief are required");
        }
        let mut local_state = if self.local_state.exists() {
            self.load_local_state()?
        } else {
            LocalState::new()?
        };
        let workspace_id = Uuid::now_v7().to_string();
        let identity = local_state.device.public_identity()?;
        let workspace_fingerprint =
            blake3::hash(format!("{workspace_id}:{}", identity.signing_public_key).as_bytes())
                .to_hex()
                .to_string();
        normalize_project_context(&mut request.context)?;
        let access = LocalWorkspaceAccess {
            workspace_id: workspace_id.clone(),
            workspace_name: request.name.clone(),
            workspace_fingerprint: workspace_fingerprint.clone(),
            role: Role::Editor,
            repository_path: request.repository_path,
            product_brief: request.product_brief,
            project_context: request.context,
        };
        local_state
            .workspaces
            .insert(workspace_id.clone(), access.clone());
        local_state.upgrade_format();
        self.local_state.save(&local_state)?;

        let mut projection = WorkspaceProjection::default();
        self.append(
            &workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::WorkspaceCreated {
                name: request.name,
                founding_device: identity,
                workspace_fingerprint,
            },
        )?;
        self.append(
            &workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::RepositoryRegistered {
                repository: RepositoryRef {
                    id: "attached-repository".into(),
                    display_name: request.repository_display_name,
                    remote_fingerprint: None,
                },
            },
        )?;
        self.append(
            &workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ProductEventRecorded {
                record: product_event_record(
                    &workspace_id,
                    ProductEventKind::WorkspaceCreated,
                    format!("activation-{workspace_id}"),
                    None,
                    None,
                ),
            },
        )?;
        write_encrypted_replace(
            &self.recent_path(),
            workspace_id.as_bytes(),
            &self.content_cipher,
            RECENT_WORKSPACE_PURPOSE,
        )?;
        Ok(projection)
    }

    /// The provider name the user last explicitly selected, or `None` when
    /// no valid preference is stored. The one-word value shares the same
    /// owner-only local encrypted-at-rest boundary as workspace state.
    pub fn provider_preference(&self) -> anyhow::Result<Option<String>> {
        let path = self.provider_preference_path();
        if !path.exists() {
            return Ok(None);
        }
        let stored =
            read_encrypted_migrating(&path, &self.content_cipher, PROVIDER_PREFERENCE_PURPOSE)?;
        let stored = String::from_utf8(stored)?;
        let stored = stored.trim();
        if matches!(stored, "claude" | "codex" | "grok") {
            Ok(Some(stored.to_string()))
        } else {
            Ok(None)
        }
    }

    pub fn set_provider_preference(&self, provider: &str) -> anyhow::Result<()> {
        if !matches!(provider, "claude" | "codex" | "grok") {
            anyhow::bail!("unknown provider preference");
        }
        write_encrypted_replace(
            &self.provider_preference_path(),
            provider.as_bytes(),
            &self.content_cipher,
            PROVIDER_PREFERENCE_PURPOSE,
        )?;
        Ok(())
    }

    /// Updates the project context of an existing workspace in place: same
    /// workspace id, no `recent-workspace-v1` write, no events appended, so
    /// approved goals and report history are untouched.
    pub fn update_workspace_context(
        &self,
        workspace_id: &str,
        mut request: UpdateWorkspaceContextRequest,
    ) -> anyhow::Result<()> {
        let _local_state_guard = self.local_state_write_lock()?;
        if request.product_brief.trim().is_empty() {
            anyhow::bail!("workspace name, repository, and product brief are required");
        }
        normalize_project_context(&mut request.context)?;
        let mut local_state = self.load_local_state()?;
        let access = local_state
            .workspaces
            .get_mut(workspace_id)
            .ok_or_else(|| anyhow::anyhow!("workspace capability is unavailable on this device"))?;
        if access.role != Role::Editor {
            anyhow::bail!("only the editing device can update the project context");
        }
        if !request.name.trim().is_empty() {
            access.workspace_name = request.name;
        }
        if !request.repository_path.trim().is_empty() {
            access.repository_path = request.repository_path;
        }
        access.product_brief = request.product_brief;
        access.project_context = request.context;
        local_state.upgrade_format();
        self.local_state.save(&local_state)
    }

    /// The persisted product brief for a workspace, or an empty string when
    /// none was recorded (for example on an imported viewer device).
    pub fn workspace_product_brief(&self, workspace_id: &str) -> anyhow::Result<String> {
        let local_state = self.load_local_state()?;
        Ok(local_state
            .workspaces
            .get(workspace_id)
            .map(|access| access.product_brief.clone())
            .unwrap_or_default())
    }

    pub fn workspace_project_context(&self, workspace_id: &str) -> anyhow::Result<ProjectContext> {
        let local_state = self.load_local_state()?;
        local_state
            .workspaces
            .get(workspace_id)
            .map(|access| access.project_context.clone())
            .ok_or_else(|| anyhow::anyhow!("workspace capability is unavailable on this device"))
    }

    pub fn recent_workspace(&self) -> anyhow::Result<Option<RecentWorkspace>> {
        // A recent pointer without local state is a pre-v2 data directory;
        // resolve to the fresh-start screen instead of erroring forever.
        if !self.local_state.exists() || !self.recent_path().exists() {
            return Ok(None);
        }
        let workspace_id = String::from_utf8(read_encrypted_migrating(
            &self.recent_path(),
            &self.content_cipher,
            RECENT_WORKSPACE_PURPOSE,
        )?)?;
        let workspace_id = workspace_id.trim();
        if workspace_id.is_empty() {
            return Ok(None);
        }
        self.load_workspace(workspace_id)
    }

    pub fn open_workspace(&self, workspace_id: &str) -> anyhow::Result<Option<RecentWorkspace>> {
        let workspace_id = workspace_id.trim();
        if workspace_id.is_empty() {
            anyhow::bail!("workspace id is required");
        }
        let local_state = self.load_local_state()?;
        if !local_state.workspaces.contains_key(workspace_id) {
            anyhow::bail!("workspace capability is unavailable on this device");
        }
        write_encrypted_replace(
            &self.recent_path(),
            workspace_id.as_bytes(),
            &self.content_cipher,
            RECENT_WORKSPACE_PURPOSE,
        )?;
        self.load_workspace(workspace_id)
    }

    fn load_workspace(&self, workspace_id: &str) -> anyhow::Result<Option<RecentWorkspace>> {
        let local_state = self.load_local_state()?;
        let access = local_state
            .workspaces
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("workspace capability is unavailable on this device"))?;
        let events = self.event_log()?.load(workspace_id)?;
        let team_projection = if events.is_empty() {
            None
        } else {
            let projection = WorkspaceProjection::rebuild(&events)?;
            if projection.workspace_fingerprint != access.workspace_fingerprint {
                anyhow::bail!("workspace fingerprint does not match the recorded capability");
            }
            Some(projection)
        };
        let mut approved_goals = team_projection
            .as_ref()
            .map(|projection| {
                projection
                    .approved_goals
                    .values()
                    .filter_map(|version_id| projection.goal_versions.get(version_id).cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        sort_goals(&mut approved_goals);
        let approved_goal = approved_goals.first().cloned();
        let latest_report = team_projection.as_ref().and_then(|projection| {
            projection
                .reports
                .values()
                .max_by_key(|report| report.completed_at)
                .cloned()
        });
        let report_heatmap = team_projection
            .as_ref()
            .map(|projection| build_report_heatmap(projection, REPORT_HISTORY_LIMIT))
            .unwrap_or_default();
        let decision_funnel = team_projection
            .as_ref()
            .map(|projection| build_decision_funnel_summary(&events, projection))
            .unwrap_or_default();
        let reliability = team_projection
            .as_ref()
            .map(|projection| build_reliability_summary(&events, projection))
            .unwrap_or_default();
        Ok(Some(RecentWorkspace {
            workspace_id: workspace_id.into(),
            // The local_state name tracks local renames from context edits; the
            // projection name comes from the immutable WorkspaceCreated
            // event (no rename event exists), so it is only a fallback.
            name: Some(access.workspace_name.clone())
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    team_projection
                        .as_ref()
                        .map(|projection| projection.name.clone())
                })
                .unwrap_or_default(),
            repository_path: access.repository_path.clone(),
            product_brief: access.product_brief.clone(),
            context: access.project_context.clone(),
            approved_goal,
            approved_goals,
            latest_report,
            report_heatmap,
            decision_funnel,
            reliability,
        }))
    }

    pub fn approve_goal(
        &self,
        workspace_id: &str,
        request: ApproveGoalRequest,
    ) -> anyhow::Result<GoalVersion> {
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        self.approve_goal_loaded(
            workspace_id,
            request,
            &local_state,
            &access,
            &mut projection,
        )
    }

    fn approve_goal_loaded(
        &self,
        workspace_id: &str,
        request: ApproveGoalRequest,
        local_state: &LocalState,
        access: &LocalWorkspaceAccess,
        projection: &mut WorkspaceProjection,
    ) -> anyhow::Result<GoalVersion> {
        let version = self.materialize_goal_version(&request, local_state, projection)?;
        let goal_id = version.goal_id.clone();
        let version_id = version.id.clone();
        if projection.goal_versions.contains_key(&version_id) {
            if projection.approved_goals.get(&goal_id) != Some(&version_id) {
                self.append(
                    workspace_id,
                    access,
                    &local_state.device,
                    projection,
                    DomainEvent::GoalVersionApproved {
                        goal_id,
                        version_id,
                    },
                )?;
            }
            return Ok(version);
        }
        self.append(
            workspace_id,
            access,
            &local_state.device,
            projection,
            DomainEvent::GoalVersionProposed {
                version: version.clone(),
            },
        )?;
        self.append(
            workspace_id,
            access,
            &local_state.device,
            projection,
            DomainEvent::GoalVersionApproved {
                goal_id,
                version_id,
            },
        )?;
        self.append(
            workspace_id,
            access,
            &local_state.device,
            projection,
            DomainEvent::ProductEventRecorded {
                record: product_event_record(
                    workspace_id,
                    ProductEventKind::GoalApproved,
                    format!("goal-approval-{}", version.id),
                    None,
                    None,
                ),
            },
        )?;
        Ok(version)
    }

    fn materialize_goal_version(
        &self,
        request: &ApproveGoalRequest,
        local_state: &LocalState,
        projection: &WorkspaceProjection,
    ) -> anyhow::Result<GoalVersion> {
        let goal_id = request.goal_id.trim().to_string();
        if goal_id.is_empty() {
            anyhow::bail!("goal id is required");
        }
        let version_material = serde_json::json!({
            "goalId": goal_id,
            "title": request.title,
            "businessOutcome": request.business_outcome,
            "criteria": request.criteria,
            "priority": request.priority,
            "position": request.position,
            "rubricDimensions": request.rubric_dimensions,
        });
        let version_hash = blake3::hash(&serde_json::to_vec(&version_material)?).to_hex();
        let version_id = format!("goal-version-{}", &version_hash[..20]);
        if let Some(existing) = projection.goal_versions.get(&version_id).cloned() {
            if existing.goal_id != goal_id {
                anyhow::bail!("goal version identity collided across logical goals");
            }
            return Ok(existing);
        }
        let version = GoalVersion {
            id: version_id,
            goal_id: goal_id.clone(),
            title: request.title.clone(),
            business_outcome: request.business_outcome.clone(),
            priority: request.priority,
            position: request.position,
            criteria: request
                .criteria
                .iter()
                .enumerate()
                .map(|(index, text)| Criterion {
                    id: format!("criterion-{}-{index}", &version_hash[..16]),
                    text: text.clone(),
                })
                .collect(),
            rubric_dimensions: request.rubric_dimensions.clone(),
            created_at: OffsetDateTime::now_utc(),
            created_by: local_state.device.actor_id.clone(),
            supersedes: projection.approved_goals.get(&goal_id).cloned(),
        };
        version.validate().map_err(anyhow::Error::msg)?;
        Ok(version)
    }

    pub fn replace_goals(
        &self,
        workspace_id: &str,
        request: ReplaceGoalsRequest,
    ) -> anyhow::Result<Vec<GoalVersion>> {
        if request.goals.is_empty() {
            anyhow::bail!("at least one goal is required");
        }
        let mut requested_ids = std::collections::BTreeSet::new();
        let mut positions = std::collections::BTreeSet::new();
        for goal in &request.goals {
            let id = goal.goal_id.trim();
            if id.is_empty() || !requested_ids.insert(id.to_string()) {
                anyhow::bail!("every goal needs a unique stable id");
            }
            if goal.position == 0
                || goal.position as usize > request.goals.len()
                || !positions.insert(goal.position)
            {
                anyhow::bail!("goal positions must be unique and contiguous from one");
            }
            GoalVersion {
                id: "validation-only".into(),
                goal_id: id.into(),
                title: goal.title.clone(),
                business_outcome: goal.business_outcome.clone(),
                priority: goal.priority,
                position: goal.position,
                criteria: goal
                    .criteria
                    .iter()
                    .enumerate()
                    .map(|(index, text)| Criterion {
                        id: format!("validation-{index}"),
                        text: text.clone(),
                    })
                    .collect(),
                rubric_dimensions: goal.rubric_dimensions.clone(),
                created_at: OffsetDateTime::UNIX_EPOCH,
                created_by: "validation-only".into(),
                supersedes: None,
            }
            .validate()
            .map_err(anyhow::Error::msg)?;
        }

        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        let approved = request
            .goals
            .iter()
            .map(|goal| self.materialize_goal_version(goal, &local_state, &projection))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let unchanged = approved.len() == projection.approved_goals.len()
            && approved.iter().all(|version| {
                projection.approved_goals.get(&version.goal_id) == Some(&version.id)
            });
        if unchanged {
            return Ok(approved);
        }
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::GoalSetReplaced {
                versions: approved.clone(),
            },
        )?;
        let approval_material = approved
            .iter()
            .map(|version| version.id.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let approval_hash = blake3::hash(approval_material.as_bytes()).to_hex();
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ProductEventRecorded {
                record: product_event_record(
                    workspace_id,
                    ProductEventKind::GoalApproved,
                    format!("goal-approval-{}", &approval_hash[..20]),
                    None,
                    None,
                ),
            },
        )?;
        Ok(approved)
    }

    pub fn approved_goals(&self, workspace_id: &str) -> anyhow::Result<Vec<GoalVersion>> {
        let (_, _, projection) = self.load_parts(workspace_id)?;
        let mut goals = projection
            .approved_goals
            .values()
            .map(|version_id| {
                projection
                    .goal_versions
                    .get(version_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("approved goal version is missing"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        sort_goals(&mut goals);
        Ok(goals)
    }

    pub fn export_word_report(&self, workspace_id: &str, destination: &Path) -> anyhow::Result<()> {
        let (_, access, projection) = self.load_parts(workspace_id)?;
        let workspace_name = if projection.name.trim().is_empty() {
            access.workspace_name.as_str()
        } else {
            projection.name.as_str()
        };
        let analyses = build_report_heatmap(&projection, REPORT_HISTORY_LIMIT);
        if analyses.is_empty() {
            anyhow::bail!("complete an analysis before downloading a Word report");
        }
        crate::export::write_goal_report(workspace_name, &analyses, destination)
    }

    /// Returns one bounded metadata-only slice of active report history.
    /// Full evidence and architecture claims are intentionally omitted.
    pub fn report_history_page(
        &self,
        workspace_id: &str,
        before_event_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<ReportHistoryPage> {
        let (_, projection) = self.workspace_parts(workspace_id)?;
        build_report_history_page(&projection, before_event_id, limit)
    }

    /// Loads one exact finding by immutable completion-event id. This is the
    /// only history API that carries the finding's bounded evidence metadata.
    pub fn report_finding(
        &self,
        workspace_id: &str,
        report_event_id: &str,
        goal_version_id: &str,
    ) -> anyhow::Result<HeatmapWeek> {
        let (_, projection) = self.workspace_parts(workspace_id)?;
        build_report_finding(&projection, report_event_id, goal_version_id)
    }

    /// Appends a logical deletion tombstone. The signed completion remains in
    /// the ledger while active history and every derived projection omit it.
    pub fn delete_report(&self, workspace_id: &str, report_event_id: &str) -> anyhow::Result<()> {
        if report_event_id.trim().is_empty() {
            anyhow::bail!("report event id is required");
        }
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ReportDeleted {
                report_event_id: report_event_id.to_string(),
            },
        )
    }

    pub fn export_recovery(&self, workspace_id: &str, destination: &Path) -> anyhow::Result<()> {
        let (local_state, access, _) = self.load_parts(workspace_id)?;
        if access.role != Role::Editor {
            anyhow::bail!("only an Editor can create a recovery export");
        }
        let payload = RecoveryPayload {
            format: "codecaddie-recovery-payload-v3".into(),
            workspace_id: workspace_id.into(),
            workspace_fingerprint: access.workspace_fingerprint,
            device: local_state.device,
            role: access.role,
            events: self.event_log()?.raw_values(workspace_id)?,
        };
        write_private_new(destination, &serde_json::to_vec_pretty(&payload)?)
    }

    /// Writes a portable, authenticated backup whose key comes only from the
    /// supplied passphrase. The passphrase is never stored in the data root or
    /// an operating-system credential manager.
    pub fn export_portable_backup(
        &self,
        workspace_id: &str,
        destination: &Path,
        passphrase: &str,
    ) -> anyhow::Result<PortableBackupReceipt> {
        let _guard = self.write_lock(workspace_id)?;
        self.export_portable_backup_unlocked(workspace_id, destination, passphrase)
    }

    fn export_portable_backup_unlocked(
        &self,
        workspace_id: &str,
        destination: &Path,
        passphrase: &str,
    ) -> anyhow::Result<PortableBackupReceipt> {
        let (local_state, access, _) = self.load_parts(workspace_id)?;
        if access.role != Role::Editor {
            anyhow::bail!("only an Editor can create a portable backup");
        }
        let events = self.event_log()?.load(workspace_id)?;
        let payload = PortableBackupPayload::new(access, local_state.device, events)?;
        let receipt = PortableBackupReceipt {
            workspace_id: workspace_id.into(),
            event_count: payload.events.len(),
            manifest_blake3: payload.manifest_digest()?,
            format: portable_backup::PORTABLE_BACKUP_FORMAT.into(),
        };
        let sealed = portable_backup::seal(&payload, passphrase)?;
        crate::persistence::write_private_atomic_new(destination, &sealed)?;
        Ok(receipt)
    }

    fn backup_schedule_directory(&self) -> PathBuf {
        self.root.join("backup-schedules-v1")
    }

    fn backup_schedule_path(&self, workspace_id: &str) -> PathBuf {
        self.backup_schedule_directory().join(format!(
            "{}.json",
            blake3::hash(workspace_id.as_bytes()).to_hex()
        ))
    }

    fn load_backup_schedule(
        &self,
        workspace_id: &str,
    ) -> anyhow::Result<Option<BackupScheduleConfig>> {
        let path = self.backup_schedule_path(workspace_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = read_encrypted_migrating(&path, &self.content_cipher, BACKUP_SCHEDULE_PURPOSE)?;
        let config: BackupScheduleConfig = serde_json::from_slice(&bytes)?;
        config.validate(workspace_id)?;
        Ok(Some(config))
    }

    pub fn backup_schedule_status(
        &self,
        workspace_id: &str,
    ) -> anyhow::Result<BackupScheduleStatus> {
        let _ = self.workspace_parts(workspace_id)?;
        Ok(self
            .load_backup_schedule(workspace_id)?
            .map(|config| config.status())
            .unwrap_or_else(disabled_backup_schedule))
    }

    pub fn enable_backup_schedule(
        &self,
        workspace_id: &str,
        destination_directory: &Path,
        passphrase: &str,
    ) -> anyhow::Result<ScheduledBackupRunReceipt> {
        portable_backup::validate_passphrase(passphrase)?;
        let metadata = fs::symlink_metadata(destination_directory)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            anyhow::bail!("portable backup destination must be a regular directory");
        }
        let destination = destination_directory.canonicalize()?;
        let data_root = self.root.canonicalize()?;
        let (access, _) = self.workspace_parts(workspace_id)?;
        let repository = Path::new(&access.repository_path).canonicalize()?;
        if destination.starts_with(&data_root) || destination.starts_with(&repository) {
            anyhow::bail!(
                "portable backups must be stored outside the CodeCaddie data root and attached repository"
            );
        }
        let _guard = self.write_lock(workspace_id)?;
        let schedule = BackupScheduleConfig {
            format: BACKUP_SCHEDULE_FORMAT.into(),
            workspace_id: workspace_id.into(),
            destination_directory: destination.to_string_lossy().into_owned(),
            passphrase: passphrase.into(),
            last_successful_at_unix: None,
        };
        schedule.validate(workspace_id)?;
        let schedule_directory = self.backup_schedule_directory();
        fs::create_dir_all(&schedule_directory)?;
        #[cfg(unix)]
        fs::set_permissions(&schedule_directory, fs::Permissions::from_mode(0o700))?;
        write_encrypted_replace(
            &self.backup_schedule_path(workspace_id),
            &serde_json::to_vec(&schedule)?,
            &self.content_cipher,
            BACKUP_SCHEDULE_PURPOSE,
        )?;
        drop(_guard);
        self.run_scheduled_backup_at(workspace_id, OffsetDateTime::now_utc(), true)
    }

    pub fn disable_backup_schedule(
        &self,
        workspace_id: &str,
    ) -> anyhow::Result<BackupScheduleStatus> {
        let _guard = self.write_lock(workspace_id)?;
        let _ = self.workspace_parts(workspace_id)?;
        let path = self.backup_schedule_path(workspace_id);
        match fs::remove_file(&path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    crate::persistence::sync_parent(parent)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(disabled_backup_schedule())
    }

    pub fn run_scheduled_backup(
        &self,
        workspace_id: &str,
        force: bool,
    ) -> anyhow::Result<ScheduledBackupRunReceipt> {
        self.run_scheduled_backup_at(workspace_id, OffsetDateTime::now_utc(), force)
    }

    fn run_scheduled_backup_at(
        &self,
        workspace_id: &str,
        now: OffsetDateTime,
        force: bool,
    ) -> anyhow::Result<ScheduledBackupRunReceipt> {
        let _guard = self.write_lock(workspace_id)?;
        let Some(mut schedule) = self.load_backup_schedule(workspace_id)? else {
            return Ok(ScheduledBackupRunReceipt {
                status: "disabled".into(),
                schedule: disabled_backup_schedule(),
                backup_file_name: None,
                manifest_blake3: None,
            });
        };
        let now_unix = now.unix_timestamp();
        let due = schedule
            .last_successful_at_unix
            .is_none_or(|last| now_unix.saturating_sub(last) >= BACKUP_CADENCE_SECONDS);
        if !force && !due {
            return Ok(ScheduledBackupRunReceipt {
                status: "not_due".into(),
                schedule: schedule.status(),
                backup_file_name: None,
                manifest_blake3: None,
            });
        }
        let prefix = format!(
            "codecaddie-{}-",
            &blake3::hash(workspace_id.as_bytes()).to_hex()[..16]
        );
        let file_name = format!("{prefix}{now_unix}-{}.codecaddie-backup", Uuid::now_v7());
        let destination = Path::new(&schedule.destination_directory).join(&file_name);
        let backup =
            self.export_portable_backup_unlocked(workspace_id, &destination, &schedule.passphrase)?;
        schedule.last_successful_at_unix = Some(now_unix);
        write_encrypted_replace(
            &self.backup_schedule_path(workspace_id),
            &serde_json::to_vec(&schedule)?,
            &self.content_cipher,
            BACKUP_SCHEDULE_PURPOSE,
        )?;
        prune_scheduled_backups(
            Path::new(&schedule.destination_directory),
            &prefix,
            BACKUP_RETENTION_COUNT,
        )?;
        Ok(ScheduledBackupRunReceipt {
            status: "created".into(),
            schedule: schedule.status(),
            backup_file_name: Some(file_name),
            manifest_blake3: Some(backup.manifest_blake3),
        })
    }

    /// Imports a portable backup after decrypting, authenticating, validating
    /// its manifest, replaying every signed event, and confirming its editing
    /// key. The event history is committed first; an interruption leaves an
    /// unreachable exact history that the same import can safely finish.
    pub fn import_portable_backup(
        &self,
        source: &Path,
        repository_path: &Path,
        passphrase: &str,
    ) -> anyhow::Result<PortableRestoreReceipt> {
        self.import_portable_backup_with(source, repository_path, passphrase, || Ok(()))
    }

    fn import_portable_backup_with(
        &self,
        source: &Path,
        repository_path: &Path,
        passphrase: &str,
        after_event_commit: impl FnOnce() -> anyhow::Result<()>,
    ) -> anyhow::Result<PortableRestoreReceipt> {
        let metadata = fs::symlink_metadata(source)?;
        if !metadata.file_type().is_file() {
            anyhow::bail!("portable backup source must be a regular file");
        }
        if metadata.len() > MAX_BACKUP_BYTES as u64 {
            anyhow::bail!("portable backup exceeds the 64 MiB limit");
        }
        let payload = portable_backup::open(&fs::read(source)?, passphrase)?;
        let _projection = payload.validate()?;
        let repository = LocalRepository::attach("restored-repository", repository_path)?;
        let repository_path = repository.path.to_string_lossy().into_owned();
        let workspace_id = payload.workspace.workspace_id.clone();
        let manifest_blake3 = payload.manifest_digest()?;

        // Always take the device-wide lock before the per-workspace lock. No
        // current mutation takes them in the opposite order.
        let _local_state_guard = self.local_state_write_lock()?;
        let _workspace_guard = self.write_lock(&workspace_id)?;
        let mut local_state = if self.local_state.exists() {
            self.load_local_state()?
        } else {
            LocalState::new()?
        };
        let restored_identity = payload.device.public_identity()?;
        if local_state.workspaces.is_empty() {
            local_state.device = payload.device.clone();
        } else if local_state.device.public_identity()? != restored_identity {
            anyhow::bail!(
                "portable backup belongs to another editing identity; import it into a fresh CodeCaddie data profile"
            );
        }

        let mut restored_access = payload.workspace.clone();
        restored_access.repository_path = repository_path;
        restored_access.project_context.context_file_paths.clear();
        for reference in &mut restored_access.project_context.context_files {
            reference.path.clear();
        }
        if let Some(existing) = local_state.workspaces.get(&workspace_id)
            && (existing.workspace_fingerprint != restored_access.workspace_fingerprint
                || existing.role != Role::Editor)
        {
            anyhow::bail!("a different workspace already uses this portable backup identity");
        }

        self.event_log()?
            .restore_exact(&workspace_id, &payload.events)?;
        after_event_commit()?;
        local_state
            .workspaces
            .insert(workspace_id.clone(), restored_access);
        local_state.upgrade_format();
        self.local_state.save(&local_state)?;
        write_encrypted_replace(
            &self.recent_path(),
            workspace_id.as_bytes(),
            &self.content_cipher,
            RECENT_WORKSPACE_PURPOSE,
        )?;
        Ok(PortableRestoreReceipt {
            workspace_id,
            event_count: payload.events.len(),
            manifest_blake3,
            status: "restored".into(),
        })
    }

    /// The channel data root backing this store. The agent gateway derives
    /// its session store and file-exchange directories from it.
    pub(crate) fn data_root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn content_cipher(&self) -> &ContentCipher {
        &self.content_cipher
    }

    pub(crate) fn workspace_parts(
        &self,
        workspace_id: &str,
    ) -> anyhow::Result<(LocalWorkspaceAccess, WorkspaceProjection)> {
        let (_, access, projection) = self.load_parts(workspace_id)?;
        Ok((access, projection))
    }

    fn codebase_map_directory(&self) -> PathBuf {
        self.root.join("codebase-maps-v1")
    }

    fn codebase_map_body_path(&self, content_hash: &str) -> PathBuf {
        self.codebase_map_directory()
            .join(format!("{content_hash}.json"))
    }

    /// Records a validated codebase map: the signed descriptor enters the
    /// event ledger and the content-addressed body is written beside the
    /// log, owner-only and atomic. Superseded bodies beyond the newest four
    /// for this workspace are pruned — the descriptor history remains in
    /// the append-only log.
    pub fn record_codebase_map(
        &self,
        workspace_id: &str,
        map: &CodebaseMap,
    ) -> anyhow::Result<CodebaseMapDescriptor> {
        let descriptor = CodebaseMapDescriptor::for_map(map)?;
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        let directory = self.codebase_map_directory();
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let body_path = self.codebase_map_body_path(&descriptor.content_hash);
        if !body_path.exists() {
            write_encrypted_atomic_new(
                &body_path,
                &serde_json::to_vec(map)?,
                &self.content_cipher,
                CODEBASE_MAP_PURPOSE,
            )?;
        }
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::CodebaseMapRecorded {
                descriptor: descriptor.clone(),
            },
        )?;
        let mut recorded = projection
            .codebase_maps
            .values()
            .cloned()
            .collect::<Vec<_>>();
        recorded.sort_by_key(|item| std::cmp::Reverse(item.generated_at));
        for stale in recorded.iter().skip(4) {
            if stale.content_hash != descriptor.content_hash {
                let _ = fs::remove_file(self.codebase_map_body_path(&stale.content_hash));
            }
        }
        Ok(descriptor)
    }

    /// Loads the newest recorded map for the given frozen repository set,
    /// re-verifying the body's BLAKE3 content hash against the signed
    /// descriptor before use. A missing or tampered body degrades to `None`
    /// — the caller regenerates instead of failing.
    pub fn load_codebase_map_for(
        &self,
        workspace_id: &str,
        frozen: &[FrozenRepository],
    ) -> anyhow::Result<Option<CodebaseMap>> {
        let (_, projection) = self.workspace_parts(workspace_id)?;
        let Some(descriptor) = projection.latest_map_for(frozen) else {
            return Ok(None);
        };
        self.load_codebase_map_body(descriptor)
    }

    /// Loads a recorded map by id (or the newest recorded map when no id is
    /// given), with the same hash verification and degrade-to-`None`
    /// posture.
    pub fn load_codebase_map(
        &self,
        workspace_id: &str,
        map_id: Option<&str>,
    ) -> anyhow::Result<Option<(CodebaseMapDescriptor, CodebaseMap)>> {
        let (_, projection) = self.workspace_parts(workspace_id)?;
        let descriptor = match map_id {
            Some(map_id) => projection.codebase_maps.get(map_id),
            None => projection
                .codebase_maps
                .values()
                .max_by_key(|descriptor| descriptor.generated_at),
        };
        let Some(descriptor) = descriptor else {
            return Ok(None);
        };
        Ok(self
            .load_codebase_map_body(descriptor)?
            .map(|map| (descriptor.clone(), map)))
    }

    fn load_codebase_map_body(
        &self,
        descriptor: &CodebaseMapDescriptor,
    ) -> anyhow::Result<Option<CodebaseMap>> {
        let path = self.codebase_map_body_path(&descriptor.content_hash);
        let Ok(bytes) = read_encrypted_migrating(&path, &self.content_cipher, CODEBASE_MAP_PURPOSE)
        else {
            return Ok(None);
        };
        let map: CodebaseMap = match serde_json::from_slice(&bytes) {
            Ok(map) => map,
            Err(_) => return Ok(None),
        };
        if map.content_hash()? != descriptor.content_hash {
            return Ok(None);
        }
        Ok(Some(map))
    }

    pub fn record_report(&self, workspace_id: &str, report: Report) -> anyhow::Result<()> {
        let (access, _) = self.workspace_parts(workspace_id)?;
        if report.repositories.len() != 1 {
            anyhow::bail!("every frozen repository needs one local persistence verifier");
        }
        let repository = LocalRepository::attach(
            &report.repositories[0].repository_id,
            &access.repository_path,
        )
        .map_err(|_| {
            anyhow::anyhow!("the registered repository is unavailable for report verification")
        })?;
        self.record_report_with_repositories(workspace_id, report, &[repository])
    }

    /// Persists a report after independently re-resolving its evidence in
    /// every local repository that participated in the frozen analysis.
    pub fn record_report_with_repositories(
        &self,
        workspace_id: &str,
        report: Report,
        repositories: &[LocalRepository],
    ) -> anyhow::Result<()> {
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        crate::report_integrity::validate_report_for_persistence(
            &report,
            &projection,
            repositories,
        )?;
        let had_prior_report = !projection.reports.is_empty();
        let created_at = self
            .event_log()?
            .load(workspace_id)?
            .into_iter()
            .find_map(|envelope| {
                matches!(envelope.event, DomainEvent::WorkspaceCreated { .. })
                    .then_some(envelope.occurred_at)
            });
        let report_id = report.id.clone();
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ReportCompleted {
                report: report.clone(),
            },
        )?;
        for kind in [
            ProductEventKind::ScorecardGenerated,
            ProductEventKind::ReportSaved,
        ] {
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ProductEventRecorded {
                    record: product_event_record(
                        workspace_id,
                        kind,
                        report_id.clone(),
                        Some(report_id.clone()),
                        None,
                    ),
                },
            )?;
        }
        if !had_prior_report {
            let elapsed_milliseconds = created_at.and_then(|created| {
                let elapsed = (report.completed_at - created).whole_milliseconds();
                (elapsed >= 0).then(|| u64::try_from(elapsed).unwrap_or(u64::MAX))
            });
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ProductEventRecorded {
                    record: product_event_record(
                        workspace_id,
                        ProductEventKind::TimeToFirstSavedReport,
                        report_id.clone(),
                        Some(report_id.clone()),
                        elapsed_milliseconds,
                    ),
                },
            )?;
        } else {
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ProductEventRecorded {
                    record: product_event_record(
                        workspace_id,
                        ProductEventKind::ComparisonGenerated,
                        report_id.clone(),
                        Some(report_id.clone()),
                        None,
                    ),
                },
            )?;
        }
        let ready: Vec<_> = projection
            .actions
            .values()
            .filter(|action| action.status == ActionStatus::ReadyForVerification)
            .cloned()
            .collect();
        for action in ready {
            let affected_goals = projection
                .reports
                .values()
                .flat_map(|prior| prior.recommendations.iter())
                .find(|recommendation| recommendation.id == action.recommendation_id)
                .map(|recommendation| recommendation.goal_version_ids.clone())
                .unwrap_or_default();
            let supported = !affected_goals.is_empty()
                && affected_goals.iter().all(|goal_id| {
                    report.assessments.iter().any(|assessment| {
                        assessment.goal_version_id == *goal_id
                            && assessment.verdict == Verdict::Supported
                    })
                });
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ActionTransitioned {
                    action_id: action.id,
                    from: ActionStatus::ReadyForVerification,
                    to: if supported {
                        ActionStatus::Verified
                    } else {
                        ActionStatus::Reopened
                    },
                    note: Some(if supported {
                        format!("Verified by report {}", report.id)
                    } else {
                        format!(
                            "Reopened by report {}: the goal is not fully supported",
                            report.id
                        )
                    }),
                },
            )?;
        }
        Ok(())
    }

    /// Records a scan start and, when a report already exists, a separate
    /// repeat-analysis marker in the same signed local ledger.
    pub fn record_analysis_started(
        &self,
        workspace_id: &str,
        analysis_session_id: &str,
    ) -> anyhow::Result<()> {
        if analysis_session_id.trim().is_empty() {
            anyhow::bail!("analysis session id is required");
        }
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        let repeat = !projection.reports.is_empty();
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ProductEventRecorded {
                record: product_event_record(
                    workspace_id,
                    ProductEventKind::AnalysisStarted,
                    analysis_session_id,
                    Some(analysis_session_id.to_string()),
                    None,
                ),
            },
        )?;
        if repeat {
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ProductEventRecorded {
                    record: product_event_record(
                        workspace_id,
                        ProductEventKind::RepeatAnalysisStarted,
                        analysis_session_id,
                        Some(analysis_session_id.to_string()),
                        None,
                    ),
                },
            )?;
        }
        Ok(())
    }

    /// Records a content-free local product action. Callers select the kind;
    /// the signed envelope supplies the timestamp and workspace provenance.
    pub fn record_decision_funnel_event(
        &self,
        workspace_id: &str,
        kind: DecisionFunnelEventKind,
    ) -> anyhow::Result<()> {
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::DecisionFunnelEventRecorded { kind },
        )
        .map(|_| ())
    }

    /// Records one versioned, content-free lifecycle event in the existing
    /// signed workspace ledger. Interaction sessions deliberately use opaque
    /// identifiers supplied by the caller; no user-authored content enters
    /// this record.
    pub fn record_product_event(
        &self,
        workspace_id: &str,
        kind: ProductEventKind,
        session_id: &str,
        report_id: Option<String>,
    ) -> anyhow::Result<()> {
        if session_id.trim().is_empty() {
            anyhow::bail!("product event session id is required");
        }
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ProductEventRecorded {
                record: product_event_record(workspace_id, kind, session_id, report_id, None),
            },
        )
        .map(|_| ())
    }

    /// Records one request outcome plus any policy-derived SLO alert in the
    /// existing signed ledger. A telemetry write failure is returned to the
    /// caller so the response can explicitly report degraded measurement.
    pub fn record_reliability_operation(
        &self,
        workspace_id: &str,
        record: ReliabilityEventRecord,
    ) -> anyhow::Result<()> {
        record.validate().map_err(anyhow::Error::msg)?;
        let alert = crate::reliability::alert_for(&record)?;
        debug_assert!(
            record.kind != ReliabilityEventKind::OperationCompleted
                || crate::reliability::trace_span_for(&record).is_some()
        );
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ReliabilityEventRecorded { record },
        )?;
        if let Some(alert) = alert {
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ReliabilityEventRecorded { record: alert },
            )?;
        }
        Ok(())
    }

    /// Starts or ends a desktop runtime session. A start consumes only the
    /// Native SDK's actual panic marker; an unmatched prior session is never
    /// inferred to be a crash because normal shutdown cannot await effects.
    pub fn record_desktop_session(
        &self,
        workspace_id: &str,
        kind: ReliabilityEventKind,
        session_id: &str,
    ) -> anyhow::Result<(bool, String)> {
        let panic_marker = RuntimeChannel::detect().native_panic_marker_path();
        self.record_desktop_session_with_panic_marker(
            workspace_id,
            kind,
            session_id,
            panic_marker.as_deref(),
        )
    }

    pub(crate) fn record_desktop_session_with_panic_marker(
        &self,
        workspace_id: &str,
        kind: ReliabilityEventKind,
        session_id: &str,
        panic_marker: Option<&Path>,
    ) -> anyhow::Result<(bool, String)> {
        if !matches!(
            kind,
            ReliabilityEventKind::DesktopSessionStarted | ReliabilityEventKind::DesktopSessionEnded
        ) {
            anyhow::bail!("desktop session recording requires a start or end event");
        }
        let correlation_id = crate::reliability::new_correlation_id();
        let record = crate::reliability::session_record(kind, session_id, correlation_id.clone());
        record.validate().map_err(anyhow::Error::msg)?;
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        let claimed_panic_marker = if kind == ReliabilityEventKind::DesktopSessionStarted {
            panic_marker
                .map(claim_native_panic_marker)
                .transpose()?
                .flatten()
        } else {
            None
        };
        let crash_detected = claimed_panic_marker.is_some();
        if crash_detected {
            let crash = crate::reliability::native_panic_record(
                session_id,
                crate::reliability::new_correlation_id(),
            );
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ReliabilityEventRecorded { record: crash },
            )?;
            if let Some(alert) = crate::reliability::crash_alert(session_id)? {
                self.append(
                    workspace_id,
                    &access,
                    &local_state.device,
                    &mut projection,
                    DomainEvent::ReliabilityEventRecorded { record: alert },
                )?;
            }
        }
        self.append(
            workspace_id,
            &access,
            &local_state.device,
            &mut projection,
            DomainEvent::ReliabilityEventRecorded { record },
        )?;
        if let Some(claimed) = claimed_panic_marker {
            fs::remove_file(claimed)?;
        }
        Ok((crash_detected, correlation_id))
    }

    pub fn record_client_cancellation(
        &self,
        workspace_id: &str,
        operation: &str,
    ) -> anyhow::Result<String> {
        let correlation_id = crate::reliability::new_correlation_id();
        let record = crate::reliability::operation_record(
            correlation_id.clone(),
            operation,
            ReliabilityOutcome::Cancelled,
            None,
            true,
            0,
        );
        self.record_reliability_operation(workspace_id, record)?;
        Ok(correlation_id)
    }

    pub fn ready_action(
        &self,
        workspace_id: &str,
        request: ReadyActionRequest,
    ) -> anyhow::Result<ActionProjection> {
        if request.recommendation_id.trim().is_empty()
            || request.title.trim().is_empty()
            || request.note.trim().is_empty()
        {
            anyhow::bail!("recommendation, title, and completion note are required");
        }
        let _guard = self.write_lock(workspace_id)?;
        let (local_state, access, mut projection) = self.load_parts(workspace_id)?;
        let action_id = format!(
            "action-{}",
            &blake3::hash(request.recommendation_id.as_bytes()).to_hex()[..20]
        );
        if !projection.actions.contains_key(&action_id) {
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ActionCreated {
                    action_id: action_id.clone(),
                    recommendation_id: request.recommendation_id,
                    title: request.title,
                },
            )?;
        }
        loop {
            let current = projection
                .actions
                .get(&action_id)
                .ok_or_else(|| anyhow::anyhow!("action projection is missing"))?
                .status;
            let next = match current {
                ActionStatus::Open | ActionStatus::Deferred => ActionStatus::Planned,
                ActionStatus::Planned | ActionStatus::Reopened => ActionStatus::InProgress,
                ActionStatus::InProgress => ActionStatus::ReadyForVerification,
                ActionStatus::Verified | ActionStatus::Dismissed => ActionStatus::Reopened,
                ActionStatus::ReadyForVerification => break,
            };
            self.append(
                workspace_id,
                &access,
                &local_state.device,
                &mut projection,
                DomainEvent::ActionTransitioned {
                    action_id: action_id.clone(),
                    from: current,
                    to: next,
                    note: (next == ActionStatus::ReadyForVerification)
                        .then(|| request.note.clone()),
                },
            )?;
        }
        projection
            .actions
            .get(&action_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("action projection is missing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_domain::{
        CriterionAssessment, DeviceIdentity, EvidenceKind, GoalAssessment, ReportOrigin,
    };
    use ed25519_dalek::SigningKey;
    use std::process::Command;

    fn test_repository(root: &Path, name: &str) -> (String, String) {
        let path = root.join(name);
        fs::create_dir_all(&path).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "CodeCaddie Test"]);
        fs::write(path.join("proof.txt"), "repository-verifiable proof\n").unwrap();
        git(&["add", "proof.txt"]);
        git(&["commit", "-qm", "proof"]);
        let commit = git(&["rev-parse", "HEAD"]);
        (path.to_string_lossy().into_owned(), commit)
    }

    fn commit_repository_file(path: &Path, name: &str, contents: &str) -> String {
        fs::write(path.join(name), contents).unwrap();
        for args in [
            ["add", name].as_slice(),
            ["commit", "-qm", "next proof"].as_slice(),
        ] {
            let output = Command::new("git")
                .arg("-C")
                .arg(path)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn unverified_report(
        id: &str,
        completed_at: OffsetDateTime,
        goals: &[GoalVersion],
        commit: &str,
    ) -> Report {
        Report {
            id: id.into(),
            completed_at,
            repositories: vec![FrozenRepository {
                repository_id: "attached-repository".into(),
                commit_sha: commit.into(),
            }],
            goal_version_ids: goals.iter().map(|goal| goal.id.clone()).collect(),
            goal_set_hash: blake3::hash(&serde_json::to_vec(goals).unwrap())
                .to_hex()
                .to_string(),
            provider: "test".into(),
            provider_version: "test-1".into(),
            origin: ReportOrigin::Scan,
            assessments: goals
                .iter()
                .map(|goal| GoalAssessment {
                    goal_version_id: goal.id.clone(),
                    verdict: Verdict::Unverified,
                    summary: "The test report remains explicitly unverified.".into(),
                    architecture_narrative: String::new(),
                    related_component_ids: vec![],
                    criteria: goal
                        .criteria
                        .iter()
                        .map(|criterion| CriterionAssessment {
                            criterion_id: criterion.id.clone(),
                            verdict: Verdict::Unverified,
                            rationale: "No claim is made without evidence.".into(),
                            confidence: 0.0,
                            evidence: vec![],
                        })
                        .collect(),
                })
                .collect(),
            architecture: vec![],
            recommendations: vec![],
            coverage: None,
            unverified_criteria: goals.iter().map(|goal| goal.criteria.len() as u32).sum(),
            partial: false,
            analysis_warnings: vec![],
            codebase_map_id: None,
            codebase_map_hash: None,
        }
    }

    #[test]
    fn decision_funnel_derives_local_timings_and_never_carries_product_content() {
        let key = SigningKey::from_bytes(&[11; 32]);
        let identity = DeviceIdentity {
            actor_id: "local-actor".into(),
            device_id: "local-device".into(),
            signing_public_key: hex::encode(key.verifying_key().to_bytes()),
            label: "Local device".into(),
        };
        let report = |id: &str, completed_at: OffsetDateTime| Report {
            id: id.into(),
            completed_at,
            repositories: vec![],
            goal_version_ids: vec![],
            goal_set_hash: "metadata-hash".into(),
            provider: "test".into(),
            provider_version: "test".into(),
            origin: ReportOrigin::Scan,
            assessments: vec![],
            architecture: vec![],
            recommendations: vec![],
            coverage: Some(1.0),
            unverified_criteria: 0,
            partial: false,
            analysis_warnings: vec![],
            codebase_map_id: None,
            codebase_map_hash: None,
        };
        let moments = [0, 1, 2, 10, 11, 20, 20, 21, 30, 31, 32];
        let domain_events = vec![
            DomainEvent::WorkspaceCreated {
                name: "Workspace".into(),
                founding_device: identity.clone(),
                workspace_fingerprint: "fingerprint".into(),
            },
            DomainEvent::GoalSetReplaced { versions: vec![] },
            DomainEvent::DecisionFunnelEventRecorded {
                kind: DecisionFunnelEventKind::AnalysisStarted,
            },
            DomainEvent::ReportCompleted {
                report: report(
                    "report-1",
                    OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(10),
                ),
            },
            DomainEvent::DecisionFunnelEventRecorded {
                kind: DecisionFunnelEventKind::ReportOpened,
            },
            DomainEvent::DecisionFunnelEventRecorded {
                kind: DecisionFunnelEventKind::AnalysisStarted,
            },
            DomainEvent::DecisionFunnelEventRecorded {
                kind: DecisionFunnelEventKind::RepeatAnalysis,
            },
            DomainEvent::GoalSetReplaced { versions: vec![] },
            DomainEvent::ReportCompleted {
                report: report(
                    "report-2",
                    OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30),
                ),
            },
            DomainEvent::DecisionFunnelEventRecorded {
                kind: DecisionFunnelEventKind::ReportOpened,
            },
            DomainEvent::DecisionFunnelEventRecorded {
                kind: DecisionFunnelEventKind::PromptCopied,
            },
        ];
        let events = domain_events
            .into_iter()
            .zip(moments)
            .enumerate()
            .map(|(index, (event, second))| {
                EventEnvelope::sign(
                    "workspace".into(),
                    index as u64 + 1,
                    OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(second),
                    identity.device_id.clone(),
                    identity.actor_id.clone(),
                    INITIAL_EPOCH,
                    event,
                    &key,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let projection = WorkspaceProjection {
            applied_event_ids: events
                .iter()
                .map(|event| event.event_id.to_string())
                .collect(),
            ..Default::default()
        };

        let summary = build_decision_funnel_summary(&events, &projection);
        assert_eq!(
            summary,
            DecisionFunnelSummary {
                workspace_creations: 1,
                goal_approvals: 2,
                analysis_starts: 2,
                analysis_completions: 2,
                report_opens: 2,
                prompt_copies: 1,
                repeat_analyses: 1,
                repeat_review_opens: 1,
                scorecards_generated: 0,
                reports_saved: 0,
                evidence_opens: 0,
                comparisons_generated: 0,
                time_to_first_report_seconds: Some(10),
                decision_cycle_average_seconds: Some(9),
                decision_cycles: 2,
            }
        );

        let marker_json = serde_json::to_string(&DomainEvent::DecisionFunnelEventRecorded {
            kind: DecisionFunnelEventKind::PromptCopied,
        })
        .unwrap();
        assert_eq!(
            marker_json,
            r#"{"type":"decision_funnel_event_recorded","data":{"kind":"prompt_copied"}}"#
        );
        for sentinel in [
            "PRIVATE SOURCE SENTINEL",
            "/secret/repository/path",
            "confidential goal text",
            "attachment contents",
        ] {
            assert!(!marker_json.contains(sentinel));
        }
    }

    #[test]
    fn privacy_adversarial_product_event_contract_is_content_free_and_local() {
        let record = product_event_record(
            "workspace",
            ProductEventKind::EvidenceOpened,
            "opaque-session",
            Some("report-1".into()),
            None,
        );
        let json = serde_json::to_string(&DomainEvent::ProductEventRecorded {
            record: record.clone(),
        })
        .unwrap();
        assert_eq!(record.schema_version, 2);
        assert_eq!(record.workspace_id, "workspace");
        assert_eq!(record.session_id, "opaque-session");
        assert!(record.platform.contains('-'));
        assert!(record.cohort.starts_with("desktop-"));
        for forbidden in [
            "repositoryPath",
            "repositorySource",
            "attachmentContent",
            "goalText",
            "prompt",
            "freeText",
            "PRIVATE_SOURCE_CANARY",
        ] {
            assert!(!json.contains(forbidden));
        }

        let key = SigningKey::from_bytes(&[12; 32]);
        let identity = DeviceIdentity {
            actor_id: "local-actor".into(),
            device_id: "local-device".into(),
            signing_public_key: hex::encode(key.verifying_key().to_bytes()),
            label: "Local device".into(),
        };
        let events = [
            ProductEventKind::AnalysisStarted,
            ProductEventKind::RepeatAnalysisStarted,
            ProductEventKind::ScorecardGenerated,
            ProductEventKind::ReportSaved,
            ProductEventKind::ReportRevisited,
            ProductEventKind::EvidenceOpened,
            ProductEventKind::ComparisonGenerated,
            ProductEventKind::PromptCopied,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            EventEnvelope::sign(
                "workspace".into(),
                index as u64 + 1,
                OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(index as i64),
                identity.device_id.clone(),
                identity.actor_id.clone(),
                INITIAL_EPOCH,
                DomainEvent::ProductEventRecorded {
                    record: product_event_record(
                        "workspace",
                        kind,
                        "report-1",
                        Some("report-1".into()),
                        None,
                    ),
                },
                &key,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
        let projection = WorkspaceProjection {
            applied_event_ids: events
                .iter()
                .map(|event| event.event_id.to_string())
                .collect(),
            ..Default::default()
        };
        let summary = build_decision_funnel_summary(&events, &projection);
        assert_eq!(summary.analysis_starts, 1);
        assert_eq!(summary.repeat_analyses, 1);
        assert_eq!(summary.scorecards_generated, 1);
        assert_eq!(summary.reports_saved, 1);
        assert_eq!(summary.report_opens, 1);
        assert_eq!(summary.evidence_opens, 1);
        assert_eq!(summary.comparisons_generated, 1);
        assert_eq!(summary.prompt_copies, 1);
    }

    #[test]
    fn privacy_adversarial_crash_markers_become_content_free_reliability_events() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Product".into(),
                repository_display_name: "product".into(),
                repository_path: "/local/product".into(),
                product_brief: "A repository-verifiable product.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        let succeeded = crate::reliability::operation_record(
            crate::reliability::new_correlation_id(),
            "scan.run",
            ReliabilityOutcome::Succeeded,
            None,
            false,
            100,
        );
        store
            .record_reliability_operation(&workspace.workspace_id, succeeded)
            .unwrap();
        let failed = crate::reliability::operation_record(
            crate::reliability::new_correlation_id(),
            "scan.run",
            ReliabilityOutcome::Failed,
            Some("provider_timeout"),
            true,
            800,
        );
        store
            .record_reliability_operation(&workspace.workspace_id, failed)
            .unwrap();
        store
            .record_client_cancellation(&workspace.workspace_id, "scan.run")
            .unwrap();

        // A schema-1 inferred event remains readable for signed-history
        // compatibility but must not affect the authoritative aggregate.
        let legacy_inferred = crate::reliability::session_record(
            ReliabilityEventKind::DesktopCrashDetected,
            "legacy-unmatched-session",
            crate::reliability::new_correlation_id(),
        );
        store
            .record_reliability_operation(&workspace.workspace_id, legacy_inferred)
            .unwrap();
        let mut legacy_alert = crate::reliability::session_record(
            ReliabilityEventKind::SloAlertRaised,
            "legacy-unmatched-session",
            crate::reliability::new_correlation_id(),
        );
        legacy_alert.alert_code = Some("desktop_crash_detected".into());
        store
            .record_reliability_operation(&workspace.workspace_id, legacy_alert)
            .unwrap();

        let panic_marker = directory.path().join("native-logs/last-panic.txt");
        fs::create_dir_all(panic_marker.parent().unwrap()).unwrap();
        fs::write(&panic_marker, "PRIVATE SOURCE SENTINEL panic detail").unwrap();
        let (first_crash, _) = store
            .record_desktop_session_with_panic_marker(
                &workspace.workspace_id,
                ReliabilityEventKind::DesktopSessionStarted,
                "desktop-session-one",
                Some(&panic_marker),
            )
            .unwrap();
        assert!(first_crash, "an actual Native SDK panic marker is detected");
        assert!(!panic_marker.exists());
        assert!(!panic_marker.with_file_name("last-panic.pending").exists());
        let (second_crash, _) = store
            .record_desktop_session_with_panic_marker(
                &workspace.workspace_id,
                ReliabilityEventKind::DesktopSessionStarted,
                "desktop-session-two",
                Some(&panic_marker),
            )
            .unwrap();
        assert!(
            !second_crash,
            "an unmatched prior start is lifecycle data, not crash evidence"
        );
        store
            .record_desktop_session_with_panic_marker(
                &workspace.workspace_id,
                ReliabilityEventKind::DesktopSessionEnded,
                "desktop-session-two",
                Some(&panic_marker),
            )
            .unwrap();

        let recent = store.recent_workspace().unwrap().unwrap();
        let summary = recent.reliability;
        assert_eq!(summary.operation_samples, 3);
        assert_eq!(summary.trace_spans_recorded, 3);
        assert_eq!(summary.operation_failures, 1);
        assert_eq!(summary.operation_cancellations, 1);
        assert_eq!(summary.provider_operation_samples, 3);
        assert_eq!(summary.provider_operation_failures, 1);
        assert_eq!(summary.provider_alerts_raised, 1);
        assert_eq!(summary.alerts_raised, 2);
        assert_eq!(summary.desktop_sessions_started, 2);
        assert_eq!(summary.desktop_sessions_ended, 1);
        assert_eq!(summary.desktop_crashes_detected, 1);
        assert_eq!(summary.average_latency_milliseconds, Some(300));
        assert_eq!(summary.crash_free_sessions_percent, Some(50.0));
        assert!(summary.availability_percent.unwrap() > 33.0);
        assert!(summary.availability_percent.unwrap() < 34.0);

        let json = serde_json::to_string(&summary).unwrap();
        for forbidden in [
            "PRIVATE SOURCE SENTINEL",
            "/local/product",
            "repositorySource",
            "attachmentContent",
            "goalText",
            "prompt",
            "freeText",
        ] {
            assert!(!json.contains(forbidden));
        }
    }

    #[test]
    fn operational_fault_matrix_records_failures_metrics_and_alerts_without_content() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Product".into(),
                repository_display_name: "product".into(),
                repository_path: "/local/product".into(),
                product_brief: "A repository-verifiable product.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        for (operation, code) in [
            ("scan.run", "provider_timeout"),
            ("scan.run", "provider_response_invalid"),
            ("workspace.recent", "storage_write_failed"),
            ("workspace.recent", "persistence_interrupted"),
        ] {
            store
                .record_reliability_operation(
                    &workspace.workspace_id,
                    crate::reliability::operation_record(
                        crate::reliability::new_correlation_id(),
                        operation,
                        ReliabilityOutcome::Failed,
                        Some(code),
                        true,
                        25,
                    ),
                )
                .unwrap();
        }

        let recent = store.recent_workspace().unwrap().unwrap();
        assert_eq!(recent.reliability.operation_samples, 4);
        assert_eq!(recent.reliability.operation_failures, 4);
        assert_eq!(recent.reliability.alerts_raised, 4);
        assert_eq!(recent.reliability.availability_percent, Some(0.0));
        let serialized = serde_json::to_string(&recent.reliability).unwrap();
        for forbidden in [
            "PRIVATE SOURCE SENTINEL",
            "/local/product",
            "repositorySource",
            "attachmentContent",
            "goalText",
            "prompt",
            "freeText",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn privacy_adversarial_runtime_lifecycle_emits_stable_workspace_and_session_identity() {
        let directory = tempfile::tempdir().unwrap();
        let (repository_path, commit) = test_repository(directory.path(), "analysis-repository");
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Product".into(),
                repository_display_name: "product".into(),
                repository_path,
                product_brief: "A repository-verifiable product.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        let goal = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "goal".into(),
                    title: "Ship safely".into(),
                    business_outcome: "Keep releases trustworthy.".into(),
                    criteria: vec!["A version-controlled release gate is enforced.".into()],
                    priority: 5,
                    position: 1,
                    rubric_dimensions: vec!["Reliability".into()],
                },
            )
            .unwrap();
        store
            .record_analysis_started(&workspace.workspace_id, "analysis-first")
            .unwrap();
        store
            .record_report(
                &workspace.workspace_id,
                unverified_report("report-1", OffsetDateTime::now_utc(), &[goal], &commit),
            )
            .unwrap();
        store
            .record_analysis_started(&workspace.workspace_id, "analysis-repeat")
            .unwrap();
        let recent = store.recent_workspace().unwrap().unwrap();
        assert_eq!(recent.decision_funnel.analysis_starts, 2);
        assert_eq!(recent.decision_funnel.repeat_analyses, 1);
        assert_eq!(recent.decision_funnel.analysis_completions, 1);
        assert_eq!(recent.decision_funnel.goal_approvals, 1);

        let product_records = store
            .event_log()
            .unwrap()
            .load(&workspace.workspace_id)
            .unwrap()
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                DomainEvent::ProductEventRecorded { record } => Some(record),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(product_records.len(), 8);
        assert!(product_records.iter().all(|record| {
            record.schema_version == 2
                && record.workspace_id == workspace.workspace_id
                && record.validate(&workspace.workspace_id).is_ok()
        }));
        assert!(product_records.iter().any(|record| {
            record.kind == ProductEventKind::AnalysisStarted
                && record.session_id == "analysis-first"
        }));
        assert!(product_records.iter().any(|record| {
            record.kind == ProductEventKind::RepeatAnalysisStarted
                && record.session_id == "analysis-repeat"
        }));
        for expected in [
            ProductEventKind::WorkspaceCreated,
            ProductEventKind::GoalApproved,
            ProductEventKind::ScorecardGenerated,
            ProductEventKind::ReportSaved,
            ProductEventKind::TimeToFirstSavedReport,
        ] {
            assert!(product_records.iter().any(|record| record.kind == expected));
        }
    }

    #[test]
    fn saved_report_reopens_immutable_evidence_after_checkout_mutation_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let (repository_path, frozen_commit) =
            test_repository(directory.path(), "saved-report-repository");
        let store = LocalWorkspaceStore::new(directory.path().join("data")).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Evidence history".into(),
                repository_display_name: "saved-report-repository".into(),
                repository_path: repository_path.clone(),
                product_brief: "Keep saved decisions tied to immutable evidence.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        let goal = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "immutable-evidence".into(),
                    title: "Saved evidence remains reviewable".into(),
                    business_outcome: "Later reviews see the code that informed the decision."
                        .into(),
                    criteria: vec![
                        "A reopened report resolves evidence from its recorded commit.".into(),
                    ],
                    priority: 5,
                    position: 1,
                    rubric_dimensions: vec!["Trust".into()],
                },
            )
            .unwrap();
        let repository = LocalRepository::attach("attached-repository", &repository_path).unwrap();
        let evidence = repository
            .evidence(&frozen_commit, "proof.txt", 1, 1, EvidenceKind::Test)
            .unwrap();
        let mut report = unverified_report(
            "saved-report-before-mutation",
            OffsetDateTime::UNIX_EPOCH,
            std::slice::from_ref(&goal),
            &frozen_commit,
        );
        report.assessments[0].verdict = Verdict::Supported;
        report.assessments[0].summary =
            "The saved decision is bound to immutable repository evidence.".into();
        report.assessments[0].criteria[0].verdict = Verdict::Supported;
        report.assessments[0].criteria[0].rationale =
            "The recorded coordinate resolves against the frozen commit.".into();
        report.assessments[0].criteria[0].confidence = 1.0;
        report.assessments[0].criteria[0].evidence = vec![evidence.clone()];
        report.coverage = Some(1.0);
        report.unverified_criteria = 0;
        store
            .record_report(&workspace.workspace_id, report)
            .unwrap();

        fs::write(
            Path::new(&repository_path).join("proof.txt"),
            "working tree now contains different evidence\n",
        )
        .unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&repository_path)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        };
        git(&["checkout", "-qb", "changed-after-report"]);
        git(&["add", "proof.txt"]);
        git(&["commit", "-qm", "change evidence after saved report"]);
        assert_ne!(repository.head().unwrap(), frozen_commit);

        drop(store);
        let reopened = LocalWorkspaceStore::new(directory.path().join("data")).unwrap();
        let recent = reopened.recent_workspace().unwrap().unwrap();
        let saved = recent.latest_report.unwrap();
        let saved_evidence = &saved.assessments[0].criteria[0].evidence[0];
        assert_eq!(saved.repositories[0].commit_sha, frozen_commit);
        assert_eq!(saved_evidence, &evidence);

        let current = fs::read_to_string(Path::new(&repository_path).join("proof.txt")).unwrap();
        assert_eq!(current, "working tree now contains different evidence\n");
        let reopened_repository =
            LocalRepository::attach("attached-repository", &repository_path).unwrap();
        reopened_repository.verify_evidence(saved_evidence).unwrap();
        assert_eq!(
            reopened_repository.read_evidence(saved_evidence).unwrap(),
            "repository-verifiable proof"
        );
    }

    #[test]
    fn identical_goal_set_replacement_is_idempotent_for_history_and_metrics() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Product".into(),
                repository_display_name: "product".into(),
                repository_path: "/local/product".into(),
                product_brief: "A repository-verifiable product.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        let request = ReplaceGoalsRequest {
            goals: vec![ApproveGoalRequest {
                goal_id: "goal".into(),
                title: "Ship safely".into(),
                business_outcome: "Keep releases trustworthy.".into(),
                criteria: vec!["A version-controlled release gate is enforced.".into()],
                priority: 5,
                position: 1,
                rubric_dimensions: vec!["Reliability".into()],
            }],
        };
        let first = store
            .replace_goals(&workspace.workspace_id, request.clone())
            .unwrap();
        let event_count = store
            .event_log()
            .unwrap()
            .load(&workspace.workspace_id)
            .unwrap()
            .len();
        let second = store
            .replace_goals(&workspace.workspace_id, request)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            store
                .event_log()
                .unwrap()
                .load(&workspace.workspace_id)
                .unwrap()
                .len(),
            event_count,
            "saving unchanged frozen goals must not append another approval"
        );
        let recent = store.recent_workspace().unwrap().unwrap();
        assert_eq!(recent.decision_funnel.goal_approvals, 1);
    }

    #[test]
    fn historical_outcome_rating_events_remain_deserializable() {
        let event = DomainEvent::OutcomeSurveyResponded {
            cycle_id: "legacy-cycle".into(),
            report_value_rating: 4,
            decision_confidence_rating: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        let restored: DomainEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            restored,
            DomainEvent::OutcomeSurveyResponded {
                report_value_rating: 4,
                decision_confidence_rating: 5,
                ..
            }
        ));
    }

    #[test]
    fn privacy_adversarial_attachment_never_enters_ipc_events_or_recovery_exports() {
        let directory = tempfile::tempdir().unwrap();
        let attachment = directory.path().join("board.md");
        let secret_text = crate::privacy_test_support::ATTACHMENT_FIXTURE;
        fs::write(&attachment, secret_text).unwrap();
        let store = LocalWorkspaceStore::new(directory.path().join("data")).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "ExampleLeave".into(),
                repository_display_name: "example-leave".into(),
                repository_path: "/local/example-leave".into(),
                product_brief: "ExampleLeave synthetic leave-planning product context.".into(),
                context: ProjectContext {
                    company: "ExampleLeave (fictional)".into(),
                    context_file_paths: vec![attachment.to_string_lossy().into_owned()],
                    ..Default::default()
                },
            })
            .unwrap();

        let event_json = serde_json::to_string(
            &store
                .event_log()
                .unwrap()
                .raw_values(&workspace.workspace_id)
                .unwrap(),
        )
        .unwrap();
        assert!(!event_json.contains(secret_text));
        assert!(!event_json.contains(&attachment.to_string_lossy().into_owned()));

        let recovery = directory.path().join("recovery.json");
        store
            .export_recovery(&workspace.workspace_id, &recovery)
            .unwrap();
        let recovery_json = fs::read_to_string(recovery).unwrap();
        assert!(!recovery_json.contains(secret_text));
        assert!(!recovery_json.contains(&attachment.to_string_lossy().into_owned()));

        let recent = store.recent_workspace().unwrap().unwrap();
        let response = crate::protocol::CoreResponse::success(
            "workspace-recent",
            serde_json::to_value(recent).unwrap(),
        );
        let mut ipc_frame = Vec::new();
        crate::protocol::write_frame(&mut ipc_frame, &response).unwrap();
        crate::privacy_test_support::assert_private_payload_absent(&ipc_frame);
        crate::privacy_test_support::assert_private_payload_absent(event_json.as_bytes());
        crate::privacy_test_support::assert_private_payload_absent(recovery_json.as_bytes());
        assert!(!event_json.contains(crate::privacy_test_support::INJECTION_TEXT));
        assert!(!recovery_json.contains(crate::privacy_test_support::INJECTION_TEXT));

        let context = store
            .workspace_project_context(&workspace.workspace_id)
            .unwrap();
        assert_eq!(context.context_files.len(), 1);
        assert_eq!(
            context.context_files[0].path,
            fs::canonicalize(&attachment).unwrap().to_string_lossy()
        );
        let stored_reference = serde_json::to_string(&context.context_files[0]).unwrap();
        assert!(!stored_reference.contains(secret_text));
    }

    fn map_fixture(
        repositories: Vec<FrozenRepository>,
        generated_at: OffsetDateTime,
    ) -> CodebaseMap {
        CodebaseMap {
            id: format!("map-{}", Uuid::new_v4()),
            schema_version: codecaddie_domain::MAP_SCHEMA_VERSION,
            generated_at,
            repositories,
            provider: "codex".into(),
            provider_version: "test".into(),
            origin: codecaddie_domain::ReportOrigin::Scan,
            overview: codecaddie_domain::MapOverview {
                system_summary: "One bounded system.".into(),
                architecture_style: "Modular".into(),
                technologies: vec![],
            },
            components: vec![codecaddie_domain::Component {
                id: codecaddie_domain::component_id("attached-repository", "Core"),
                name: "Core".into(),
                kind: codecaddie_domain::ComponentKind::Service,
                repository_id: "attached-repository".into(),
                root_paths: vec!["src/".into()],
                responsibility: "Owns the domain rules.".into(),
                key_interfaces: vec![],
                concerns: vec![],
                evidence: vec![],
            }],
            relationships: vec![],
            data_flows: vec![],
            entry_points: vec![],
            partial: false,
            analysis_warnings: vec![],
            supersedes: None,
        }
    }

    #[test]
    fn codebase_maps_round_trip_with_hash_verification_and_pruning() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "CodeCaddie".into(),
                repository_display_name: "codecaddie".into(),
                repository_path: "/local/codecaddie".into(),
                product_brief: "Analyze CodeCaddie.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        let frozen = vec![FrozenRepository {
            repository_id: "attached-repository".into(),
            commit_sha: "0123456789012345678901234567890123456789".into(),
        }];

        let map = map_fixture(frozen.clone(), OffsetDateTime::UNIX_EPOCH);
        let descriptor = store
            .record_codebase_map(&workspace.workspace_id, &map)
            .unwrap();
        assert_eq!(descriptor.content_hash, map.content_hash().unwrap());

        // Round trip: matched by frozen set and by id, hash-verified.
        let loaded = store
            .load_codebase_map_for(&workspace.workspace_id, &frozen)
            .unwrap()
            .unwrap();
        assert_eq!(loaded, map);
        let (loaded_descriptor, by_id) = store
            .load_codebase_map(&workspace.workspace_id, Some(&map.id))
            .unwrap()
            .unwrap();
        assert_eq!(loaded_descriptor, descriptor);
        assert_eq!(by_id, map);
        let encrypted_body = fs::read_to_string(
            directory
                .path()
                .join("codebase-maps-v1")
                .join(format!("{}.json", descriptor.content_hash)),
        )
        .unwrap();
        assert!(encrypted_body.contains(crate::at_rest::ENVELOPE_FORMAT));
        assert!(!encrypted_body.contains("Owns the domain rules"));

        // A different frozen set does not match.
        let other = vec![FrozenRepository {
            repository_id: "attached-repository".into(),
            commit_sha: "9999999999999999999999999999999999999999".into(),
        }];
        assert!(
            store
                .load_codebase_map_for(&workspace.workspace_id, &other)
                .unwrap()
                .is_none()
        );

        // A tampered body degrades to None instead of loading.
        let body_path = directory
            .path()
            .join("codebase-maps-v1")
            .join(format!("{}.json", descriptor.content_hash));
        fs::write(&body_path, b"{\"tampered\":true}").unwrap();
        assert!(
            store
                .load_codebase_map_for(&workspace.workspace_id, &frozen)
                .unwrap()
                .is_none()
        );
        fs::write(&body_path, serde_json::to_vec(&map).unwrap()).unwrap();

        // Recording more maps prunes bodies beyond the newest four while the
        // descriptor history stays in the append-only log.
        for offset in 1..=5_i64 {
            let newer = map_fixture(
                frozen.clone(),
                OffsetDateTime::UNIX_EPOCH + time::Duration::days(offset),
            );
            store
                .record_codebase_map(&workspace.workspace_id, &newer)
                .unwrap();
        }
        assert!(!body_path.exists(), "the oldest body should be pruned");
        let (_, projection) = store.workspace_parts(&workspace.workspace_id).unwrap();
        assert_eq!(projection.codebase_maps.len(), 6);
        let newest = store
            .load_codebase_map_for(&workspace.workspace_id, &frozen)
            .unwrap()
            .unwrap();
        assert_eq!(
            newest.generated_at,
            OffsetDateTime::UNIX_EPOCH + time::Duration::days(5)
        );
    }

    #[test]
    fn workspace_context_updates_in_place_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let (repository_path, commit) = test_repository(directory.path(), "context-repository");
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "CodeCaddie".into(),
                repository_display_name: "codecaddie".into(),
                repository_path,
                product_brief: "Analyze CodeCaddie. Additional context: local-first trust.".into(),
                context: ProjectContext {
                    company: "CodeCaddie".into(),
                    website: "https://codecaddie.ai".into(),
                    notes: "local-first trust".into(),
                    context_file_names: vec!["deck.pdf".into()],
                    context_files: Vec::new(),
                    context_file_paths: Vec::new(),
                },
            })
            .unwrap();
        let recent = store.recent_workspace().unwrap().unwrap();
        assert_eq!(recent.context.notes, "local-first trust");
        assert_eq!(recent.context.context_file_names, ["deck.pdf".to_string()]);

        // Seed a goal and a report so "goals and reports survive the
        // update" is a real assertion, not a vacuous one.
        let goal = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "release".into(),
                    title: "Ship releases".into(),
                    business_outcome: "Ship reliably".into(),
                    criteria: vec!["Builds pass".into()],
                    priority: 5,
                    position: 1,
                    rubric_dimensions: vec!["Trust".into()],
                },
            )
            .unwrap();
        store
            .record_report(
                &workspace.workspace_id,
                unverified_report(
                    "report-1",
                    OffsetDateTime::UNIX_EPOCH,
                    std::slice::from_ref(&goal),
                    &commit,
                ),
            )
            .unwrap();

        store
            .update_workspace_context(
                &workspace.workspace_id,
                UpdateWorkspaceContextRequest {
                    name: "Acme".into(),
                    repository_path: "/local/acme-moved".into(),
                    product_brief: "Analyze Acme. Additional context: rewritten notes.".into(),
                    context: ProjectContext {
                        company: "Acme".into(),
                        website: String::new(),
                        notes: "rewritten notes".into(),
                        context_file_names: Vec::new(),
                        context_files: Vec::new(),
                        context_file_paths: Vec::new(),
                    },
                },
            )
            .unwrap();

        let (local_state, access, _) = store.load_parts(&workspace.workspace_id).unwrap();
        assert_eq!(
            local_state.workspaces.len(),
            1,
            "update must not mint a new workspace"
        );
        assert_eq!(access.workspace_name, "Acme");
        let updated = store.recent_workspace().unwrap().unwrap();
        assert_eq!(updated.workspace_id, workspace.workspace_id);
        assert_eq!(
            updated.name, "Acme",
            "the rename must survive resume, not be shadowed by the projection name"
        );
        assert_eq!(updated.repository_path, "/local/acme-moved");
        assert_eq!(updated.context.notes, "rewritten notes");
        assert!(updated.context.context_file_names.is_empty());
        assert_eq!(
            updated.approved_goals.len(),
            1,
            "approved goals must survive a context update"
        );
        assert!(
            updated.latest_report.is_some(),
            "report history must survive a context update"
        );
        assert_eq!(
            store
                .workspace_product_brief(&workspace.workspace_id)
                .unwrap(),
            "Analyze Acme. Additional context: rewritten notes."
        );

        assert!(
            store
                .update_workspace_context(
                    &workspace.workspace_id,
                    UpdateWorkspaceContextRequest {
                        name: "X".into(),
                        repository_path: String::new(),
                        product_brief: "   ".into(),
                        context: ProjectContext::default(),
                    },
                )
                .is_err(),
            "a blank brief must be rejected"
        );
    }

    #[test]
    fn invalid_history_in_another_workspace_does_not_block_recent_project() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let create = |name: &str| CreateWorkspaceRequest {
            name: name.into(),
            repository_display_name: name.to_lowercase(),
            repository_path: format!("/local/{}", name.to_lowercase()),
            product_brief: format!("{name} has a durable product strategy and engineering plan."),
            context: ProjectContext::default(),
        };
        let older = store.create_workspace(create("Older")).unwrap();
        let current = store.create_workspace(create("Current")).unwrap();

        let (local_state, _, projection) = store.load_parts(&older.workspace_id).unwrap();
        let device = local_state.device;
        let mut invalid = EventEnvelope::sign(
            older.workspace_id.clone(),
            projection.applied_event_ids.len() as u64 + 1,
            OffsetDateTime::now_utc(),
            device.device_id.clone(),
            device.actor_id.clone(),
            projection.team_epoch(),
            DomainEvent::ActionCreated {
                action_id: "action-tampered".into(),
                recommendation_id: "recommendation-tampered".into(),
                title: "Tampered event".into(),
            },
            &device.signing_key().unwrap(),
        )
        .unwrap();
        invalid.signature.replace_range(..2, "00");
        store
            .event_log()
            .unwrap()
            .append(&older.workspace_id, &invalid)
            .unwrap();

        let recent = store.recent_workspace().unwrap().unwrap();
        assert_eq!(recent.workspace_id, current.workspace_id);
        assert_eq!(recent.name, "Current");
    }

    #[test]
    fn provider_preference_round_trips_and_rejects_unknown_values() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        assert_eq!(store.provider_preference().unwrap(), None);
        store.set_provider_preference("grok").unwrap();
        assert_eq!(store.provider_preference().unwrap(), Some("grok".into()));
        assert!(store.set_provider_preference("copilot").is_err());
        fs::write(directory.path().join("provider-preference-v1"), "garbage").unwrap();
        assert_eq!(store.provider_preference().unwrap(), None);
    }

    #[test]
    fn privacy_adversarial_startup_migrates_every_active_private_surface() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Private workspace".into(),
                repository_display_name: "private-repository".into(),
                repository_path: "/private/source-canary-repository".into(),
                product_brief: crate::privacy_test_support::REPOSITORY_FIXTURE.into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        store.set_provider_preference("codex").unwrap();
        let cipher = store.content_cipher.clone();

        let make_plain = |path: &Path, purpose: &str| {
            let decoded = cipher
                .open_or_plain(purpose, &fs::read(path).unwrap())
                .unwrap();
            assert!(decoded.encrypted);
            fs::write(path, decoded.plaintext).unwrap();
        };
        make_plain(
            &directory.path().join("local-state-v2.json"),
            "local-state-v2.json",
        );
        make_plain(
            &directory.path().join("recent-workspace-v1"),
            RECENT_WORKSPACE_PURPOSE,
        );
        make_plain(
            &directory.path().join("provider-preference-v1"),
            PROVIDER_PREFERENCE_PURPOSE,
        );

        let event_path = directory.path().join("events-v2").join(format!(
            "{}.events",
            blake3::hash(workspace.workspace_id.as_bytes()).to_hex()
        ));
        let mut plaintext_log = Vec::new();
        for line in fs::read(&event_path).unwrap().split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            plaintext_log.extend(
                cipher
                    .open_or_plain(crate::storage::EVENT_LOG_PURPOSE, line)
                    .unwrap()
                    .plaintext,
            );
            plaintext_log.push(b'\n');
        }
        fs::write(&event_path, plaintext_log).unwrap();

        let map_directory = directory.path().join("codebase-maps-v1");
        fs::create_dir_all(&map_directory).unwrap();
        let map_path = map_directory.join("source-canary.json");
        fs::write(
            &map_path,
            crate::privacy_test_support::REPOSITORY_FIXTURE.as_bytes(),
        )
        .unwrap();
        let session_directory = directory.path().join("agent-sessions");
        fs::create_dir_all(&session_directory).unwrap();
        let session_path = session_directory.join("analysis-source-canary.json");
        fs::write(
            &session_path,
            crate::privacy_test_support::REPOSITORY_FIXTURE.as_bytes(),
        )
        .unwrap();
        fs::remove_file(directory.path().join(AT_REST_MIGRATION_MARKER)).unwrap();
        drop(store);

        let reopened = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        assert!(reopened.recent_workspace().unwrap().is_some());
        for path in [
            directory.path().join("local-state-v2.json"),
            directory.path().join("recent-workspace-v1"),
            directory.path().join("provider-preference-v1"),
            map_path,
            session_path,
            directory.path().join(AT_REST_MIGRATION_MARKER),
        ] {
            let bytes = fs::read(path).unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains(crate::at_rest::ENVELOPE_FORMAT));
            crate::privacy_test_support::assert_private_payload_absent(&bytes);
        }
        for line in fs::read(event_path).unwrap().split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            assert!(String::from_utf8_lossy(line).contains(crate::at_rest::ENVELOPE_FORMAT));
            crate::privacy_test_support::assert_private_payload_absent(line);
        }
    }

    #[test]
    fn privacy_adversarial_portable_backup_authenticates_and_restores_transactionally() {
        let directory = tempfile::tempdir().unwrap();
        let (repository_path, first_commit) =
            test_repository(directory.path(), "portable-repository");
        let context_path = directory.path().join("private-context.md");
        fs::write(
            &context_path,
            crate::privacy_test_support::ATTACHMENT_FIXTURE,
        )
        .unwrap();
        let source_root = directory.path().join("source-data");
        let source = LocalWorkspaceStore::new(source_root).unwrap();
        let workspace = source
            .create_workspace(CreateWorkspaceRequest {
                name: "Portable project".into(),
                repository_display_name: "portable-repository".into(),
                repository_path: repository_path.clone(),
                product_brief: crate::privacy_test_support::REPOSITORY_FIXTURE.into(),
                context: ProjectContext {
                    context_file_paths: vec![context_path.to_string_lossy().into_owned()],
                    ..ProjectContext::default()
                },
            })
            .unwrap();
        let goal = source
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "portable-history".into(),
                    title: "Keep decision history portable".into(),
                    business_outcome: "A local owner can recover saved decisions.".into(),
                    criteria: vec![
                        "An authenticated backup import test restores signed history.".into(),
                    ],
                    priority: 5,
                    position: 1,
                    rubric_dimensions: vec!["Recovery".into()],
                },
            )
            .unwrap();

        source
            .record_analysis_started(&workspace.workspace_id, "portable-report-1")
            .unwrap();
        source
            .record_report(
                &workspace.workspace_id,
                unverified_report(
                    "portable-report-1",
                    OffsetDateTime::now_utc(),
                    std::slice::from_ref(&goal),
                    &first_commit,
                ),
            )
            .unwrap();
        let second_commit = commit_repository_file(
            Path::new(&repository_path),
            "second-proof.txt",
            "second repository-verifiable proof\n",
        );
        source
            .record_analysis_started(&workspace.workspace_id, "portable-report-2")
            .unwrap();
        source
            .record_report(
                &workspace.workspace_id,
                unverified_report(
                    "portable-report-2",
                    OffsetDateTime::now_utc() + time::Duration::seconds(1),
                    std::slice::from_ref(&goal),
                    &second_commit,
                ),
            )
            .unwrap();

        let backup = directory.path().join("project.codecaddie-backup");
        let passphrase = "correct horse battery staple";
        let exported = source
            .export_portable_backup(&workspace.workspace_id, &backup, passphrase)
            .unwrap();
        assert!(exported.event_count >= 4);
        assert_eq!(exported.manifest_blake3.len(), 64);
        let encrypted = fs::read(&backup).unwrap();
        let encrypted_text = String::from_utf8_lossy(&encrypted);
        assert!(encrypted_text.contains(portable_backup::PORTABLE_BACKUP_FORMAT));
        crate::privacy_test_support::assert_private_payload_absent(&encrypted);
        assert!(!encrypted_text.contains(&repository_path));
        assert!(!encrypted_text.contains(passphrase));
        assert!(!encrypted_text.contains("Keep decision history portable"));
        assert!(portable_backup::open(&encrypted, "wrong backup passphrase").is_err());
        let opened_payload = portable_backup::open(&encrypted, passphrase).unwrap();
        let mut payload_value = serde_json::to_value(&opened_payload).unwrap();
        assert_eq!(payload_value["manifest"]["schemaVersion"], 1);
        assert_eq!(
            payload_value["manifest"]["createdAt"],
            payload_value["createdAt"]
        );
        assert_eq!(
            payload_value["manifest"]["encryption"]["algorithm"],
            "XChaCha20-Poly1305"
        );
        assert_eq!(payload_value["manifest"]["encryption"]["kdf"], "Argon2id");
        let mut legacy_value = payload_value.clone();
        let legacy_manifest = legacy_value["manifest"].as_object_mut().unwrap();
        legacy_manifest.remove("schemaVersion");
        legacy_manifest.remove("createdAt");
        legacy_manifest.remove("encryption");
        let legacy_payload: portable_backup::PortableBackupPayload =
            serde_json::from_value(legacy_value).unwrap();
        legacy_payload.validate().unwrap();
        portable_backup::open(
            &portable_backup::seal(&legacy_payload, passphrase).unwrap(),
            passphrase,
        )
        .unwrap();
        payload_value["manifest"]["schemaVersion"] = serde_json::json!(2);
        let incompatible_payload: portable_backup::PortableBackupPayload =
            serde_json::from_value(payload_value).unwrap();
        assert!(incompatible_payload.validate().is_err());
        let incompatible_backup = directory.path().join("incompatible.codecaddie-backup");
        fs::write(
            &incompatible_backup,
            portable_backup::seal_without_validation_for_test(&incompatible_payload, passphrase)
                .unwrap(),
        )
        .unwrap();
        let incompatible_store =
            LocalWorkspaceStore::new(directory.path().join("incompatible-data")).unwrap();
        let incompatible_error = incompatible_store
            .import_portable_backup(
                &incompatible_backup,
                Path::new(&repository_path),
                passphrase,
            )
            .unwrap_err();
        assert!(
            incompatible_error
                .to_string()
                .contains("manifest metadata is unsupported")
        );
        assert!(incompatible_store.recent_workspace().unwrap().is_none());
        let mut tampered: serde_json::Value = serde_json::from_slice(&encrypted).unwrap();
        let ciphertext = tampered["ciphertext"].as_str().unwrap();
        let replacement = if ciphertext.starts_with('A') {
            "B"
        } else {
            "A"
        };
        tampered["ciphertext"] =
            serde_json::Value::String(format!("{replacement}{}", &ciphertext[1..]));
        assert!(
            portable_backup::open(&serde_json::to_vec(&tampered).unwrap(), passphrase).is_err()
        );

        let scheduled_directory = directory.path().join("scheduled-backups");
        fs::create_dir(&scheduled_directory).unwrap();
        let scheduled = source
            .enable_backup_schedule(&workspace.workspace_id, &scheduled_directory, passphrase)
            .unwrap();
        assert_eq!(scheduled.status, "created");
        assert_eq!(scheduled.schedule.cadence_hours, 24);
        assert_eq!(scheduled.schedule.recovery_point_objective_hours, 24);
        assert_eq!(scheduled.schedule.recovery_time_objective_minutes, 30);
        let schedule_bytes =
            fs::read(source.backup_schedule_path(&workspace.workspace_id)).unwrap();
        assert!(String::from_utf8_lossy(&schedule_bytes).contains(crate::at_rest::ENVELOPE_FORMAT));
        assert!(!String::from_utf8_lossy(&schedule_bytes).contains(passphrase));
        crate::privacy_test_support::assert_private_payload_absent(&schedule_bytes);
        let last = scheduled.schedule.last_successful_at_unix.unwrap();
        let not_due = source
            .run_scheduled_backup_at(
                &workspace.workspace_id,
                OffsetDateTime::from_unix_timestamp(last + BACKUP_CADENCE_SECONDS - 1).unwrap(),
                false,
            )
            .unwrap();
        assert_eq!(not_due.status, "not_due");
        let due = source
            .run_scheduled_backup_at(
                &workspace.workspace_id,
                OffsetDateTime::from_unix_timestamp(last + BACKUP_CADENCE_SECONDS).unwrap(),
                false,
            )
            .unwrap();
        assert_eq!(due.status, "created");
        assert_eq!(
            fs::read_dir(&scheduled_directory)
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            2
        );
        let public_status = serde_json::to_string(
            &source
                .backup_schedule_status(&workspace.workspace_id)
                .unwrap(),
        )
        .unwrap();
        assert!(!public_status.contains(passphrase));

        let interrupted_root = directory.path().join("interrupted-data");
        let interrupted = LocalWorkspaceStore::new(interrupted_root).unwrap();
        let error = interrupted
            .import_portable_backup_with(&backup, Path::new(&repository_path), passphrase, || {
                anyhow::bail!("injected interruption after exact event commit")
            })
            .unwrap_err();
        assert!(error.to_string().contains("injected interruption"));
        assert!(interrupted.recent_workspace().unwrap().is_none());
        let restore_started = std::time::Instant::now();
        let restored = interrupted
            .import_portable_backup(&backup, Path::new(&repository_path), passphrase)
            .unwrap();
        let restore_elapsed = restore_started.elapsed();
        assert!(
            restore_elapsed <= std::time::Duration::from_secs(u64::from(BACKUP_RTO_MINUTES) * 60),
            "backup restore took {restore_elapsed:?}, exceeding the documented {BACKUP_RTO_MINUTES}-minute RTO"
        );
        assert_eq!(restored.workspace_id, workspace.workspace_id);
        assert_eq!(restored.event_count, exported.event_count);
        assert_eq!(restored.manifest_blake3, exported.manifest_blake3);

        // A retry after the complete commit is also idempotent and does not
        // duplicate any signed event.
        interrupted
            .import_portable_backup(&backup, Path::new(&repository_path), passphrase)
            .unwrap();
        let events = interrupted
            .event_log()
            .unwrap()
            .load(&workspace.workspace_id)
            .unwrap();
        assert_eq!(events.len(), exported.event_count);
        let recent = interrupted.recent_workspace().unwrap().unwrap();
        assert_eq!(recent.workspace_id, workspace.workspace_id);
        assert_eq!(recent.approved_goals.len(), 1);
        assert_eq!(
            recent.latest_report.as_ref().unwrap().id,
            "portable-report-2"
        );
        assert_eq!(recent.report_heatmap.len(), 2);
        assert_eq!(recent.report_heatmap[0].report_id, "portable-report-1");
        assert_eq!(recent.report_heatmap[1].report_id, "portable-report-2");
        assert_eq!(
            recent.report_heatmap[0].repositories,
            vec![format!("attached-repository @ {first_commit}")]
        );
        assert_eq!(
            recent.report_heatmap[1].repositories,
            vec![format!("attached-repository @ {second_commit}")]
        );
        assert_eq!(recent.decision_funnel.repeat_analyses, 1);
        assert_eq!(recent.decision_funnel.comparisons_generated, 1);
        assert_eq!(
            Path::new(&recent.repository_path).canonicalize().unwrap(),
            Path::new(&repository_path).canonicalize().unwrap()
        );
        assert_eq!(recent.context.context_files.len(), 1);
        assert!(recent.context.context_files[0].path.is_empty());
    }

    #[test]
    fn scheduled_backup_retention_prunes_only_owned_regular_files() {
        let directory = tempfile::tempdir().unwrap();
        let prefix = "codecaddie-workspace-";
        for index in 0..16 {
            fs::write(
                directory
                    .path()
                    .join(format!("{prefix}{index:02}.codecaddie-backup")),
                b"backup",
            )
            .unwrap();
        }
        fs::write(
            directory.path().join("unrelated.codecaddie-backup"),
            b"keep",
        )
        .unwrap();
        fs::create_dir(
            directory
                .path()
                .join(format!("{prefix}directory.codecaddie-backup")),
        )
        .unwrap();
        prune_scheduled_backups(directory.path(), prefix, 14).unwrap();
        let mut retained = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().unwrap().is_file())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        retained.sort();
        assert_eq!(
            retained
                .iter()
                .filter(|name| name.starts_with(prefix))
                .count(),
            14
        );
        assert!(retained.contains(&"unrelated.codecaddie-backup".into()));
        assert!(!retained.contains(&format!("{prefix}00.codecaddie-backup")));
        assert!(!retained.contains(&format!("{prefix}01.codecaddie-backup")));
    }

    #[cfg(unix)]
    #[test]
    fn local_state_directories_and_encrypted_pointers_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        store.set_provider_preference("codex").unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Private workspace".into(),
                repository_display_name: "private-repository".into(),
                repository_path: "/private/repository".into(),
                product_brief: "A private product brief long enough for this test.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        assert!(!workspace.workspace_id.is_empty());

        for path in [
            directory.path().to_path_buf(),
            directory.path().join("events-v2"),
            directory.path().join("locks-v1"),
        ] {
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        for path in [
            directory.path().join("recent-workspace-v1"),
            directory.path().join("provider-preference-v1"),
        ] {
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let encrypted = fs::read_to_string(&path).unwrap();
            assert!(encrypted.contains(crate::at_rest::ENVELOPE_FORMAT));
            assert!(!encrypted.contains("codex"));
            assert!(!encrypted.contains(&workspace.workspace_id));
        }
    }

    #[test]
    fn approvals_round_trip_with_encrypted_local_state_and_explicit_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let (repository_path, commit) = test_repository(directory.path(), "approval-repository");
        let store = LocalWorkspaceStore::new(directory.path().into()).unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "CodeCaddie".into(),
                repository_display_name: "codecaddie".into(),
                repository_path,
                product_brief: "Analyze local code against business promises".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        let (_, access, _) = store.load_parts(&workspace.workspace_id).unwrap();
        assert_eq!(access.role, Role::Editor);
        let approved = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "primary-goal".into(),
                    title: "Make approvals deliberate".into(),
                    business_outcome: "Leaders score only intentional promises".into(),
                    criteria: vec!["Every scan uses an approved immutable version".into()],
                    priority: 2,
                    position: 1,
                    rubric_dimensions: vec!["Trust".into()],
                },
            )
            .unwrap();
        let secondary = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "secondary-goal".into(),
                    title: "Keep analysis understandable".into(),
                    business_outcome: "Leaders can act on every finding".into(),
                    criteria: vec!["Every finding explains its impact".into()],
                    priority: 5,
                    position: 2,
                    rubric_dimensions: vec!["Clarity".into()],
                },
            )
            .unwrap();
        let approved_again = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "primary-goal".into(),
                    title: "Make approvals deliberate".into(),
                    business_outcome: "Leaders score only intentional promises".into(),
                    criteria: vec!["Every scan uses an approved immutable version".into()],
                    priority: 2,
                    position: 1,
                    rubric_dimensions: vec!["Trust".into()],
                },
            )
            .unwrap();
        assert_eq!(approved_again.id, approved.id);
        assert_eq!(approved_again.created_at, approved.created_at);
        assert_eq!(
            store.approved_goals(&workspace.workspace_id).unwrap(),
            vec![approved.clone(), secondary.clone()]
        );
        assert_eq!(
            store.recent_workspace().unwrap().unwrap().approved_goals,
            vec![approved.clone(), secondary.clone()]
        );
        let report = unverified_report(
            "report-board-safe",
            OffsetDateTime::UNIX_EPOCH,
            &[secondary.clone(), approved.clone()],
            &commit,
        );
        store
            .record_report(&workspace.workspace_id, report)
            .unwrap();
        let persisted = fs::read_to_string(
            fs::read_dir(directory.path().join("events-v2"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(persisted.contains(crate::at_rest::ENVELOPE_FORMAT));
        assert!(!persisted.contains("Make approvals deliberate"));
        let recovery_path = directory.path().join("codecaddie-recovery.json");
        store
            .export_recovery(&workspace.workspace_id, &recovery_path)
            .unwrap();
        let recovery_text = fs::read_to_string(&recovery_path).unwrap();
        assert!(!recovery_text.contains("workspaceKey"));
        assert!(recovery_text.contains("Make approvals deliberate"));
        assert!(serde_json::from_str::<RecoveryPayload>(&recovery_text).is_ok());
    }
}
