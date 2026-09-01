//! Local-only reliability contracts shared by request dispatch and the
//! signed workspace ledger. This module intentionally accepts identifiers
//! selected by CodeCaddie code, never repository text or provider output.

use codecaddie_domain::{
    ReliabilityErrorCategory, ReliabilityEventKind, ReliabilityEventRecord, ReliabilityOutcome,
};
use serde::Deserialize;
use uuid::Uuid;

pub const RELIABILITY_EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityPolicy {
    pub schema_version: u16,
    pub owner: String,
    pub review_cadence: String,
    pub local_only: bool,
    pub operations: Vec<OperationSlo>,
    pub customer_journeys: Vec<CustomerJourneySlo>,
    pub desktop_session: DesktopSessionSlo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSlo {
    pub operation: String,
    pub availability_percent: f64,
    pub latency_milliseconds: u64,
    pub alert_after_consecutive_failures: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerJourneySlo {
    pub journey: String,
    pub availability_percent: f64,
    pub latency_milliseconds: u64,
    pub owner: String,
    pub alert_code: String,
    pub measured_by: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSessionSlo {
    pub crash_free_percent: f64,
    pub alert_on_detected_crash: bool,
}

pub fn policy() -> anyhow::Result<ReliabilityPolicy> {
    let policy: ReliabilityPolicy = serde_json::from_str(include_str!(
        "../../../config/service-level-objectives.json"
    ))?;
    if policy.schema_version != RELIABILITY_EVENT_SCHEMA_VERSION
        || policy.owner.trim().is_empty()
        || policy.review_cadence.trim().is_empty()
        || !policy.local_only
        || policy.operations.is_empty()
        || policy.customer_journeys.is_empty()
        || !(0.0..=100.0).contains(&policy.desktop_session.crash_free_percent)
    {
        anyhow::bail!("the local reliability policy is incomplete");
    }
    for operation in &policy.operations {
        if operation.operation.trim().is_empty()
            || !(0.0..=100.0).contains(&operation.availability_percent)
            || operation.latency_milliseconds == 0
            || operation.alert_after_consecutive_failures == 0
        {
            anyhow::bail!("the local reliability operation policy is incomplete");
        }
    }
    for journey in &policy.customer_journeys {
        if journey.journey.trim().is_empty()
            || !(0.0..=100.0).contains(&journey.availability_percent)
            || journey.latency_milliseconds == 0
            || journey.owner.trim().is_empty()
            || journey.alert_code.trim().is_empty()
            || journey.measured_by.is_empty()
            || journey
                .measured_by
                .iter()
                .any(|event| event.trim().is_empty())
        {
            anyhow::bail!("the local reliability customer-journey policy is incomplete");
        }
    }
    Ok(policy)
}

pub fn new_correlation_id() -> String {
    Uuid::now_v7().to_string()
}

pub fn classify_error(operation: &str, code: &str) -> ReliabilityErrorCategory {
    if operation.starts_with("reports.") || code.contains("export") {
        ReliabilityErrorCategory::Export
    } else if code.contains("migration") {
        ReliabilityErrorCategory::Migration
    } else if operation.starts_with("scan.")
        || operation.starts_with("goals.generate")
        || operation.starts_with("map.generate")
        || code.contains("provider")
        || code.contains("scan")
    {
        ReliabilityErrorCategory::Provider
    } else if operation.starts_with("workspace.")
        && (code.contains("repository") || code.contains("workspace_load"))
    {
        ReliabilityErrorCategory::Repository
    } else if code.contains("persist")
        || code.contains("storage")
        || code.contains("write")
        || code.contains("state")
    {
        ReliabilityErrorCategory::Storage
    } else if code.contains("protocol") || code.contains("params") || code.contains("method") {
        ReliabilityErrorCategory::Protocol
    } else {
        ReliabilityErrorCategory::Internal
    }
}

/// Returns whether an operation crosses CodeCaddie's repository-owned provider
/// boundary. The operation name is selected by CodeCaddie, never by repository
/// or provider text, so it is safe to use for device-local aggregation.
pub fn is_provider_operation(operation: &str) -> bool {
    operation == "scan.run"
        || operation == "goals.generate"
        || operation == "map.generate"
        || operation.starts_with("provider.")
}

fn product_metadata() -> (String, String) {
    (
        env!("CARGO_PKG_VERSION").to_string(),
        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
    )
}

pub fn operation_record(
    correlation_id: String,
    operation: &str,
    outcome: ReliabilityOutcome,
    error_code: Option<&str>,
    retryable: bool,
    elapsed_milliseconds: u64,
) -> ReliabilityEventRecord {
    let (product_version, platform) = product_metadata();
    let error_category = error_code.map(|code| classify_error(operation, code));
    ReliabilityEventRecord {
        schema_version: RELIABILITY_EVENT_SCHEMA_VERSION,
        kind: ReliabilityEventKind::OperationCompleted,
        correlation_id,
        session_id: None,
        operation: Some(operation.to_string()),
        outcome: Some(outcome),
        error_category,
        error_code: error_code.map(str::to_string),
        retryable,
        elapsed_milliseconds: Some(elapsed_milliseconds),
        alert_code: None,
        product_version,
        platform,
    }
}

/// Treats the signed operation record itself as its source-safe local trace
/// span. Reusing the existing wire record keeps newly written history readable
/// by every supported prior binary; `TraceSpanCompleted` remains a read-only
/// compatibility variant for history written by earlier builds.
pub fn trace_span_for(record: &ReliabilityEventRecord) -> Option<&ReliabilityEventRecord> {
    (record.kind == ReliabilityEventKind::OperationCompleted).then_some(record)
}

pub fn session_record(
    kind: ReliabilityEventKind,
    session_id: &str,
    correlation_id: String,
) -> ReliabilityEventRecord {
    let (product_version, platform) = product_metadata();
    ReliabilityEventRecord {
        schema_version: RELIABILITY_EVENT_SCHEMA_VERSION,
        kind,
        correlation_id,
        session_id: Some(session_id.to_string()),
        operation: None,
        outcome: None,
        error_category: None,
        error_code: None,
        retryable: false,
        elapsed_milliseconds: None,
        alert_code: None,
        product_version,
        platform,
    }
}

pub fn native_panic_record(session_id: &str, correlation_id: String) -> ReliabilityEventRecord {
    let mut record = session_record(
        ReliabilityEventKind::DesktopCrashDetected,
        session_id,
        correlation_id,
    );
    record.error_category = Some(ReliabilityErrorCategory::Internal);
    record.error_code = Some("native_panic_detected".into());
    record
}

pub fn alert_for(
    record: &ReliabilityEventRecord,
) -> anyhow::Result<Option<ReliabilityEventRecord>> {
    let operation = match record.operation.as_deref() {
        Some(operation) => operation,
        None => return Ok(None),
    };
    let policy = policy()?;
    let Some(slo) = policy
        .operations
        .iter()
        .find(|candidate| candidate.operation == operation)
    else {
        return Ok(None);
    };
    let alert_code = if record.outcome == Some(ReliabilityOutcome::Failed) {
        Some("customer_operation_failed")
    } else if record.elapsed_milliseconds.unwrap_or_default() > slo.latency_milliseconds {
        Some("operation_latency_slo_breached")
    } else {
        None
    };
    let Some(alert_code) = alert_code else {
        return Ok(None);
    };
    let mut alert = record.clone();
    alert.kind = ReliabilityEventKind::SloAlertRaised;
    alert.alert_code = Some(alert_code.to_string());
    Ok(Some(alert))
}

pub fn crash_alert(session_id: &str) -> anyhow::Result<Option<ReliabilityEventRecord>> {
    if !policy()?.desktop_session.alert_on_detected_crash {
        return Ok(None);
    }
    let mut alert = session_record(
        ReliabilityEventKind::SloAlertRaised,
        session_id,
        new_correlation_id(),
    );
    // Tag alerts produced from the authoritative Native SDK marker so the
    // projection can distinguish them from the short-lived schema-1 behavior
    // that inferred a crash from an unmatched session start.
    alert.error_category = Some(ReliabilityErrorCategory::Internal);
    alert.error_code = Some("native_panic_detected".into());
    alert.alert_code = Some("desktop_crash_detected".into());
    Ok(Some(alert))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reliability_policy_is_versioned_owned_local_and_bounded() {
        let policy = policy().unwrap();
        assert_eq!(policy.schema_version, 1);
        assert!(policy.local_only);
        assert!(!policy.owner.is_empty());
        assert!(
            policy
                .operations
                .iter()
                .any(|item| item.operation == "scan.run")
        );
        assert_eq!(policy.customer_journeys.len(), 4);
        for journey in &policy.customer_journeys {
            assert!(!journey.owner.is_empty());
            assert!(!journey.alert_code.is_empty());
            assert!(!journey.measured_by.is_empty());
        }
        assert!(
            policy
                .operations
                .iter()
                .all(|item| item.latency_milliseconds <= 15 * 60 * 1_000)
        );
    }

    #[test]
    fn failure_records_are_categorized_and_raise_content_free_alerts() {
        let record = operation_record(
            new_correlation_id(),
            "scan.run",
            ReliabilityOutcome::Failed,
            Some("scan_failed"),
            true,
            100,
        );
        record.validate().unwrap();
        assert_eq!(
            record.error_category,
            Some(ReliabilityErrorCategory::Provider)
        );
        let alert = alert_for(&record).unwrap().unwrap();
        alert.validate().unwrap();
        assert_eq!(alert.correlation_id, record.correlation_id);
        let json = serde_json::to_string(&alert).unwrap();
        for forbidden in [
            "repositoryPath",
            "sourceText",
            "prompt",
            "message",
            "goalText",
        ] {
            assert!(!json.contains(forbidden));
        }
    }

    #[test]
    fn provider_boundary_operations_are_closed_and_code_owned() {
        for operation in [
            "scan.run",
            "goals.generate",
            "map.generate",
            "provider.probe",
        ] {
            assert!(is_provider_operation(operation));
        }
        for operation in [
            "workspace.recent",
            "reports.export_word",
            "scan.run.untrusted",
        ] {
            assert!(!is_provider_operation(operation));
        }
    }

    #[test]
    fn trace_spans_reuse_the_supported_operation_wire_shape() {
        let record = operation_record(
            new_correlation_id(),
            "scan.run",
            ReliabilityOutcome::Succeeded,
            None,
            false,
            25,
        );
        let trace = trace_span_for(&record).unwrap();
        assert!(std::ptr::eq(trace, &record));
        let json = serde_json::to_value(trace).unwrap();
        assert_eq!(json["kind"], "operation_completed");
        assert_eq!(json["correlationId"], record.correlation_id);
        assert_eq!(json["operation"], "scan.run");
    }

    #[test]
    fn crash_alerts_are_bound_to_authoritative_native_panic_evidence() {
        let alert = crash_alert("desktop-session").unwrap().unwrap();
        alert.validate().unwrap();
        assert_eq!(alert.alert_code.as_deref(), Some("desktop_crash_detected"));
        assert_eq!(alert.error_code.as_deref(), Some("native_panic_detected"));
        assert_eq!(
            alert.error_category,
            Some(ReliabilityErrorCategory::Internal)
        );
    }
}
