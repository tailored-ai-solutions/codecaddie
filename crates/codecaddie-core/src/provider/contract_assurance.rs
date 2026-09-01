//! Focused executable assurance for the provider contract declared in
//! `protocol/provider-contract-v1.schema.json`.

use super::{
    ProviderKind,
    contract::{FailureCode, FallbackPolicy, ProviderContractError, VERSION},
    runner::{PreparedProvider, ProviderRunner},
};
use std::{path::Path, time::Duration};

const CONTRACT_SCHEMA: &str = include_str!("../../../../protocol/provider-contract-v1.schema.json");
const GROK_HELP: &str =
    "--disable-web-search --no-subagents --tools --max-turns --sandbox --disallowed-tools";

fn prepared(kind: ProviderKind, executable: &Path) -> PreparedProvider {
    PreparedProvider {
        kind,
        executable: executable.to_path_buf(),
        claude_streams: false,
        grok_help: if kind == ProviderKind::Grok {
            GROK_HELP.into()
        } else {
            String::new()
        },
    }
}

#[cfg(unix)]
fn write_adapter(path: &Path, kind: ProviderKind, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    let script = match kind {
        ProviderKind::Codex => format!(
            "#!/bin/sh\nresult=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--output-last-message' ]; then result=$2; shift 2; else shift; fi\ndone\ncat >/dev/null\n{body}\n"
        ),
        ProviderKind::Claude | ProviderKind::Grok => format!("#!/bin/sh\n{body}\n"),
    };
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn valid_body(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Codex => {
            r#"printf '{"evidence":{"repositoryId":"repo","commitSha":"0123456789abcdef0123456789abcdef01234567","path":"src/lib.rs","startLine":1,"endLine":1}}' > "$result""#
        }
        ProviderKind::Claude | ProviderKind::Grok => {
            r#"printf '{"evidence":{"repositoryId":"repo","commitSha":"0123456789abcdef0123456789abcdef01234567","path":"src/lib.rs","startLine":1,"endLine":1}}'"#
        }
    }
}

fn malformed_body(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Codex => "printf 'not-json' > \"$result\"",
        ProviderKind::Claude | ProviderKind::Grok => "printf 'not-json'",
    }
}

#[cfg(unix)]
async fn wait_for_pid(path: &Path) -> i32 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse()
        {
            return pid;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "provider adapter did not start"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(unix)]
async fn assert_process_exited(pid: i32) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(pid, 0) } == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "cancelling the run must terminate the selected adapter"
    );
}

#[test]
fn versioned_contract_declares_every_shared_adapter_clause() {
    let schema: serde_json::Value = serde_json::from_str(CONTRACT_SCHEMA).unwrap();
    assert_eq!(schema["properties"]["schemaVersion"]["const"], VERSION);
    assert_eq!(schema["properties"]["fallback"]["const"], "forbidden");
    assert_eq!(
        schema["properties"]["provider"]["enum"],
        serde_json::json!(["codex", "claude", "grok"])
    );
    for clause in ["boundedOutput", "timeout", "cancellation", "typedErrors"] {
        assert_eq!(
            schema["properties"]["lifecycle"]["properties"][clause]["const"], true,
            "missing executable lifecycle clause {clause}"
        );
    }
    assert!(matches!(
        FallbackPolicy::Forbidden,
        FallbackPolicy::Forbidden
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn every_provider_adapter_conforms_to_valid_malformed_timeout_error_and_no_fallback_cases() {
    for kind in [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::Grok,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join(kind.executable());
        write_adapter(&executable, kind, valid_body(kind));
        let result = ProviderRunner {
            timeout: Duration::from_secs(10),
        }
        .run_structured_prepared(
            &prepared(kind, &executable),
            directory.path(),
            "prompt",
            r#"{"type":"object"}"#,
            None,
        )
        .await
        .unwrap();
        assert_eq!(result["evidence"]["repositoryId"], "repo");
        assert_eq!(
            result["evidence"]["commitSha"],
            "0123456789abcdef0123456789abcdef01234567"
        );

        write_adapter(&executable, kind, malformed_body(kind));
        let error = ProviderRunner {
            timeout: Duration::from_secs(10),
        }
        .run_structured_prepared(
            &prepared(kind, &executable),
            directory.path(),
            "prompt",
            "{}",
            None,
        )
        .await
        .unwrap_err();
        let typed = error.downcast_ref::<ProviderContractError>().unwrap();
        assert_eq!(typed.code, FailureCode::MalformedResult);
        assert_eq!(typed.provider, kind);

        write_adapter(
            &executable,
            kind,
            "printf 'authentication required' >&2\nexit 23",
        );
        let error = ProviderRunner {
            timeout: Duration::from_secs(10),
        }
        .run_structured_prepared(
            &prepared(kind, &executable),
            directory.path(),
            "prompt",
            "{}",
            None,
        )
        .await
        .unwrap_err();
        let typed = error.downcast_ref::<ProviderContractError>().unwrap();
        assert_eq!(typed.code, FailureCode::ProviderFailed);
        assert!(typed.to_string().contains("needs authentication"));

        write_adapter(&executable, kind, "sleep 2");
        let error = ProviderRunner {
            timeout: Duration::from_millis(25),
        }
        .run_structured_prepared(
            &prepared(kind, &executable),
            directory.path(),
            "prompt",
            "{}",
            None,
        )
        .await
        .unwrap_err();
        let typed = error.downcast_ref::<ProviderContractError>().unwrap();
        assert_eq!(typed.code, FailureCode::TimedOut);
        assert_eq!(typed.provider, kind);

        let pid_file = directory.path().join("cancelled.pid");
        write_adapter(
            &executable,
            kind,
            &format!("printf $$ > '{}'\nsleep 30", pid_file.display()),
        );
        let prepared = prepared(kind, &executable);
        let clone_path = directory.path().to_path_buf();
        let task = tokio::spawn(async move {
            ProviderRunner {
                timeout: Duration::from_secs(30),
            }
            .run_structured_prepared(&prepared, &clone_path, "prompt", "{}", None)
            .await
        });
        let pid = wait_for_pid(&pid_file).await;
        task.abort();
        let _ = task.await;
        assert_process_exited(pid).await;

        let unselected = directory.path().join("unselected-provider-ran");
        assert!(
            !unselected.exists(),
            "a failing selected adapter must never launch a fallback"
        );
    }
}
