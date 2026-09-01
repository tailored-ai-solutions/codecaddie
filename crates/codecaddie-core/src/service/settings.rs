//! Handlers for the `settings.*` methods: launch-at-login registration and
//! the persisted provider preference.

use super::parsed_params;
use crate::{
    launch_at_login,
    local_state::LocalWorkspaceStore,
    protocol::{CoreRequest, CoreResponse},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchAtLoginRequest {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderPreferenceRequest {
    provider: String,
}

pub(super) async fn launch_at_login_get(request: CoreRequest) -> CoreResponse {
    match launch_at_login::is_enabled() {
        Ok(enabled) => CoreResponse::success(
            request.id,
            serde_json::json!({ "enabled": enabled, "supported": true }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "launch_at_login_unavailable",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn launch_at_login_set(request: CoreRequest) -> CoreResponse {
    let change: LaunchAtLoginRequest = match parsed_params(&request.id, request.params) {
        Ok(change) => change,
        Err(failure) => return *failure,
    };
    match launch_at_login::set_enabled(change.enabled) {
        Ok(enabled) => CoreResponse::success(
            request.id,
            serde_json::json!({ "enabled": enabled, "supported": true }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "launch_at_login_failed",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn provider_get(request: CoreRequest) -> CoreResponse {
    match LocalWorkspaceStore::from_environment().and_then(|store| store.provider_preference()) {
        Ok(provider) => {
            CoreResponse::success(request.id, serde_json::json!({ "provider": provider }))
        }
        Err(error) => CoreResponse::failure(
            request.id,
            "provider_preference_unavailable",
            error.to_string(),
            false,
        ),
    }
}

pub(super) async fn provider_set(request: CoreRequest) -> CoreResponse {
    let change: ProviderPreferenceRequest = match parsed_params(&request.id, request.params) {
        Ok(change) => change,
        Err(failure) => return *failure,
    };
    match LocalWorkspaceStore::from_environment()
        .and_then(|store| store.set_provider_preference(&change.provider))
    {
        Ok(()) => CoreResponse::success(
            request.id,
            serde_json::json!({ "provider": change.provider }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "provider_preference_failed",
            error.to_string(),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{error_of, request};

    #[tokio::test]
    async fn launch_at_login_set_rejects_missing_params() {
        let response = launch_at_login_set(request(
            "settings.launchAtLogin.set",
            None,
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn provider_set_rejects_missing_params_before_touching_storage() {
        let response = provider_set(request(
            "settings.provider.set",
            None,
            serde_json::json!({}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn provider_set_rejects_params_of_the_wrong_shape() {
        let response = provider_set(request(
            "settings.provider.set",
            None,
            serde_json::json!({ "provider": 7 }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
    }
}
