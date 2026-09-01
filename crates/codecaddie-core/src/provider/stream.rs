//! Shared bounded handling of provider output streams: NDJSON parsing with
//! hard size limits, mapping raw events to fixed application-owned progress
//! copy, terminal-result extraction, and safe failure classification.
//! Provider stderr is deliberately never forwarded because CoreResponse
//! travels over the desktop IPC boundary.

use super::{
    MAX_PROGRESS_CHARS, MAX_PROVIDER_LINE, MAX_PROVIDER_OUTPUT, ProgressSink, ProviderActivity,
    ProviderKind, display_file_count,
};
use std::collections::BTreeMap;
use tokio::io::AsyncReadExt;

/// Retains only stream records required after the child exits. A terminal
/// result replaces earlier diagnostic records because successful structured
/// output is the only payload the caller needs. Codex error events remain
/// available for safe failure classification when no result is produced.
fn retain_stream_line(
    kind: ProviderKind,
    line: &str,
    retained: &mut Vec<u8>,
) -> anyhow::Result<()> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Ok(());
    };
    let event_type = value.get("type").and_then(serde_json::Value::as_str);
    let terminal = event_type == Some("result");
    let diagnostic =
        kind == ProviderKind::Codex && matches!(event_type, Some("error" | "turn.failed"));
    if !terminal && !diagnostic {
        return Ok(());
    }
    if line.len() + 1 > MAX_PROVIDER_OUTPUT {
        anyhow::bail!("provider result exceeded 16 MiB");
    }
    if terminal {
        retained.clear();
    } else if retained.len() + line.len() + 1 > MAX_PROVIDER_OUTPUT {
        return Ok(());
    }
    retained.extend_from_slice(line.as_bytes());
    retained.push(b'\n');
    Ok(())
}

/// Reads a provider pipe in fixed-size chunks and fails before retaining more
/// than `MAX_PROVIDER_OUTPUT` bytes.
pub(super) async fn read_bounded(
    mut pipe: impl tokio::io::AsyncRead + Unpin,
) -> anyhow::Result<Vec<u8>> {
    read_bounded_with_limit(&mut pipe, MAX_PROVIDER_OUTPUT).await
}

async fn read_bounded_with_limit(
    mut pipe: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut kept = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = pipe.read(&mut chunk).await?;
        if count == 0 {
            return Ok(kept);
        }
        if kept.len().saturating_add(count) > limit {
            anyhow::bail!("provider output exceeded 16 MiB");
        }
        kept.extend_from_slice(&chunk[..count]);
    }
}

/// Parses a provider's NDJSON stream without allowing an unbounded line or
/// transcript allocation. Only terminal results and bounded diagnostics are
/// retained; progress messages are mapped to fixed application-owned copy.
pub(super) async fn read_stream_output(
    pipe: impl tokio::io::AsyncRead + Unpin,
    kind: ProviderKind,
    sink: ProgressSink,
    activity: Option<ProviderActivity>,
) -> anyhow::Result<Vec<u8>> {
    read_stream_output_with_limits(
        pipe,
        kind,
        sink,
        activity,
        MAX_PROVIDER_OUTPUT,
        MAX_PROVIDER_LINE,
    )
    .await
}

async fn read_stream_output_with_limits(
    mut pipe: impl tokio::io::AsyncRead + Unpin,
    kind: ProviderKind,
    sink: ProgressSink,
    activity: Option<ProviderActivity>,
    output_limit: usize,
    line_limit: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut pending = Vec::new();
    let mut total = 0_usize;
    let mut progress = ProviderProgressState::new(activity);
    let mut chunk = [0_u8; 8192];
    loop {
        let count = pipe.read(&mut chunk).await?;
        if count == 0 {
            if !pending.is_empty() {
                process_stream_line(
                    kind,
                    &pending,
                    &mut retained,
                    &sink,
                    &mut progress,
                    line_limit,
                )?;
            }
            return Ok(retained);
        }
        total = total.saturating_add(count);
        if total > output_limit {
            anyhow::bail!("provider output exceeded 16 MiB");
        }
        pending.extend_from_slice(&chunk[..count]);
        let mut consumed = 0_usize;
        while let Some(relative_end) = pending[consumed..].iter().position(|byte| *byte == b'\n') {
            let end = consumed + relative_end;
            process_stream_line(
                kind,
                &pending[consumed..end],
                &mut retained,
                &sink,
                &mut progress,
                line_limit,
            )?;
            consumed = end + 1;
        }
        if consumed > 0 {
            pending.drain(..consumed);
        }
        if pending.len() > line_limit {
            anyhow::bail!("provider output contained an oversized event line");
        }
    }
}

fn process_stream_line(
    kind: ProviderKind,
    bytes: &[u8],
    retained: &mut Vec<u8>,
    sink: &ProgressSink,
    progress: &mut ProviderProgressState,
    line_limit: usize,
) -> anyhow::Result<()> {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    if bytes.len() > line_limit {
        anyhow::bail!("provider output contained an oversized event line");
    }
    let line = std::str::from_utf8(bytes)?;
    retain_stream_line(kind, line, retained)?;
    if let Some(message) = progress.message_for_line(kind, line) {
        sink(message);
    }
    Ok(())
}

/// Maps one raw provider stdout line to a display-ready progress message,
/// or `None` for lines with no user-facing signal. Every returned string
/// has passed `sanitize_progress`.
#[cfg(test)]
fn provider_progress_line(kind: ProviderKind, line: &str) -> Option<String> {
    ProviderProgressState::default().message_for_line(kind, line)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RepositoryActivity {
    Read(String),
    List(Option<String>),
    Search(Option<String>),
}

#[derive(Default)]
struct ProviderProgressState {
    context: ProviderActivity,
    files_read: BTreeMap<String, usize>,
}

impl ProviderProgressState {
    fn new(context: Option<ProviderActivity>) -> Self {
        Self {
            context: context.unwrap_or_default(),
            files_read: BTreeMap::new(),
        }
    }

    fn message_for_line(&mut self, kind: ProviderKind, line: &str) -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let event_type = value.get("type").and_then(serde_json::Value::as_str)?;
        let message = if let Some(activity) = repository_activity(kind, &value, event_type) {
            self.repository_message(activity)
        } else {
            match kind {
                ProviderKind::Codex => super::codex::progress_message(&value, event_type)?,
                ProviderKind::Claude => {
                    messages_stream_progress(&value, event_type, "Starting Claude", false)?
                }
                ProviderKind::Grok => {
                    messages_stream_progress(&value, event_type, "Starting Grok", true)?
                }
            }
        };
        let labeled = match self.context.phase.as_deref() {
            Some(phase) if !phase.trim().is_empty() => format!("{phase}: {message}"),
            _ => message,
        };
        let sanitized = sanitize_progress(&labeled);
        (!sanitized.is_empty()).then_some(sanitized)
    }

    fn repository_message(&mut self, activity: RepositoryActivity) -> String {
        match activity {
            RepositoryActivity::Read(path) => {
                let next = self.files_read.len() + 1;
                let (ordinal, repeated) = match self.files_read.get(&path).copied() {
                    Some(ordinal) => (ordinal, true),
                    None => {
                        self.files_read.insert(path.clone(), next);
                        (next, false)
                    }
                };
                let verb = if repeated { "Re-reading" } else { "Reading" };
                match self.context.repository_file_total {
                    Some(total) => {
                        format!(
                            "{verb} {path} (file {} of {} this pass)",
                            display_file_count(ordinal),
                            display_file_count(total)
                        )
                    }
                    None => format!("{verb} {path} (distinct file {ordinal} this pass)"),
                }
            }
            RepositoryActivity::List(Some(path)) => format!("Listing files under {path}"),
            RepositoryActivity::List(None) => "Listing repository files".to_string(),
            RepositoryActivity::Search(Some(path)) => format!("Searching under {path}"),
            RepositoryActivity::Search(None) => "Searching repository files".to_string(),
        }
    }
}

fn repository_activity(
    kind: ProviderKind,
    value: &serde_json::Value,
    event_type: &str,
) -> Option<RepositoryActivity> {
    match kind {
        ProviderKind::Claude | ProviderKind::Grok if event_type == "assistant" => {
            let content = value.get("message")?.get("content")?.as_array()?;
            content.iter().rev().find_map(|block| {
                (block.get("type").and_then(serde_json::Value::as_str) == Some("tool_use"))
                    .then(|| activity_from_tool_block(block))?
            })
        }
        ProviderKind::Codex if event_type == "item.started" => {
            activity_from_codex_item(value.get("item")?)
        }
        _ => None,
    }
}

fn activity_from_codex_item(item: &serde_json::Value) -> Option<RepositoryActivity> {
    let item_type = item
        .get("item_type")
        .or_else(|| item.get("type"))
        .and_then(serde_json::Value::as_str)?;
    if !matches!(item_type, "mcp_tool_call" | "tool_call") {
        return None;
    }
    activity_from_tool_block(item)
}

fn activity_from_tool_block(block: &serde_json::Value) -> Option<RepositoryActivity> {
    let name = block
        .get("name")
        .or_else(|| block.get("tool"))
        .and_then(serde_json::Value::as_str)?
        .to_ascii_lowercase();
    let name = name
        .rsplit_once("__")
        .map_or(name.as_str(), |(_, tool)| tool);
    let input = block
        .get("input")
        .or_else(|| block.get("arguments"))
        .and_then(tool_input);
    match name {
        "read" | "read_file" | "read_repository_file" => {
            let path = input
                .as_ref()
                .and_then(|input| safe_path_from(input, &["path", "file_path", "filePath"]))?;
            Some(RepositoryActivity::Read(path))
        }
        "glob" | "list_dir" | "list_repository_files" => {
            Some(RepositoryActivity::List(input.as_ref().and_then(|input| {
                safe_path_from(input, &["prefix", "path", "dir_path", "directory"])
            })))
        }
        "grep" | "search_repository" => {
            Some(RepositoryActivity::Search(input.as_ref().and_then(
                |input| safe_path_from(input, &["prefix", "path"]),
            )))
        }
        _ => None,
    }
}

fn tool_input(value: &serde_json::Value) -> Option<serde_json::Value> {
    if value.is_object() {
        return Some(value.clone());
    }
    value
        .as_str()
        .and_then(|encoded| serde_json::from_str(encoded).ok())
}

fn safe_path_from(input: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        input
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .and_then(safe_repository_activity_path)
    })
}

/// Accepts only a repository-relative display path. If a provider reports an
/// absolute disposable-clone path, retain only the suffix beginning at the
/// application-owned `repository-N` directory. Host paths and traversal
/// components never reach progress IPC.
fn safe_repository_activity_path(raw: &str) -> Option<String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.contains('\0') {
        return None;
    }
    let mut parts = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if normalized.starts_with('/') || normalized.as_bytes().get(1) == Some(&b':') {
        let repository = parts.iter().position(|part| {
            part.strip_prefix("repository-").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
            })
        })?;
        parts.drain(..repository);
    } else if parts.first() == Some(&".") {
        parts.remove(0);
    }
    if parts.is_empty()
        || parts.iter().any(|part| *part == "." || *part == "..")
        || parts[0].contains(':')
    {
        return None;
    }
    Some(parts.join("/"))
}

/// Maps one Anthropic Messages wire-format NDJSON event (Claude's
/// `stream-json`, Grok's `streaming-messages-json`) to a progress message.
/// The last displayable block of an assistant message wins; Grok's readable
/// `thinking` blocks are included because its terminal text block is usually
/// the JSON payload and would otherwise leave whole turns silent.
fn messages_stream_progress(
    value: &serde_json::Value,
    event_type: &str,
    start_message: &str,
    _include_thinking: bool,
) -> Option<String> {
    match event_type {
        "system" => Some(start_message.to_string()),
        "assistant" => {
            let content = value.get("message")?.get("content")?.as_array()?;
            let mut summary = None;
            for block in content {
                let block_type = block.get("type").and_then(serde_json::Value::as_str);
                match block_type {
                    Some("tool_use") => summary = Some("Inspecting repository files".to_string()),
                    Some("text") => summary = Some("Reviewing gathered evidence".to_string()),
                    Some("thinking") => summary = Some("Evaluating the goal criteria".to_string()),
                    _ => {}
                }
            }
            summary
        }
        "result" => Some("Finalizing the result".to_string()),
        _ => None,
    }
}

/// An agent's terminal message is often the structured JSON payload itself;
/// that already arrives through the result channel and is noise in a
/// human-readable feed.
/// Unicode format characters that can visually reorder or hide adjacent
/// text in a rendered feed. `char::is_control` covers only Cc, so bidi
/// overrides and zero-width joiners riding in provider text (derived from
/// untrusted repository content) must be dropped explicitly.
fn is_text_spoofing_format(character: char) -> bool {
    matches!(character,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Collapses provider text to one bounded display line: whitespace runs
/// become single spaces, control and text-spoofing format characters are
/// dropped, and the result is truncated on a character boundary.
fn sanitize_progress(message: &str) -> String {
    let mut sanitized = String::with_capacity(message.len().min(MAX_PROGRESS_CHARS + 4));
    let mut chars = 0_usize;
    let mut pending_space = false;
    for character in message.chars() {
        if character.is_whitespace() {
            pending_space = !sanitized.is_empty();
            continue;
        }
        if character.is_control() || is_text_spoofing_format(character) {
            continue;
        }
        if pending_space {
            if chars + 1 >= MAX_PROGRESS_CHARS {
                break;
            }
            sanitized.push(' ');
            chars += 1;
            pending_space = false;
        }
        if chars >= MAX_PROGRESS_CHARS {
            sanitized.push('…');
            break;
        }
        sanitized.push(character);
        chars += 1;
    }
    sanitized
}

/// The terminal `result` event of a Messages wire-format stream (Claude
/// stream-json, Grok streaming-messages-json) carries the structured output.
/// Scan lines in reverse so the terminal event wins, then fall back to the
/// generic envelope unwrapping.
pub(super) fn extract_stream_result(stdout: &[u8]) -> anyhow::Result<serde_json::Value> {
    let text = String::from_utf8_lossy(stdout);
    for line in text.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("result") {
            continue;
        }
        for key in ["structuredOutput", "structured_output", "result"] {
            if let Some(inner) = value.get(key) {
                if let Some(encoded) = inner.as_str() {
                    if let Ok(parsed) = serde_json::from_str(encoded) {
                        return Ok(parsed);
                    }
                    continue;
                }
                if !inner.is_null() {
                    return Ok(inner.clone());
                }
            }
        }
    }
    structured_json(stdout)
}

pub(super) fn provider_failure_message(kind: ProviderKind, stdout: &[u8], stderr: &[u8]) -> String {
    let mut diagnostic = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if kind == ProviderKind::Codex {
        super::codex::append_failure_diagnostics(stdout, &mut diagnostic);
    }
    let reason = if diagnostic.contains("max turns reached") {
        "reached CodeCaddie's turn limit with an incomplete result"
    } else if diagnostic.contains("invalid schema") || diagnostic.contains("output schema") {
        "rejected the structured-output schema"
    } else if diagnostic.contains("authentication")
        || diagnostic.contains("no auth credentials")
        || diagnostic.contains("not signed in")
        || diagnostic.contains("not logged in")
        || diagnostic.contains("login required")
        || diagnostic.contains("sign in")
        || diagnostic.contains("unauthorized")
        || diagnostic.contains("status 401")
    {
        "needs authentication in its own application"
    } else if diagnostic.contains("rate limit")
        || diagnostic.contains("usage limit")
        || diagnostic.contains("quota")
    {
        "reported an account or usage limit"
    } else if diagnostic.contains("sandbox") || diagnostic.contains("permission denied") {
        "could not start with the required read-only permissions"
    } else if diagnostic.contains("network")
        || diagnostic.contains("connection")
        || diagnostic.contains("timed out")
    {
        "could not reach its configured service"
    } else {
        "exited before returning a result"
    };
    format!(
        "{} {reason}; no provider output was retained",
        kind.executable()
    )
}

pub(super) fn structured_json(output: &[u8]) -> anyhow::Result<serde_json::Value> {
    let text = std::str::from_utf8(output)?.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        for key in [
            "structuredOutput",
            "structured_output",
            "result",
            "output",
            "text",
        ] {
            if let Some(inner) = value.get(key) {
                if let Some(encoded) = inner.as_str()
                    && let Ok(parsed) = serde_json::from_str(encoded)
                {
                    return Ok(parsed);
                }
                if !inner.is_null() {
                    return Ok(inner.clone());
                }
            }
        }
        return Ok(value);
    }
    for line in text.lines().rev() {
        if let Ok(value) = serde_json::from_str(line) {
            return Ok(value);
        }
    }
    anyhow::bail!("provider returned no structured JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn unwraps_common_structured_cli_envelopes() {
        assert_eq!(
            structured_json(br#"{"structuredOutput":{"goals":[]}}"#).unwrap()["goals"],
            serde_json::json!([])
        );
    }

    #[test]
    fn codex_events_map_to_progress_lines() {
        assert_eq!(
            provider_progress_line(
                ProviderKind::Codex,
                r#"{"type":"item.started","item":{"item_type":"command_execution","command":"ls src"}}"#,
            ),
            Some("Inspecting repository files".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Codex,
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"Drafting outcomes now"}}"#,
            ),
            Some("Reviewing gathered evidence".to_string())
        );
        assert_eq!(
            provider_progress_line(ProviderKind::Codex, r#"{"type":"turn.started"}"#),
            Some("Reading the project context".to_string())
        );
        assert_eq!(
            provider_progress_line(ProviderKind::Codex, r#"{"type":"turn.completed"}"#),
            Some("Assembling the drafts".to_string())
        );
        assert_eq!(
            provider_progress_line(ProviderKind::Codex, r#"{"type":"token_count","count":9}"#),
            None
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Codex,
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"{\"goals\":[]}"}}"#,
            ),
            Some("Reviewing gathered evidence".to_string()),
            "provider-authored JSON stays out of progress IPC"
        );
        assert_eq!(
            provider_progress_line(ProviderKind::Codex, "not json"),
            None
        );
    }

    #[test]
    fn claude_stream_events_map_to_progress_lines() {
        assert_eq!(
            provider_progress_line(
                ProviderKind::Claude,
                r#"{"type":"system","subtype":"init"}"#,
            ),
            Some("Starting Claude".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Claude,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"rg goals"}}]}}"#,
            ),
            Some("Inspecting repository files".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Claude,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"a.rs"}}]}}"#,
            ),
            Some("Reading a.rs (distinct file 1 this pass)".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Claude,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Reviewing the brief"}]}}"#,
            ),
            Some("Reviewing gathered evidence".to_string())
        );
    }

    #[test]
    fn grok_stream_events_map_to_progress_lines() {
        assert_eq!(
            provider_progress_line(
                ProviderKind::Grok,
                r#"{"type":"system","subtype":"init","model":"grok-4.5"}"#
            ),
            Some("Starting Grok".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Grok,
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Scanning the brief for outcomes.","signature":"abc"}]}}"#,
            ),
            Some("Evaluating the goal criteria".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Grok,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"run_terminal_command","input":{"command":"rg goals"}}]}}"#,
            ),
            Some("Inspecting repository files".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Grok,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"read_file","input":{"path":"repository-0/src/main.rs"}}]}}"#,
            ),
            Some("Reading repository-0/src/main.rs (distinct file 1 this pass)".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Grok,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"{\"goals\":[]}"}]}}"#,
            ),
            Some("Reviewing gathered evidence".to_string()),
            "provider-authored text never reaches progress IPC"
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Grok,
                r#"{"type":"result","subtype":"success"}"#
            ),
            Some("Finalizing the result".to_string())
        );
        assert_eq!(
            provider_progress_line(
                ProviderKind::Claude,
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"internal"}]}}"#,
            ),
            Some("Evaluating the goal criteria".to_string()),
            "provider reasoning stays private"
        );
    }

    #[test]
    fn repository_activity_shows_safe_paths_and_honest_per_pass_counts() {
        let mut progress = ProviderProgressState::new(Some(ProviderActivity {
            phase: Some("Goal batch 2 of 3".to_string()),
            repository_file_total: Some(13_345),
        }));
        let first = progress
            .message_for_line(
                ProviderKind::Grok,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"read_file","input":{"path":"repository-0/src/leave/pay.ts"}}]}}"#,
            )
            .unwrap();
        assert_eq!(
            first,
            "Goal batch 2 of 3: Reading repository-0/src/leave/pay.ts (file 1 of 13,345 this pass)"
        );
        let repeated = progress
            .message_for_line(
                ProviderKind::Grok,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"read_file","input":{"path":"repository-0/src/leave/pay.ts"}}]}}"#,
            )
            .unwrap();
        assert_eq!(
            repeated,
            "Goal batch 2 of 3: Re-reading repository-0/src/leave/pay.ts (file 1 of 13,345 this pass)"
        );
        let second = progress
            .message_for_line(
                ProviderKind::Codex,
                r#"{"type":"item.started","item":{"type":"mcp_tool_call","tool":"read_repository_file","arguments":"{\"path\":\"repository-0/src/tenant.rs\"}"}}"#,
            )
            .unwrap();
        assert_eq!(
            second,
            "Goal batch 2 of 3: Reading repository-0/src/tenant.rs (file 2 of 13,345 this pass)"
        );
    }

    #[test]
    fn repository_activity_never_exposes_search_text_or_host_paths() {
        assert_eq!(
            provider_progress_line(
                ProviderKind::Grok,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"grep","input":{"pattern":"source-secret","path":"repository-0/src"}}]}}"#,
            ),
            Some("Searching under repository-0/src".to_string())
        );
        let absolute = provider_progress_line(
            ProviderKind::Claude,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/private/tmp/secret/repository-0/src/safe.rs"}}]}}"#,
        )
        .unwrap();
        assert_eq!(
            absolute,
            "Reading repository-0/src/safe.rs (distinct file 1 this pass)"
        );
        assert!(!absolute.contains("/private"));
        assert_eq!(
            provider_progress_line(
                ProviderKind::Claude,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/Users/example/private.rs"}}]}}"#,
            ),
            Some("Inspecting repository files".to_string())
        );
        assert_eq!(safe_repository_activity_path("../outside.rs"), None);
    }

    #[test]
    fn progress_lines_drop_bidi_and_zero_width_format_characters() {
        assert_eq!(
            sanitize_progress("safe \u{202E}dexater\u{202C} text\u{200B}\u{FEFF}"),
            "safe dexater text",
            "bidi overrides and zero-width characters must not reach the feed"
        );
        assert_eq!(sanitize_progress("\u{2066}\u{2069}"), "");
    }

    #[test]
    fn progress_lines_are_single_line_and_bounded() {
        let mapped = provider_progress_line(
            ProviderKind::Codex,
            &format!(
                r#"{{"type":"item.completed","item":{{"item_type":"agent_message","text":"line one\nline\ttwo {}"}}}}"#,
                "x".repeat(400)
            ),
        )
        .unwrap();
        assert!(!mapped.contains('\n'));
        assert!(!mapped.contains('\t'));
        assert_eq!(mapped, "Reviewing gathered evidence");
        assert!(mapped.chars().count() <= MAX_PROGRESS_CHARS + 1);
        assert_eq!(sanitize_progress("  \u{7} \n "), "");
    }

    #[test]
    fn stream_results_come_from_the_terminal_result_event() {
        let transcript = concat!(
            "{\"type\":\"system\",\"subtype\":\"init\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}\n",
            "{\"type\":\"result\",\"subtype\":\"success\",\"structuredOutput\":{\"goals\":[{\"title\":\"T\"}]}}\n",
        );
        let value = extract_stream_result(transcript.as_bytes()).unwrap();
        assert_eq!(value["goals"][0]["title"], "T");

        let string_result =
            "{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"{\\\"goals\\\":[]}\"}\n";
        let value = extract_stream_result(string_result.as_bytes()).unwrap();
        assert_eq!(value["goals"], serde_json::json!([]));

        // Grok's streaming-messages-json result line uses snake_case.
        let grok_result = "{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"{\\\"goals\\\":[]}\",\"structured_output\":{\"goals\":[{\"title\":\"G\"}]}}\n";
        let value = extract_stream_result(grok_result.as_bytes()).unwrap();
        assert_eq!(value["goals"][0]["title"], "G");
    }

    #[test]
    fn progress_transcripts_do_not_consume_the_result_buffer() {
        let mut retained = Vec::new();
        for index in 0..20_000 {
            retain_stream_line(
                ProviderKind::Grok,
                &format!(
                    "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"thinking\",\"thinking\":\"event {index}\"}}]}}}}"
                ),
                &mut retained,
            )
            .unwrap();
        }
        assert!(retained.is_empty());

        retain_stream_line(
            ProviderKind::Codex,
            r#"{"type":"turn.failed","message":"temporary failure"}"#,
            &mut retained,
        )
        .unwrap();
        assert!(!retained.is_empty());

        retain_stream_line(
            ProviderKind::Grok,
            r#"{"type":"result","structured_output":{"goals":[]}}"#,
            &mut retained,
        )
        .unwrap();
        assert_eq!(
            extract_stream_result(&retained).unwrap()["goals"],
            serde_json::json!([])
        );
        assert!(
            !String::from_utf8(retained)
                .unwrap()
                .contains("temporary failure")
        );
    }

    #[tokio::test]
    async fn bounded_reader_rejects_output_before_growing_past_the_limit() {
        let (mut writer, reader) = tokio::io::duplex(256);
        let write = tokio::spawn(async move {
            writer.write_all(&[b'x'; 65]).await.unwrap();
        });
        let error = read_bounded_with_limit(reader, 64).await.unwrap_err();
        write.await.unwrap();
        assert!(error.to_string().contains("exceeded"));
    }

    #[tokio::test]
    async fn stream_reader_rejects_a_single_oversized_line() {
        let (mut writer, reader) = tokio::io::duplex(256);
        let write = tokio::spawn(async move {
            writer.write_all(&[b'x'; 17]).await.unwrap();
        });
        let sink: ProgressSink = Arc::new(|_| {});
        let error = read_stream_output_with_limits(reader, ProviderKind::Grok, sink, None, 64, 16)
            .await
            .unwrap_err();
        write.await.unwrap();
        assert!(error.to_string().contains("oversized event line"));
    }

    #[tokio::test]
    async fn stream_reader_rejects_total_output_over_the_limit() {
        let (mut writer, reader) = tokio::io::duplex(256);
        let write = tokio::spawn(async move {
            writer.write_all(b"{}\n{}\n{}\n").await.unwrap();
        });
        let sink: ProgressSink = Arc::new(|_| {});
        let error = read_stream_output_with_limits(reader, ProviderKind::Grok, sink, None, 8, 16)
            .await
            .unwrap_err();
        write.await.unwrap();
        assert!(error.to_string().contains("exceeded"));
    }

    #[test]
    fn provider_failures_are_actionable_without_forwarding_stderr() {
        let message = provider_failure_message(
            ProviderKind::Codex,
            b"{\"type\":\"error\",\"message\":\"invalid output schema\"}",
            b"invalid schema near /Users/example/private/repository.rs source-secret",
        );
        assert!(message.contains("structured-output schema"));
        assert!(!message.contains("/Users"));
        assert!(!message.contains("source-secret"));

        let bounded = provider_failure_message(
            ProviderKind::Grok,
            b"",
            b"sandbox read-only; Error: max turns reached",
        );
        assert!(bounded.contains("turn limit"));
        assert!(bounded.contains("incomplete result"));

        let signed_out = provider_failure_message(
            ProviderKind::Grok,
            br#"{"type":"result","subtype":"error_during_execution","errors":["Not signed in"]}"#,
            b"Failed to fetch models: Auth(\"No auth credentials for cli-chat-proxy\")\nError: Not signed in. To authenticate, run grok login. /Users/example/private/repository.rs source-secret",
        );
        assert_eq!(
            signed_out,
            "grok needs authentication in its own application; no provider output was retained"
        );
        assert!(!signed_out.contains("/Users"));
        assert!(!signed_out.contains("source-secret"));
    }

    #[test]
    fn privacy_adversarial_progress_diagnostics_and_logs_never_echo_untrusted_payloads() {
        let sentinel = crate::privacy_test_support::REPOSITORY_SENTINEL;
        let injection = crate::privacy_test_support::INJECTION_TEXT;
        let progress = provider_progress_line(
            ProviderKind::Grok,
            &format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"grep","input":{{"pattern":"{sentinel} {injection}","path":"repository-0/src"}}}}]}}}}"#
            ),
        )
        .unwrap();
        assert_eq!(progress, "Searching under repository-0/src");

        let diagnostic = provider_failure_message(
            ProviderKind::Codex,
            br#"{"type":"error","message":"invalid output schema"}"#,
            format!("provider log: {sentinel} {injection} /Users/example/private/repo.rs")
                .as_bytes(),
        );
        let surfaces = format!("{progress}\n{diagnostic}");
        crate::privacy_test_support::assert_private_payload_absent(surfaces.as_bytes());
        assert!(!surfaces.contains(injection));
        assert!(!surfaces.contains("/Users/example"));
        assert!(diagnostic.contains("structured-output schema"));
    }
}
