//! Handlers for the `updates.*` methods: checking, downloading, and
//! installing signed application updates.

use super::{parsed_params, serialized_success};
use crate::{
    protocol::{CoreRequest, CoreResponse},
    update,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallUpdateRequest {
    staged_path: std::path::PathBuf,
    parent_pid: u32,
}

pub(super) async fn check(request: CoreRequest) -> CoreResponse {
    match update::check().await {
        Ok(status) => serialized_success(request.id, "update status", &status),
        Err(error) => {
            CoreResponse::failure(request.id, "update_check_failed", error.to_string(), true)
        }
    }
}

pub(super) async fn download(request: CoreRequest) -> CoreResponse {
    match update::download().await {
        Ok(staged) => serialized_success(request.id, "staged update", &staged),
        Err(error) => CoreResponse::failure(
            request.id,
            "update_download_failed",
            error.to_string(),
            true,
        ),
    }
}

pub(super) async fn install(request: CoreRequest) -> CoreResponse {
    let install: InstallUpdateRequest = match parsed_params(&request.id, request.params) {
        Ok(install) => install,
        Err(failure) => return *failure,
    };
    match update::install(&install.staged_path, install.parent_pid) {
        Ok(staged) => CoreResponse::success(
            request.id,
            serde_json::json!({
                "status": "readyToRestart",
                "version": staged.version,
                "build": staged.build
            }),
        ),
        Err(error) => {
            let code = match error {
                update::UpdateError::MacAppOnMountedVolume => "update_install_from_volume",
                update::UpdateError::MacAppTranslocated => "update_install_translocated",
                update::UpdateError::MacDestinationNotWritable => {
                    "update_install_parent_unwritable"
                }
                update::UpdateError::MacAppBundleNotFound => "update_install_bundle_missing",
                _ => "update_install_failed",
            };
            CoreResponse::failure(request.id, code, error.to_string(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{error_of, request};

    #[tokio::test]
    async fn install_rejects_missing_params_without_touching_the_filesystem() {
        let response = install(request("updates.install", None, serde_json::json!({}))).await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn install_rejects_params_of_the_wrong_shape() {
        let response = install(request(
            "updates.install",
            None,
            serde_json::json!({ "stagedPath": "/tmp/update.bin", "parentPid": "not-a-pid" }),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "invalid_params");
        assert!(!error.retryable);
    }
}
