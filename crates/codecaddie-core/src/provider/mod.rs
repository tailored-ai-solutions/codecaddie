//! Detection of installed provider CLIs and structured analysis runs.
//!
//! Each supported CLI contract lives in its own module (`claude`, `codex`,
//! `grok`); `runner` owns the shared process lifecycle and `stream` owns
//! bounded output parsing and progress sanitization. Provider stderr never
//! crosses the IPC boundary and no credentials are accepted or persisted.

mod claude;
mod codex;
pub(crate) mod contract;
#[cfg(test)]
mod contract_assurance;
mod grok;
mod runner;
mod stream;

pub use runner::{PreparedProvider, ProviderActivity, ProviderRunner};

pub(crate) fn contract_failure_code(error: &anyhow::Error) -> Option<contract::FailureCode> {
    error
        .chain()
        .find_map(|source| source.downcast_ref::<contract::ProviderContractError>())
        .map(|error| error.code)
}

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Seek},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_PROVIDER_OUTPUT: usize = 16 * 1024 * 1024;
const MAX_PROVIDER_LINE: usize = 1024 * 1024;
const MAX_PROGRESS_CHARS: usize = 160;

/// Receives sanitized, display-ready progress lines while a provider runs.
/// Never receives raw provider stderr.
pub type ProgressSink = Arc<dyn Fn(String) + Send + Sync>;

pub(crate) fn display_file_count(value: usize) -> String {
    let digits = value.to_string();
    let mut displayed = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            displayed.push(',');
        }
        displayed.push(character);
    }
    displayed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Claude,
    Codex,
    Grok,
}

impl ProviderKind {
    pub fn executable(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Grok => "grok",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapability {
    pub kind: ProviderKind,
    pub installed: bool,
    pub executable: Option<PathBuf>,
    pub version: Option<String>,
    pub permissions_summary: String,
}

pub fn detect_all() -> Vec<ProviderCapability> {
    [
        ProviderKind::Grok,
        ProviderKind::Codex,
        ProviderKind::Claude,
    ]
    .into_iter()
    .map(detect)
    .collect()
}

fn detect(kind: ProviderKind) -> ProviderCapability {
    let executable = find_compatible_on_path(kind).map(|(path, _)| path);
    let version = executable.as_ref().and_then(|path| {
        bounded_probe(path, &["--version"])
            .map(|output| String::from_utf8_lossy(&output).trim().to_owned())
    });
    ProviderCapability {
        kind,
        installed: executable.is_some(),
        executable,
        version,
        permissions_summary: "Receives a history-free single-commit snapshot. The provider may process its files under the installation's existing authorization, settings, privacy terms, and organizational policy.".into(),
    }
}

fn find_compatible_on_path(kind: ProviderKind) -> Option<(PathBuf, String)> {
    let paths = std::env::var_os("PATH")?;
    let mut seen = BTreeSet::new();
    std::env::split_paths(&paths)
        .map(|path| path.join(kind.executable()))
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| {
            let identity = std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.clone());
            seen.insert(identity)
        })
        .find_map(|candidate| {
            provider_contract_help(kind, &candidate).map(|help| (candidate, help))
        })
}

fn provider_contract_help(kind: ProviderKind, executable: &Path) -> Option<String> {
    let output = match kind {
        ProviderKind::Codex => bounded_probe(executable, &["exec", "--help"]),
        ProviderKind::Claude | ProviderKind::Grok => bounded_probe(executable, &["--help"]),
    }?;
    let help = String::from_utf8_lossy(&output).into_owned();
    let supported = match kind {
        ProviderKind::Codex => codex::contract_supported(&help),
        ProviderKind::Claude => claude::contract_supported(&help),
        ProviderKind::Grok => grok::contract_supported(executable, &help),
    };
    supported.then_some(help)
}

fn bounded_probe(executable: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    const MAX_PROBE_OUTPUT: usize = 1024 * 1024;
    const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
    // A regular anonymous file avoids a pipe-reader thread that can remain
    // blocked when a misbehaving CLI forks a descendant which inherits
    // stdout. The file length is polled while the direct child runs and read
    // through a hard MAX+1 bound after exit.
    let mut output = tempfile::tempfile().ok()?;
    let child_output = output.try_clone().ok()?;
    let mut command = std::process::Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::from(child_output))
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().ok()?;
    #[cfg(unix)]
    let process_group = child.id();
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let success = loop {
        if output.metadata().ok()?.len() > MAX_PROBE_OUTPUT as u64 {
            break false;
        }
        match child.try_wait().ok()? {
            Some(status) => break status.success(),
            None if Instant::now() >= deadline => {
                break false;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    #[cfg(unix)]
    // SAFETY: the child was placed in a new process group whose ID is its
    // positive PID. A negative target addresses only that isolated group.
    unsafe {
        libc::kill(-(process_group as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    if !success {
        return None;
    }
    output.seek(std::io::SeekFrom::Start(0)).ok()?;
    let mut retained = Vec::new();
    output
        .take((MAX_PROBE_OUTPUT + 1) as u64)
        .read_to_end(&mut retained)
        .ok()?;
    (retained.len() <= MAX_PROBE_OUTPUT).then_some(retained)
}

#[cfg(all(test, unix))]
fn executable_script(contents: &str) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("provider-probe");
    std::fs::write(&path, contents).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_installed_tools_and_never_api_credentials() {
        let detections = detect_all();
        assert_eq!(detections.len(), 3);
        assert!(detections.iter().all(|item| matches!(
            item.kind,
            ProviderKind::Claude | ProviderKind::Codex | ProviderKind::Grok
        )));
    }

    #[test]
    fn file_counts_are_easy_to_scan() {
        assert_eq!(display_file_count(0), "0");
        assert_eq!(display_file_count(999), "999");
        assert_eq!(display_file_count(13_345), "13,345");
    }

    #[cfg(unix)]
    #[test]
    fn provider_probe_times_out_and_bounds_retained_output() {
        let hanging = executable_script("#!/bin/sh\nwhile :; do :; done\n");
        let started = Instant::now();
        assert!(bounded_probe(&hanging.path().join("provider-probe"), &[]).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));

        let noisy =
            executable_script("#!/bin/sh\ndd if=/dev/zero bs=1048576 count=2 2>/dev/null\n");
        assert!(bounded_probe(&noisy.path().join("provider-probe"), &[]).is_none());

        let inherited_output = executable_script("#!/bin/sh\n(sleep 30) &\nprintf compatible\n");
        let started = Instant::now();
        let output = bounded_probe(&inherited_output.path().join("provider-probe"), &[]).unwrap();
        assert_eq!(output, b"compatible");
        assert!(started.elapsed() < Duration::from_secs(5));

        let pid_file = inherited_output.path().join("descendant.pid");
        let flooding = executable_script(&format!(
            "#!/bin/sh\nprintf compatible\n(while :; do printf x; sleep 0.01; done) &\nprintf $! > '{}'\n",
            pid_file.display()
        ));
        let started = Instant::now();
        let output = bounded_probe(&flooding.path().join("provider-probe"), &[]).unwrap();
        assert!(output.starts_with(b"compatible"));
        assert!(output.len() <= 1024 * 1024);
        assert!(started.elapsed() < Duration::from_secs(5));
        let descendant_pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while unsafe { libc::kill(descendant_pid, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(unsafe { libc::kill(descendant_pid, 0) }, 0);
    }
}
