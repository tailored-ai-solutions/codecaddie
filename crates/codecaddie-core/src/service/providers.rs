//! Handler for `providers.detect`: reporting which supported provider CLIs
//! are installed, without ever accepting or storing their credentials.

use super::serialized_success;
use crate::{
    protocol::{CoreRequest, CoreResponse},
    provider,
};

pub(super) async fn detect(request: CoreRequest) -> CoreResponse {
    serialized_success(request.id, "provider capabilities", &provider::detect_all())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::request;

    #[tokio::test]
    async fn detect_reports_every_supported_provider_without_credentials() {
        let response = detect(request("providers.detect", None, serde_json::json!({}))).await;
        assert!(response.ok);
        let result = response.result.expect("detection returns a result");
        let capabilities = result.as_array().expect("capabilities are a list");
        assert_eq!(capabilities.len(), 3);
        for capability in capabilities {
            assert!(capability["kind"].is_string());
            assert!(capability["installed"].is_boolean());
            assert!(
                capability.get("credentials").is_none(),
                "provider detection must never carry credentials"
            );
        }
    }
}
