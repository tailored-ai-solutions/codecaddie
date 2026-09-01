//! Handlers for the environment-independent service methods: the handshake
//! ping and the privacy promise the desktop shows verbatim.

use crate::{
    protocol::{CoreRequest, CoreResponse, PROTOCOL_VERSION},
    update,
};

pub(super) async fn ping(request: CoreRequest) -> CoreResponse {
    // Correlation IDs are opaque. Only the desktop's explicit startup flag may
    // consume the one-shot helper result; ordinary ping calls remain read-only
    // and retain their exact historical response shape.
    let consume_updater_result = request
        .params
        .get("consumeUpdaterResult")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    // The external helper writes only a fixed result code, never installer
    // output or repository text. Consume it as part of the first startup
    // handshake so a failed update is visible even when the network is down.
    #[cfg(not(test))]
    let updater_result = if consume_updater_result {
        match update::take_updater_result() {
            Ok(result) => result,
            Err(_) => Some(update::UpdaterResultV1::failed(
                update::UpdaterResultCode::ResultUnreadable,
            )),
        }
    } else {
        None
    };
    // Unit tests must never consume a developer's real one-shot mailbox;
    // update module tests exercise the same read/validate/delete path against
    // an explicit temporary data root.
    #[cfg(test)]
    let updater_result: Option<update::UpdaterResultV1> = None;
    let mut result = serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "service": "codecaddie-core",
        "build": update::update_summary()
    });
    if consume_updater_result {
        result["updaterResult"] = serde_json::to_value(updater_result)
            .expect("the fixed updater result schema is serializable");
    }
    CoreResponse::success(request.id, result)
}

pub(super) async fn privacy_promise(request: CoreRequest) -> CoreResponse {
    CoreResponse::success(
        request.id,
        serde_json::json!({
            "sourceUploadByCodeCaddie": false,
            "attachedContextSentToProvider": true,
            "providerCredentialsAccepted": false,
            "repositoryWrites": false,
            "providerTrustBoundary": "Your selected installed provider may process the disposable repository clone and bounded text from explicitly attached context files under its existing organizational policy."
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::request;

    #[tokio::test]
    async fn ping_reports_the_protocol_version_and_build_summary() {
        let response = ping(request("system.ping", None, serde_json::json!({}))).await;
        assert!(response.ok);
        assert_eq!(response.id, "req-test");
        let result = response.result.expect("ping returns a result");
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["service"], "codecaddie-core");
        assert!(result["build"].is_object());
        assert!(result.get("updaterResult").is_none());
    }

    #[tokio::test]
    async fn explicit_startup_ping_includes_the_one_shot_updater_result_field() {
        let boot = request(
            "system.ping",
            None,
            serde_json::json!({ "consumeUpdaterResult": true }),
        );
        let response = ping(boot).await;
        assert!(response.ok);
        let result = response.result.expect("ping returns a result");
        // Tests never consume a developer's real mailbox, but the desktop
        // handshake retains its additive nullable field.
        assert!(result.get("updaterResult").is_some());
        assert!(result["updaterResult"].is_null());
    }

    #[tokio::test]
    async fn the_privacy_promise_never_relaxes_the_trust_boundary() {
        let response =
            privacy_promise(request("privacy.promise", None, serde_json::json!({}))).await;
        assert!(response.ok);
        let result = response.result.expect("privacy promise returns a result");
        assert_eq!(result["sourceUploadByCodeCaddie"], false);
        assert_eq!(result["attachedContextSentToProvider"], true);
        assert_eq!(result["providerCredentialsAccepted"], false);
        assert_eq!(result["repositoryWrites"], false);
        assert!(result["providerTrustBoundary"].is_string());
    }
}
