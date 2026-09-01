//! Direct cross-boundary assurance for local runtime-health emission. The
//! records remain device-local and content-free, traverse the signed event
//! store, survive restart, and drive the same aggregates rendered by the app.

use crate::{
    local_state::{CreateWorkspaceRequest, LocalWorkspaceStore, ProjectContext},
    reliability,
};
use codecaddie_domain::{ReliabilityEventKind, ReliabilityOutcome};
use std::fs;

#[test]
fn privacy_adversarial_runtime_health_and_trace_spans_drive_local_aggregates() {
    let directory = tempfile::tempdir().unwrap();
    let repository_path = directory.path().join("repository");
    fs::create_dir(&repository_path).unwrap();
    let data_root = directory.path().join("data");
    let store = LocalWorkspaceStore::new(data_root.clone()).unwrap();
    let workspace = store
        .create_workspace(CreateWorkspaceRequest {
            name: "Runtime health assurance".into(),
            repository_display_name: "repository".into(),
            repository_path: repository_path.to_string_lossy().into_owned(),
            product_brief: "Measure local runtime failures without product content.".into(),
            context: ProjectContext::default(),
        })
        .unwrap();
    let records = [
        reliability::operation_record(
            reliability::new_correlation_id(),
            "scan.run",
            ReliabilityOutcome::Failed,
            Some("provider_failed"),
            true,
            25,
        ),
        reliability::native_panic_record(
            "desktop-assurance-session",
            reliability::new_correlation_id(),
        ),
        reliability::session_record(
            ReliabilityEventKind::DesktopSessionStarted,
            "desktop-assurance-session",
            reliability::new_correlation_id(),
        ),
    ];
    for record in records {
        let serialized = serde_json::to_string(&record).unwrap();
        for forbidden in [
            "repositorySource",
            "attachmentContent",
            "goalText",
            "freeText",
            "credential",
        ] {
            assert!(!serialized.contains(forbidden));
        }
        store
            .record_reliability_operation(&workspace.workspace_id, record)
            .unwrap();
    }
    drop(store);

    let reopened = LocalWorkspaceStore::new(data_root).unwrap();
    let summary = reopened.recent_workspace().unwrap().unwrap().reliability;
    assert_eq!(summary.provider_operation_samples, 1);
    assert_eq!(summary.trace_spans_recorded, 1);
    assert_eq!(summary.provider_operation_failures, 1);
    assert_eq!(summary.provider_alerts_raised, 1);
    assert_eq!(summary.desktop_crashes_detected, 1);
    assert_eq!(summary.desktop_sessions_started, 1);
    assert_eq!(summary.crash_free_sessions_percent, Some(0.0));
}

#[test]
fn privacy_adversarial_all_runtime_telemetry_surfaces_exclude_private_content() {
    let directory = tempfile::tempdir().unwrap();
    let repository_path = directory.path().join("repository");
    fs::create_dir(&repository_path).unwrap();
    let store = LocalWorkspaceStore::new(directory.path().join("data")).unwrap();
    let workspace = store
        .create_workspace(CreateWorkspaceRequest {
            name: "Runtime telemetry privacy assurance".into(),
            repository_display_name: "repository".into(),
            repository_path: repository_path.to_string_lossy().into_owned(),
            product_brief: "Prove every local runtime telemetry surface is content-free.".into(),
            context: ProjectContext::default(),
        })
        .unwrap();

    let operation = reliability::operation_record(
        reliability::new_correlation_id(),
        "scan.run",
        ReliabilityOutcome::Failed,
        Some("provider_timeout"),
        true,
        750,
    );
    let trace = reliability::trace_span_for(&operation).unwrap();
    let alert = reliability::alert_for(&operation).unwrap().unwrap();
    store
        .record_reliability_operation(&workspace.workspace_id, operation.clone())
        .unwrap();

    let panic_marker = directory.path().join("native/last-panic.txt");
    fs::create_dir_all(panic_marker.parent().unwrap()).unwrap();
    let private_marker_body = format!(
        "{}\n{}\nperson@example.invalid\n555-55-5555\nsk-local-secret-canary\nmedical-diagnosis-canary\nfree-form panic detail",
        crate::privacy_test_support::REPOSITORY_FIXTURE,
        crate::privacy_test_support::ATTACHMENT_FIXTURE,
    );
    fs::write(&panic_marker, &private_marker_body).unwrap();
    let (crash_detected, _) = store
        .record_desktop_session_with_panic_marker(
            &workspace.workspace_id,
            ReliabilityEventKind::DesktopSessionStarted,
            "desktop-runtime-assurance",
            Some(&panic_marker),
        )
        .unwrap();
    assert!(crash_detected);
    assert!(!panic_marker.exists());
    assert!(!panic_marker.with_file_name("last-panic.pending").exists());

    let crash_report = reliability::native_panic_record(
        "desktop-runtime-assurance",
        reliability::new_correlation_id(),
    );
    let crash_alert = reliability::crash_alert("desktop-runtime-assurance")
        .unwrap()
        .unwrap();
    let metric_summary = store.recent_workspace().unwrap().unwrap().reliability;
    let every_surface = serde_json::to_vec(&serde_json::json!({
        "log": operation,
        "trace": trace,
        "metric": metric_summary,
        "alert": alert,
        "crashReport": crash_report,
        "crashAlert": crash_alert,
    }))
    .unwrap();

    crate::privacy_test_support::assert_private_payload_absent(&every_surface);
    let serialized = String::from_utf8(every_surface).unwrap();
    for forbidden in [
        "person@example.invalid",
        "555-55-5555",
        "sk-local-secret-canary",
        "medical-diagnosis-canary",
        "free-form panic detail",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "logs, traces, metrics, alerts, and crash reports must share the closed content-free schema",
        );
    }
}
