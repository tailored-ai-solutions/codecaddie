use crate::{
    ActionStatus, CodebaseMapDescriptor, DeviceAccess, DomainEvent, EventEnvelope,
    FrozenRepository, GoalVersion, Report, RepositoryRef, Role, ScopeState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionProjection {
    pub id: String,
    pub recommendation_id: String,
    pub title: String,
    pub status: ActionStatus,
    pub last_note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProjection {
    pub workspace_id: String,
    pub workspace_fingerprint: String,
    pub name: String,
    pub team: ScopeState,
    pub devices: BTreeMap<String, DeviceAccess>,
    pub repositories: BTreeMap<String, RepositoryRef>,
    pub goal_versions: BTreeMap<String, GoalVersion>,
    pub approved_goals: BTreeMap<String, String>,
    pub reports: BTreeMap<String, Report>,
    /// Storage-key to immutable `ReportCompleted` envelope id. Report ids are
    /// not sufficient because historical desktop releases could reuse them.
    #[serde(default)]
    pub report_event_ids: BTreeMap<String, String>,
    /// Stable, one-based ordinal assigned from the completion event stream.
    #[serde(default)]
    pub report_ordinals: BTreeMap<String, u32>,
    #[serde(default)]
    pub report_completion_count: u32,
    #[serde(default)]
    pub deleted_report_event_ids: BTreeSet<String>,
    #[serde(default)]
    pub codebase_maps: BTreeMap<String, CodebaseMapDescriptor>,
    pub actions: BTreeMap<String, ActionProjection>,
    pub applied_event_ids: BTreeSet<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProjectionError {
    #[error("event signature is invalid")]
    InvalidSignature,
    #[error("event belongs to a different workspace")]
    WrongWorkspace,
    #[error("workspace must be created before other events")]
    WorkspaceMissing,
    #[error("actor is not authorized for this event")]
    Forbidden,
    #[error("event references missing state: {0}")]
    Missing(String),
    #[error("invalid state transition")]
    InvalidTransition,
    #[error("event epoch does not match the current team epoch")]
    StaleEpoch,
    #[error("domain value is invalid: {0}")]
    InvalidValue(String),
}

impl WorkspaceProjection {
    pub fn team_epoch(&self) -> u64 {
        self.team.epoch
    }

    /// The newest recorded map whose frozen repository set matches exactly,
    /// preferring non-superseded maps.
    pub fn latest_map_for(&self, frozen: &[FrozenRepository]) -> Option<&CodebaseMapDescriptor> {
        let superseded: BTreeSet<&str> = self
            .codebase_maps
            .values()
            .filter_map(|descriptor| descriptor.supersedes.as_deref())
            .collect();
        self.codebase_maps
            .values()
            .filter(|descriptor| descriptor.matches_repositories(frozen))
            .filter(|descriptor| !superseded.contains(descriptor.map_id.as_str()))
            .max_by_key(|descriptor| descriptor.generated_at)
    }

    pub fn rebuild(events: &[EventEnvelope]) -> Result<Self, ProjectionError> {
        let mut ordered = events.to_vec();
        ordered.sort_by(|left, right| {
            (left.lamport, left.event_id, &left.device_id).cmp(&(
                right.lamport,
                right.event_id,
                &right.device_id,
            ))
        });
        let mut projection = Self::default();
        for event in &ordered {
            // A signed old-epoch event can arrive after a winning concurrent
            // Team rotation. It is valid history from an abandoned namespace,
            // not a reason to make every retained replica unrebuildable.
            match projection.apply(event) {
                Ok(()) | Err(ProjectionError::StaleEpoch) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(projection)
    }

    pub fn apply(&mut self, envelope: &EventEnvelope) -> Result<(), ProjectionError> {
        let event_id = envelope.event_id.to_string();
        if self.applied_event_ids.contains(&event_id) {
            return Ok(());
        }
        if !envelope.verify() {
            return Err(ProjectionError::InvalidSignature);
        }
        if !self.workspace_id.is_empty() && self.workspace_id != envelope.workspace_id {
            return Err(ProjectionError::WrongWorkspace);
        }

        if let DomainEvent::WorkspaceCreated {
            name,
            founding_device,
            workspace_fingerprint,
        } = &envelope.event
        {
            if !self.workspace_id.is_empty()
                || founding_device.actor_id != envelope.actor_id
                || founding_device.device_id != envelope.device_id
                || founding_device.signing_public_key != envelope.signing_public_key
                || founding_device.validate().is_err()
                || workspace_fingerprint.trim().is_empty()
            {
                return Err(ProjectionError::Forbidden);
            }
            self.workspace_id.clone_from(&envelope.workspace_id);
            self.workspace_fingerprint.clone_from(workspace_fingerprint);
            self.name.clone_from(name);
            self.team = ScopeState {
                epoch: envelope.epoch,
            };
            self.devices.insert(
                founding_device.device_id.clone(),
                DeviceAccess {
                    identity: founding_device.clone(),
                    role: Role::Editor,
                    grant_id: None,
                },
            );
            self.applied_event_ids.insert(event_id);
            return Ok(());
        }

        if self.workspace_id.is_empty() {
            return Err(ProjectionError::WorkspaceMissing);
        }
        if envelope.epoch != self.team_epoch() {
            return Err(ProjectionError::StaleEpoch);
        }
        let access = self
            .devices
            .get(&envelope.device_id)
            .ok_or(ProjectionError::Forbidden)?;
        if access.role != Role::Editor
            || access.identity.actor_id != envelope.actor_id
            || access.identity.signing_public_key != envelope.signing_public_key
        {
            return Err(ProjectionError::Forbidden);
        }

        match &envelope.event {
            // Documented invariant: the `if let` block above intercepts every
            // `WorkspaceCreated` envelope and returns on all of its paths
            // (`Err(Forbidden)` or `Ok(())`), so this arm can only execute if
            // that block is removed — a programming error, not a data path.
            DomainEvent::WorkspaceCreated { .. } => {
                unreachable!("WorkspaceCreated is fully handled before the event match")
            }
            DomainEvent::RepositoryRegistered { repository } => {
                self.repositories
                    .insert(repository.id.clone(), repository.clone());
            }
            DomainEvent::GoalVersionProposed { version } => {
                version
                    .validate()
                    .map_err(|reason| ProjectionError::InvalidValue(reason.into()))?;
                self.goal_versions
                    .insert(version.id.clone(), version.clone());
            }
            DomainEvent::GoalVersionApproved {
                goal_id,
                version_id,
            } => {
                let version = self
                    .goal_versions
                    .get(version_id)
                    .ok_or_else(|| ProjectionError::Missing(version_id.clone()))?;
                if &version.goal_id != goal_id {
                    return Err(ProjectionError::InvalidValue(
                        "version does not belong to goal".into(),
                    ));
                }
                self.approved_goals
                    .insert(goal_id.clone(), version_id.clone());
            }
            DomainEvent::GoalSetReplaced { versions } => {
                if versions.is_empty() {
                    return Err(ProjectionError::InvalidValue(
                        "a goal set cannot be empty".into(),
                    ));
                }
                let mut goal_ids = BTreeSet::new();
                let mut version_ids = BTreeSet::new();
                let mut positions = BTreeSet::new();
                for version in versions {
                    version
                        .validate()
                        .map_err(|reason| ProjectionError::InvalidValue(reason.into()))?;
                    if !goal_ids.insert(version.goal_id.clone())
                        || !version_ids.insert(version.id.clone())
                        || version.position == 0
                        || version.position as usize > versions.len()
                        || !positions.insert(version.position)
                    {
                        return Err(ProjectionError::InvalidValue(
                            "replacement goals need unique ids and contiguous positions".into(),
                        ));
                    }
                }
                for version in versions {
                    self.goal_versions
                        .insert(version.id.clone(), version.clone());
                }
                self.approved_goals = versions
                    .iter()
                    .map(|version| (version.goal_id.clone(), version.id.clone()))
                    .collect();
            }
            DomainEvent::ReportCompleted { report } => {
                let current: BTreeSet<&str> =
                    self.approved_goals.values().map(String::as_str).collect();
                let report_set: BTreeSet<&str> =
                    report.goal_version_ids.iter().map(String::as_str).collect();
                if current != report_set {
                    return Err(ProjectionError::InvalidValue(
                        "report goal versions were not the approved set".into(),
                    ));
                }
                // Historical releases could restart their desktop report
                // counter after relaunch. Keep both signed reports while the
                // canonical id continues to resolve to the newest event.
                if let Some(previous) = self.reports.remove(&report.id) {
                    let previous_event_id = self.report_event_ids.remove(&report.id);
                    let mut historical_key = format!("\0history:{event_id}");
                    while self.reports.contains_key(&historical_key) {
                        historical_key.push('\0');
                    }
                    if let Some(previous_event_id) = previous_event_id {
                        self.report_event_ids
                            .insert(historical_key.clone(), previous_event_id);
                    }
                    self.reports.insert(historical_key, previous);
                }
                self.reports.insert(report.id.clone(), report.clone());
                self.report_completion_count = self.report_completion_count.saturating_add(1);
                self.report_ordinals
                    .insert(event_id.clone(), self.report_completion_count);
                self.report_event_ids
                    .insert(report.id.clone(), event_id.clone());
            }
            DomainEvent::ReportDeleted { report_event_id } => {
                let Some(storage_key) = self
                    .report_event_ids
                    .iter()
                    .find_map(|(key, value)| (value == report_event_id).then(|| key.clone()))
                else {
                    if self.deleted_report_event_ids.contains(report_event_id) {
                        return Err(ProjectionError::InvalidValue(
                            "report is already removed from history".into(),
                        ));
                    }
                    return Err(ProjectionError::Missing(report_event_id.clone()));
                };
                let latest_event_id = self
                    .report_event_ids
                    .iter()
                    .filter_map(|(key, event_id)| {
                        let report = self.reports.get(key)?;
                        let ordinal = self.report_ordinals.get(event_id).copied().unwrap_or(0);
                        Some((report.completed_at, ordinal, event_id))
                    })
                    .max_by_key(|(completed_at, ordinal, _)| (*completed_at, *ordinal))
                    .map(|(_, _, event_id)| event_id.as_str());
                if latest_event_id == Some(report_event_id.as_str()) {
                    return Err(ProjectionError::InvalidValue(
                        "the latest report cannot be removed from history".into(),
                    ));
                }
                self.reports.remove(&storage_key);
                self.report_event_ids.remove(&storage_key);
                self.deleted_report_event_ids
                    .insert(report_event_id.clone());
            }
            // Decision-funnel markers are immutable local observations. They
            // intentionally do not project content into workspace state;
            // summaries are derived from their signed envelope timestamps.
            DomainEvent::DecisionFunnelEventRecorded { .. } => {}
            DomainEvent::ProductEventRecorded { record } => {
                record
                    .validate(&envelope.workspace_id)
                    .map_err(|reason| ProjectionError::InvalidValue(reason.into()))?;
            }
            DomainEvent::ReliabilityEventRecorded { record } => {
                record
                    .validate()
                    .map_err(|reason| ProjectionError::InvalidValue(reason.into()))?;
            }
            // Outcome-survey cycles and their two bounded numeric ratings are
            // reduced from signed event timestamps on demand. No free text or
            // repository material enters the projection.
            DomainEvent::OutcomeSurveyPrompted { .. }
            | DomainEvent::OutcomeSurveyDeferred { .. }
            | DomainEvent::OutcomeSurveyResponded { .. } => {}
            // Maps are deliberately goal-independent: no approved-goal-set
            // check, so goal edits never invalidate a map and it stays
            // reusable across scans at the same frozen commit set.
            DomainEvent::CodebaseMapRecorded { descriptor } => {
                if descriptor.map_id.trim().is_empty()
                    || descriptor.content_hash.len() != 64
                    || descriptor.repositories.is_empty()
                {
                    return Err(ProjectionError::InvalidValue(
                        "codebase map descriptor is incomplete".into(),
                    ));
                }
                self.codebase_maps
                    .insert(descriptor.map_id.clone(), descriptor.clone());
            }
            DomainEvent::ActionCreated {
                action_id,
                recommendation_id,
                title,
            } => {
                self.actions.insert(
                    action_id.clone(),
                    ActionProjection {
                        id: action_id.clone(),
                        recommendation_id: recommendation_id.clone(),
                        title: title.clone(),
                        status: ActionStatus::Open,
                        last_note: None,
                    },
                );
            }
            DomainEvent::ActionTransitioned {
                action_id,
                from,
                to,
                note,
            } => {
                let action = self
                    .actions
                    .get_mut(action_id)
                    .ok_or_else(|| ProjectionError::Missing(action_id.clone()))?;
                if action.status != *from || !from.may_transition_to(*to) {
                    return Err(ProjectionError::InvalidTransition);
                }
                if *to == ActionStatus::ReadyForVerification
                    && note.as_ref().is_none_or(|value| value.trim().is_empty())
                {
                    return Err(ProjectionError::InvalidValue(
                        "completion note is required".into(),
                    ));
                }
                action.status = *to;
                action.last_note.clone_from(note);
            }
        }
        self.applied_event_ids.insert(event_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Criterion, DeviceIdentity, ReportOrigin};
    use ed25519_dalek::SigningKey;
    use time::OffsetDateTime;

    fn identity(device: &str, key: &SigningKey) -> DeviceIdentity {
        DeviceIdentity {
            actor_id: format!("actor-{device}"),
            device_id: device.into(),
            signing_public_key: hex::encode(key.verifying_key().to_bytes()),
            label: device.into(),
        }
    }

    fn signed(
        lamport: u64,
        device: &DeviceIdentity,
        key: &SigningKey,
        epoch: u64,
        event: DomainEvent,
    ) -> EventEnvelope {
        EventEnvelope::sign(
            "ws".into(),
            lamport,
            OffsetDateTime::UNIX_EPOCH,
            device.device_id.clone(),
            device.actor_id.clone(),
            epoch,
            event,
            key,
        )
        .unwrap()
    }

    #[test]
    fn unregistered_or_mismatched_devices_cannot_append_events() {
        let editor_key = SigningKey::from_bytes(&[3; 32]);
        let editor = identity("editor", &editor_key);
        let created = signed(
            1,
            &editor,
            &editor_key,
            1,
            DomainEvent::WorkspaceCreated {
                name: "Acme".into(),
                founding_device: editor.clone(),
                workspace_fingerprint: "fp".into(),
            },
        );
        let mut projection = WorkspaceProjection::rebuild(&[created]).unwrap();

        let stranger_key = SigningKey::from_bytes(&[4; 32]);
        let stranger = identity("stranger", &stranger_key);
        let forbidden = signed(
            2,
            &stranger,
            &stranger_key,
            1,
            DomainEvent::RepositoryRegistered {
                repository: RepositoryRef {
                    id: "repo".into(),
                    display_name: "Repo".into(),
                    remote_fingerprint: None,
                },
            },
        );
        assert_eq!(
            projection.apply(&forbidden),
            Err(ProjectionError::Forbidden)
        );

        // A registered device id with a different signing key must also fail:
        // membership is bound to the recorded signing public key.
        let impostor_key = SigningKey::from_bytes(&[5; 32]);
        let impostor = DeviceIdentity {
            signing_public_key: hex::encode(impostor_key.verifying_key().to_bytes()),
            ..editor.clone()
        };
        let impersonated = signed(
            2,
            &impostor,
            &impostor_key,
            1,
            DomainEvent::RepositoryRegistered {
                repository: RepositoryRef {
                    id: "repo".into(),
                    display_name: "Repo".into(),
                    remote_fingerprint: None,
                },
            },
        );
        assert_eq!(
            projection.apply(&impersonated),
            Err(ProjectionError::Forbidden)
        );
    }

    #[test]
    fn duplicate_report_ids_preserve_history_and_keep_the_latest_canonical() {
        let key = SigningKey::from_bytes(&[6; 32]);
        let editor = identity("editor", &key);
        let goal = GoalVersion {
            id: "goal-version".into(),
            goal_id: "goal".into(),
            title: "Make a durable decision".into(),
            business_outcome: "Leaders can act on immutable evidence".into(),
            priority: 5,
            position: 1,
            criteria: vec![Criterion {
                id: "evidence".into(),
                text: "Automated tests verify the evidence contract".into(),
            }],
            rubric_dimensions: vec!["Architecture & platform".into()],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: editor.actor_id.clone(),
            supersedes: None,
        };
        let goal_version_id = goal.id.clone();
        let report = |completed_at| Report {
            id: "desktop-report-1".into(),
            completed_at,
            repositories: vec![],
            goal_version_ids: vec![goal_version_id.clone()],
            goal_set_hash: "goal-set".into(),
            provider: "test".into(),
            provider_version: "1".into(),
            origin: ReportOrigin::Scan,
            assessments: vec![],
            architecture: vec![],
            recommendations: vec![],
            coverage: None,
            unverified_criteria: 0,
            partial: false,
            analysis_warnings: vec![],
            codebase_map_id: None,
            codebase_map_hash: None,
        };
        let first_at = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1);
        let latest_at = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(2);
        let mut events = vec![
            signed(
                1,
                &editor,
                &key,
                1,
                DomainEvent::WorkspaceCreated {
                    name: "Acme".into(),
                    founding_device: editor.clone(),
                    workspace_fingerprint: "fp".into(),
                },
            ),
            signed(
                2,
                &editor,
                &key,
                1,
                DomainEvent::GoalSetReplaced {
                    versions: vec![goal],
                },
            ),
            signed(
                3,
                &editor,
                &key,
                1,
                DomainEvent::ReportCompleted {
                    report: report(first_at),
                },
            ),
            signed(
                4,
                &editor,
                &key,
                1,
                DomainEvent::ReportCompleted {
                    report: report(latest_at),
                },
            ),
        ];

        let mut projection = WorkspaceProjection::rebuild(&events).unwrap();
        assert_eq!(projection.reports.len(), 2);
        assert_eq!(
            projection.reports["desktop-report-1"].completed_at,
            latest_at
        );
        assert!(
            projection
                .reports
                .values()
                .any(|saved| saved.completed_at == first_at)
        );
        assert_eq!(projection.report_completion_count, 2);
        assert_eq!(projection.report_event_ids.len(), 2);
        let first_event_id = events[2].event_id.to_string();
        let latest_event_id = events[3].event_id.to_string();
        assert_eq!(projection.report_ordinals[&first_event_id], 1);
        assert_eq!(projection.report_ordinals[&latest_event_id], 2);

        let deletion = signed(
            5,
            &editor,
            &key,
            1,
            DomainEvent::ReportDeleted {
                report_event_id: first_event_id.clone(),
            },
        );
        projection.apply(&deletion).unwrap();
        events.push(deletion.clone());
        assert_eq!(projection.reports.len(), 1);
        assert!(
            projection
                .deleted_report_event_ids
                .contains(&first_event_id)
        );
        let replayed = WorkspaceProjection::rebuild(&events).unwrap();
        assert_eq!(replayed.reports, projection.reports);
        assert_eq!(replayed.report_ordinals, projection.report_ordinals);
        assert!(replayed.deleted_report_event_ids.contains(&first_event_id));
        let duplicate_deletion = signed(
            6,
            &editor,
            &key,
            1,
            DomainEvent::ReportDeleted {
                report_event_id: first_event_id.clone(),
            },
        );
        assert_eq!(
            projection.apply(&duplicate_deletion),
            Err(ProjectionError::InvalidValue(
                "report is already removed from history".into()
            ))
        );

        let protected = signed(
            7,
            &editor,
            &key,
            1,
            DomainEvent::ReportDeleted {
                report_event_id: latest_event_id,
            },
        );
        assert_eq!(
            projection.apply(&protected),
            Err(ProjectionError::InvalidValue(
                "the latest report cannot be removed from history".into()
            ))
        );
    }
}
