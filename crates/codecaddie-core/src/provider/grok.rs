//! The Grok CLI contract: verified strict-sandbox support, bounded
//! single-agent read-only runs, and an isolated authorization home that
//! carries only the installation's own credentials into the run.

use super::bounded_probe;
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub(super) fn contract_supported(executable: &Path, help: &str) -> bool {
    [
        "--disable-web-search",
        "--no-subagents",
        "--tools",
        "--max-turns",
        "--sandbox",
    ]
    .iter()
    .all(|flag| help.contains(flag))
        && bounded_probe(executable, &["--sandbox", "strict", "--version"]).is_some()
}

pub(super) fn streams(help: &str) -> bool {
    help.contains("streaming-messages-json")
}

fn grok_bounded_run_args(help: &str, repository_tools: bool) -> Option<Vec<&'static str>> {
    let required = [
        "--disable-web-search",
        "--no-subagents",
        "--tools",
        "--max-turns",
        "--sandbox",
    ];
    if required.iter().any(|flag| !help.contains(flag)) {
        return None;
    }
    if !repository_tools && !help.contains("--disallowed-tools") {
        return None;
    }
    let mut args = vec!["--disable-web-search"];
    // Grok 1.0.5 removed --no-memory. CodeCaddie does not depend on that
    // switch for isolation: every run already receives a temporary HOME that
    // contains only auth.json, so user config, sessions, and memory are absent.
    if help.contains("--no-memory") {
        args.push("--no-memory");
    }
    if help.contains("--no-plan") {
        args.push("--no-plan");
    }
    args.push("--no-subagents");
    if repository_tools {
        args.extend(["--tools", "list_dir,grep,read_file"]);
    } else {
        // Grok treats an empty --tools value as "use the default tool set".
        // Select one harmless sentinel, then deny it and the two meta-tools
        // that otherwise accompany an explicit allowlist. Current supported
        // CLIs consequently report an empty tool set for brief-only runs.
        args.extend([
            "--tools",
            "todo_write",
            "--disallowed-tools",
            "todo_write,search_tool,use_tool",
        ]);
    }
    args.extend(["--max-turns", "24"]);
    Some(args)
}

/// Points the run's `HOME`, `USERPROFILE`, and `GROK_HOME` at a temporary
/// home holding only the installation's authorization file, so user-level
/// configuration and plugins never load into a run over untrusted
/// repository content. The returned guard keeps the home alive for the
/// run's duration.
pub(super) fn isolated_authorization_home(
    command: &mut Command,
) -> anyhow::Result<tempfile::TempDir> {
    let provider_home = std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .map(|home| home.join(".grok"))
        })
        .ok_or_else(|| anyhow::anyhow!("Grok authorization home is unavailable"))?;
    let isolated = isolated_grok_home(&provider_home)?;
    command
        .env("HOME", isolated.path())
        .env("USERPROFILE", isolated.path())
        .env("GROK_HOME", isolated.path());
    Ok(isolated)
}

pub(super) fn isolated_grok_home(provider_home: &Path) -> anyhow::Result<tempfile::TempDir> {
    let isolated = tempfile::Builder::new()
        .prefix("codecaddie-grok-home-")
        .tempdir()?;
    let source = provider_home.join("auth.json");
    if source.is_file() {
        let destination = isolated.path().join("auth.json");
        std::fs::copy(source, &destination)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(isolated)
}

pub(super) fn configure_command(
    command: &mut Command,
    clone_path: &Path,
    prompt: &str,
    schema: &str,
    help: &str,
    streams: bool,
    repository_tools: bool,
) -> anyhow::Result<()> {
    // streaming-messages-json emits Anthropic Messages wire-format
    // NDJSON, so mid-run activity can surface in the progress feed;
    // --json-schema still constrains the terminal result.
    let output_format = if streams {
        "streaming-messages-json"
    } else {
        "json"
    };
    // The prompt must ride behind -p (single-turn headless mode).
    // A positional prompt makes the CLI start its interactive TUI,
    // which dies with "Device not configured" on piped stdio.
    command
        .args([
            "-p",
            prompt,
            "--output-format",
            output_format,
            "--json-schema",
            schema,
            "--cwd",
        ])
        .arg(clone_path)
        .arg("--verbatim");
    // Whole messages only arrive at turn boundaries, which leaves
    // the feed silent for the entire single-completion stretch of
    // a run. Partial messages add thinking deltas throughout.
    if streams {
        command.arg("--include-partial-messages");
    }
    // Repository text is untrusted and the host app owns the
    // operation lifecycle. Keep Grok single-agent, offline, and
    // bounded when the installed CLI exposes those controls.
    let bounded_args = grok_bounded_run_args(help, repository_tools).ok_or_else(|| {
        anyhow::anyhow!("Grok no longer exposes CodeCaddie's required read-only permissions")
    })?;
    command.args(bounded_args);
    // Match the Codex posture over untrusted repository content
    // when the installed CLI supports a sandbox profile.
    command.args(["--sandbox", "strict"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_runs_are_single_agent_offline_and_turn_bounded_when_supported() {
        let args = grok_bounded_run_args(
            "--disable-web-search --no-plan --no-subagents --tools <TOOLS> --max-turns <N> --sandbox <PROFILE>",
            true,
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "--disable-web-search",
                "--no-plan",
                "--no-subagents",
                "--tools",
                "list_dir,grep,read_file",
                "--max-turns",
                "24",
            ]
        );
        let no_repository_tools = grok_bounded_run_args(
            "--disable-web-search --no-subagents --tools <TOOLS> --disallowed-tools <TOOLS> --max-turns <N> --sandbox <PROFILE>",
            false,
        )
        .unwrap();
        assert!(
            no_repository_tools
                .windows(2)
                .any(|pair| pair == ["--tools", "todo_write"])
        );
        assert!(
            no_repository_tools
                .windows(2)
                .any(|pair| pair == ["--disallowed-tools", "todo_write,search_tool,use_tool"])
        );
        assert!(grok_bounded_run_args(
            "--disable-web-search --no-subagents --tools <TOOLS> --max-turns <N> --sandbox <PROFILE>",
            false,
        )
        .is_none());
        assert!(grok_bounded_run_args("older grok build", true).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn grok_contract_probe_reads_and_validates_capabilities_once() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("grok");
        std::fs::write(
            &path,
            "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then\n  echo 'streaming-messages-json --disable-web-search --no-subagents --tools --max-turns --sandbox <PROFILE>'\nelif [ \"$1\" = \"--sandbox\" ] && [ \"$2\" = \"strict\" ] && [ \"$3\" = \"--version\" ]; then\n  echo 'grok 1.0.5'\nelse\n  exit 2\nfi\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let help =
            crate::provider::provider_contract_help(crate::provider::ProviderKind::Grok, &path)
                .unwrap();
        assert!(help.contains("streaming-messages-json"));
        assert!(help.contains("--sandbox"));
        assert!(
            crate::provider::provider_contract_help(
                crate::provider::ProviderKind::Grok,
                &directory.path().join("missing")
            )
            .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn grok_contract_probe_rejects_an_unusable_strict_sandbox_profile() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("grok");
        std::fs::write(
            &path,
            "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then\n  echo '--disable-web-search --no-memory --no-subagents --tools --max-turns --sandbox <PROFILE>'\n  exit 0\nfi\nexit 2\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            crate::provider::provider_contract_help(crate::provider::ProviderKind::Grok, &path)
                .is_none()
        );
    }
}
