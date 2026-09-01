//! Core request dispatch. `handle` validates the envelope and routes each
//! method through the `DISPATCH` table to a per-domain handler module; the handlers
//! own their param parsing, storage access, and response shaping so each
//! method is independently testable.

mod actions;
mod context;
mod goals;
mod instrumentation;
mod map;
mod providers;
mod recommendations;
mod reliability;
mod reports;
mod scan;
mod settings;
mod system;
mod updates;
mod workspace;

use crate::{
    protocol::{CoreRequest, CoreResponse},
    provider::ProgressSink,
};

/// A dispatchable handler: takes ownership of the validated request and
/// resolves to exactly one terminal response.
type HandlerFuture = std::pin::Pin<Box<dyn Future<Output = CoreResponse> + Send>>;
type Handler = fn(CoreRequest) -> HandlerFuture;

/// The dispatch table: one entry per implemented core method, in the same
/// order as [`METHODS`]. Adding a method means adding a handler here, its
/// name to [`METHODS`], and a row to the catalog in `protocol/README.md`;
/// the tests below fail if the table and the catalog drift apart.
const DISPATCH: &[(&str, Handler)] = &[
    ("system.ping", |request| Box::pin(system::ping(request))),
    ("updates.check", |request| Box::pin(updates::check(request))),
    ("updates.download", |request| {
        Box::pin(updates::download(request))
    }),
    ("updates.install", |request| {
        Box::pin(updates::install(request))
    }),
    ("settings.launchAtLogin.get", |request| {
        Box::pin(settings::launch_at_login_get(request))
    }),
    ("settings.launchAtLogin.set", |request| {
        Box::pin(settings::launch_at_login_set(request))
    }),
    ("settings.provider.get", |request| {
        Box::pin(settings::provider_get(request))
    }),
    ("settings.provider.set", |request| {
        Box::pin(settings::provider_set(request))
    }),
    ("providers.detect", |request| {
        Box::pin(providers::detect(request))
    }),
    ("privacy.promise", |request| {
        Box::pin(system::privacy_promise(request))
    }),
    ("context.files.inspect", |request| {
        Box::pin(context::inspect_files(request))
    }),
    ("workspace.create", |request| {
        Box::pin(workspace::create(request))
    }),
    ("workspace.recent", |request| {
        Box::pin(workspace::recent(request))
    }),
    ("workspace.open", |request| {
        Box::pin(workspace::open(request))
    }),
    ("workspace.context.update", |request| {
        Box::pin(workspace::context_update(request))
    }),
    ("workspace.recovery.export", |request| {
        Box::pin(workspace::recovery_export(request))
    }),
    ("workspace.backup.export", |request| {
        Box::pin(workspace::backup_export(request))
    }),
    ("workspace.backup.import", |request| {
        Box::pin(workspace::backup_import(request))
    }),
    ("workspace.backup.schedule.status", |request| {
        Box::pin(workspace::backup_schedule_status(request))
    }),
    ("workspace.backup.schedule.enable", |request| {
        Box::pin(workspace::backup_schedule_enable(request))
    }),
    ("workspace.backup.schedule.disable", |request| {
        Box::pin(workspace::backup_schedule_disable(request))
    }),
    ("workspace.backup.schedule.run", |request| {
        Box::pin(workspace::backup_schedule_run(request))
    }),
    ("reports.export_word", |request| {
        Box::pin(reports::export_word(request))
    }),
    ("reports.history.list", |request| {
        Box::pin(reports::history_list(request))
    }),
    ("reports.finding.get", |request| {
        Box::pin(reports::finding_get(request))
    }),
    ("reports.delete", |request| {
        Box::pin(reports::delete(request))
    }),
    ("goals.approve", |request| Box::pin(goals::approve(request))),
    ("goals.replace", |request| Box::pin(goals::replace(request))),
    ("actions.ready", |request| Box::pin(actions::ready(request))),
    ("instrumentation.record", |request| {
        Box::pin(instrumentation::record(request))
    }),
    ("recommendations.prompt", |request| {
        Box::pin(recommendations::prompt(request))
    }),
    ("recommendations.copy_prompt", |request| {
        Box::pin(recommendations::copy_prompt(request))
    }),
    ("reliability.record", |request| {
        Box::pin(reliability::record(request))
    }),
    ("scan.run", |request| Box::pin(scan::run(request, None))),
    ("goals.generate", |request| {
        Box::pin(goals::generate(request, None))
    }),
    ("map.generate", |request| {
        Box::pin(map::generate(request, None))
    }),
    ("map.get", |request| Box::pin(map::get(request))),
];

/// Serializes a result payload into a success response, degrading to a
/// non-retryable `internal_error` failure instead of panicking the core
/// process. Serde errors describe Rust types, never repository text, so the
/// message is safe to cross the desktop IPC boundary.
fn serialized_success<T: serde::Serialize>(id: String, what: &str, value: &T) -> CoreResponse {
    match serde_json::to_value(value) {
        Ok(value) => CoreResponse::success(id, value),
        Err(error) => CoreResponse::failure(
            id,
            "internal_error",
            format!("the {what} could not be serialized: {error}"),
            false,
        ),
    }
}

/// Parses the request params into the method's typed request, mapping any
/// deserialization error to the standard non-retryable `invalid_params`
/// failure. The error text describes Rust types and JSON shapes, never
/// repository text.
fn parsed_params<T: serde::de::DeserializeOwned>(
    id: &str,
    params: serde_json::Map<String, serde_json::Value>,
) -> Result<T, Box<CoreResponse>> {
    serde_json::from_value(serde_json::Value::Object(params)).map_err(|error| {
        Box::new(CoreResponse::failure(
            id,
            "invalid_params",
            error.to_string(),
            false,
        ))
    })
}

/// Extracts the workspace id a workspace-scoped method requires, mapping
/// its absence to the standard non-retryable `workspace_required` failure
/// with the method's own guidance message.
fn required_workspace(
    request: &CoreRequest,
    message: &'static str,
) -> Result<String, Box<CoreResponse>> {
    match request.workspace_id.as_deref() {
        Some(workspace_id) => Ok(workspace_id.to_owned()),
        None => Err(Box::new(CoreResponse::failure(
            request.id.clone(),
            "workspace_required",
            message,
            false,
        ))),
    }
}

/// Whether a request opted into NDJSON progress streaming. Only the two
/// provider-backed long-running methods support it, and only when the host
/// asked with `"stream": true` — everything else stays length-prefixed.
pub fn streams_progress(request: &CoreRequest) -> bool {
    matches!(
        request.method.as_str(),
        "goals.generate" | "scan.run" | "map.generate"
    ) && request
        .params
        .get("stream")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Handles a streaming request, forwarding sanitized progress lines to the
/// sink while the provider runs. Non-streaming methods delegate to
/// [`handle`]. The terminal response for `scan.run` is a slim receipt (the
/// report is persisted and re-read through `workspace.recent`) so it always
/// fits a line-framed transport.
pub async fn handle_with_progress(request: CoreRequest, progress: ProgressSink) -> CoreResponse {
    let started = std::time::Instant::now();
    if let Err(error) = request.validate() {
        let response = CoreResponse::failure(
            request.id.clone(),
            "protocol_version",
            error.to_string(),
            false,
        );
        return finalize_response(&request, response, started.elapsed());
    }
    if let Some(response) = runtime_control_response(&request) {
        return finalize_response(&request, response, started.elapsed());
    }
    let response = match request.method.as_str() {
        "goals.generate" => goals::generate(request.clone(), Some(progress)).await,
        "scan.run" => scan::run(request.clone(), Some(progress)).await,
        "map.generate" => map::generate(request.clone(), Some(progress)).await,
        _ => return handle(request).await,
    };
    finalize_response(&request, response, started.elapsed())
}

pub async fn handle(request: CoreRequest) -> CoreResponse {
    let started = std::time::Instant::now();
    if let Err(error) = request.validate() {
        let response = CoreResponse::failure(
            request.id.clone(),
            "protocol_version",
            error.to_string(),
            false,
        );
        return finalize_response(&request, response, started.elapsed());
    }
    if let Some(response) = runtime_control_response(&request) {
        return finalize_response(&request, response, started.elapsed());
    }
    let Some(handler) = DISPATCH
        .iter()
        .find(|(method, _)| *method == request.method)
        .map(|(_, handler)| *handler)
    else {
        let response = CoreResponse::failure(
            request.id.clone(),
            "method_not_found",
            format!("Unknown core method: {}", request.method),
            false,
        );
        return finalize_response(&request, response, started.elapsed());
    };
    let response = handler(request.clone()).await;
    finalize_response(&request, response, started.elapsed())
}

fn runtime_control_response(request: &CoreRequest) -> Option<CoreResponse> {
    if !crate::runtime_controls::method_is_controlled(&request.method) {
        return None;
    }
    runtime_control_response_with(
        request,
        crate::runtime_controls::method_is_paused(&request.method).map_err(|_| ()),
    )
}

fn runtime_control_response_with(
    request: &CoreRequest,
    paused: Result<bool, ()>,
) -> Option<CoreResponse> {
    match paused {
        Ok(false) => None,
        Ok(true) => Some(CoreResponse::failure(
            request.id.clone(),
            "feature_paused",
            "This operation is temporarily paused by the local release control. Existing customer state is unchanged.",
            false,
        )),
        Err(_) => Some(CoreResponse::failure(
            request.id.clone(),
            "feature_control_unavailable",
            "The local release control could not be verified. The operation was not started and existing customer state is unchanged.",
            false,
        )),
    }
}

fn response_workspace_id(request: &CoreRequest, response: &CoreResponse) -> Option<String> {
    request.workspace_id.clone().or_else(|| {
        response
            .result
            .as_ref()
            .and_then(|result| result.get("workspaceId"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("workspace"))
                    .and_then(|workspace| workspace.get("workspaceId"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
    })
}

fn finalize_response(
    request: &CoreRequest,
    response: CoreResponse,
    elapsed: std::time::Duration,
) -> CoreResponse {
    finalize_response_with(request, response, elapsed, |workspace_id, record| {
        crate::local_state::LocalWorkspaceStore::from_environment()?
            .record_reliability_operation(workspace_id, record)
    })
}

pub(crate) fn finalize_response_with<F>(
    request: &CoreRequest,
    mut response: CoreResponse,
    elapsed: std::time::Duration,
    recorder: F,
) -> CoreResponse
where
    F: FnOnce(&str, codecaddie_domain::ReliabilityEventRecord) -> anyhow::Result<()>,
{
    let correlation_id = crate::reliability::new_correlation_id();
    let error = response.error.as_ref();
    let outcome = if response.ok {
        codecaddie_domain::ReliabilityOutcome::Succeeded
    } else {
        codecaddie_domain::ReliabilityOutcome::Failed
    };
    let record = crate::reliability::operation_record(
        correlation_id.clone(),
        &request.method,
        outcome,
        error.map(|error| error.code.as_str()),
        error.is_some_and(|error| error.retryable),
        elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
    );
    let workspace_id = response_workspace_id(request, &response);
    let telemetry_recorded = workspace_id
        .as_deref()
        .is_some_and(|workspace_id| recorder(workspace_id, record).is_ok());
    if let Some(error) = response.error.as_mut() {
        let mut details = error
            .details
            .take()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        details.insert(
            "correlationId".into(),
            serde_json::Value::String(correlation_id),
        );
        details.insert(
            "operation".into(),
            serde_json::Value::String(request.method.clone()),
        );
        details.insert(
            "telemetryRecorded".into(),
            serde_json::Value::Bool(telemetry_recorded),
        );
        error.details = Some(serde_json::Value::Object(details));
    } else if workspace_id.is_some()
        && !telemetry_recorded
        && let Some(result) = response
            .result
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
    {
        result.insert(
            "reliabilityWarning".into(),
            serde_json::json!({
                "code": "local_reliability_unavailable",
                "correlationId": correlation_id,
            }),
        );
    }
    response
}

pub(crate) fn provider_failure_response(id: &str, error: &anyhow::Error) -> CoreResponse {
    use crate::provider::contract::FailureCode;

    let Some(code) = crate::provider::contract_failure_code(error) else {
        return CoreResponse::failure(
            id,
            "scan_failed",
            "The repository analysis could not be completed.",
            true,
        );
    };
    let (code, message) = match code {
        FailureCode::TimedOut => (
            "provider_timeout",
            "The selected provider did not finish in time.",
        ),
        FailureCode::MalformedResult => (
            "provider_response_invalid",
            "The selected provider returned an invalid structured result.",
        ),
        FailureCode::InvalidContract => (
            "provider_contract_invalid",
            "The selected provider no longer satisfies CodeCaddie's local execution contract.",
        ),
        FailureCode::StartFailed | FailureCode::IoFailed | FailureCode::ProviderFailed => (
            "provider_failed",
            "The selected provider could not complete the local analysis.",
        ),
    };
    CoreResponse::failure(id, code, message, true)
}

/// Every method the dispatch table above implements, in dispatch order. The
/// protocol fixture tests assert each `request-*.json` fixture uses one of
/// these methods, and the catalog in `protocol/README.md` documents them.
/// Keep all three in step when adding or removing a method.
pub const METHODS: &[&str] = &[
    "system.ping",
    "updates.check",
    "updates.download",
    "updates.install",
    "settings.launchAtLogin.get",
    "settings.launchAtLogin.set",
    "settings.provider.get",
    "settings.provider.set",
    "providers.detect",
    "privacy.promise",
    "context.files.inspect",
    "workspace.create",
    "workspace.recent",
    "workspace.open",
    "workspace.context.update",
    "workspace.recovery.export",
    "workspace.backup.export",
    "workspace.backup.import",
    "workspace.backup.schedule.status",
    "workspace.backup.schedule.enable",
    "workspace.backup.schedule.disable",
    "workspace.backup.schedule.run",
    "reports.export_word",
    "reports.history.list",
    "reports.finding.get",
    "reports.delete",
    "goals.approve",
    "goals.replace",
    "actions.ready",
    "instrumentation.record",
    "recommendations.prompt",
    "recommendations.copy_prompt",
    "reliability.record",
    "scan.run",
    "goals.generate",
    "map.generate",
    "map.get",
];

/// Builders the per-domain handler tests share so every test speaks the
/// same envelope the desktop host sends.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::protocol::{CoreRequest, CoreResponse, PROTOCOL_VERSION};

    pub(crate) fn request(
        method: &str,
        workspace_id: Option<&str>,
        params: serde_json::Value,
    ) -> CoreRequest {
        let serde_json::Value::Object(params) = params else {
            panic!("test params must be a JSON object");
        };
        CoreRequest {
            id: "req-test".into(),
            protocol_version: PROTOCOL_VERSION,
            workspace_id: workspace_id.map(str::to_owned),
            method: method.into(),
            params,
        }
    }

    pub(crate) fn error_of(response: CoreResponse) -> crate::protocol::CoreError {
        assert!(!response.ok, "expected a failure response");
        assert_eq!(
            response.id, "req-test",
            "responses must echo the request id"
        );
        response.error.expect("failure responses carry an error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROTOCOL_VERSION;
    use test_support::request;

    struct NeverSerializes;

    impl serde::Serialize for NeverSerializes {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("this value refuses to serialize"))
        }
    }

    #[test]
    fn a_result_that_cannot_serialize_becomes_a_typed_failure_not_a_panic() {
        let response = serialized_success("req-1".to_string(), "test payload", &NeverSerializes);
        assert!(!response.ok);
        let error = response.error.expect("failure carries an error");
        assert_eq!(error.code, "internal_error");
        assert!(!error.retryable);
        assert!(error.message.contains("test payload"));
    }

    #[test]
    fn a_serializable_result_still_produces_a_success_response() {
        let response = serialized_success(
            "req-2".to_string(),
            "test payload",
            &serde_json::json!({ "ready": true }),
        );
        assert!(response.ok);
        assert_eq!(response.result, Some(serde_json::json!({ "ready": true })));
    }

    #[test]
    fn paused_or_unverifiable_runtime_controls_fail_safely_before_customer_state_changes() {
        let controlled = request("scan.run", Some("workspace-1"), serde_json::json!({}));
        let paused = runtime_control_response_with(&controlled, Ok(true)).unwrap();
        assert!(!paused.ok);
        let error = paused.error.unwrap();
        assert_eq!(error.code, "feature_paused");
        assert!(error.message.contains("customer state is unchanged"));

        let unavailable = runtime_control_response_with(&controlled, Err(())).unwrap();
        assert!(!unavailable.ok);
        let error = unavailable.error.unwrap();
        assert_eq!(error.code, "feature_control_unavailable");
        assert!(error.message.contains("operation was not started"));

        let read_only = request(
            "workspace.recent",
            Some("workspace-1"),
            serde_json::json!({}),
        );
        assert!(runtime_control_response(&read_only).is_none());
    }

    #[test]
    fn privacy_adversarial_provider_errors_receive_content_free_local_correlation_details() {
        let request = request("scan.run", Some("workspace-1"), serde_json::json!({}));
        let response = CoreResponse::failure(
            "req-test",
            "provider_timeout",
            "The provider did not finish in time.",
            true,
        );
        let finalized = finalize_response_with(
            &request,
            response,
            std::time::Duration::from_millis(420),
            |workspace_id, record| {
                assert_eq!(workspace_id, "workspace-1");
                assert_eq!(record.operation.as_deref(), Some("scan.run"));
                assert_eq!(
                    record.outcome,
                    Some(codecaddie_domain::ReliabilityOutcome::Failed)
                );
                assert_eq!(record.error_code.as_deref(), Some("provider_timeout"));
                assert_eq!(record.elapsed_milliseconds, Some(420));
                let json = serde_json::to_string(&record).unwrap();
                for forbidden in [
                    "PRIVATE SOURCE SENTINEL",
                    "repositorySource",
                    "attachmentContent",
                    "goalText",
                    "prompt",
                    "freeText",
                ] {
                    assert!(!json.contains(forbidden));
                }
                Ok(())
            },
        );
        let error = finalized.error.unwrap();
        assert_eq!(error.code, "provider_timeout");
        assert_eq!(error.message, "The provider did not finish in time.");
        let details = error.details.unwrap();
        let correlation_id = details["correlationId"].as_str().unwrap();
        uuid::Uuid::parse_str(correlation_id).unwrap();
        assert_eq!(details["operation"], "scan.run");
        assert_eq!(details["telemetryRecorded"], true);
    }

    #[test]
    fn local_telemetry_outages_never_replace_the_customer_result() {
        let request = request(
            "workspace.recent",
            Some("workspace-1"),
            serde_json::json!({}),
        );
        let success = finalize_response_with(
            &request,
            CoreResponse::success("req-test", serde_json::json!({"workspaceId":"workspace-1"})),
            std::time::Duration::from_millis(5),
            |_workspace_id, _record| anyhow::bail!("injected disk exhaustion"),
        );
        assert!(success.ok);
        let result = success.result.unwrap();
        assert_eq!(result["workspaceId"], "workspace-1");
        assert_eq!(
            result["reliabilityWarning"]["code"],
            "local_reliability_unavailable"
        );
        uuid::Uuid::parse_str(
            result["reliabilityWarning"]["correlationId"]
                .as_str()
                .unwrap(),
        )
        .unwrap();

        let failed = finalize_response_with(
            &request,
            CoreResponse::failure(
                "req-test",
                "workspace_load_failed",
                "The local workspace could not be loaded.",
                true,
            ),
            std::time::Duration::from_millis(6),
            |_workspace_id, _record| anyhow::bail!("injected interrupted telemetry write"),
        );
        assert!(!failed.ok);
        let error = failed.error.unwrap();
        assert_eq!(error.code, "workspace_load_failed");
        assert_eq!(error.message, "The local workspace could not be loaded.");
        assert_eq!(error.details.unwrap()["telemetryRecorded"], false);
    }

    #[tokio::test]
    async fn unknown_methods_fail_without_aborting_the_service() {
        let response = handle(CoreRequest {
            id: "req-3".into(),
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            method: "workspace.snapshot".into(),
            params: Default::default(),
        })
        .await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "method_not_found");
    }

    #[test]
    fn the_method_catalog_is_unique() {
        let mut methods = METHODS.to_vec();
        methods.sort_unstable();
        methods.dedup();
        assert_eq!(methods.len(), METHODS.len());
    }

    #[test]
    fn the_dispatch_table_and_the_method_catalog_agree_exactly() {
        let dispatched: Vec<&str> = DISPATCH.iter().map(|(method, _)| *method).collect();
        assert_eq!(
            dispatched, METHODS,
            "DISPATCH and METHODS must list the same methods in the same order"
        );
    }

    #[tokio::test]
    async fn handle_rejects_unsupported_protocol_versions_before_dispatch() {
        let mut ping = request("system.ping", None, serde_json::json!({}));
        ping.protocol_version = PROTOCOL_VERSION + 1;
        let response = handle(ping).await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "protocol_version");
    }

    #[tokio::test]
    async fn handle_with_progress_rejects_unsupported_protocol_versions_too() {
        let mut generate = request(
            "goals.generate",
            None,
            serde_json::json!({ "stream": true }),
        );
        generate.protocol_version = PROTOCOL_VERSION + 1;
        let sink: crate::provider::ProgressSink = std::sync::Arc::new(|_message| {});
        let response = handle_with_progress(generate, sink).await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "protocol_version");
    }

    #[tokio::test]
    async fn handle_with_progress_delegates_non_streaming_methods_to_handle() {
        let sink: crate::provider::ProgressSink = std::sync::Arc::new(|_message| {});
        let response = handle_with_progress(
            request("workspace.snapshot", None, serde_json::json!({})),
            sink,
        )
        .await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "method_not_found");
    }

    #[test]
    fn only_provider_backed_methods_opt_into_progress_streaming() {
        let streaming = request("scan.run", None, serde_json::json!({ "stream": true }));
        assert!(streams_progress(&streaming));
        let streaming = request(
            "goals.generate",
            None,
            serde_json::json!({ "stream": true }),
        );
        assert!(streams_progress(&streaming));
        let unstated = request("scan.run", None, serde_json::json!({}));
        assert!(!streams_progress(&unstated));
        let declined = request(
            "goals.generate",
            None,
            serde_json::json!({ "stream": false }),
        );
        assert!(!streams_progress(&declined));
        let unsupported = request("system.ping", None, serde_json::json!({ "stream": true }));
        assert!(!streams_progress(&unsupported));
    }

    #[tokio::test]
    async fn dispatched_responses_echo_the_request_id() {
        let response = handle(request("privacy.promise", None, serde_json::json!({}))).await;
        assert!(response.ok);
        assert_eq!(response.id, "req-test");
    }
}
