//! The shared provider run lifecycle: probing one installation, spawning
//! the confined CLI configured by its contract module, bounding its output,
//! and terminating the isolated provider process tree on completion,
//! timeout, or cancellation.

use super::{
    MAX_PROVIDER_OUTPUT, ProgressSink, ProviderKind, claude, codex,
    contract::{ExecutionContract, FailureCode},
    find_compatible_on_path, grok,
    stream::{
        extract_stream_result, provider_failure_message, read_bounded, read_stream_output,
        structured_json,
    },
};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::Command,
    time::{Instant, timeout_at},
};

pub struct ProviderRunner {
    pub timeout: Duration,
}

struct ProviderProcessTree {
    #[cfg(unix)]
    pid: u32,
    armed: bool,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: `job` is an owned handle to an unnamed kernel job object. Kernel
// handles are process-wide rather than bound to the creating thread, this
// struct is the handle's only owner, and the `armed` guard makes
// terminate/close run at most once, so moving the value across threads (the
// dispatch futures holding it must be `Send`) is sound. The raw pointer type
// of `HANDLE` is what blocks the auto impl, not any thread affinity.
#[cfg(windows)]
unsafe impl Send for ProviderProcessTree {}

impl ProviderProcessTree {
    fn attach(child: &tokio::process::Child) -> anyhow::Result<Self> {
        #[cfg(unix)]
        let pid = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("provider process ID was unavailable"))?;
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::CloseHandle,
                System::JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                    SetInformationJobObject,
                },
            };
            let process = child
                .raw_handle()
                .ok_or_else(|| anyhow::anyhow!("provider process handle was unavailable"))?;
            // SAFETY: the job is unnamed, the information buffer is a valid
            // repr(C) value for the requested class, and the child handle is
            // live until Tokio reaps it below.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(std::io::Error::last_os_error().into());
                }
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                    || AssignProcessToJobObject(job, process as _) == 0
                {
                    let error = std::io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(error.into());
                }
                return Ok(Self { armed: true, job });
            }
        }
        #[cfg(not(windows))]
        Ok(Self {
            #[cfg(unix)]
            pid,
            armed: true,
        })
    }

    fn terminate(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        #[cfg(unix)]
        // SAFETY: provider commands are placed in a fresh process group whose
        // positive group ID is the direct child's PID. The negative target is
        // confined to that provider tree.
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::CloseHandle, System::JobObjects::TerminateJobObject,
            };
            // SAFETY: `job` was created and retained by `attach`; this is the
            // only path that terminates and closes it because `armed` is
            // cleared before entering the platform branch.
            unsafe {
                TerminateJobObject(self.job, 1);
                CloseHandle(self.job);
            }
        }
    }
}

impl Drop for ProviderProcessTree {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Debug, Clone)]
pub struct PreparedProvider {
    pub(crate) kind: ProviderKind,
    pub(crate) executable: PathBuf,
    pub(crate) claude_streams: bool,
    pub(crate) grok_help: String,
}

/// Application-owned context for a repository-reading provider pass. The
/// stream parser uses this only to label sanitized activity and to report an
/// honest denominator for distinct files actually read during the pass.
#[derive(Debug, Clone, Default)]
pub struct ProviderActivity {
    pub phase: Option<String>,
    pub repository_file_total: Option<usize>,
}

enum ProviderAccess {
    Repository(Option<ProviderActivity>),
    Disabled,
}

impl Default for ProviderRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30 * 60),
        }
    }
}

impl ProviderRunner {
    /// Resolves and probes one provider installation outside the async runtime.
    /// A scan reuses this result for every goal batch, avoiding concurrent
    /// capability probes against one CLI installation.
    pub async fn prepare(&self, kind: ProviderKind) -> anyhow::Result<PreparedProvider> {
        tokio::task::spawn_blocking(move || {
            let (executable, help) = find_compatible_on_path(kind)
                .ok_or_else(|| anyhow::anyhow!("{} is not installed", kind.executable()))?;
            let claude_streams = kind == ProviderKind::Claude && claude::streams(&help);
            let grok_help = if kind == ProviderKind::Grok {
                help
            } else {
                String::new()
            };
            Ok(PreparedProvider {
                kind,
                executable,
                claude_streams,
                grok_help,
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("provider capability probe failed: {error}"))?
    }

    pub async fn run_structured(
        &self,
        kind: ProviderKind,
        clone_path: &Path,
        prompt: &str,
        schema: &str,
        progress: Option<ProgressSink>,
    ) -> anyhow::Result<serde_json::Value> {
        let prepared = self.prepare(kind).await?;
        self.run_structured_prepared(&prepared, clone_path, prompt, schema, progress)
            .await
    }

    pub async fn run_structured_prepared(
        &self,
        prepared: &PreparedProvider,
        clone_path: &Path,
        prompt: &str,
        schema: &str,
        progress: Option<ProgressSink>,
    ) -> anyhow::Result<serde_json::Value> {
        self.run_structured_prepared_with_access(
            prepared,
            clone_path,
            prompt,
            schema,
            progress,
            ProviderAccess::Repository(None),
        )
        .await
    }

    pub async fn run_structured_prepared_with_activity(
        &self,
        prepared: &PreparedProvider,
        clone_path: &Path,
        prompt: &str,
        schema: &str,
        progress: Option<ProgressSink>,
        activity: ProviderActivity,
    ) -> anyhow::Result<serde_json::Value> {
        self.run_structured_prepared_with_access(
            prepared,
            clone_path,
            prompt,
            schema,
            progress,
            ProviderAccess::Repository(Some(activity)),
        )
        .await
    }

    pub async fn run_structured_prepared_without_repository_tools(
        &self,
        prepared: &PreparedProvider,
        clone_path: &Path,
        prompt: &str,
        schema: &str,
        progress: Option<ProgressSink>,
    ) -> anyhow::Result<serde_json::Value> {
        self.run_structured_prepared_with_access(
            prepared,
            clone_path,
            prompt,
            schema,
            progress,
            ProviderAccess::Disabled,
        )
        .await
    }

    async fn run_structured_prepared_with_access(
        &self,
        prepared: &PreparedProvider,
        clone_path: &Path,
        prompt: &str,
        schema: &str,
        progress: Option<ProgressSink>,
        access: ProviderAccess,
    ) -> anyhow::Result<serde_json::Value> {
        let (repository_tools, activity) = match access {
            ProviderAccess::Repository(activity) => (true, activity),
            ProviderAccess::Disabled => (false, None),
        };
        let kind = prepared.kind;
        let execution_contract = ExecutionContract::new(kind, self.timeout, MAX_PROVIDER_OUTPUT);
        debug_assert_eq!(execution_contract.provider, kind);
        debug_assert_eq!(execution_contract.output_limit_bytes, MAX_PROVIDER_OUTPUT);
        debug_assert!(matches!(
            execution_contract.fallback,
            super::contract::FallbackPolicy::Forbidden
        ));
        let executable = &prepared.executable;
        let claude_streams = prepared.claude_streams && progress.is_some();
        let grok_help = &prepared.grok_help;
        let grok_streams = progress.is_some() && grok::streams(grok_help);
        let mut command = Command::new(executable);
        let grok_isolated_home = if kind == ProviderKind::Grok {
            Some(
                grok::isolated_authorization_home(&mut command).map_err(|_| {
                    execution_contract.failure(
                        FailureCode::InvalidContract,
                        "the isolated authorization home could not be prepared",
                    )
                })?,
            )
        } else {
            None
        };
        let codex_contract = if kind == ProviderKind::Codex {
            Some(codex::prepare_contract(schema).map_err(|_| {
                execution_contract.failure(
                    FailureCode::InvalidContract,
                    "the private structured-output directory could not be prepared",
                )
            })?)
        } else {
            None
        };
        command
            .current_dir(clone_path)
            .env("NO_COLOR", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        match kind {
            ProviderKind::Claude => {
                claude::configure_command(
                    &mut command,
                    prompt,
                    schema,
                    claude_streams,
                    repository_tools,
                );
            }
            ProviderKind::Codex => {
                let contract = codex_contract.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("the Codex output contract directory was not initialized")
                })?;
                codex::configure_command(
                    &mut command,
                    contract.path(),
                    clone_path,
                    repository_tools,
                )?;
            }
            ProviderKind::Grok => {
                grok::configure_command(
                    &mut command,
                    clone_path,
                    prompt,
                    schema,
                    grok_help,
                    grok_streams,
                    repository_tools,
                )
                .map_err(|_| {
                    execution_contract.failure(
                        FailureCode::InvalidContract,
                        "the adapter no longer exposes the required read-only permissions",
                    )
                })?;
            }
        }
        let mut child = command.spawn().map_err(|error| {
            execution_contract.failure(
                FailureCode::StartFailed,
                format!(
                    "{} could not be started from its detected installation: {error}",
                    kind.executable()
                ),
            )
        })?;
        let _grok_isolated_home = grok_isolated_home;
        let mut process_tree = match ProviderProcessTree::attach(&child) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(execution_contract.failure(
                    FailureCode::StartFailed,
                    format!("provider process tree could not be isolated: {error}"),
                ));
            }
        };
        let deadline = Instant::now() + self.timeout;
        if kind == ProviderKind::Codex {
            let stdin = child.stdin.as_mut().ok_or_else(|| {
                execution_contract.failure(
                    FailureCode::IoFailed,
                    "provider input channel was unavailable",
                )
            })?;
            match timeout_at(deadline, stdin.write_all(prompt.as_bytes())).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    process_tree.terminate();
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Err(execution_contract.failure(
                        FailureCode::IoFailed,
                        format!("provider input could not be written: {error}"),
                    ));
                }
                Err(_) => {
                    process_tree.terminate();
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Err(
                        execution_contract.failure(FailureCode::TimedOut, "provider timed out")
                    );
                }
            }
            child.stdin.take();
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("provider stdout was not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("provider stderr was not available"))?;
        let execution = async {
            let stdout_reader = async {
                if let Some(sink) = progress {
                    read_stream_output(stdout, kind, sink, activity).await
                } else {
                    read_bounded(stdout).await
                }
            };
            let stderr_reader = read_bounded(stderr);
            let child_wait = async { child.wait().await.map_err(anyhow::Error::from) };
            tokio::try_join!(stdout_reader, stderr_reader, child_wait)
        };
        let execution = timeout_at(deadline, execution).await;
        let (stdout_bytes, stderr_bytes, status) = match execution {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                process_tree.terminate();
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(execution_contract.failure(FailureCode::IoFailed, error.to_string()));
            }
            Err(_) => {
                process_tree.terminate();
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(execution_contract.failure(FailureCode::TimedOut, "provider timed out"));
            }
        };
        // The direct CLI may exit after leaving a detached descendant. Kill
        // the isolated provider process group before accepting any terminal
        // result. On a clean exit this is a no-op beyond disarming the guard.
        process_tree.terminate();
        if !status.success() {
            // Provider stderr is deliberately not forwarded: it may contain
            // repository source, prompts, or paths and CoreResponse travels
            // over the desktop IPC boundary.
            return Err(execution_contract.failure(
                FailureCode::ProviderFailed,
                provider_failure_message(kind, &stdout_bytes, &stderr_bytes),
            ));
        }
        let result = if let Some(contract) = codex_contract {
            codex::read_result(contract.path())
        } else if claude_streams || grok_streams {
            extract_stream_result(&stdout_bytes)
        } else {
            structured_json(&stdout_bytes)
        };
        result.map_err(|_| {
            execution_contract.failure(
                FailureCode::MalformedResult,
                "provider returned no result matching the requested structured-output contract",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::contract::ProviderContractError;
    use std::time::Instant;

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_input_pipe_failures_remain_typed_contract_errors() {
        let directory = super::super::executable_script("#!/bin/sh\nexec 0<&-\nsleep 1\n");
        let prepared = PreparedProvider {
            kind: ProviderKind::Codex,
            executable: directory.path().join("provider-probe"),
            claude_streams: false,
            grok_help: String::new(),
        };
        let prompt = "x".repeat(1024 * 1024);
        let error = ProviderRunner {
            timeout: Duration::from_secs(5),
        }
        .run_structured_prepared(&prepared, directory.path(), &prompt, "{}", None)
        .await
        .unwrap_err();
        let typed = error
            .downcast_ref::<ProviderContractError>()
            .expect("Codex input failures must stay inside the typed provider contract");
        assert_eq!(typed.code, FailureCode::IoFailed);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_input_backpressure_stays_inside_the_provider_timeout() {
        let directory = super::super::executable_script("#!/bin/sh\nsleep 10\n");
        let prepared = PreparedProvider {
            kind: ProviderKind::Codex,
            executable: directory.path().join("provider-probe"),
            claude_streams: false,
            grok_help: String::new(),
        };
        let prompt = "x".repeat(1024 * 1024);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            ProviderRunner {
                timeout: Duration::from_millis(50),
            }
            .run_structured_prepared(&prepared, directory.path(), &prompt, "{}", None),
        )
        .await
        .expect("provider input backpressure escaped the lifecycle timeout");
        let error = result.unwrap_err();
        let typed = error
            .downcast_ref::<ProviderContractError>()
            .expect("provider input timeout must stay inside the typed contract");
        assert_eq!(typed.code, FailureCode::TimedOut);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repeatable_provider_execution_load_stays_within_versioned_p95_budget() {
        const SAMPLE_COUNT: usize = 12;
        let policy: serde_json::Value =
            serde_json::from_str(include_str!("../../../../config/reliability-gates.json"))
                .unwrap();
        let budget_milliseconds = policy["performance"]["providerExecutionP95Seconds"]
            .as_u64()
            .unwrap()
            * 1_000;
        let directory = super::super::executable_script(
            "#!/bin/sh\ncat >/dev/null\nresult=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--output-last-message' ]; then result=$2; shift 2; else shift; fi\ndone\ncat >/dev/null\nprintf '{\"ok\":true}' > \"$result\"\n",
        );
        let prepared = PreparedProvider {
            kind: ProviderKind::Codex,
            executable: directory.path().join("provider-probe"),
            claude_streams: false,
            grok_help: String::new(),
        };
        let runner = ProviderRunner {
            timeout: Duration::from_secs(10),
        };
        let mut latencies = Vec::with_capacity(SAMPLE_COUNT);

        for _ in 0..SAMPLE_COUNT {
            let started = Instant::now();
            let output = runner
                .run_structured_prepared(
                    &prepared,
                    directory.path(),
                    "bounded performance fixture",
                    r#"{"type":"object"}"#,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(output, serde_json::json!({ "ok": true }));
            latencies.push(started.elapsed().as_millis());
        }

        latencies.sort_unstable();
        let p95_index = ((latencies.len() * 95).div_ceil(100)).saturating_sub(1);
        let p95 = latencies[p95_index];
        assert!(
            p95 <= u128::from(budget_milliseconds),
            "repeatable provider-execution p95 {p95}ms exceeded the versioned {budget_milliseconds}ms target"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grok_runs_copy_only_authorization_into_an_isolated_home() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("grok");
        let home_file = directory.path().join("home.txt");
        let grok_home_file = directory.path().join("grok-home.txt");
        let arguments_file = directory.path().join("arguments.txt");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf '%s' \"$HOME\" > '{}'\nprintf '%s' \"$GROK_HOME\" > '{}'\nprintf '%s\\n' \"$@\" > '{}'\nprintf '{{\"ok\":true}}'\n",
                home_file.display(),
                grok_home_file.display(),
                arguments_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let prepared = PreparedProvider {
            kind: ProviderKind::Grok,
            executable,
            claude_streams: false,
            grok_help:
                "--disable-web-search --no-memory --no-subagents --tools --max-turns --sandbox strict"
                    .into(),
        };
        let output = ProviderRunner {
            timeout: Duration::from_secs(10),
        }
        .run_structured_prepared(&prepared, directory.path(), "prompt", "{}", None)
        .await
        .unwrap();
        assert_eq!(output, serde_json::json!({ "ok": true }));
        let isolated_home = std::fs::read_to_string(home_file).unwrap();
        let authorization_home = std::fs::read_to_string(grok_home_file).unwrap();
        assert!(isolated_home.contains("codecaddie-grok-home-"));
        assert_eq!(
            PathBuf::from(authorization_home),
            PathBuf::from(isolated_home)
        );
        let arguments = std::fs::read_to_string(arguments_file).unwrap();
        let arguments = arguments.lines().collect::<Vec<_>>();
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--sandbox", "strict"]),
            "Grok executions must use the verified strict sandbox profile"
        );

        let provider_home = tempfile::tempdir().unwrap();
        std::fs::write(provider_home.path().join("auth.json"), "authorization").unwrap();
        std::fs::write(provider_home.path().join("config.toml"), "malicious plugin").unwrap();
        std::fs::create_dir(provider_home.path().join("plugins")).unwrap();
        std::fs::write(
            provider_home.path().join("plugins/sentinel"),
            "must not load",
        )
        .unwrap();
        let isolated = grok::isolated_grok_home(provider_home.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(isolated.path().join("auth.json")).unwrap(),
            "authorization"
        );
        assert!(!isolated.path().join("config.toml").exists());
        assert!(!isolated.path().join("plugins").exists());
        assert_eq!(
            std::fs::metadata(isolated.path().join("auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grok_run_fails_closed_when_cached_contract_is_missing_controls() {
        let directory = super::super::executable_script("#!/bin/sh\ntouch should-not-run\n");
        let executable = directory.path().join("provider-probe");
        let prepared = PreparedProvider {
            kind: ProviderKind::Grok,
            executable,
            claude_streams: false,
            grok_help: "streaming-messages-json".into(),
        };
        let runner = ProviderRunner {
            timeout: Duration::from_secs(1),
        };
        let error = runner
            .run_structured_prepared(&prepared, directory.path(), "prompt", "{}", None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("read-only permissions"));
        assert!(!directory.path().join("should-not-run").exists());
    }

    #[cfg(unix)]
    async fn wait_for_process_exit(pid: i32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    }

    #[cfg(unix)]
    async fn wait_for_pid_file(path: &Path, deadline: Instant) -> Option<i32> {
        loop {
            if let Ok(contents) = std::fs::read_to_string(path)
                && let Ok(pid) = contents.trim().parse()
            {
                return Some(pid);
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn provider_timeout_and_cancellation_terminate_descendant_processes() {
        use std::os::unix::fs::PermissionsExt;

        const GROK_HELP: &str =
            "--disable-web-search --no-memory --no-subagents --tools --max-turns --sandbox strict";

        let timeout_directory = tempfile::tempdir().unwrap();
        let timeout_pid_file = timeout_directory.path().join("descendant.pid");
        let timeout_executable = timeout_directory.path().join("grok");
        std::fs::write(
            &timeout_executable,
            format!(
                "#!/bin/sh\n(sleep 30) &\nprintf $! > '{}'\nwhile :; do sleep 1; done\n",
                timeout_pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&timeout_executable, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let prepared = PreparedProvider {
            kind: ProviderKind::Grok,
            executable: timeout_executable,
            claude_streams: false,
            grok_help: GROK_HELP.into(),
        };
        let clone_path = timeout_directory.path().to_path_buf();
        let task = tokio::spawn(async move {
            ProviderRunner {
                timeout: Duration::from_secs(10),
            }
            .run_structured_prepared(&prepared, &clone_path, "prompt", "{}", None)
            .await
        });
        let deadline = Instant::now() + Duration::from_secs(8);
        let Some(timeout_pid) = wait_for_pid_file(&timeout_pid_file, deadline).await else {
            let result = task.await.expect("timed provider task panicked");
            panic!("timed provider did not start its descendant: {result:?}");
        };
        let error = task.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("timed out"));
        wait_for_process_exit(timeout_pid).await;

        let cancel_directory = tempfile::tempdir().unwrap();
        let cancel_pid_file = cancel_directory.path().join("descendant.pid");
        let cancel_executable = cancel_directory.path().join("grok");
        std::fs::write(
            &cancel_executable,
            format!(
                "#!/bin/sh\n(sleep 30) &\nprintf $! > '{}'\nwhile :; do sleep 1; done\n",
                cancel_pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cancel_executable, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let prepared = PreparedProvider {
            kind: ProviderKind::Grok,
            executable: cancel_executable,
            claude_streams: false,
            grok_help: GROK_HELP.into(),
        };
        let clone_path = cancel_directory.path().to_path_buf();
        let task = tokio::spawn(async move {
            ProviderRunner {
                timeout: Duration::from_secs(30),
            }
            .run_structured_prepared(&prepared, &clone_path, "prompt", "{}", None)
            .await
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        let Some(cancel_pid) = wait_for_pid_file(&cancel_pid_file, deadline).await else {
            let result = task.await.expect("cancelled provider task panicked");
            panic!("cancelled provider did not start its descendant: {result:?}");
        };
        task.abort();
        let _ = task.await;
        wait_for_process_exit(cancel_pid).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_codex_runs_use_only_confined_tools_and_reap_descendants() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("codex");
        let pid_file = directory.path().join("descendant.pid");
        let args_file = directory.path().join("args.txt");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' \"$*\" > '{}'\nresult=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--output-last-message' ]; then result=$2; shift 2; else shift; fi\ndone\n(sleep 30 </dev/null >/dev/null 2>&1) &\nprintf $! > '{}'\nprintf '{{\"ok\":true}}' > \"$result\"\n",
                args_file.display(),
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let prepared = PreparedProvider {
            kind: ProviderKind::Codex,
            executable,
            claude_streams: false,
            grok_help: String::new(),
        };
        let output = ProviderRunner {
            timeout: Duration::from_secs(10),
        }
        .run_structured_prepared(&prepared, directory.path(), "prompt", "{}", None)
        .await
        .unwrap();
        assert_eq!(output, serde_json::json!({ "ok": true }));
        let arguments = std::fs::read_to_string(args_file).unwrap();
        assert!(arguments.contains("--ignore-user-config"));
        assert!(arguments.contains("--disable shell_tool"));
        assert!(arguments.contains("--disable unified_exec"));
        assert!(arguments.contains("provider-repository-mcp"));
        assert!(arguments.contains("enabled_tools"));
        let descendant_pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        wait_for_process_exit(descendant_pid).await;
    }
}
