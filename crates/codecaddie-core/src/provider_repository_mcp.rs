//! Snapshot-confined repository tools for provider processes.
//!
//! Codex runs with its built-in shell and file tools disabled. This private
//! stdio MCP server is the only repository interface exposed to that model.
//! Every path is resolved below one history-free disposable snapshot, and all
//! requests and responses are bounded. Source returned here travels only to
//! the selected provider process; it never enters CodeCaddie's desktop IPC,
//! report ledger, or workspace state.

use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_LIST_RESULTS: usize = 2_000;
const MAX_SEARCH_RESULTS: usize = 200;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_READ_LINES: usize = 400;

pub fn run(root: impl AsRef<Path>) -> anyhow::Result<()> {
    let root = root.as_ref().canonicalize()?;
    if !root.is_dir() {
        anyhow::bail!("provider repository root is not a directory");
    }
    let server = ProviderRepositoryServer { root };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    loop {
        let Some(line) = read_bounded_line(&mut reader)? else {
            return Ok(());
        };
        let response = match serde_json::from_slice::<Value>(&line) {
            Ok(message) => server.handle(message),
            Err(error) => Some(error_response(
                Value::Null,
                -32700,
                format!("parse error: {error}"),
            )),
        };
        if let Some(response) = response {
            let encoded = serde_json::to_vec(&response)?;
            if encoded.len() > MAX_RESPONSE_BYTES {
                let fallback = error_response(
                    response.get("id").cloned().unwrap_or(Value::Null),
                    -32603,
                    "repository tool response exceeded the 2 MiB limit",
                );
                serde_json::to_writer(&mut writer, &fallback)?;
            } else {
                writer.write_all(&encoded)?;
            }
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}

fn read_bounded_line(reader: &mut impl BufRead) -> anyhow::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let count = reader
        .by_ref()
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if count == 0 {
        return Ok(None);
    }
    if line.len() > MAX_MESSAGE_BYTES {
        discard_through_newline(reader)?;
        anyhow::bail!("provider repository request exceeded the 1 MiB limit");
    }
    Ok(Some(line))
}

fn discard_through_newline(reader: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        let ended = available.get(count.saturating_sub(1)) == Some(&b'\n');
        reader.consume(count);
        if ended {
            return Ok(());
        }
    }
}

struct ProviderRepositoryServer {
    root: PathBuf,
}

impl ProviderRepositoryServer {
    fn handle(&self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str)?;
        let id = id?;
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "codecaddie-repository",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": "Inspect only the frozen repository snapshot through these bounded read-only tools. Cite repository-relative paths and one-based line ranges."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_descriptors() })),
            "tools/call" => self.call_tool(params),
            _ => Err(anyhow::anyhow!("method not found: {method}")),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => error_response(id, -32602, format!("{error:#}")),
        })
    }

    fn call_tool(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("tools/call requires a tool name"))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let result = match name {
            "list_repository_files" => self.list_files(arguments),
            "search_repository" => self.search(arguments),
            "read_repository_file" => self.read_file(arguments),
            _ => Err(anyhow::anyhow!("unknown repository tool: {name}")),
        };
        Ok(match result {
            Ok(value) => json!({
                "content": [{ "type": "text", "text": value.to_string() }],
                "structuredContent": value,
                "isError": false
            }),
            Err(error) => json!({
                "content": [{ "type": "text", "text": format!("{error:#}") }],
                "isError": true
            }),
        })
    }

    fn list_files(&self, arguments: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Arguments {
            #[serde(default)]
            prefix: String,
            #[serde(default)]
            cursor: usize,
            #[serde(default = "default_list_limit")]
            limit: usize,
        }
        let arguments: Arguments = serde_json::from_value(arguments)?;
        let limit = arguments.limit.clamp(1, MAX_LIST_RESULTS);
        validate_relative_prefix(&arguments.prefix)?;
        let mut files = self.repository_files()?;
        files.retain(|path| arguments.prefix.is_empty() || path.starts_with(&arguments.prefix));
        let end = arguments.cursor.saturating_add(limit).min(files.len());
        let page = if arguments.cursor < files.len() {
            files[arguments.cursor..end].to_vec()
        } else {
            Vec::new()
        };
        Ok(json!({
            "files": page,
            "nextCursor": (end < files.len()).then_some(end),
            "total": files.len()
        }))
    }

    fn search(&self, arguments: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Arguments {
            query: String,
            #[serde(default)]
            prefix: String,
            #[serde(default = "default_search_limit")]
            max_results: usize,
        }
        let arguments: Arguments = serde_json::from_value(arguments)?;
        let query = arguments.query.trim().to_lowercase();
        if query.chars().count() < 2 || query.chars().count() > 160 {
            anyhow::bail!("search query must contain 2 to 160 characters");
        }
        validate_relative_prefix(&arguments.prefix)?;
        let limit = arguments.max_results.clamp(1, MAX_SEARCH_RESULTS);
        let mut matches = Vec::new();
        'files: for relative in self.repository_files()? {
            if !arguments.prefix.is_empty() && !relative.starts_with(&arguments.prefix) {
                continue;
            }
            let path = self.resolve_file(&relative)?;
            let mut reader = BufReader::new(std::fs::File::open(path)?);
            if reader.fill_buf()?.contains(&0) {
                continue;
            }
            let mut line_number = 0_usize;
            loop {
                let line = match read_source_line(&mut reader) {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    // One generated or binary file must not make every
                    // targeted repository search fail. The snapshot and each
                    // response remain bounded; skip this file and continue.
                    Err(_) => continue 'files,
                };
                line_number += 1;
                let Ok(text) = std::str::from_utf8(&line) else {
                    continue;
                };
                if text.to_lowercase().contains(&query) {
                    matches.push(json!({
                        "path": relative,
                        "line": line_number,
                        "preview": bounded_preview(text, 320)
                    }));
                    if matches.len() >= limit {
                        break 'files;
                    }
                }
            }
        }
        Ok(json!({ "matches": matches, "truncated": matches.len() == limit }))
    }

    fn read_file(&self, arguments: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Arguments {
            path: String,
            #[serde(default = "default_start_line")]
            start_line: usize,
            #[serde(default = "default_end_line")]
            end_line: usize,
        }
        let arguments: Arguments = serde_json::from_value(arguments)?;
        if arguments.start_line == 0
            || arguments.end_line < arguments.start_line
            || arguments.end_line - arguments.start_line + 1 > MAX_READ_LINES
        {
            anyhow::bail!("read range must contain 1 to 400 one-based lines");
        }
        let path = self.resolve_file(&arguments.path)?;
        let mut reader = BufReader::new(std::fs::File::open(path)?);
        let mut selected = Vec::new();
        let mut line_number = 0_usize;
        while let Some(line) = read_source_line(&mut reader)? {
            line_number += 1;
            if line_number > arguments.end_line {
                break;
            }
            if line_number < arguments.start_line {
                continue;
            }
            let text = std::str::from_utf8(&line)
                .map_err(|_| anyhow::anyhow!("requested file is not UTF-8 text"))?;
            selected.push(json!({ "line": line_number, "text": text }));
        }
        Ok(json!({
            "path": arguments.path,
            "startLine": arguments.start_line,
            "endLine": selected.last().and_then(|line| line.get("line")).and_then(Value::as_u64),
            "lines": selected
        }))
    }

    fn repository_files(&self) -> anyhow::Result<Vec<String>> {
        let mut pending = VecDeque::from([self.root.clone()]);
        let mut files = Vec::new();
        let mut total_bytes = 0_u64;
        while let Some(directory) = pending.pop_front() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if file_type.is_symlink() {
                    anyhow::bail!("provider snapshot contains a live symlink");
                }
                if file_type.is_dir() {
                    pending.push_back(entry.path());
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let bytes = entry.metadata()?.len();
                if bytes > MAX_FILE_BYTES {
                    anyhow::bail!("provider snapshot contains an oversized file");
                }
                total_bytes = checked_repository_capacity(files.len() + 1, total_bytes, bytes)?;
                let relative = entry.path().strip_prefix(&self.root)?.to_path_buf();
                files.push(relative_path_text(&relative)?);
            }
        }
        files.sort();
        Ok(files)
    }

    fn resolve_file(&self, relative: &str) -> anyhow::Result<PathBuf> {
        validate_relative_prefix(relative)?;
        if relative.is_empty() {
            anyhow::bail!("repository file path is required");
        }
        let path = self.root.join(relative).canonicalize()?;
        if !path.starts_with(&self.root) || !path.is_file() {
            anyhow::bail!("repository path escapes the frozen snapshot");
        }
        if path.metadata()?.len() > MAX_FILE_BYTES {
            anyhow::bail!("repository file exceeds the 8 MiB limit");
        }
        Ok(path)
    }
}

fn checked_repository_capacity(
    file_count: usize,
    current_bytes: u64,
    next_file_bytes: u64,
) -> anyhow::Result<u64> {
    if file_count > MAX_FILES {
        anyhow::bail!("provider snapshot exceeds the 100,000-file limit");
    }
    let total_bytes = current_bytes
        .checked_add(next_file_bytes)
        .ok_or_else(|| anyhow::anyhow!("provider snapshot size overflowed"))?;
    if total_bytes > MAX_TOTAL_BYTES {
        anyhow::bail!("provider snapshot exceeds the 2 GiB content limit");
    }
    Ok(total_bytes)
}

fn default_list_limit() -> usize {
    500
}

fn default_search_limit() -> usize {
    100
}

fn default_start_line() -> usize {
    1
}

fn default_end_line() -> usize {
    200
}

fn validate_relative_prefix(value: &str) -> anyhow::Result<()> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("repository path must stay below the frozen snapshot");
    }
    Ok(())
}

fn relative_path_text(path: &Path) -> anyhow::Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("repository path is not valid UTF-8"))?;
    Ok(text.replace('\\', "/"))
}

fn read_source_line(reader: &mut impl BufRead) -> anyhow::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let count = reader
        .by_ref()
        .take((MAX_LINE_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if count == 0 {
        return Ok(None);
    }
    if line.len() > MAX_LINE_BYTES {
        discard_through_newline(reader)?;
        anyhow::bail!("repository file contains a line over the 1 MiB limit");
    }
    while line
        .last()
        .is_some_and(|byte| matches!(byte, b'\r' | b'\n'))
    {
        line.pop();
    }
    Ok(Some(line))
}

fn bounded_preview(value: &str, limit: usize) -> String {
    let mut output = String::new();
    for character in value.chars().take(limit) {
        if character.is_control() && character != '\t' {
            output.push(' ');
        } else {
            output.push(character);
        }
    }
    if value.chars().count() > limit {
        output.push('…');
    }
    output
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn tool_descriptors() -> Vec<Value> {
    vec![
        json!({
            "name": "list_repository_files",
            "description": "List repository-relative files in the frozen commit snapshot. Paginate with nextCursor.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "prefix": { "type": "string" },
                    "cursor": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 2000 }
                }
            }
        }),
        json!({
            "name": "search_repository",
            "description": "Search frozen UTF-8 repository text for a literal case-insensitive query. Results include repository-relative paths and one-based lines.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            },
            "inputSchema": {
                "type": "object",
                "required": ["query"],
                "additionalProperties": false,
                "properties": {
                    "query": { "type": "string", "minLength": 2, "maxLength": 160 },
                    "prefix": { "type": "string" },
                    "maxResults": { "type": "integer", "minimum": 1, "maximum": 200 }
                }
            }
        }),
        json!({
            "name": "read_repository_file",
            "description": "Read at most 400 numbered lines from one UTF-8 file in the frozen snapshot.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            },
            "inputSchema": {
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string" },
                    "startLine": { "type": "integer", "minimum": 1 },
                    "endLine": { "type": "integer", "minimum": 1 }
                }
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_tools_reject_paths_outside_the_snapshot() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("snapshot");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("inside.rs"), "first\nsecond needle\nthird\n").unwrap();
        std::fs::write(parent.path().join("outside.txt"), "private sentinel").unwrap();
        let server = ProviderRepositoryServer {
            root: root.canonicalize().unwrap(),
        };

        assert!(server.resolve_file("inside.rs").is_ok());
        assert!(server.resolve_file("../outside.txt").is_err());
        assert!(
            server
                .resolve_file(parent.path().join("outside.txt").to_str().unwrap())
                .is_err()
        );
        let search = server
            .search(json!({ "query": "needle", "maxResults": 5 }))
            .unwrap();
        assert_eq!(search["matches"][0]["path"], "inside.rs");
        assert_eq!(search["matches"][0]["line"], 2);
        let read = server
            .read_file(json!({ "path": "inside.rs", "startLine": 2, "endLine": 3 }))
            .unwrap();
        assert_eq!(read["lines"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn every_repository_tool_declares_a_read_only_closed_world_contract() {
        for descriptor in tool_descriptors() {
            assert_eq!(descriptor["annotations"]["readOnlyHint"], true);
            assert_eq!(descriptor["annotations"]["destructiveHint"], false);
            assert_eq!(descriptor["annotations"]["idempotentHint"], true);
            assert_eq!(descriptor["annotations"]["openWorldHint"], false);
        }
    }

    #[test]
    fn repository_search_skips_binary_and_oversized_line_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("00-binary.bin"), [0_u8, 1, 2, 3]).unwrap();
        let oversized = "x".repeat(MAX_LINE_BYTES + 1);
        std::fs::write(root.path().join("01-generated.txt"), oversized).unwrap();
        std::fs::write(
            root.path().join("02-implementation.rs"),
            "first\nvalidated evidence needle\nthird\n",
        )
        .unwrap();
        let server = ProviderRepositoryServer {
            root: root.path().canonicalize().unwrap(),
        };

        let search = server
            .search(json!({ "query": "evidence needle", "maxResults": 5 }))
            .unwrap();
        assert_eq!(search["matches"].as_array().unwrap().len(), 1);
        assert_eq!(search["matches"][0]["path"], "02-implementation.rs");
        assert_eq!(search["matches"][0]["line"], 2);
    }

    #[test]
    fn repository_capacity_gate_accepts_each_boundary_and_rejects_regression() {
        assert_eq!(
            checked_repository_capacity(MAX_FILES, MAX_TOTAL_BYTES - 1, 1).unwrap(),
            MAX_TOTAL_BYTES
        );
        assert!(checked_repository_capacity(MAX_FILES + 1, 0, 0).is_err());
        assert!(checked_repository_capacity(1, MAX_TOTAL_BYTES, 1).is_err());
        assert!(checked_repository_capacity(1, u64::MAX, 1).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn repository_tools_reject_live_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("snapshot");
        std::fs::create_dir(&root).unwrap();
        let outside = parent.path().join("outside.txt");
        std::fs::write(&outside, "private sentinel").unwrap();
        symlink(&outside, root.join("escape.txt")).unwrap();
        let server = ProviderRepositoryServer {
            root: root.canonicalize().unwrap(),
        };
        assert!(server.repository_files().is_err());
        assert!(server.resolve_file("escape.txt").is_err());
    }
}
