//! Inspection of explicitly selected product-context files. Only metadata
//! crosses IPC; extracted text remains inside the core process.

use super::parsed_params;
use crate::{
    context_documents,
    protocol::{CoreRequest, CoreResponse},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectContextFilesRequest {
    paths: Vec<String>,
}

pub(super) async fn inspect_files(request: CoreRequest) -> CoreResponse {
    let inspection: InspectContextFilesRequest = match parsed_params(&request.id, request.params) {
        Ok(inspection) => inspection,
        Err(failure) => return *failure,
    };
    match context_documents::inspect_paths(&inspection.paths) {
        Ok(files) => CoreResponse::success(
            request.id,
            serde_json::json!({ "files": files, "status": "ready" }),
        ),
        Err(error) => CoreResponse::failure(
            request.id,
            "context_file_inspection_failed",
            error.to_string(),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{error_of, request};

    #[tokio::test]
    async fn inspection_rejects_missing_paths() {
        let response = inspect_files(request(
            "context.files.inspect",
            None,
            serde_json::json!({"paths":["relative.md"]}),
        ))
        .await;
        let error = error_of(response);
        assert_eq!(error.code, "context_file_inspection_failed");
        assert!(error.message.contains("absolute"));
    }
}
