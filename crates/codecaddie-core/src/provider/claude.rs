//! The Claude CLI contract: required read-only capability flags and the
//! argument set for one structured `--print` run.

use tokio::process::Command;

pub(super) fn contract_supported(help: &str) -> bool {
    [
        "--safe-mode",
        "--tools",
        "--allowedTools",
        "--permission-mode",
        "--json-schema",
        "--no-session-persistence",
    ]
    .iter()
    .all(|flag| help.contains(flag))
}

/// Whether the installed Claude CLI accepts `--output-format stream-json`
/// in print mode. Checked from `--help` the same way the Codex contract
/// flags are probed; a CLI without the flag falls back to the blocking
/// `json` mode with no mid-run events.
pub(super) fn streams(help: &str) -> bool {
    help.contains("stream-json")
}

pub(super) fn configure_command(
    command: &mut Command,
    prompt: &str,
    schema: &str,
    streams: bool,
    repository_tools: bool,
) {
    const READ_TOOLS: &str = "Read,Glob,Grep";
    const READ_ALLOWLIST: &str = "Read(./**),Glob(./**),Grep(./**)";
    let tools = if repository_tools { READ_TOOLS } else { "" };
    let allowed_tools = if repository_tools { READ_ALLOWLIST } else { "" };
    if streams {
        // --print with stream-json requires --verbose on current CLIs.
        command.args([
            "--print",
            "--safe-mode",
            "--tools",
            tools,
            "--allowedTools",
            allowed_tools,
            "--permission-mode",
            "manual",
            "--verbose",
            "--output-format",
            "stream-json",
            "--json-schema",
            schema,
            "--no-session-persistence",
            prompt,
        ]);
    } else {
        command.args([
            "--print",
            "--safe-mode",
            "--tools",
            tools,
            "--allowedTools",
            allowed_tools,
            "--permission-mode",
            "manual",
            "--output-format",
            "json",
            "--json-schema",
            schema,
            "--no-session-persistence",
            prompt,
        ]);
    }
}
