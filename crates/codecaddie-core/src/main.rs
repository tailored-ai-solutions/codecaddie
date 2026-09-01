use codecaddie_core::{
    protocol::{CoreEvent, CoreRequest, read_frame, write_frame, write_json_line},
    provider::ProgressSink,
    service,
};
use std::io::{self, BufReader, BufWriter};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args();
    let _ = args.next();
    let mode = args.next();
    if mode.as_deref() == Some("--health-check") {
        println!(
            "CodeCaddie {}+{} {}",
            codecaddie_core::update::current_version(),
            codecaddie_core::update::current_build(),
            codecaddie_core::update::current_commit()
        );
        return Ok(());
    }
    if mode.as_deref() == Some("mcp") {
        let workspace = match args.next().as_deref() {
            Some("--workspace") => Some(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("--workspace requires a workspace id"))?,
            ),
            Some(other) => anyhow::bail!("unknown mcp argument: {other}"),
            None => None,
        };
        return codecaddie_core::mcp::run(workspace);
    }
    if mode.as_deref() == Some("provider-repository-mcp") {
        let root = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("provider repository MCP requires a snapshot root"))?;
        if args.next().is_some() {
            anyhow::bail!("provider repository MCP accepts one snapshot root");
        }
        return codecaddie_core::provider_repository_mcp::run(root);
    }
    if mode.as_deref() == Some("agent") {
        // One-shot agent verbs speak a strict stdout contract: exactly one
        // JSON object per invocation, exit 0 on ok and 1 otherwise. Every
        // failure surfaces as the JSON error object, never as anyhow text.
        let rest: Vec<String> = args.collect();
        std::process::exit(codecaddie_core::agent_cli::run(&rest));
    }
    if mode.as_deref() == Some("--request-file") {
        let path = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("--request-file requires a path"))?;
        respond(read_request_file(&path)?).await?;
        return Ok(());
    }
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    while let Some(request) = read_frame::<CoreRequest>(&mut reader)? {
        respond(request).await?;
    }
    Ok(())
}

/// Handles one request and writes its response to stdout. Streaming
/// requests (`"stream": true` on a supporting method) emit NDJSON:
/// `CoreEvent` progress lines while the work runs, then exactly one
/// terminal `CoreResponse` line. Everything else stays a single
/// length-prefixed frame. The stdout lock is taken per write so progress
/// events never contend with a held response writer.
async fn respond(request: CoreRequest) -> anyhow::Result<()> {
    if service::streams_progress(&request) {
        let workspace_id = request.workspace_id.clone();
        let sequence = Arc::new(AtomicU64::new(0));
        let topic: &'static str = match request.method.as_str() {
            "scan.run" => "scan.progress",
            "map.generate" => "map.generate.progress",
            _ => "goals.generate.progress",
        };
        let sink: ProgressSink = Arc::new(move |message: String| {
            let event = CoreEvent {
                sequence: sequence.fetch_add(1, Ordering::Relaxed),
                workspace_id: workspace_id.clone(),
                topic: topic.to_string(),
                payload: serde_json::json!({ "message": message }),
            };
            let stdout = io::stdout();
            let mut lock = stdout.lock();
            let _ = write_json_line(&mut lock, &event);
        });
        let response = service::handle_with_progress(request, sink).await;
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        write_json_line(&mut lock, &response)?;
    } else {
        let response = service::handle(request).await;
        let stdout = io::stdout();
        let mut writer = BufWriter::new(stdout.lock());
        write_frame(&mut writer, &response)?;
    }
    Ok(())
}

/// Reads every length-prefixed request frame from a staging file written
/// by the desktop host (payloads beyond the host's spawn-stdin budget),
/// then deletes the file so user text does not linger on disk.
fn read_request_file(path: &str) -> anyhow::Result<CoreRequest> {
    let path = std::path::Path::new(path);
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| anyhow::anyhow!("request file name is invalid"))?;
    if !file_name.starts_with("codecaddie-") || !file_name.ends_with(".request") {
        anyhow::bail!("request file name is outside the CodeCaddie staging contract");
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        anyhow::bail!("request path must be a regular staging file");
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("request file has no staging directory"))?
        .canonicalize()?;
    let temp_root = std::env::temp_dir().canonicalize()?;
    if !parent.starts_with(&temp_root) {
        anyhow::bail!("request file is outside the operating-system staging directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    struct RequestFileGuard<'a>(&'a str);
    impl Drop for RequestFileGuard<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0);
        }
    }
    let path_text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("request file path is not valid UTF-8"))?;
    let _cleanup = RequestFileGuard(path_text);
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let request = read_frame::<CoreRequest>(&mut reader)?
        .ok_or_else(|| anyhow::anyhow!("request staging file is empty"))?;
    if read_frame::<CoreRequest>(&mut reader)?.is_some() {
        anyhow::bail!("request staging files must contain exactly one frame");
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_core::protocol::PROTOCOL_VERSION;

    #[test]
    fn request_files_are_read_whole_and_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codecaddie-test-staged.request");
        let request = CoreRequest {
            id: "staged".into(),
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            method: "system.ping".into(),
            params: Default::default(),
        };
        let mut bytes = vec![];
        write_frame(&mut bytes, &request).unwrap();
        std::fs::write(&path, bytes).unwrap();
        let staged_request = read_request_file(path.to_str().unwrap()).unwrap();
        assert_eq!(staged_request, request);
        assert!(!path.exists());
    }

    #[test]
    fn malformed_request_files_are_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codecaddie-test-malformed.request");
        std::fs::write(&path, [0, 0, 0, 8, b'{']).unwrap();
        assert!(read_request_file(path.to_str().unwrap()).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn oversized_request_files_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codecaddie-test-oversized.request");
        let mut bytes = ((16 * 1024 * 1024_u32) + 1).to_be_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        std::fs::write(&path, bytes).unwrap();
        assert!(read_request_file(path.to_str().unwrap()).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn request_files_reject_a_second_frame_and_are_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codecaddie-test-two-frames.request");
        let request = CoreRequest {
            id: "first".into(),
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            method: "system.ping".into(),
            params: Default::default(),
        };
        let mut bytes = vec![];
        write_frame(&mut bytes, &request).unwrap();
        write_frame(&mut bytes, &request).unwrap();
        std::fs::write(&path, bytes).unwrap();
        assert!(read_request_file(path.to_str().unwrap()).is_err());
        assert!(!path.exists());
    }
}
