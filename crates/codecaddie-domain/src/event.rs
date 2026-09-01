use crate::{
    ActionStatus, ActorId, CodebaseMapDescriptor, DeviceId, DeviceIdentity, GoalVersion, Report,
    RepositoryRef, WorkspaceId,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Privacy-preserving product actions that are not already represented by
/// an authoritative workspace event. The signed envelope supplies the time;
/// the payload deliberately carries no repository, attachment, or goal data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionFunnelEventKind {
    AnalysisStarted,
    RepeatAnalysis,
    ReportOpened,
    PromptCopied,
}

/// Versioned, content-free lifecycle observations used to derive local
/// product metrics. The enclosing signed event supplies the immutable
/// workspace identity and timestamp; this record deliberately contains no
/// repository path, source, attachment contents, or goal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductEventKind {
    WorkspaceCreated,
    GoalApproved,
    AnalysisStarted,
    ScorecardGenerated,
    ReportSaved,
    TimeToFirstSavedReport,
    ReportRevisited,
    EvidenceOpened,
    RepeatAnalysisStarted,
    ComparisonGenerated,
    PromptCopied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductEventRecord {
    pub schema_version: u16,
    pub kind: ProductEventKind,
    /// Stable opaque workspace identity duplicated from the signed envelope so
    /// metric consumers can validate a standalone record. Schema-1 history did
    /// not carry this field and therefore serializes exactly as it was signed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workspace_id: String,
    pub session_id: String,
    pub product_version: String,
    pub platform: String,
    pub cohort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_milliseconds: Option<u64>,
}

impl ProductEventRecord {
    pub fn validate(&self, envelope_workspace_id: &str) -> Result<(), &'static str> {
        if !matches!(self.schema_version, 1 | 2) {
            return Err("product event schema version is unsupported");
        }
        if self.schema_version == 2
            && (self.workspace_id.is_empty() || self.workspace_id != envelope_workspace_id)
        {
            return Err("product event workspace identity must match its signed envelope");
        }
        if self.schema_version == 1
            && !self.workspace_id.is_empty()
            && self.workspace_id != envelope_workspace_id
        {
            return Err("legacy product event workspace identity is inconsistent");
        }
        for value in [
            self.workspace_id.as_str(),
            self.session_id.as_str(),
            self.report_id.as_deref().unwrap_or_default(),
        ] {
            if value.len() > 128
                || value.bytes().any(|byte| {
                    !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
                })
            {
                return Err("product event identities must use the bounded identifier alphabet");
            }
        }
        if self.session_id.is_empty()
            || self.product_version.is_empty()
            || self.product_version.len() > 32
            || self.platform.is_empty()
            || self.platform.len() > 64
            || self.cohort.is_empty()
            || self.cohort.len() > 64
        {
            return Err("product event identity and release metadata are incomplete");
        }
        Ok(())
    }
}

/// Content-free reliability observations for the local desktop runtime.
/// These records share the signed workspace ledger with reports and product
/// events so reliability evidence cannot drift into a second telemetry store.
/// Operation and error values are selected from core-owned allowlists; no
/// repository, attachment, goal, provider output, or free-form error text is
/// permitted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityEventKind {
    OperationCompleted,
    TraceSpanCompleted,
    DesktopSessionStarted,
    DesktopSessionEnded,
    DesktopCrashDetected,
    SloAlertRaised,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityErrorCategory {
    Repository,
    Provider,
    Storage,
    Migration,
    Export,
    Protocol,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReliabilityEventRecord {
    pub schema_version: u16,
    pub kind: ReliabilityEventKind,
    pub correlation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ReliabilityOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<ReliabilityErrorCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_milliseconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert_code: Option<String>,
    pub product_version: String,
    pub platform: String,
}

impl ReliabilityEventRecord {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("reliability event schema version is unsupported");
        }
        if uuid::Uuid::parse_str(&self.correlation_id).is_err() {
            return Err("reliability correlation id must be a UUID");
        }
        for value in [
            self.session_id.as_deref(),
            self.operation.as_deref(),
            self.error_code.as_deref(),
            self.alert_code.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if value.is_empty()
                || value.len() > 96
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return Err("reliability identifiers must use the bounded identifier alphabet");
            }
        }
        if self.product_version.is_empty()
            || self.product_version.len() > 32
            || self.platform.is_empty()
            || self.platform.len() > 64
        {
            return Err("reliability product metadata is incomplete");
        }
        match self.kind {
            ReliabilityEventKind::OperationCompleted | ReliabilityEventKind::TraceSpanCompleted => {
                if self.operation.is_none() || self.outcome.is_none() {
                    return Err(
                        "operation and trace reliability events require an operation and outcome",
                    );
                }
            }
            ReliabilityEventKind::DesktopSessionStarted
            | ReliabilityEventKind::DesktopSessionEnded => {
                if self.session_id.is_none() {
                    return Err("desktop reliability events require a session id");
                }
            }
            ReliabilityEventKind::DesktopCrashDetected => {
                if self.session_id.is_none() {
                    return Err("desktop reliability events require a session id");
                }
                if self.error_code.is_some()
                    && (self.error_code.as_deref() != Some("native_panic_detected")
                        || self.error_category != Some(ReliabilityErrorCategory::Internal))
                {
                    return Err("desktop crash events require the native panic marker code");
                }
            }
            ReliabilityEventKind::SloAlertRaised => {
                if self.alert_code.is_none() {
                    return Err("reliability alerts require an alert code");
                }
            }
        }
        if self.outcome == Some(ReliabilityOutcome::Succeeded)
            && (self.error_category.is_some() || self.error_code.is_some())
        {
            return Err("successful reliability events cannot carry errors");
        }
        if self.outcome == Some(ReliabilityOutcome::Failed)
            && (self.error_category.is_none() || self.error_code.is_none())
        {
            return Err("failed reliability events require a categorized error code");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeSurveyDeferral {
    Skip,
    RemindLater,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DomainEvent {
    WorkspaceCreated {
        name: String,
        founding_device: DeviceIdentity,
        workspace_fingerprint: String,
    },
    RepositoryRegistered {
        repository: RepositoryRef,
    },
    GoalVersionProposed {
        version: GoalVersion,
    },
    GoalVersionApproved {
        goal_id: String,
        version_id: String,
    },
    GoalSetReplaced {
        versions: Vec<GoalVersion>,
    },
    ReportCompleted {
        report: Report,
    },
    /// A logical removal from active report history. The referenced signed
    /// completion event remains in the append-only ledger so recovery and
    /// audit integrity are preserved.
    ReportDeleted {
        report_event_id: String,
    },
    DecisionFunnelEventRecorded {
        kind: DecisionFunnelEventKind,
    },
    ProductEventRecorded {
        record: ProductEventRecord,
    },
    ReliabilityEventRecorded {
        record: ReliabilityEventRecord,
    },
    OutcomeSurveyPrompted {
        cycle_id: String,
    },
    OutcomeSurveyDeferred {
        cycle_id: String,
        action: OutcomeSurveyDeferral,
    },
    OutcomeSurveyResponded {
        cycle_id: String,
        report_value_rating: u8,
        decision_confidence_rating: u8,
    },
    /// A validated codebase map was generated or submitted. The signed
    /// descriptor is the system of record; the content-addressed body lives
    /// as a prunable file beside the event log, hash-verified on every read.
    CodebaseMapRecorded {
        descriptor: CodebaseMapDescriptor,
    },
    ActionCreated {
        action_id: String,
        recommendation_id: String,
        title: String,
    },
    ActionTransitioned {
        action_id: String,
        from: ActionStatus,
        to: ActionStatus,
        note: Option<String>,
    },
}

fn verify_hex_signature(public_key: &str, signature: &str, bytes: &[u8]) -> bool {
    let Ok(key_bytes): Result<[u8; 32], _> = hex::decode(public_key).and_then(|bytes| {
        bytes
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)
    }) else {
        return false;
    };
    let Ok(signature_bytes): Result<[u8; 64], _> = hex::decode(signature).and_then(|bytes| {
        bytes
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)
    }) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
        return false;
    };
    key.verify(bytes, &Signature::from_bytes(&signature_bytes))
        .is_ok()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub event_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub lamport: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub device_id: DeviceId,
    pub actor_id: ActorId,
    pub epoch: u64,
    pub event: DomainEvent,
    pub signing_public_key: String,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_signature: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignableEvent<'a> {
    event_id: Uuid,
    workspace_id: &'a str,
    lamport: u64,
    #[serde(with = "time::serde::rfc3339")]
    occurred_at: OffsetDateTime,
    device_id: &'a str,
    actor_id: &'a str,
    epoch: u64,
    event: &'a DomainEvent,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct SignableEventOwned {
    event_id: Uuid,
    workspace_id: WorkspaceId,
    lamport: u64,
    #[serde(with = "time::serde::rfc3339")]
    occurred_at: OffsetDateTime,
    device_id: DeviceId,
    actor_id: ActorId,
    epoch: u64,
    event: DomainEvent,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacySignableEvent<'a> {
    event_id: Uuid,
    workspace_id: &'a str,
    lamport: u64,
    #[serde(with = "time::serde::rfc3339")]
    occurred_at: OffsetDateTime,
    device_id: &'a str,
    actor_id: &'a str,
    epoch: u64,
    event: LegacyDomainEvent<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
enum LegacyDomainEvent<'a> {
    GoalVersionProposed { version: LegacyGoalVersion<'a> },
    ReportCompleted { report: LegacyReport<'a> },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyGoalVersion<'a> {
    id: &'a str,
    goal_id: &'a str,
    title: &'a str,
    business_outcome: &'a str,
    priority: u8,
    criteria: &'a [crate::Criterion],
    rubric_dimensions: &'a [String],
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    created_by: &'a str,
    supersedes: &'a Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyReport<'a> {
    id: &'a str,
    #[serde(with = "time::serde::rfc3339")]
    completed_at: OffsetDateTime,
    repositories: &'a [crate::FrozenRepository],
    goal_version_ids: &'a [String],
    goal_set_hash: &'a str,
    provider: &'a str,
    provider_version: &'a str,
    origin: crate::ReportOrigin,
    assessments: &'a [crate::GoalAssessment],
    architecture: &'a [crate::ArchitectureClaim],
    recommendations: &'a [crate::Recommendation],
    coverage: Option<f64>,
    unverified_criteria: u32,
}

impl EventEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        workspace_id: WorkspaceId,
        lamport: u64,
        occurred_at: OffsetDateTime,
        device_id: DeviceId,
        actor_id: ActorId,
        epoch: u64,
        event: DomainEvent,
        signing_key: &SigningKey,
    ) -> Result<Self, serde_json::Error> {
        let event_id = Uuid::now_v7();
        let signable = SignableEvent {
            event_id,
            workspace_id: &workspace_id,
            lamport,
            occurred_at,
            device_id: &device_id,
            actor_id: &actor_id,
            epoch,
            event: &event,
        };
        // Canonicalize through JSON bytes before storing a `Value`. Directly
        // converting an f32-bearing event into `Value` promotes numbers to
        // f64; one later wire round trip can choose a different shortest
        // decimal and invalidate the extended signature.
        let signing_payload: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&signable)?)?;
        let extended_bytes = serde_json::to_vec(&signing_payload)?;
        // Store the same canonical values that the extended signature covers.
        // Provider reports contain f32 confidence values and an f64 coverage
        // value. JSON's number representation can normalize one of those
        // values while constructing `signing_payload`; retaining the original
        // in-memory event would then make a freshly signed envelope fail its
        // own payload-equivalence check before it was ever persisted.
        let canonical: SignableEventOwned = serde_json::from_slice(&extended_bytes)?;
        let mut envelope = Self {
            event_id: canonical.event_id,
            workspace_id: canonical.workspace_id,
            lamport: canonical.lamport,
            occurred_at: canonical.occurred_at,
            device_id: canonical.device_id,
            actor_id: canonical.actor_id,
            epoch: canonical.epoch,
            event: canonical.event,
            signing_public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            signature: String::new(),
            signing_payload: Some(signing_payload),
            extended_signature: None,
        };
        let compatible_bytes = envelope.n_minus_one_signing_bytes()?;
        envelope.signature = hex::encode(signing_key.sign(&compatible_bytes).to_bytes());
        envelope.extended_signature =
            Some(hex::encode(signing_key.sign(&extended_bytes).to_bytes()));
        Ok(envelope)
    }

    pub fn verify(&self) -> bool {
        let Ok(key_bytes): Result<[u8; 32], _> =
            hex::decode(&self.signing_public_key).and_then(|bytes| {
                bytes
                    .try_into()
                    .map_err(|_| hex::FromHexError::InvalidStringLength)
            })
        else {
            return false;
        };
        let Ok(signature_bytes): Result<[u8; 64], _> =
            hex::decode(&self.signature).and_then(|bytes| {
                bytes
                    .try_into()
                    .map_err(|_| hex::FromHexError::InvalidStringLength)
            })
        else {
            return false;
        };
        let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
            return false;
        };
        let signature = Signature::from_bytes(&signature_bytes);
        if let Some(payload) = &self.signing_payload {
            let Ok(bytes) = serde_json::to_vec(payload) else {
                return false;
            };
            let Ok(signed): Result<SignableEventOwned, _> = serde_json::from_slice(&bytes) else {
                return false;
            };
            if signed.event_id != self.event_id
                || signed.workspace_id != self.workspace_id
                || signed.lamport != self.lamport
                || signed.occurred_at != self.occurred_at
                || signed.device_id != self.device_id
                || signed.actor_id != self.actor_id
                || signed.epoch != self.epoch
                || signed.event != self.event
            {
                return false;
            }
            if let Some(extended_signature) = &self.extended_signature {
                let Ok(primary_bytes) = self.n_minus_one_signing_bytes() else {
                    return false;
                };
                let extended_valid =
                    verify_hex_signature(&self.signing_public_key, extended_signature, &bytes)
                        || serde_json::to_value(SignableEvent {
                            event_id: self.event_id,
                            workspace_id: &self.workspace_id,
                            lamport: self.lamport,
                            occurred_at: self.occurred_at,
                            device_id: &self.device_id,
                            actor_id: &self.actor_id,
                            epoch: self.epoch,
                            event: &self.event,
                        })
                        .ok()
                        .and_then(|legacy_payload| serde_json::to_vec(&legacy_payload).ok())
                        .is_some_and(|legacy_bytes| {
                            verify_hex_signature(
                                &self.signing_public_key,
                                extended_signature,
                                &legacy_bytes,
                            )
                        });
                return extended_valid && key.verify(&primary_bytes, &signature).is_ok();
            }
            return key.verify(&bytes, &signature).is_ok();
        }
        if self.extended_signature.is_some() {
            return false;
        }
        let bytes = {
            let Ok(bytes) = serde_json::to_vec(&SignableEvent {
                event_id: self.event_id,
                workspace_id: &self.workspace_id,
                lamport: self.lamport,
                occurred_at: self.occurred_at,
                device_id: &self.device_id,
                actor_id: &self.actor_id,
                epoch: self.epoch,
                event: &self.event,
            }) else {
                return false;
            };
            bytes
        };
        if key.verify(&bytes, &signature).is_ok() {
            return true;
        }
        self.legacy_signing_bytes()
            .is_some_and(|bytes| key.verify(&bytes, &signature).is_ok())
    }

    fn n_minus_one_signing_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        match &self.event {
            DomainEvent::GoalVersionProposed { version } => {
                self.legacy_event_signing_bytes(LegacyDomainEvent::GoalVersionProposed {
                    version: LegacyGoalVersion {
                        id: &version.id,
                        goal_id: &version.goal_id,
                        title: &version.title,
                        business_outcome: &version.business_outcome,
                        priority: version.priority,
                        criteria: &version.criteria,
                        rubric_dimensions: &version.rubric_dimensions,
                        created_at: version.created_at,
                        created_by: &version.created_by,
                        supersedes: &version.supersedes,
                    },
                })
            }
            DomainEvent::ReportCompleted { report } => {
                self.legacy_event_signing_bytes(LegacyDomainEvent::ReportCompleted {
                    report: LegacyReport {
                        id: &report.id,
                        completed_at: report.completed_at,
                        repositories: &report.repositories,
                        goal_version_ids: &report.goal_version_ids,
                        goal_set_hash: &report.goal_set_hash,
                        provider: &report.provider,
                        provider_version: &report.provider_version,
                        origin: report.origin,
                        assessments: &report.assessments,
                        architecture: &report.architecture,
                        recommendations: &report.recommendations,
                        coverage: report.coverage,
                        unverified_criteria: report.unverified_criteria,
                    },
                })
            }
            _ => serde_json::to_vec(&SignableEvent {
                event_id: self.event_id,
                workspace_id: &self.workspace_id,
                lamport: self.lamport,
                occurred_at: self.occurred_at,
                device_id: &self.device_id,
                actor_id: &self.actor_id,
                epoch: self.epoch,
                event: &self.event,
            }),
        }
    }

    fn legacy_event_signing_bytes(
        &self,
        event: LegacyDomainEvent<'_>,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&LegacySignableEvent {
            event_id: self.event_id,
            workspace_id: &self.workspace_id,
            lamport: self.lamport,
            occurred_at: self.occurred_at,
            device_id: &self.device_id,
            actor_id: &self.actor_id,
            epoch: self.epoch,
            event,
        })
    }

    fn legacy_signing_bytes(&self) -> Option<Vec<u8>> {
        let event = match &self.event {
            DomainEvent::GoalVersionProposed { version } if version.position == 0 => {
                LegacyDomainEvent::GoalVersionProposed {
                    version: LegacyGoalVersion {
                        id: &version.id,
                        goal_id: &version.goal_id,
                        title: &version.title,
                        business_outcome: &version.business_outcome,
                        priority: version.priority,
                        criteria: &version.criteria,
                        rubric_dimensions: &version.rubric_dimensions,
                        created_at: version.created_at,
                        created_by: &version.created_by,
                        supersedes: &version.supersedes,
                    },
                }
            }
            DomainEvent::ReportCompleted { report }
                if !report.partial && report.analysis_warnings.is_empty() =>
            {
                LegacyDomainEvent::ReportCompleted {
                    report: LegacyReport {
                        id: &report.id,
                        completed_at: report.completed_at,
                        repositories: &report.repositories,
                        goal_version_ids: &report.goal_version_ids,
                        goal_set_hash: &report.goal_set_hash,
                        provider: &report.provider,
                        provider_version: &report.provider_version,
                        origin: report.origin,
                        assessments: &report.assessments,
                        architecture: &report.architecture,
                        recommendations: &report.recommendations,
                        coverage: report.coverage,
                        unverified_criteria: report.unverified_criteria,
                    },
                }
            }
            _ => return None,
        };
        self.legacy_event_signing_bytes(event).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Criterion, CriterionAssessment, GoalAssessment, GoalVersion, Report, Verdict};

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct NMinusOneSignableEvent<'a> {
        event_id: Uuid,
        workspace_id: &'a str,
        lamport: u64,
        #[serde(with = "time::serde::rfc3339")]
        occurred_at: OffsetDateTime,
        device_id: &'a str,
        actor_id: &'a str,
        epoch: u64,
        event: NMinusOneDomainEvent<'a>,
    }

    #[derive(Serialize)]
    #[serde(tag = "type", content = "data", rename_all = "snake_case")]
    enum NMinusOneDomainEvent<'a> {
        WorkspaceCreated {
            name: &'a str,
            founding_device: &'a DeviceIdentity,
            workspace_fingerprint: &'a str,
        },
        GoalVersionProposed {
            version: LegacyGoalVersion<'a>,
        },
        ReportCompleted {
            report: LegacyReport<'a>,
        },
    }

    fn n_minus_one_verifies(event: &EventEnvelope) -> bool {
        let compatible_event = match &event.event {
            DomainEvent::WorkspaceCreated {
                name,
                founding_device,
                workspace_fingerprint,
            } => NMinusOneDomainEvent::WorkspaceCreated {
                name,
                founding_device,
                workspace_fingerprint,
            },
            DomainEvent::GoalVersionProposed { version } => {
                NMinusOneDomainEvent::GoalVersionProposed {
                    version: LegacyGoalVersion {
                        id: &version.id,
                        goal_id: &version.goal_id,
                        title: &version.title,
                        business_outcome: &version.business_outcome,
                        priority: version.priority,
                        criteria: &version.criteria,
                        rubric_dimensions: &version.rubric_dimensions,
                        created_at: version.created_at,
                        created_by: &version.created_by,
                        supersedes: &version.supersedes,
                    },
                }
            }
            DomainEvent::ReportCompleted { report } => NMinusOneDomainEvent::ReportCompleted {
                report: LegacyReport {
                    id: &report.id,
                    completed_at: report.completed_at,
                    repositories: &report.repositories,
                    goal_version_ids: &report.goal_version_ids,
                    goal_set_hash: &report.goal_set_hash,
                    provider: &report.provider,
                    provider_version: &report.provider_version,
                    origin: report.origin,
                    assessments: &report.assessments,
                    architecture: &report.architecture,
                    recommendations: &report.recommendations,
                    coverage: report.coverage,
                    unverified_criteria: report.unverified_criteria,
                },
            },
            _ => return false,
        };
        let bytes = serde_json::to_vec(&NMinusOneSignableEvent {
            event_id: event.event_id,
            workspace_id: &event.workspace_id,
            lamport: event.lamport,
            occurred_at: event.occurred_at,
            device_id: &event.device_id,
            actor_id: &event.actor_id,
            epoch: event.epoch,
            event: compatible_event,
        })
        .unwrap();
        verify_hex_signature(&event.signing_public_key, &event.signature, &bytes)
    }

    fn legacy_wire_fixture(
        mut event: EventEnvelope,
        key: &SigningKey,
        omitted: &[&str],
    ) -> (EventEnvelope, Vec<u8>) {
        let legacy_bytes = event.legacy_signing_bytes().unwrap();
        event.signature = hex::encode(key.sign(&legacy_bytes).to_bytes());
        event.signing_payload = None;
        event.extended_signature = None;
        let mut value = serde_json::to_value(event).unwrap();
        let payload = value["event"]["data"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap();
        for field in omitted {
            payload.remove(*field);
        }
        let wire = serde_json::to_vec(&value).unwrap();
        (serde_json::from_slice(&wire).unwrap(), wire)
    }

    #[test]
    fn signed_events_reject_tampering() {
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let mut event = EventEnvelope::sign(
            "workspace".into(),
            1,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::WorkspaceCreated {
                name: "Acme".into(),
                founding_device: DeviceIdentity {
                    actor_id: "owner".into(),
                    device_id: "device".into(),
                    signing_public_key: hex::encode(key.verifying_key().to_bytes()),
                    label: "Owner Mac".into(),
                },
                workspace_fingerprint: "fingerprint".into(),
            },
            &key,
        )
        .unwrap();
        assert!(event.verify());
        assert!(n_minus_one_verifies(&event));

        let restored: EventEnvelope =
            serde_json::from_str(&serde_json::to_string(&event).expect("signed event serializes"))
                .expect("signed event deserializes");
        assert!(restored.verify());

        let mut payload_tampered = event.clone();
        payload_tampered.signing_payload.as_mut().unwrap()["lamport"] = serde_json::json!(2);
        assert!(!payload_tampered.verify());

        let legacy_bytes = serde_json::to_vec(&SignableEvent {
            event_id: event.event_id,
            workspace_id: &event.workspace_id,
            lamport: event.lamport,
            occurred_at: event.occurred_at,
            device_id: &event.device_id,
            actor_id: &event.actor_id,
            epoch: event.epoch,
            event: &event.event,
        })
        .unwrap();
        let mut legacy = event.clone();
        legacy.signature = hex::encode(key.sign(&legacy_bytes).to_bytes());
        legacy.signing_payload = None;
        legacy.extended_signature = None;
        assert!(legacy.verify());

        event.lamport = 2;
        assert!(!event.verify());
    }

    #[test]
    fn current_goal_and_report_events_remain_verifiable_by_n_minus_one() {
        let key = SigningKey::from_bytes(&[10_u8; 32]);
        let goal = EventEnvelope::sign(
            "workspace".into(),
            2,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::GoalVersionProposed {
                version: GoalVersion {
                    id: "goal-version-current".into(),
                    goal_id: "goal-current".into(),
                    title: "Customers reach value".into(),
                    business_outcome: "Adoption improves".into(),
                    priority: 5,
                    position: 4,
                    criteria: vec![Criterion {
                        id: "activation".into(),
                        text: "Activation is measured".into(),
                    }],
                    rubric_dimensions: vec!["Business & product".into()],
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    created_by: "owner".into(),
                    supersedes: None,
                },
            },
            &key,
        )
        .unwrap();
        assert!(goal.verify());
        assert!(n_minus_one_verifies(&goal));

        let report = EventEnvelope::sign(
            "workspace".into(),
            3,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::ReportCompleted {
                report: Report {
                    id: "report-current".into(),
                    completed_at: OffsetDateTime::UNIX_EPOCH,
                    repositories: vec![],
                    goal_version_ids: vec!["goal-version-current".into()],
                    goal_set_hash: "goals".into(),
                    provider: "test".into(),
                    provider_version: "1".into(),
                    origin: crate::ReportOrigin::Scan,
                    assessments: vec![GoalAssessment {
                        goal_version_id: "goal-version-current".into(),
                        verdict: Verdict::Partial,
                        summary: "Some evidence was found".into(),
                        architecture_narrative: String::new(),
                        related_component_ids: vec![],
                        criteria: vec![CriterionAssessment {
                            criterion_id: "criterion-current".into(),
                            verdict: Verdict::Partial,
                            rationale: "One bounded citation was validated".into(),
                            confidence: 0.96,
                            evidence: vec![],
                        }],
                    }],
                    architecture: vec![],
                    recommendations: vec![],
                    coverage: None,
                    unverified_criteria: 1,
                    partial: true,
                    analysis_warnings: vec!["one batch did not finish".into()],
                    codebase_map_id: None,
                    codebase_map_hash: None,
                },
            },
            &key,
        )
        .unwrap();
        assert!(report.verify());
        assert!(n_minus_one_verifies(&report));

        // Releases between the introduction of `signingPayload` and its
        // canonicalization signed a direct Value conversion. A confidence
        // such as 0.96 can change its f64 decimal representation after the
        // envelope's wire round trip. Preserve those already-written events.
        let mut precanonical = report.clone();
        let payload = serde_json::to_value(SignableEvent {
            event_id: precanonical.event_id,
            workspace_id: &precanonical.workspace_id,
            lamport: precanonical.lamport,
            occurred_at: precanonical.occurred_at,
            device_id: &precanonical.device_id,
            actor_id: &precanonical.actor_id,
            epoch: precanonical.epoch,
            event: &precanonical.event,
        })
        .unwrap();
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        precanonical.signing_payload = Some(payload);
        precanonical.extended_signature = Some(hex::encode(key.sign(&payload_bytes).to_bytes()));
        let restored: EventEnvelope =
            serde_json::from_str(&serde_json::to_string(&precanonical).unwrap()).unwrap();
        assert!(restored.verify());

        let DomainEvent::ReportCompleted {
            report: base_report,
        } = report.event
        else {
            panic!("expected report event")
        };
        for (index, confidence) in [0.01_f32, 0.07, 0.13, 0.51, 0.56, 0.82, 0.96, 0.99, 1.0]
            .into_iter()
            .enumerate()
        {
            let mut provider_report = base_report.clone();
            provider_report.id = format!("provider-report-{index}");
            provider_report.assessments[0].criteria[0].confidence = confidence;
            provider_report.coverage = Some(f64::from(index as u32 * 17 + 1) / 320.0);
            let event = EventEnvelope::sign(
                "workspace".into(),
                index as u64 + 4,
                OffsetDateTime::UNIX_EPOCH,
                "device".into(),
                "owner".into(),
                1,
                DomainEvent::ReportCompleted {
                    report: provider_report,
                },
                &key,
            )
            .unwrap();
            assert!(
                event.verify(),
                "provider report {index} verifies immediately"
            );
            let restored: EventEnvelope =
                serde_json::from_slice(&serde_json::to_vec(&event).unwrap()).unwrap();
            assert!(
                restored.verify(),
                "provider report {index} verifies after wire round trip"
            );
            assert!(n_minus_one_verifies(&event));
        }
    }

    #[test]
    fn signed_historical_reports_remain_valid_without_summary_fields() {
        let key = SigningKey::from_bytes(&[8_u8; 32]);
        let event = EventEnvelope::sign(
            "workspace".into(),
            2,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::ReportCompleted {
                report: Report {
                    id: "report-historical".into(),
                    completed_at: OffsetDateTime::UNIX_EPOCH,
                    repositories: vec![],
                    goal_version_ids: vec!["goal-version".into()],
                    goal_set_hash: "goals".into(),
                    provider: "test".into(),
                    provider_version: "1".into(),
                    origin: crate::ReportOrigin::Scan,
                    assessments: vec![GoalAssessment {
                        goal_version_id: "goal-version".into(),
                        verdict: Verdict::Unverified,
                        summary: String::new(),
                        architecture_narrative: String::new(),
                        related_component_ids: vec![],
                        criteria: vec![CriterionAssessment {
                            criterion_id: "criterion".into(),
                            verdict: Verdict::Unverified,
                            rationale: "Not yet verified".into(),
                            confidence: 0.5,
                            evidence: vec![],
                        }],
                    }],
                    architecture: vec![],
                    recommendations: vec![],
                    coverage: None,
                    unverified_criteria: 1,
                    partial: false,
                    analysis_warnings: vec![],
                    codebase_map_id: None,
                    codebase_map_hash: None,
                },
            },
            &key,
        )
        .unwrap();
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(!serialized.contains("\"summary\""));
        let restored: EventEnvelope = serde_json::from_str(&serialized).unwrap();
        assert!(restored.verify());

        let (legacy, wire) = legacy_wire_fixture(event, &key, &["partial", "analysisWarnings"]);
        assert_eq!(legacy.event, restored.event);
        assert!(legacy.verify());
        let wire = String::from_utf8(wire).unwrap();
        assert!(!wire.contains("signingPayload"));
        assert!(!wire.contains("\"partial\""));
        assert!(!wire.contains("analysisWarnings"));
    }

    #[test]
    fn signed_historical_goal_versions_remain_valid_without_positions() {
        let key = SigningKey::from_bytes(&[9_u8; 32]);
        let event = EventEnvelope::sign(
            "workspace".into(),
            3,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::GoalVersionProposed {
                version: GoalVersion {
                    id: "goal-version-historical".into(),
                    goal_id: "goal-historical".into(),
                    title: "Customers reach value".into(),
                    business_outcome: "Adoption improves".into(),
                    priority: 5,
                    position: 0,
                    criteria: vec![Criterion {
                        id: "activation".into(),
                        text: "Activation is measured".into(),
                    }],
                    rubric_dimensions: vec!["Business & product".into()],
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    created_by: "owner".into(),
                    supersedes: None,
                },
            },
            &key,
        )
        .unwrap();
        let (legacy, wire) = legacy_wire_fixture(event, &key, &["position"]);
        assert!(legacy.verify());
        let wire = String::from_utf8(wire).unwrap();
        assert!(!wire.contains("signingPayload"));
        assert!(!wire.contains("\"position\""));
    }

    #[test]
    fn schema_one_product_events_remain_signed_history_compatible() {
        let record = ProductEventRecord {
            schema_version: 1,
            kind: ProductEventKind::AnalysisStarted,
            workspace_id: String::new(),
            session_id: "analysis-legacy".into(),
            product_version: "0.2.0".into(),
            platform: "macos-aarch64".into(),
            cohort: "desktop-0.2.0".into(),
            report_id: Some("report-legacy".into()),
            elapsed_milliseconds: None,
        };
        assert!(record.validate("workspace").is_ok());
        assert!(
            !serde_json::to_string(&record)
                .unwrap()
                .contains("workspaceId")
        );

        let key = SigningKey::from_bytes(&[11_u8; 32]);
        let event = EventEnvelope::sign(
            "workspace".into(),
            4,
            OffsetDateTime::UNIX_EPOCH,
            "device".into(),
            "owner".into(),
            1,
            DomainEvent::ProductEventRecorded { record },
            &key,
        )
        .unwrap();
        let restored: EventEnvelope =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert!(restored.verify());
    }

    #[test]
    fn local_measurement_records_reject_unreviewed_serialized_fields() {
        let product = ProductEventRecord {
            schema_version: 2,
            kind: ProductEventKind::EvidenceOpened,
            workspace_id: "workspace".into(),
            session_id: "session".into(),
            product_version: "0.3.0".into(),
            platform: "macos-aarch64".into(),
            cohort: "desktop-0.3.0".into(),
            report_id: Some("report".into()),
            elapsed_milliseconds: None,
        };
        let mut product_json = serde_json::to_value(product).unwrap();
        product_json["repositorySource"] = serde_json::json!("must not be admitted");
        assert!(serde_json::from_value::<ProductEventRecord>(product_json).is_err());

        let reliability = ReliabilityEventRecord {
            schema_version: 1,
            kind: ReliabilityEventKind::OperationCompleted,
            correlation_id: Uuid::nil().to_string(),
            session_id: None,
            operation: Some("scan.run".into()),
            outcome: Some(ReliabilityOutcome::Failed),
            error_category: Some(ReliabilityErrorCategory::Provider),
            error_code: Some("provider_failed".into()),
            retryable: true,
            elapsed_milliseconds: Some(1),
            alert_code: None,
            product_version: "0.3.0".into(),
            platform: "macos-aarch64".into(),
        };
        let mut reliability_json = serde_json::to_value(reliability).unwrap();
        reliability_json["freeText"] = serde_json::json!("must not be admitted");
        assert!(serde_json::from_value::<ReliabilityEventRecord>(reliability_json).is_err());
    }
}
