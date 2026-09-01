//! Local MCP (Model Context Protocol) server for coding-agent sessions.
//!
//! An installed agent spawns `codecaddie mcp` and speaks JSON-RPC 2.0 over
//! stdio, one message per line. The server is metadata-only: it serves the
//! approved goal set and validation results, never source text, and every
//! submitted claim is re-derived against the local git object database by the
//! same pipeline the app's own scans use before anything enters the signed
//! ledger. There is no listening socket and no credential handling.

use crate::{
    agent_gateway::{AgentGateway, AnalysisAttachment},
    analyzer,
    local_state::{LocalWorkspaceStore, ReadyActionRequest},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};

/// Matches MAX_FRAME_BYTES in the desktop IPC protocol and the provider
/// output cap: no single message may exceed 16 MiB.
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

pub fn run(workspace_override: Option<String>) -> anyhow::Result<()> {
    let store = LocalWorkspaceStore::from_environment()?;
    let mut server = McpServer::new(store, workspace_override);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    loop {
        let line = match read_bounded_line(&mut reader)? {
            BoundedLine::Eof => return Ok(()),
            BoundedLine::Oversized => {
                serde_json::to_writer(
                    &mut writer,
                    &error_response(
                        Value::Null,
                        RpcError::new(-32600, "message exceeds the 16 MiB limit"),
                    ),
                )?;
                writer.write_all(b"\n")?;
                writer.flush()?;
                continue;
            }
            BoundedLine::Line(line) => line,
        };
        if let Some(response) = server.handle_line(&String::from_utf8_lossy(&line)) {
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}

enum BoundedLine {
    Eof,
    Line(Vec<u8>),
    Oversized,
}

fn read_bounded_line(reader: &mut impl BufRead) -> std::io::Result<BoundedLine> {
    read_bounded_line_with_limit(reader, MAX_MESSAGE_BYTES)
}

fn read_bounded_line_with_limit(
    reader: &mut impl BufRead,
    limit: usize,
) -> std::io::Result<BoundedLine> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() && !oversized {
                Ok(BoundedLine::Eof)
            } else if oversized {
                Ok(BoundedLine::Oversized)
            } else {
                Ok(BoundedLine::Line(line))
            };
        }
        let end = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if !oversized {
            if line.len() + end > limit + 1 {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..end]);
            }
        }
        let found_newline = available[..end].ends_with(b"\n");
        reader.consume(end);
        if found_newline {
            return if oversized {
                Ok(BoundedLine::Oversized)
            } else {
                Ok(BoundedLine::Line(line))
            };
        }
    }
}

pub struct McpServer {
    gateway: AgentGateway,
    client_slug: String,
}

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl McpServer {
    pub fn new(store: LocalWorkspaceStore, workspace_override: Option<String>) -> Self {
        Self {
            gateway: AgentGateway::new(store, workspace_override),
            client_slug: "unknown".into(),
        }
    }

    /// Processes one wire line and returns the response to write, if any.
    /// Notifications and blank lines produce no response.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.len() > MAX_MESSAGE_BYTES {
            return Some(error_response(
                Value::Null,
                RpcError::new(-32600, "message exceeds the 16 MiB limit"),
            ));
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                return Some(error_response(
                    Value::Null,
                    RpcError::new(-32700, format!("parse error: {error}")),
                ));
            }
        };
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A message without a method is a response to a request; this
            // server never issues requests, so there is nothing to route.
            return None;
        };
        let params = message
            .get("params")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));
        let Some(id) = id else {
            // Notifications (initialized, cancelled, …) need no reply.
            return None;
        };
        Some(match self.dispatch(method, params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => error_response(id, error),
        })
    }

    fn dispatch(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => tool_descriptors()
                .map(|tools| json!({ "tools": tools }))
                .map_err(|_| {
                    RpcError::new(-32603, "the embedded analysis contract schema is invalid")
                }),
            "tools/call" => self.call_tool(params),
            _ => Err(RpcError::new(-32601, format!("method not found: {method}"))),
        }
    }

    fn initialize(&mut self, params: Value) -> Value {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(LATEST_PROTOCOL_VERSION);
        let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
            requested
        } else {
            LATEST_PROTOCOL_VERSION
        };
        if let Some(name) = params
            .get("clientInfo")
            .and_then(|client| client.get("name"))
            .and_then(Value::as_str)
        {
            let slug: String = name
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                .take(40)
                .collect();
            if !slug.is_empty() {
                self.client_slug = slug;
            }
        }
        json!({
            "protocolVersion": negotiated,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "codecaddie",
                "title": "CodeCaddie",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "CodeCaddie serves the approved business goals for this device's workspace and records validated, evidence-cited analysis reports in a signed local ledger. Call get_workspace_status first. Use begin_analysis to pin frozen commits, then submit_analysis with citations; every citation is re-validated locally and source excerpts are rejected. This server never returns source code."
        })
    }

    fn call_tool(&mut self, params: Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(-32602, "tools/call requires a tool name"))?
            .to_string();
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));
        let outcome = match name.as_str() {
            "get_workspace_status" => self.gateway.workspace_status(),
            "get_approved_goals" => self.gateway.approved_goals(),
            "begin_analysis" => self.begin_analysis(arguments),
            "submit_analysis" => self.submit_analysis(arguments),
            "get_codebase_map" => self.get_codebase_map(arguments),
            "submit_codebase_map" => self.submit_codebase_map(arguments),
            "get_action_backlog" => self.gateway.action_backlog(),
            "record_action_note" => self.record_action_note(arguments),
            _ => return Err(RpcError::new(-32602, format!("unknown tool: {name}"))),
        };
        Ok(match outcome {
            Ok(result) => json!({
                "content": [{ "type": "text", "text": result.to_string() }],
                "structuredContent": result,
                "isError": false
            }),
            Err(error) => json!({
                "content": [{ "type": "text", "text": format!("{error:#}") }],
                "isError": true
            }),
        })
    }

    fn begin_analysis(&mut self, arguments: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct BeginAnalysisParams {
            repositories: Vec<AnalysisAttachment>,
        }
        let params: BeginAnalysisParams = serde_json::from_value(arguments)?;
        self.gateway.begin_analysis(params.repositories)
    }

    fn submit_analysis(&mut self, arguments: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct SubmitAnalysisParams {
            analysis_session_id: String,
            analysis: Value,
        }
        let params: SubmitAnalysisParams = serde_json::from_value(arguments)?;
        self.gateway.submit_analysis(
            &params.analysis_session_id,
            params.analysis,
            format!("mcp:{}", self.client_slug),
        )
    }

    fn get_codebase_map(&mut self, arguments: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct GetCodebaseMapParams {
            #[serde(default)]
            analysis_session_id: Option<String>,
        }
        let params: GetCodebaseMapParams = serde_json::from_value(arguments)?;
        self.gateway
            .get_codebase_map(params.analysis_session_id.as_deref())
    }

    fn submit_codebase_map(&mut self, arguments: Value) -> anyhow::Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct SubmitCodebaseMapParams {
            analysis_session_id: String,
            map: Value,
        }
        let params: SubmitCodebaseMapParams = serde_json::from_value(arguments)?;
        self.gateway.submit_codebase_map(
            &params.analysis_session_id,
            params.map,
            format!("mcp:{}", self.client_slug),
        )
    }

    fn record_action_note(&mut self, arguments: Value) -> anyhow::Result<Value> {
        let request: ReadyActionRequest = serde_json::from_value(arguments)?;
        self.gateway.record_action_note(request)
    }
}

fn error_response(id: Value, error: RpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": error.code, "message": error.message }
    })
}

fn tool_descriptors() -> anyhow::Result<Vec<Value>> {
    Ok(vec![
        json!({
            "name": "get_workspace_status",
            "description": "Report whether a CodeCaddie workspace with approved goals is available on this device (attested mode), plus this device's role.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "get_approved_goals",
            "description": "Return the approved, immutable goal versions and the goal-set hash. These goals cannot be edited or substituted.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "begin_analysis",
            "description": "Pin the frozen commit for each repository path and open an analysis session. Returns the repositoryIds to cite, the approved goals, and the citation rules.",
            "inputSchema": {
                "type": "object",
                "required": ["repositories"],
                "additionalProperties": false,
                "properties": {
                    "repositories": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "required": ["path"],
                            "additionalProperties": false,
                            "properties": {
                                "path": { "type": "string", "description": "Absolute path to a local git work tree" },
                                "repositoryId": { "type": "string", "description": "Workspace repository id; optional when the workspace registers exactly one repository" }
                            }
                        }
                    }
                }
            }
        }),
        json!({
            "name": "submit_analysis",
            "description": "Submit a completed analysis for validation and recording. Every citation is re-derived against the frozen commit; unknown goals, missing criteria, evidence-free verdicts, and source excerpts are rejected. Requires the Editor role.",
            "inputSchema": submit_analysis_schema()?
        }),
        json!({
            "name": "get_codebase_map",
            "description": "Return the newest validated codebase architecture map matching an analysis session's frozen commits (or the workspace's newest map). Components, relationships, data flows, and entry points with immutable evidence coordinates — never source text. When unavailable, survey the codebase and submit one with submit_codebase_map.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "analysisSessionId": { "type": "string", "description": "Optional session whose frozen commits the map must match" }
                }
            }
        }),
        json!({
            "name": "submit_codebase_map",
            "description": "Submit a surveyed codebase architecture map for validation and recording against a session's frozen commits. Every citation is re-derived; source excerpts are rejected outright. The session stays open for submit_analysis. Requires the Editor role.",
            "inputSchema": submit_codebase_map_schema()?
        }),
        json!({
            "name": "get_action_backlog",
            "description": "List tracked actions and the latest report's recommendations (metadata only, no evidence text).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "record_action_note",
            "description": "Record a completion note for a recommendation, moving its action to Ready-for-Verification. Only a subsequent scan can mark it Verified. Requires the Editor role.",
            "inputSchema": {
                "type": "object",
                "required": ["recommendationId", "title", "note"],
                "additionalProperties": false,
                "properties": {
                    "recommendationId": { "type": "string" },
                    "title": { "type": "string" },
                    "note": { "type": "string" }
                }
            }
        }),
    ])
}

/// Wraps the shared analysis contract schema, hoisting its `$defs` so the
/// `#/$defs/evidence` references keep resolving from the new document root.
fn submit_analysis_schema() -> anyhow::Result<Value> {
    let mut analysis: Value = serde_json::from_str(analyzer::ANALYSIS_SCHEMA)?;
    let defs = analysis
        .as_object_mut()
        .and_then(|object| object.remove("$defs"));
    let mut schema = json!({
        "type": "object",
        "required": ["analysisSessionId", "analysis"],
        "additionalProperties": false,
        "properties": {
            "analysisSessionId": { "type": "string" },
            "analysis": analysis
        }
    });
    if let Some(defs) = defs {
        schema["$defs"] = defs;
    }
    Ok(schema)
}

/// Wraps the shared map survey and deep-dive schemas the same way: `$defs`
/// hoisted to the wrapper root so evidence references keep resolving.
fn submit_codebase_map_schema() -> anyhow::Result<Value> {
    let mut survey: Value = serde_json::from_str(analyzer::CODEBASE_MAP_SCHEMA)?;
    let defs = survey
        .as_object_mut()
        .and_then(|object| object.remove("$defs"));
    let mut deep_dive: Value = serde_json::from_str(analyzer::CODEBASE_MAP_DEEP_DIVE_SCHEMA)?;
    deep_dive
        .as_object_mut()
        .and_then(|object| object.remove("$defs"));
    let mut schema = json!({
        "type": "object",
        "required": ["analysisSessionId", "map"],
        "additionalProperties": false,
        "properties": {
            "analysisSessionId": { "type": "string" },
            "map": {
                "type": "object",
                "required": ["survey"],
                "additionalProperties": false,
                "properties": {
                    "survey": survey,
                    "deepDives": { "type": "array", "maxItems": 4, "items": deep_dive }
                }
            }
        }
    });
    if let Some(defs) = defs {
        schema["$defs"] = defs;
    }
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_state::{ApproveGoalRequest, CreateWorkspaceRequest};
    use codecaddie_domain::{ReportOrigin, Verdict};
    use std::{fs, process::Command};
    use tempfile::TempDir;

    fn git_fixture() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        for arguments in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@codecaddie.local"],
            vec!["config", "user.name", "CodeCaddie Test"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(&arguments)
                .status()
                .unwrap();
        }
        fs::write(
            directory.path().join("tenant.rs"),
            "fn invoice(tenant: Id) {\n    scoped(tenant);\n}\n",
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["commit", "--quiet", "-m", "fixture"])
            .status()
            .unwrap();
        directory
    }

    fn store_at(root: &std::path::Path) -> LocalWorkspaceStore {
        LocalWorkspaceStore::new(root.to_path_buf()).unwrap()
    }

    fn request(id: u64, method: &str, params: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
    }

    fn call(server: &mut McpServer, tool: &str, args: Value) -> Value {
        let response = server
            .handle_line(&request(
                9,
                "tools/call",
                json!({ "name": tool, "arguments": args }),
            ))
            .unwrap();
        response["result"].clone()
    }

    fn structured(result: &Value) -> Value {
        assert_eq!(
            result["isError"], false,
            "tool call failed: {}",
            result["content"][0]["text"]
        );
        result["structuredContent"].clone()
    }

    #[test]
    fn initialize_negotiates_supported_protocol_versions_and_lists_tools() {
        let data = tempfile::tempdir().unwrap();
        let mut server = McpServer::new(store_at(data.path()), None);
        let response = server
            .handle_line(&request(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2025-03-26",
                    "clientInfo": { "name": "Claude Code", "version": "2.0" },
                    "capabilities": {}
                }),
            ))
            .unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(response["result"]["serverInfo"]["name"], "codecaddie");
        assert_eq!(server.client_slug, "claudecode");

        let response = server
            .handle_line(&request(
                2,
                "initialize",
                json!({ "protocolVersion": "1999-01-01" }),
            ))
            .unwrap();
        assert_eq!(
            response["result"]["protocolVersion"],
            LATEST_PROTOCOL_VERSION
        );

        // Notifications get no reply; unknown methods get a JSON-RPC error.
        assert!(
            server
                .handle_line(
                    &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string()
                )
                .is_none()
        );
        let response = server
            .handle_line(&request(3, "resources/list", json!({})))
            .unwrap();
        assert_eq!(response["error"]["code"], -32601);

        let response = server
            .handle_line(&request(4, "tools/list", json!({})))
            .unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 8);
        // The submit_analysis schema hoists $defs so evidence refs resolve.
        let submit = tools
            .iter()
            .find(|tool| tool["name"] == "submit_analysis")
            .unwrap();
        assert!(submit["inputSchema"]["$defs"]["evidence"].is_object());
        assert!(
            submit["inputSchema"]["properties"]["analysis"]["properties"]["assessments"]
                .is_object()
        );
        // The submit_codebase_map schema wraps the survey and deep-dive
        // contracts with the same hoisted $defs arrangement.
        let submit_map = tools
            .iter()
            .find(|tool| tool["name"] == "submit_codebase_map")
            .unwrap();
        assert!(submit_map["inputSchema"]["$defs"]["evidence"].is_object());
        assert!(
            submit_map["inputSchema"]["properties"]["map"]["properties"]["survey"]["properties"]
                ["components"]
                .is_object()
        );
    }

    #[test]
    fn workspace_status_degrades_gracefully_without_a_workspace() {
        let data = tempfile::tempdir().unwrap();
        let mut server = McpServer::new(store_at(data.path()), None);
        let result = call(&mut server, "get_workspace_status", json!({}));
        let status = structured(&result);
        assert_eq!(status["attestedModeAvailable"], false);
    }

    #[test]
    fn attested_flow_freezes_validates_and_records_an_agent_report() {
        let data = tempfile::tempdir().unwrap();
        let repository = git_fixture();
        let store = store_at(data.path());
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Acme".into(),
                repository_display_name: "acme-app".into(),
                repository_path: repository.path().display().to_string(),
                product_brief: "Acme promises tenant-isolated invoicing.".into(),
                context: Default::default(),
            })
            .unwrap();
        let goal = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "primary-goal".into(),
                    title: "Tenant isolation".into(),
                    business_outcome: "Customers cannot cross boundaries".into(),
                    criteria: vec!["Every invoice read is tenant scoped".into()],
                    priority: 5,
                    position: 1,
                    rubric_dimensions: vec!["security".into()],
                },
            )
            .unwrap();
        let mut server = McpServer::new(store, Some(workspace.workspace_id.clone()));

        let status = structured(&call(&mut server, "get_workspace_status", json!({})));
        assert_eq!(status["attestedModeAvailable"], true);
        assert_eq!(status["canSubmit"], true);

        let begun = structured(&call(
            &mut server,
            "begin_analysis",
            json!({ "repositories": [{ "path": repository.path().display().to_string() }] }),
        ));
        let session_id = begun["analysisSessionId"].as_str().unwrap().to_string();
        assert_eq!(
            begun["repositories"][0]["repositoryId"],
            "attached-repository"
        );
        let commit = begun["repositories"][0]["commitSha"].as_str().unwrap();
        assert_eq!(commit.len(), 40);
        assert_eq!(begun["goals"][0]["id"], goal.id);

        let analysis = |rationale: &str| {
            json!({
                "providerVersion": "test-agent 1.0",
                "assessments": [{
                    "goalVersionId": goal.id,
                    "summary": "Tenant isolation holds at the frozen commit.",
                    "criteria": [{
                        "criterionId": goal.criteria[0].id,
                        "verdict": "supported",
                        "rationale": rationale,
                        "confidence": 0.9,
                        "evidence": [{
                            "repositoryId": "attached-repository",
                            "path": "tenant.rs",
                            "startLine": 1,
                            "endLine": 2,
                            "kind": "implementation"
                        }]
                    }]
                }],
                "architecture": [],
                "recommendations": []
            })
        };

        // A rationale quoting cited source is rejected and the session survives
        // for a corrected resubmission.
        let rejected = call(
            &mut server,
            "submit_analysis",
            json!({
                "analysisSessionId": session_id,
                "analysis": analysis("The code literally reads fn invoice(tenant: Id) { here.")
            }),
        );
        assert_eq!(rejected["isError"], true);

        let recorded = structured(&call(
            &mut server,
            "submit_analysis",
            json!({
                "analysisSessionId": session_id,
                "analysis": analysis("Invoice reads are tenant scoped.")
            }),
        ));
        assert_eq!(recorded["recorded"], true);
        assert_eq!(recorded["origin"], "agent_session");
        assert_eq!(recorded["coverage"], 1.0);
        assert_eq!(recorded["goalVerdicts"][0]["verdict"], "supported");

        // The report is in the ledger with agent-session provenance, validated
        // through a second store handle over the same local state.
        let verify = store_at(data.path());
        let recent = verify.recent_workspace().unwrap().unwrap();
        let report = recent.latest_report.unwrap();
        assert_eq!(report.origin, ReportOrigin::AgentSession);
        assert_eq!(report.provider, "mcp:unknown");
        assert_eq!(report.assessments[0].verdict, Verdict::Supported);

        // The consumed session cannot be replayed.
        let replay = call(
            &mut server,
            "submit_analysis",
            json!({ "analysisSessionId": session_id, "analysis": analysis("Second try.") }),
        );
        assert_eq!(replay["isError"], true);
    }

    #[test]
    fn begin_analysis_requires_approved_goals() {
        let data = tempfile::tempdir().unwrap();
        let repository = git_fixture();
        let store = store_at(data.path());
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Acme".into(),
                repository_display_name: "acme-app".into(),
                repository_path: repository.path().display().to_string(),
                product_brief: "Acme promises tenant-isolated invoicing.".into(),
                context: Default::default(),
            })
            .unwrap();
        let mut server = McpServer::new(store, Some(workspace.workspace_id));
        let result = call(
            &mut server,
            "begin_analysis",
            json!({ "repositories": [{ "path": repository.path().display().to_string() }] }),
        );
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("approve")
        );
    }
}
#[test]
fn oversized_newline_free_input_is_rejected_with_bounded_retention() {
    let mut reader = BufReader::new(std::io::Cursor::new(vec![b'x'; 64]));
    assert!(matches!(
        read_bounded_line_with_limit(&mut reader, 16).unwrap(),
        BoundedLine::Oversized
    ));
}
