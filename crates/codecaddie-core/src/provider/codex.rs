//! The Codex CLI contract: a confined `exec` run whose only repository
//! access is CodeCaddie's read-only MCP server, with the output schema and
//! structured result exchanged through a private contract directory.

use super::{MAX_PROVIDER_OUTPUT, ProviderKind};
use std::{io::Read, path::Path, process::Stdio};
use tokio::process::Command;

pub(super) fn contract_supported(help: &str) -> bool {
    help.contains("--output-schema")
        && help.contains("--output-last-message")
        && help.contains("--sandbox")
        && help.contains("--ephemeral")
        && help.contains("--ignore-user-config")
        && help.contains("--ignore-rules")
        && help.contains("--disable")
        && help.contains("--strict-config")
}

/// Creates the private directory that carries the output schema into the
/// run and receives the structured result back.
pub(super) fn prepare_contract(schema: &str) -> anyhow::Result<tempfile::TempDir> {
    let directory = tempfile::Builder::new()
        .prefix("codecaddie-codex-contract-")
        .tempdir()?;
    std::fs::write(directory.path().join("schema.json"), schema)?;
    Ok(directory)
}

pub(super) fn configure_command(
    command: &mut Command,
    contract_path: &Path,
    clone_path: &Path,
    repository_tools: bool,
) -> anyhow::Result<()> {
    command
        .args([
            "exec",
            "--json",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--strict-config",
            "--disable",
            "shell_tool",
            "--disable",
            "unified_exec",
            "--disable",
            "js_repl",
            "--disable",
            "apply_patch_freeform",
            "--disable",
            "search_tool",
            "--disable",
            "apps",
            "--disable",
            "plugins",
            "--disable",
            "multi_agent",
            "--disable",
            "computer_use",
            "--disable",
            "browser_use",
            "--sandbox",
            "read-only",
            "--skip-git-repo-check",
        ])
        .args(["-c", "approval_policy=\"never\""])
        .args(["-c", "web_search=\"disabled\""]);
    if repository_tools {
        let core_executable = std::env::current_exe()?.canonicalize()?;
        let mcp_command = serde_json::to_string(&core_executable.to_string_lossy())?;
        let mcp_arguments = serde_json::to_string(&vec![
            "provider-repository-mcp".to_string(),
            clone_path.to_string_lossy().into_owned(),
        ])?;
        command
            .args(["-c", &format!("mcp_servers.codecaddie_repository.command={mcp_command}")])
            .args(["-c", &format!("mcp_servers.codecaddie_repository.args={mcp_arguments}")])
            .args(["-c", "mcp_servers.codecaddie_repository.required=true"])
            .args(["-c", "mcp_servers.codecaddie_repository.enabled_tools=[\"list_repository_files\",\"search_repository\",\"read_repository_file\"]"]);
    }
    command
        .arg("--output-schema")
        .arg(contract_path.join("schema.json"))
        .arg("--output-last-message")
        .arg(contract_path.join("result.json"))
        .arg("-C")
        .arg(clone_path)
        .arg("-")
        .stdin(Stdio::piped());
    Ok(())
}

/// Reads the bounded structured result the run wrote into the contract
/// directory.
pub(super) fn read_result(contract_path: &Path) -> anyhow::Result<serde_json::Value> {
    let result_path = contract_path.join("result.json");
    let result_file = std::fs::File::open(&result_path).map_err(|_| {
        anyhow::anyhow!(
            "{} exited without writing the required structured result; update the installed tool",
            ProviderKind::Codex.executable()
        )
    })?;
    if result_file.metadata()?.len() > MAX_PROVIDER_OUTPUT as u64 {
        anyhow::bail!("provider result exceeded 16 MiB");
    }
    let mut result = Vec::new();
    result_file
        .take((MAX_PROVIDER_OUTPUT + 1) as u64)
        .read_to_end(&mut result)?;
    if result.len() > MAX_PROVIDER_OUTPUT {
        anyhow::bail!("provider result exceeded 16 MiB");
    }
    super::stream::structured_json(&result)
}

/// Maps one Codex NDJSON event to a display-ready progress message, or
/// `None` for lines with no user-facing signal.
pub(super) fn progress_message(value: &serde_json::Value, event_type: &str) -> Option<String> {
    let message = match event_type {
        "thread.started" | "turn.started" => "Reading the project context".to_string(),
        "turn.completed" => "Assembling the drafts".to_string(),
        "item.started" | "item.completed" | "item.updated" => {
            let item = value.get("item")?;
            let item_type = item
                .get("item_type")
                .or_else(|| item.get("type"))
                .and_then(serde_json::Value::as_str)?;
            match item_type {
                "command_execution" => "Inspecting repository files".to_string(),
                "agent_message" => "Reviewing gathered evidence".to_string(),
                "reasoning" => "Evaluating the goal criteria".to_string(),
                "web_search" => "Checking external context".to_string(),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(message)
}

/// Folds retained Codex `error` and `turn.failed` events into the failure
/// diagnostic so failures classify safely when no result is produced.
pub(super) fn append_failure_diagnostics(stdout: &[u8], diagnostic: &mut String) {
    for line in stdout.split(|byte| *byte == b'\n') {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        let event_type = value.get("type").and_then(serde_json::Value::as_str);
        if matches!(event_type, Some("error" | "turn.failed")) {
            diagnostic.push(' ');
            diagnostic.push_str(&value.to_string().to_ascii_lowercase());
        }
    }
}
