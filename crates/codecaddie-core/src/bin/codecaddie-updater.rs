use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    thread,
    time::{Duration, Instant},
};

const MAX_MACOS_ZIP_ENTRIES: usize = 4096;
const MAX_MACOS_ZIP_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MACOS_ZIP_COMPRESSION_RATIO: u64 = 200;
const MACOS_ZIP_RATIO_GRACE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
struct MacosZipLimits {
    entries: usize,
    uncompressed_bytes: u64,
    compression_ratio: u64,
    ratio_grace_bytes: u64,
}

const MACOS_ZIP_LIMITS: MacosZipLimits = MacosZipLimits {
    entries: MAX_MACOS_ZIP_ENTRIES,
    uncompressed_bytes: MAX_MACOS_ZIP_UNCOMPRESSED_BYTES,
    compression_ratio: MAX_MACOS_ZIP_COMPRESSION_RATIO,
    ratio_grace_bytes: MACOS_ZIP_RATIO_GRACE_BYTES,
};

struct UpdateArguments {
    artifact: PathBuf,
    parent_pid: u32,
    current_executable: PathBuf,
}

#[derive(Debug)]
struct InstallStateUnverified {
    code: codecaddie_core::update::UpdaterResultCode,
    message: &'static str,
}

impl std::fmt::Display for InstallStateUnverified {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for InstallStateUnverified {}

fn recovery_policy(error: &anyhow::Error) -> (codecaddie_core::update::UpdaterResultCode, bool) {
    if let Some(unverified) = error.downcast_ref::<InstallStateUnverified>() {
        return (unverified.code, false);
    }
    (
        codecaddie_core::update::UpdaterResultCode::InstallFailed,
        true,
    )
}

fn main() -> ExitCode {
    let raw_arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let fallback_current = argument_value(&raw_arguments, "--current-executable");
    let fallback_parent = argument_value(&raw_arguments, "--parent-pid")
        .and_then(|value| value.to_string_lossy().parse::<u32>().ok());
    let arguments = match parse_arguments(raw_arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            if let Some(current_executable) = fallback_current {
                let _ = recover_failed_update(
                    &current_executable,
                    fallback_parent,
                    codecaddie_core::update::UpdaterResultCode::InstallFailed,
                    true,
                );
            }
            eprintln!("CodeCaddie updater failed: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let (result_code, allow_reopen) = recovery_policy(&error);
            if let Err(recovery_error) = recover_failed_update(
                &arguments.current_executable,
                Some(arguments.parent_pid),
                result_code,
                allow_reopen,
            ) {
                eprintln!("CodeCaddie updater recovery failed: {recovery_error:#}");
            }
            show_fixed_failure_notice(result_code);
            eprintln!("CodeCaddie updater failed: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn argument_value(arguments: &[OsString], name: &str) -> Option<PathBuf> {
    arguments
        .windows(2)
        .find(|pair| pair[0].to_string_lossy() == name)
        .map(|pair| PathBuf::from(&pair[1]))
}

fn parse_arguments(arguments: Vec<OsString>) -> anyhow::Result<UpdateArguments> {
    let mut args = arguments.into_iter();
    let mut artifact = None;
    let mut parent_pid = None;
    let mut current_executable = None;
    while let Some(argument) = args.next() {
        match argument.to_string_lossy().as_ref() {
            "--artifact" => artifact = args.next().map(PathBuf::from),
            "--parent-pid" => {
                parent_pid = args
                    .next()
                    .and_then(|value| value.to_string_lossy().parse::<u32>().ok())
            }
            "--current-executable" => current_executable = args.next().map(PathBuf::from),
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let artifact = artifact.ok_or_else(|| anyhow::anyhow!("--artifact is required"))?;
    let parent_pid = parent_pid.ok_or_else(|| anyhow::anyhow!("--parent-pid is required"))?;
    let current_executable =
        current_executable.ok_or_else(|| anyhow::anyhow!("--current-executable is required"))?;
    Ok(UpdateArguments {
        artifact,
        parent_pid,
        current_executable,
    })
}

fn run(arguments: &UpdateArguments) -> anyhow::Result<()> {
    wait_for_parent(arguments.parent_pid, Duration::from_secs(300))?;
    // Revalidate signed metadata, platform selection, downgrade rules, size, and
    // checksum after the application has exited and immediately before install.
    let staged = codecaddie_core::update::validate_staged(&arguments.artifact)?;
    if cfg!(target_os = "macos") {
        install_macos(
            &staged.artifact_path,
            &arguments.current_executable,
            &staged.version,
            staged.build,
            &staged.source_commit,
        )
    } else if cfg!(target_os = "windows") {
        install_windows(
            &staged.artifact_path,
            &arguments.current_executable,
            &staged.version,
            staged.build,
            &staged.source_commit,
        )
    } else {
        anyhow::bail!("automatic installation is supported only on macOS and Windows")
    }
}

fn recover_failed_update_with<Record, Reopen>(
    initial_code: codecaddie_core::update::UpdaterResultCode,
    should_reopen: bool,
    mut record: Record,
    reopen: Reopen,
) -> anyhow::Result<()>
where
    Record: FnMut(codecaddie_core::update::UpdaterResultCode) -> anyhow::Result<()>,
    Reopen: FnOnce() -> anyhow::Result<()>,
{
    let record_error = record(initial_code).err();
    if !should_reopen {
        return match record_error {
            Some(error) => Err(error),
            None => Ok(()),
        };
    }
    if let Err(reopen_error) = reopen() {
        let final_record_error =
            record(codecaddie_core::update::UpdaterResultCode::ReopenFailed).err();
        return match (record_error, final_record_error) {
            (Some(first), Some(second)) => anyhow::bail!(
                "the failure result could not be recorded ({first}); reopening failed ({reopen_error}); the final result could not be recorded ({second})"
            ),
            (Some(first), None) => anyhow::bail!(
                "the failure result could not initially be recorded ({first}); reopening failed: {reopen_error}"
            ),
            (None, Some(second)) => anyhow::bail!(
                "reopening failed ({reopen_error}); the final result could not be recorded ({second})"
            ),
            (None, None) => Err(reopen_error),
        };
    }
    match record_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn recover_failed_update(
    current_executable: &Path,
    parent_pid: Option<u32>,
    result_code: codecaddie_core::update::UpdaterResultCode,
    allow_reopen: bool,
) -> anyhow::Result<()> {
    let parent_is_running = parent_pid
        .filter(|pid| *pid != 0)
        .and_then(|pid| process_exists(pid).ok())
        .unwrap_or(false);
    let mut record_failed = false;
    let mut reopen_attempted = false;
    let mut reopen_failed = false;
    let recovery = recover_failed_update_with(
        result_code,
        allow_reopen && !parent_is_running,
        |code| {
            let result = codecaddie_core::update::record_updater_result(code).map_err(Into::into);
            if result.is_err() {
                record_failed = true;
            }
            result
        },
        || {
            reopen_attempted = true;
            match reopen_current_application(current_executable) {
                Ok(()) => Ok(()),
                Err(error) => {
                    reopen_failed = true;
                    Err(error)
                }
            }
        },
    );
    if let Some(notice_code) = recovery_notice_code(record_failed, reopen_attempted, reopen_failed)
    {
        show_fixed_failure_notice(notice_code);
    }
    recovery
}

fn recovery_notice_code(
    record_failed: bool,
    reopen_attempted: bool,
    reopen_failed: bool,
) -> Option<codecaddie_core::update::UpdaterResultCode> {
    if reopen_failed {
        return Some(codecaddie_core::update::UpdaterResultCode::ReopenFailed);
    }
    if record_failed && reopen_attempted {
        return Some(codecaddie_core::update::UpdaterResultCode::ResultUnreadable);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FixedFailureNotice {
    title: &'static str,
    message: &'static str,
}

fn fixed_failure_notice(
    platform: &str,
    code: codecaddie_core::update::UpdaterResultCode,
) -> Option<FixedFailureNotice> {
    let title = "CodeCaddie update needs attention";
    let message = match (platform, code) {
        ("windows", codecaddie_core::update::UpdaterResultCode::RestartRequired) => {
            "Windows Installer could not prove that the CodeCaddie update is ready to run. Restart Windows before opening CodeCaddie. If it still will not open after restart, download the latest signed release from codecaddie.ai and choose Repair or reinstall."
        }
        ("windows", codecaddie_core::update::UpdaterResultCode::ReopenFailed) => {
            "CodeCaddie could not reopen after an update failure. Open CodeCaddie from the Start menu. If it will not open, download the latest signed release from codecaddie.ai and choose Repair or reinstall. Your local projects were not changed."
        }
        ("windows", codecaddie_core::update::UpdaterResultCode::ResultUnreadable) => {
            "CodeCaddie reopened, but could not save the update result. Check the installed version in Settings before trying again. If the version did not advance, download the latest signed release from codecaddie.ai and choose Repair or reinstall."
        }
        ("macos", codecaddie_core::update::UpdaterResultCode::ManualRepairRequired) => {
            "CodeCaddie could not confirm a safe rollback. Do not open the current copy. Download the latest signed release from codecaddie.ai and replace CodeCaddie. Your local projects were not changed."
        }
        ("macos", codecaddie_core::update::UpdaterResultCode::ReopenFailed) => {
            "CodeCaddie could not reopen after an update failure. Open CodeCaddie from Applications. If it will not open, download the latest signed release from codecaddie.ai and replace CodeCaddie. Your local projects were not changed."
        }
        ("macos", codecaddie_core::update::UpdaterResultCode::ResultUnreadable) => {
            "CodeCaddie reopened, but could not save the update result. Check the installed version in Settings before trying again. If the version did not advance, download the latest signed release from codecaddie.ai and replace CodeCaddie."
        }
        _ => return None,
    };
    Some(FixedFailureNotice { title, message })
}

#[cfg(target_os = "windows")]
fn show_fixed_failure_notice(code: codecaddie_core::update::UpdaterResultCode) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
    };

    let Some(notice) = fixed_failure_notice("windows", code) else {
        return;
    };
    let title = format!("{}\0", notice.title)
        .encode_utf16()
        .collect::<Vec<_>>();
    let message = format!("{}\0", notice.message)
        .encode_utf16()
        .collect::<Vec<_>>();
    // This notice contains only fixed product text. Never display the raw OS,
    // filesystem, or installer error because those values may contain local
    // paths or other private data.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn show_fixed_failure_notice(code: codecaddie_core::update::UpdaterResultCode) {
    let notice = fixed_failure_notice(std::env::consts::OS, code);
    #[cfg(target_os = "macos")]
    if let Some(notice) = notice {
        let _ = Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display alert (item 1 of argv) message (item 2 of argv) as critical buttons {\"OK\"} default button \"OK\"",
                "-e",
                "end run",
                "--",
                notice.title,
                notice.message,
            ])
            .status();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = notice;
}

fn reopen_current_application(current_executable: &Path) -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        let application = current_executable
            .ancestors()
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
            .ok_or_else(|| {
                anyhow::anyhow!("current CodeCaddie application bundle was not found")
            })?;
        if !application.is_dir() {
            anyhow::bail!("the current CodeCaddie application bundle is missing")
        }
        let status = Command::new("/usr/bin/open").arg(application).status()?;
        if status.success() {
            return Ok(());
        }
        anyhow::bail!("macOS could not reopen CodeCaddie")
    }
    if cfg!(target_os = "windows") {
        let application = current_executable.with_file_name("codecaddie.exe");
        if !application.is_file() {
            anyhow::bail!("the current CodeCaddie application is missing")
        }
        Command::new(application).spawn()?;
        return Ok(());
    }
    anyhow::bail!("automatic recovery is supported only on macOS and Windows")
}

fn wait_for_parent(parent_pid: u32, timeout: Duration) -> anyhow::Result<()> {
    // The desktop uses zero when the platform UI runtime does not expose its
    // process ID. It exits immediately after the core acknowledges staging;
    // this short delay keeps the helper outside that process teardown path.
    if parent_pid == 0 {
        thread::sleep(Duration::from_secs(2));
        return Ok(());
    }
    let started = Instant::now();
    while process_exists(parent_pid)? {
        if started.elapsed() >= timeout {
            anyhow::bail!(
                "CodeCaddie did not close within {} seconds",
                timeout.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn process_exists(pid: u32) -> anyhow::Result<bool> {
    if cfg!(target_os = "windows") {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()?;
        return Ok(output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()));
    }
    Ok(Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()?
        .success())
}

fn zip32_entry_count(file: &mut fs::File) -> anyhow::Result<usize> {
    const END_HEADER_BYTES: usize = 22;
    const MAX_COMMENT_BYTES: usize = u16::MAX as usize;
    let length = file.metadata()?.len();
    if length < END_HEADER_BYTES as u64 {
        anyhow::bail!("the updater ZIP has no valid central directory")
    }
    let tail_length = length.min((END_HEADER_BYTES + MAX_COMMENT_BYTES) as u64) as usize;
    file.seek(SeekFrom::Start(length - tail_length as u64))?;
    let mut tail = vec![0_u8; tail_length];
    file.read_exact(&mut tail)?;

    for offset in (0..=tail.len() - END_HEADER_BYTES).rev() {
        if tail[offset..offset + 4] != *b"PK\x05\x06" {
            continue;
        }
        let u16_at = |relative: usize| {
            u16::from_le_bytes([tail[offset + relative], tail[offset + relative + 1]])
        };
        let comment_length = usize::from(u16_at(20));
        if offset + END_HEADER_BYTES + comment_length != tail.len() {
            continue;
        }
        let disk = u16_at(4);
        let directory_disk = u16_at(6);
        let entries_on_disk = u16_at(8);
        let entries = u16_at(10);
        if disk != 0 || directory_disk != 0 || entries_on_disk != entries || entries == u16::MAX {
            anyhow::bail!("multi-disk and ZIP64 updater archives are not supported")
        }
        return Ok(usize::from(entries));
    }
    anyhow::bail!("the updater ZIP has no valid central directory")
}

fn normalized_macos_zip_entry_name(raw_name: &[u8], is_dir: bool) -> anyhow::Result<String> {
    let name = std::str::from_utf8(raw_name)
        .map_err(|_| anyhow::anyhow!("the updater ZIP contains a non-UTF-8 path"))?;
    if name.is_empty()
        || name.len() > 1024
        || !name.is_ascii()
        || name.contains(['\\', '\0', ':'])
        || name.starts_with('/')
        || is_dir != name.ends_with('/')
    {
        anyhow::bail!("the updater ZIP contains an unsafe path")
    }
    let normalized = if is_dir {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    if normalized.is_empty() || normalized.ends_with('/') {
        anyhow::bail!("the updater ZIP contains an empty path component")
    }
    let components: Vec<_> = normalized.split('/').collect();
    if components.first().copied() != Some("CodeCaddie.app")
        || (components.len() == 1 && !is_dir)
        || components.iter().any(|component| {
            component.is_empty()
                || *component == "."
                || *component == ".."
                || component.len() > 255
                || component.bytes().any(|byte| byte.is_ascii_control())
        })
    {
        anyhow::bail!("the updater ZIP escapes the canonical CodeCaddie application bundle")
    }
    Ok(components.join("/"))
}

fn validate_macos_zip_inventory(artifact: &Path) -> anyhow::Result<()> {
    validate_macos_zip_inventory_with_limits(artifact, MACOS_ZIP_LIMITS)
}

fn validate_macos_zip_inventory_with_limits(
    artifact: &Path,
    limits: MacosZipLimits,
) -> anyhow::Result<()> {
    let mut file = fs::File::open(artifact)?;
    let central_entry_count = zip32_entry_count(&mut file)?;
    if central_entry_count == 0 || central_entry_count > limits.entries {
        anyhow::bail!("the updater ZIP entry count is outside the allowed bound")
    }
    file.seek(SeekFrom::Start(0))?;
    let mut archive = zip::ZipArchive::new(file)?;
    if archive.len() != central_entry_count {
        anyhow::bail!("the updater ZIP contains duplicate entry names")
    }

    let mut normalized_names = BTreeSet::new();
    let mut required_files = BTreeSet::from([
        "CodeCaddie.app/Contents/Info.plist".to_owned(),
        "CodeCaddie.app/Contents/MacOS/codecaddie".to_owned(),
        "CodeCaddie.app/Contents/MacOS/codecaddie-core".to_owned(),
        "CodeCaddie.app/Contents/MacOS/codecaddie-updater".to_owned(),
    ]);
    let mut total_uncompressed = 0_u64;
    let mut total_compressed = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let is_dir = entry.is_dir();
        let normalized = normalized_macos_zip_entry_name(entry.name_raw(), is_dir)?;
        if !normalized_names.insert(normalized.to_ascii_lowercase()) {
            anyhow::bail!("the updater ZIP contains duplicate normalized paths")
        }
        if entry.encrypted() || entry.is_symlink() {
            anyhow::bail!("the updater ZIP contains an encrypted or symbolic-link entry")
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            let expected_kind = if is_dir { 0o040000 } else { 0o100000 };
            if kind != 0 && kind != expected_kind {
                anyhow::bail!("the updater ZIP contains a special filesystem entry")
            }
        }
        if required_files.contains(&normalized) {
            if is_dir {
                anyhow::bail!("the updater ZIP is missing a required regular file")
            }
            required_files.remove(&normalized);
        }
        if is_dir {
            continue;
        }
        total_uncompressed = total_uncompressed
            .checked_add(entry.size())
            .ok_or_else(|| anyhow::anyhow!("the updater ZIP size overflows its bound"))?;
        total_compressed = total_compressed
            .checked_add(entry.compressed_size())
            .ok_or_else(|| anyhow::anyhow!("the updater ZIP size overflows its bound"))?;
        if total_uncompressed > limits.uncompressed_bytes
            || (entry.size() > limits.ratio_grace_bytes
                && (entry.compressed_size() == 0
                    || entry.size()
                        > entry
                            .compressed_size()
                            .saturating_mul(limits.compression_ratio)))
        {
            anyhow::bail!("the updater ZIP exceeds its decompression bounds")
        }
    }
    if total_uncompressed > limits.ratio_grace_bytes
        && total_uncompressed
            > total_compressed
                .saturating_mul(limits.compression_ratio)
                .saturating_add(limits.ratio_grace_bytes)
    {
        anyhow::bail!("the updater ZIP exceeds its aggregate compression-ratio bound")
    }
    if !required_files.is_empty() {
        anyhow::bail!("the updater ZIP is missing required CodeCaddie application files")
    }
    Ok(())
}

fn install_macos(
    artifact: &Path,
    current_executable: &Path,
    expected_version: &str,
    expected_build: u64,
    expected_commit: &str,
) -> anyhow::Result<()> {
    if artifact.extension().and_then(|value| value.to_str()) != Some("zip") {
        anyhow::bail!("macOS updates must be ZIP payloads")
    }
    validate_macos_zip_inventory(artifact)?;
    let target = current_executable
        .ancestors()
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
        .ok_or_else(|| anyhow::anyhow!("current CodeCaddie application bundle was not found"))?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("application destination has no parent directory"))?;
    let temporary = parent.join(format!(".codecaddie-update-{}", std::process::id()));
    let replacement = parent.join(format!(
        ".codecaddie-replacement-{}.app",
        std::process::id()
    ));
    let backup = parent.join(format!(".codecaddie-previous-{}.app", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    fs::create_dir(&temporary)?;
    let extraction = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(artifact)
        .arg(&temporary)
        .status()?;
    if !extraction.success() {
        anyhow::bail!("macOS updater payload could not be extracted")
    }
    let entries = fs::read_dir(&temporary)?.collect::<Result<Vec<_>, _>>()?;
    if entries.len() != 1 {
        anyhow::bail!("updater ZIP must contain exactly one top-level application bundle")
    }
    let entry = &entries[0];
    let app = entry.path();
    if !entry.file_type()?.is_dir()
        || app.file_name().and_then(|value| value.to_str()) != Some("CodeCaddie.app")
    {
        anyhow::bail!("updater ZIP does not contain the canonical CodeCaddie application bundle")
    }
    verify_macos_application(&app, expected_version, expected_build, expected_commit)?;
    if replacement.exists() {
        fs::remove_dir_all(&replacement)?;
    }
    fs::rename(&app, &replacement)?;
    fs::remove_dir_all(&temporary)?;
    let failed = parent.join(format!(".codecaddie-failed-{}.app", std::process::id()));
    replace_application_transaction(
        target,
        &replacement,
        &backup,
        &failed,
        |installed| {
            let core = installed.join("Contents/MacOS/codecaddie-core");
            let health = Command::new(&core).arg("--health-check").output()?;
            if health_identity_matches(
                health.status.success(),
                &health.stdout,
                expected_version,
                expected_build,
                expected_commit,
            ) {
                Ok(())
            } else {
                anyhow::bail!("the installed update did not report the expected release identity")
            }
        },
        |installed| {
            let status = Command::new("/usr/bin/open").arg(installed).status()?;
            if status.success() {
                Ok(())
            } else {
                anyhow::bail!("macOS could not launch CodeCaddie")
            }
        },
    )
}

fn replace_application_transaction<Health, Activate>(
    target: &Path,
    replacement: &Path,
    backup: &Path,
    failed: &Path,
    health: Health,
    mut activate: Activate,
) -> anyhow::Result<()>
where
    Health: FnOnce(&Path) -> anyhow::Result<()>,
    Activate: FnMut(&Path) -> anyhow::Result<()>,
{
    if backup.exists() || failed.exists() {
        anyhow::bail!("a previous update transaction is still present")
    }
    fs::rename(target, backup)?;
    if let Err(error) = fs::rename(replacement, target) {
        if fs::rename(backup, target).is_err() {
            return Err(InstallStateUnverified {
                code: codecaddie_core::update::UpdaterResultCode::ManualRepairRequired,
                message: "the replacement failed and the prior application could not be restored",
            }
            .into());
        }
        return Err(error.into());
    }
    if let Err(error) = health(target).and_then(|()| activate(target)) {
        if fs::rename(target, failed).is_err() {
            return Err(InstallStateUnverified {
                code: codecaddie_core::update::UpdaterResultCode::ManualRepairRequired,
                message: "the failed candidate could not be quarantined before rollback",
            }
            .into());
        }
        if fs::rename(backup, target).is_err() {
            return Err(InstallStateUnverified {
                code: codecaddie_core::update::UpdaterResultCode::ManualRepairRequired,
                message: "the prior application could not be restored after the failed candidate was quarantined",
            }
            .into());
        }
        let _ = fs::remove_dir_all(failed);
        // The top-level failure path records its fixed-code result before it
        // reopens this restored application. Keeping recovery in one place
        // prevents a startup race with the one-shot result mailbox.
        anyhow::bail!("the installed update was rolled back: {error}")
    }
    // The prior version stays recoverable through health verification and
    // successful relaunch. Once launch succeeds, cleanup cannot safely turn
    // the completed update into a failure: the new app may already have
    // consumed the one-shot mailbox. A future manual cleanup may remove a
    // retained hidden backup if the filesystem refuses this best effort.
    let _ = fs::remove_dir_all(backup);
    Ok(())
}

fn verify_macos_application(
    app: &Path,
    expected_version: &str,
    expected_build: u64,
    expected_commit: &str,
) -> anyhow::Result<()> {
    let codesign = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(app)
        .status()?;
    if !codesign.success() {
        anyhow::bail!("the update has an invalid Apple code signature")
    }
    let gatekeeper = Command::new("/usr/sbin/spctl")
        .args(["-a", "-vv", "--type", "execute"])
        .arg(app)
        .status()?;
    if !gatekeeper.success() {
        anyhow::bail!("Gatekeeper rejected the update")
    }
    let expected_team = option_env!("CODECADDIE_APPLE_TEAM_ID")
        .ok_or_else(|| anyhow::anyhow!("this build has no pinned Apple Team ID"))?;
    let details = Command::new("/usr/bin/codesign")
        .args(["-d", "--verbose=4"])
        .arg(app)
        .output()?;
    if !details.status.success() {
        anyhow::bail!("the update code-signing identity could not be inspected")
    }
    let output = String::from_utf8_lossy(&details.stderr);
    if !macos_code_identity_matches(&output, expected_team) {
        anyhow::bail!(
            "the update publisher or bundle identifier does not match CodeCaddie's pinned identity"
        )
    }
    verify_macos_bundle_metadata(app, expected_version, expected_build)?;
    for executable in ["codecaddie", "codecaddie-core", "codecaddie-updater"] {
        let path = app.join("Contents/MacOS").join(executable);
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            anyhow::bail!("the update application executable inventory is incomplete")
        }
    }
    let binary = app.join("Contents/MacOS/codecaddie");
    let architectures = Command::new("/usr/bin/lipo")
        .arg("-archs")
        .arg(binary)
        .output()?;
    let expected_architecture = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    if !architectures.status.success()
        || !String::from_utf8_lossy(&architectures.stdout)
            .split_whitespace()
            .any(|value| value == expected_architecture)
    {
        anyhow::bail!("the update does not contain the running Mac architecture")
    }
    let core = app.join("Contents/MacOS/codecaddie-core");
    let health = Command::new(core).arg("--health-check").output()?;
    if !health_identity_matches(
        health.status.success(),
        &health.stdout,
        expected_version,
        expected_build,
        expected_commit,
    ) {
        anyhow::bail!("the extracted update does not match the signed release identity")
    }
    Ok(())
}

fn verify_macos_bundle_metadata(
    app: &Path,
    expected_version: &str,
    expected_build: u64,
) -> anyhow::Result<()> {
    let plist = plist::Value::from_file(app.join("Contents/Info.plist"))?;
    let dictionary = plist
        .as_dictionary()
        .ok_or_else(|| anyhow::anyhow!("the update application Info.plist is invalid"))?;
    let value = |key: &str| dictionary.get(key).and_then(plist::Value::as_string);
    let expected_short_version = expected_version
        .split('-')
        .next()
        .unwrap_or(expected_version);
    let expected_build = expected_build.to_string();
    if value("CFBundleIdentifier") != Some("org.codecaddie.desktop")
        || value("CFBundleExecutable") != Some("codecaddie")
        || value("CFBundlePackageType") != Some("APPL")
        || value("CodeCaddieChannel") != Some("stable")
        || value("CFBundleShortVersionString") != Some(expected_short_version)
        || value("CFBundleVersion") != Some(expected_build.as_str())
    {
        anyhow::bail!("the update application metadata does not match the signed release identity")
    }
    Ok(())
}

fn macos_code_identity_matches(output: &str, expected_team: &str) -> bool {
    let expected_team = format!("TeamIdentifier={expected_team}");
    let teams: Vec<_> = output
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("TeamIdentifier="))
        .collect();
    let identifiers: Vec<_> = output
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("Identifier="))
        .collect();
    teams == [expected_team.as_str()] && identifiers == ["Identifier=org.codecaddie.desktop"]
}

fn install_windows(
    artifact: &Path,
    current_executable: &Path,
    expected_version: &str,
    expected_build: u64,
    expected_commit: &str,
) -> anyhow::Result<()> {
    if artifact.extension().and_then(|value| value.to_str()) != Some("msi") {
        anyhow::bail!("Windows updates must be MSI packages")
    }
    let publisher = option_env!("CODECADDIE_WINDOWS_PUBLISHER")
        .ok_or_else(|| anyhow::anyhow!("this build has no pinned Windows publisher"))?;
    let escaped = artifact.to_string_lossy().replace('\'', "''");
    let escaped_publisher = publisher.replace('\'', "''");
    let script = format!(
        "$s = Get-AuthenticodeSignature -LiteralPath '{escaped}'; \
         if ($s.Status -ne 'Valid') {{ exit 20 }}; \
         if ($s.SignerCertificate.Subject -ne '{escaped_publisher}') {{ exit 21 }}"
    );
    let signature = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()?;
    if !signature.success() {
        anyhow::bail!("the MSI signature or publisher is invalid")
    }
    // A failure to start Windows Installer leaves the existing installation
    // untouched, so the ordinary recovery path may safely reopen it. Once the
    // installer process exists, however, a wait failure makes the resulting
    // install state unknowable and must fail closed without reopening.
    let mut installer = Command::new("msiexec.exe")
        .arg("/i")
        .arg(artifact)
        .args(["/passive", "/norestart"])
        .spawn()?;
    let status = installer.wait().map_err(|_| InstallStateUnverified {
        code: codecaddie_core::update::UpdaterResultCode::RestartRequired,
        message: "Windows Installer started, but CodeCaddie could not verify its result",
    })?;
    // ERROR_SUCCESS_REBOOT_REQUIRED (3010) is not an immediately runnable
    // install: Windows documents that its changes are not effective until a
    // reboot. Fail closed into the fixed recovery path until a dedicated
    // reboot-required UX exists instead of claiming an automatic restart.
    if !windows_installer_completed_without_reboot(status.code()) {
        return Err(InstallStateUnverified {
            code: codecaddie_core::update::UpdaterResultCode::RestartRequired,
            message: "Windows Installer did not complete an immediately runnable update",
        }
        .into());
    }
    let health = Command::new(current_executable)
        .arg("--health-check")
        .output()
        .map_err(|_| {
            InstallStateUnverified {
                code: codecaddie_core::update::UpdaterResultCode::RestartRequired,
                message: "the installed CodeCaddie core could not be checked after Windows Installer completed",
            }
        })?;
    if !health_identity_matches(
        health.status.success(),
        &health.stdout,
        expected_version,
        expected_build,
        expected_commit,
    ) {
        return Err(InstallStateUnverified {
            code: codecaddie_core::update::UpdaterResultCode::RestartRequired,
            message: "the installed CodeCaddie core did not report the expected release identity",
        }
        .into());
    }
    let application = current_executable.with_file_name("codecaddie.exe");
    if !application.is_file() {
        return Err(InstallStateUnverified {
            code: codecaddie_core::update::UpdaterResultCode::RestartRequired,
            message: "the updated CodeCaddie application is missing",
        }
        .into());
    }
    Command::new(application).spawn()?;
    Ok(())
}

fn windows_installer_completed_without_reboot(code: Option<i32>) -> bool {
    matches!(code, Some(0))
}

fn health_identity_matches(
    succeeded: bool,
    stdout: &[u8],
    expected_version: &str,
    expected_build: u64,
    expected_commit: &str,
) -> bool {
    if !succeeded {
        return false;
    }
    let text = String::from_utf8_lossy(stdout);
    let expected_prefix = format!("CodeCaddie {expected_version}+{expected_build} ");
    text.trim().strip_prefix(&expected_prefix) == Some(expected_commit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_core::{
        local_state::{
            ApproveGoalRequest, CreateWorkspaceRequest, LocalWorkspaceStore, ProjectContext,
        },
        repository::LocalRepository,
    };
    use codecaddie_domain::{
        CriterionAssessment, EvidenceKind, FrozenRepository, GoalAssessment, Report, ReportOrigin,
        Verdict,
    };
    use std::process::Command;
    use time::OffsetDateTime;

    struct DataRootOverride {
        previous: Option<std::ffi::OsString>,
    }

    impl DataRootOverride {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os("CODECADDIE_DATA_DIR");
            // This is the only updater test that mutates the process
            // environment; the focused transaction tests use explicit paths.
            unsafe { std::env::set_var("CODECADDIE_DATA_DIR", path) };
            Self { previous }
        }
    }

    impl Drop for DataRootOverride {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                unsafe { std::env::set_var("CODECADDIE_DATA_DIR", previous) };
            } else {
                unsafe { std::env::remove_var("CODECADDIE_DATA_DIR") };
            }
        }
    }

    fn git(path: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn assert_private_state_is_encrypted(path: &Path, sentinel: &[u8]) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                assert_private_state_is_encrypted(&path, sentinel);
            } else {
                let bytes = fs::read(path).unwrap();
                assert!(
                    !bytes
                        .windows(sentinel.len())
                        .any(|window| window == sentinel),
                    "managed local state exposed the upgrade sentinel"
                );
            }
        }
    }

    fn marker_application(path: &Path, version: &str) {
        fs::create_dir(path).unwrap();
        fs::write(path.join("version.txt"), version).unwrap();
    }

    #[test]
    fn failed_health_check_restores_previous_application_for_central_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("CodeCaddie.app");
        let replacement = directory.path().join("replacement.app");
        let backup = directory.path().join("backup.app");
        let failed = directory.path().join("failed.app");
        marker_application(&target, "prior");
        marker_application(&replacement, "candidate");

        let mut activated_versions = Vec::new();
        let error = replace_application_transaction(
            &target,
            &replacement,
            &backup,
            &failed,
            |_| anyhow::bail!("simulated health failure"),
            |installed| {
                activated_versions.push(fs::read_to_string(installed.join("version.txt")).unwrap());
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("rolled back"));
        assert!(activated_versions.is_empty());
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            "prior"
        );
        assert!(!backup.exists());
        assert!(!failed.exists());
    }

    #[test]
    fn failed_candidate_launch_restores_previous_application_for_central_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("CodeCaddie.app");
        let replacement = directory.path().join("replacement.app");
        let backup = directory.path().join("backup.app");
        let failed = directory.path().join("failed.app");
        marker_application(&target, "prior");
        marker_application(&replacement, "candidate");

        let mut activated_versions = Vec::new();
        let error = replace_application_transaction(
            &target,
            &replacement,
            &backup,
            &failed,
            |_| Ok(()),
            |installed| {
                let version = fs::read_to_string(installed.join("version.txt")).unwrap();
                activated_versions.push(version);
                if activated_versions.len() == 1 {
                    anyhow::bail!("simulated launch failure")
                }
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("rolled back"));
        assert_eq!(activated_versions, ["candidate"]);
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            "prior"
        );
        assert!(!backup.exists());
        assert!(!failed.exists());
    }

    #[test]
    fn failed_candidate_is_never_activated_when_quarantine_cannot_be_confirmed() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("CodeCaddie.app");
        let replacement = directory.path().join("replacement.app");
        let backup = directory.path().join("backup.app");
        let failed = directory.path().join("failed.app");
        marker_application(&target, "prior");
        marker_application(&replacement, "candidate");

        let mut activated = false;
        let error = replace_application_transaction(
            &target,
            &replacement,
            &backup,
            &failed,
            |_| {
                // A non-empty destination appearing after preflight makes the
                // candidate quarantine rename fail deterministically.
                marker_application(&failed, "occupied");
                anyhow::bail!("simulated candidate health failure")
            },
            |_| {
                activated = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(!activated);
        assert_eq!(
            recovery_policy(&error),
            (
                codecaddie_core::update::UpdaterResultCode::ManualRepairRequired,
                false
            )
        );
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            "candidate"
        );
        assert_eq!(
            fs::read_to_string(backup.join("version.txt")).unwrap(),
            "prior"
        );
    }

    #[test]
    fn cleanup_failure_after_successful_launch_does_not_relabel_the_update_failed() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("CodeCaddie.app");
        let replacement = directory.path().join("replacement.app");
        let backup = directory.path().join("backup.app");
        let failed = directory.path().join("failed.app");
        marker_application(&target, "prior");
        marker_application(&replacement, "candidate");

        replace_application_transaction(
            &target,
            &replacement,
            &backup,
            &failed,
            |_| Ok(()),
            |_| {
                // Make the best-effort directory cleanup fail after the
                // candidate has already been accepted and launched.
                fs::remove_dir_all(&backup)?;
                fs::write(&backup, b"retained cleanup marker")?;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            "candidate"
        );
        assert!(backup.is_file());
        assert!(!failed.exists());
    }

    #[test]
    fn recovery_records_before_reopening_and_replaces_the_code_if_reopen_fails() {
        use std::cell::RefCell;

        let events = RefCell::new(Vec::new());
        recover_failed_update_with(
            codecaddie_core::update::UpdaterResultCode::InstallFailed,
            true,
            |code| {
                events.borrow_mut().push(format!("record:{code:?}"));
                Ok(())
            },
            || {
                events.borrow_mut().push("reopen".into());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(events.into_inner(), ["record:InstallFailed", "reopen"]);

        let events = RefCell::new(Vec::new());
        let error = recover_failed_update_with(
            codecaddie_core::update::UpdaterResultCode::InstallFailed,
            true,
            |code| {
                events.borrow_mut().push(format!("record:{code:?}"));
                Ok(())
            },
            || {
                events.borrow_mut().push("reopen".into());
                anyhow::bail!("simulated recovery launch failure")
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("simulated recovery launch failure")
        );
        assert_eq!(
            events.into_inner(),
            ["record:InstallFailed", "reopen", "record:ReopenFailed"]
        );

        let mut record_failed = false;
        let mut reopen_attempted = false;
        let mut reopen_failed = false;
        let error = recover_failed_update_with(
            codecaddie_core::update::UpdaterResultCode::InstallFailed,
            true,
            |_| {
                record_failed = true;
                anyhow::bail!("simulated mailbox write failure")
            },
            || {
                reopen_attempted = true;
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("mailbox write failure"));
        assert_eq!(
            recovery_notice_code(record_failed, reopen_attempted, reopen_failed),
            Some(codecaddie_core::update::UpdaterResultCode::ResultUnreadable)
        );
        reopen_failed = true;
        assert_eq!(
            recovery_notice_code(record_failed, reopen_attempted, reopen_failed),
            Some(codecaddie_core::update::UpdaterResultCode::ReopenFailed)
        );
    }

    #[test]
    fn windows_installer_requires_completion_without_a_reboot() {
        assert!(windows_installer_completed_without_reboot(Some(0)));
        assert!(!windows_installer_completed_without_reboot(Some(3010)));
        assert!(!windows_installer_completed_without_reboot(Some(1603)));
        assert!(!windows_installer_completed_without_reboot(None));
    }

    #[test]
    fn post_installer_uncertainty_never_reopens_the_application() {
        let uncertain = anyhow::Error::new(InstallStateUnverified {
            code: codecaddie_core::update::UpdaterResultCode::RestartRequired,
            message: "fixed test reason",
        });
        assert_eq!(
            recovery_policy(&uncertain),
            (
                codecaddie_core::update::UpdaterResultCode::RestartRequired,
                false
            )
        );

        let ordinary = anyhow::anyhow!("fixed test reason");
        assert_eq!(
            recovery_policy(&ordinary),
            (
                codecaddie_core::update::UpdaterResultCode::InstallFailed,
                true
            )
        );
    }

    #[test]
    fn fixed_failure_notices_are_platform_specific_and_content_free() {
        let windows = fixed_failure_notice(
            "windows",
            codecaddie_core::update::UpdaterResultCode::RestartRequired,
        )
        .unwrap();
        assert!(windows.message.contains("Restart Windows"));
        assert!(windows.message.contains("codecaddie.ai"));

        let macos = fixed_failure_notice(
            "macos",
            codecaddie_core::update::UpdaterResultCode::ReopenFailed,
        )
        .unwrap();
        assert!(macos.message.contains("Applications"));
        assert!(macos.message.contains("codecaddie.ai"));

        let manual = fixed_failure_notice(
            "macos",
            codecaddie_core::update::UpdaterResultCode::ManualRepairRequired,
        )
        .unwrap();
        assert!(manual.message.contains("Do not open the current copy"));
        let unreadable = fixed_failure_notice(
            "macos",
            codecaddie_core::update::UpdaterResultCode::ResultUnreadable,
        )
        .unwrap();
        assert!(unreadable.message.contains("Check the installed version"));
        for notice in [windows, macos, manual, unreadable] {
            assert_eq!(notice.title, "CodeCaddie update needs attention");
            assert!(!notice.message.contains("/Users/"));
            assert!(!notice.message.contains("PRIVATE SOURCE CANARY"));
            assert!(!notice.message.contains("SECRET CANARY"));
        }
        assert!(
            fixed_failure_notice(
                "macos",
                codecaddie_core::update::UpdaterResultCode::InstallFailed,
            )
            .is_none()
        );
        assert!(
            fixed_failure_notice(
                "linux",
                codecaddie_core::update::UpdaterResultCode::ReopenFailed,
            )
            .is_none()
        );
    }

    #[test]
    fn installed_health_output_must_match_the_staged_version_build_and_commit() {
        let expected_commit = "1111111111111111111111111111111111111111";
        assert!(health_identity_matches(
            true,
            b"CodeCaddie 0.3.0+1234 1111111111111111111111111111111111111111\n",
            "0.3.0",
            1234,
            expected_commit,
        ));
        assert!(!health_identity_matches(
            true,
            b"CodeCaddie 0.3.0+1233 1111111111111111111111111111111111111111\n",
            "0.3.0",
            1234,
            expected_commit,
        ));
        assert!(!health_identity_matches(
            false,
            b"CodeCaddie 0.3.0+1234 1111111111111111111111111111111111111111\n",
            "0.3.0",
            1234,
            expected_commit,
        ));
        assert!(!health_identity_matches(
            true,
            b"CodeCaddie 0.3.0+1234 2222222222222222222222222222222222222222\n",
            "0.3.0",
            1234,
            expected_commit,
        ));
    }

    #[test]
    fn macos_code_identity_requires_the_exact_team_and_stable_bundle_identifier() {
        let stable = "Executable=/Applications/CodeCaddie.app/Contents/MacOS/codecaddie\n\
                      Identifier=org.codecaddie.desktop\n\
                      TeamIdentifier=EXAMPLETM1\n";
        assert!(macos_code_identity_matches(stable, "EXAMPLETM1"));
        assert!(!macos_code_identity_matches(stable, "OTHERTEAM"));
        assert!(!macos_code_identity_matches(
            &stable.replace(
                "Identifier=org.codecaddie.desktop",
                "Identifier=org.codecaddie.desktop.dev"
            ),
            "EXAMPLETM1"
        ));
        assert!(!macos_code_identity_matches(
            "Identifier=org.codecaddie.desktop.extra\nTeamIdentifier=EXAMPLETM1\n",
            "EXAMPLETM1"
        ));
        assert!(!macos_code_identity_matches(
            &format!("{stable}TeamIdentifier=EXAMPLETM1\n"),
            "EXAMPLETM1"
        ));
        assert!(!macos_code_identity_matches(
            &format!("{stable}Identifier=org.codecaddie.desktop\n"),
            "EXAMPLETM1"
        ));
    }

    fn write_test_info_plist(app: &Path, version: &str, build: &str, channel: &str) {
        let contents = app.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        let mut dictionary = plist::Dictionary::new();
        for (key, value) in [
            ("CFBundleIdentifier", "org.codecaddie.desktop"),
            ("CFBundleExecutable", "codecaddie"),
            ("CFBundlePackageType", "APPL"),
            ("CodeCaddieChannel", channel),
            ("CFBundleShortVersionString", version),
            ("CFBundleVersion", build),
        ] {
            dictionary.insert(key.into(), plist::Value::String(value.into()));
        }
        plist::Value::Dictionary(dictionary)
            .to_file_xml(contents.join("Info.plist"))
            .unwrap();
    }

    #[test]
    fn macos_bundle_metadata_must_match_the_signed_version_and_build() {
        let directory = tempfile::tempdir().unwrap();
        let app = directory.path().join("CodeCaddie.app");
        write_test_info_plist(&app, "0.4.0", "2001", "stable");
        verify_macos_bundle_metadata(&app, "0.4.0", 2001).unwrap();

        write_test_info_plist(&app, "0.4.0", "2002", "stable");
        assert!(verify_macos_bundle_metadata(&app, "0.4.0", 2001).is_err());
        write_test_info_plist(&app, "0.4.1", "2001", "stable");
        assert!(verify_macos_bundle_metadata(&app, "0.4.0", 2001).is_err());
        write_test_info_plist(&app, "0.4.0", "2001", "dev");
        assert!(verify_macos_bundle_metadata(&app, "0.4.0", 2001).is_err());
    }

    enum TestZipEntry {
        File(&'static str, &'static [u8]),
        Directory(&'static str),
        Symlink(&'static str, &'static str),
    }

    fn write_test_update_zip(path: &Path, extras: &[TestZipEntry]) {
        use std::io::Write as _;
        use zip::write::SimpleFileOptions;

        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let regular = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let executable = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);
        for (name, bytes, options) in [
            (
                "CodeCaddie.app/Contents/Info.plist",
                b"plist".as_slice(),
                regular,
            ),
            (
                "CodeCaddie.app/Contents/MacOS/codecaddie",
                b"app".as_slice(),
                executable,
            ),
            (
                "CodeCaddie.app/Contents/MacOS/codecaddie-core",
                b"core".as_slice(),
                executable,
            ),
            (
                "CodeCaddie.app/Contents/MacOS/codecaddie-updater",
                b"updater".as_slice(),
                executable,
            ),
        ] {
            writer.start_file(name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        for extra in extras {
            match extra {
                TestZipEntry::File(name, bytes) => {
                    writer.start_file(*name, regular).unwrap();
                    writer.write_all(bytes).unwrap();
                }
                TestZipEntry::Directory(name) => {
                    writer.add_directory(*name, regular).unwrap();
                }
                TestZipEntry::Symlink(name, target) => {
                    writer.add_symlink(*name, *target, regular).unwrap();
                }
            }
        }
        writer.finish().unwrap();
    }

    #[test]
    fn macos_zip_inventory_accepts_only_the_canonical_application_tree() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("valid.zip");
        write_test_update_zip(
            &archive,
            &[TestZipEntry::Directory(
                "CodeCaddie.app/Contents/Resources/",
            )],
        );
        validate_macos_zip_inventory(&archive).unwrap();
    }

    #[test]
    fn macos_zip_inventory_rejects_unsafe_paths_before_extraction() {
        for (label, path) in [
            ("traversal", "CodeCaddie.app/Contents/../escape"),
            ("absolute", "/CodeCaddie.app/Contents/escape"),
            ("backslash", "CodeCaddie.app\\Contents\\escape"),
            ("drive", "C:/CodeCaddie.app/Contents/escape"),
            ("top-level", "Other.app/Contents/escape"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let archive = directory.path().join(format!("{label}.zip"));
            write_test_update_zip(&archive, &[TestZipEntry::File(path, b"evil")]);
            assert!(
                validate_macos_zip_inventory(&archive).is_err(),
                "{label} path unexpectedly passed inventory validation"
            );
        }
    }

    #[test]
    fn macos_zip_inventory_rejects_symlinks_and_duplicate_names() {
        let directory = tempfile::tempdir().unwrap();
        let symlink = directory.path().join("symlink.zip");
        write_test_update_zip(
            &symlink,
            &[TestZipEntry::Symlink(
                "CodeCaddie.app/Contents/Resources/link",
                "../../../../outside",
            )],
        );
        assert!(validate_macos_zip_inventory(&symlink).is_err());

        let duplicate = directory.path().join("duplicate.zip");
        write_test_update_zip(
            &duplicate,
            &[TestZipEntry::Directory(
                "CodeCaddie.app/Contents/Info.plist/",
            )],
        );
        assert!(validate_macos_zip_inventory(&duplicate).is_err());
    }

    #[test]
    fn macos_zip_inventory_enforces_entry_and_uncompressed_size_bounds() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("bounded.zip");
        write_test_update_zip(&archive, &[]);

        assert!(
            validate_macos_zip_inventory_with_limits(
                &archive,
                MacosZipLimits {
                    entries: 3,
                    ..MACOS_ZIP_LIMITS
                },
            )
            .is_err()
        );
        assert!(
            validate_macos_zip_inventory_with_limits(
                &archive,
                MacosZipLimits {
                    uncompressed_bytes: 3,
                    ..MACOS_ZIP_LIMITS
                },
            )
            .is_err()
        );
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct SupportedUpgradeMatrix {
        schema_version: u16,
        current_version: String,
        version_identity: String,
        support_scope: String,
        first_public_baseline: FirstPublicBaseline,
        supported_prior_builds: Vec<SupportedPriorBuild>,
        required_journeys: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FirstPublicBaseline {
        status: String,
        version: String,
        build: u64,
        reason: String,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct SupportedPriorBuild {
        version: String,
        build: u64,
        source_commit: String,
        local_state_format: String,
    }

    #[test]
    fn supported_prior_version_upgrade_and_rollback_matrix_preserves_real_encrypted_workspace_state()
     {
        let matrix: SupportedUpgradeMatrix = serde_json::from_str(include_str!(
            "../../../../config/supported-upgrade-matrix.json"
        ))
        .unwrap();
        assert_eq!(matrix.schema_version, 2);
        assert!(semver::Version::parse(&matrix.current_version).is_ok());
        assert_eq!(matrix.version_identity, "semantic-version-plus-build");
        assert_eq!(
            matrix.support_scope,
            "Only public semantic-version-plus-build identities listed below are supported prior versions; private development builds and unlisted public builds are outside the supported set."
        );
        assert_eq!(matrix.first_public_baseline.version, matrix.current_version);
        assert!(matrix.first_public_baseline.build > 0);
        assert!(!matrix.first_public_baseline.reason.trim().is_empty());
        match matrix.first_public_baseline.status.as_str() {
            "pending" => assert!(matrix.supported_prior_builds.is_empty()),
            "established" => {
                assert!(!matrix.supported_prior_builds.is_empty());
                assert_eq!(matrix.current_version, env!("CARGO_PKG_VERSION"));
            }
            other => panic!("unexpected first-public-baseline status {other}"),
        }
        assert_eq!(
            matrix.required_journeys,
            [
                "failed_upgrade_rolls_back",
                "healthy_upgrade_preserves_state",
                "restart_reopens_encrypted_state",
                "failed_install_transaction_relaunches_prior_version",
                "immutable_evidence_still_resolves",
                "source_privacy_canary_remains_absent",
            ]
        );
        let mut builds = std::collections::BTreeSet::new();
        let mut commits = std::collections::BTreeSet::new();
        for prior_build in &matrix.supported_prior_builds {
            assert!(builds.insert(prior_build.build));
            assert!(commits.insert(prior_build.source_commit.as_str()));
            assert!(semver::Version::parse(&prior_build.version).is_ok());
            assert_eq!(prior_build.source_commit.len(), 40);
            assert!(
                prior_build
                    .source_commit
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            );
            assert_eq!(prior_build.local_state_format, "codecaddie-local-state-v3");
            run_supported_prior_build(&matrix.current_version, prior_build);
        }
    }

    fn run_supported_prior_build(current_version: &str, prior_build: &SupportedPriorBuild) {
        let prior_identity = format!("{}+{}", prior_build.version, prior_build.build);
        let current_identity = format!("{current_version}+candidate");
        let sentinel = format!("PRIVATE UPGRADE STATE SENTINEL {prior_identity}");
        let directory = tempfile::tempdir().unwrap();
        let repository_path = directory.path().join("repository");
        fs::create_dir(&repository_path).unwrap();
        git(&repository_path, &["init", "-q"]);
        git(
            &repository_path,
            &["config", "user.email", "test@example.com"],
        );
        git(
            &repository_path,
            &["config", "user.name", "CodeCaddie Test"],
        );
        fs::write(
            repository_path.join("evidence.txt"),
            "evidence from the frozen release\n",
        )
        .unwrap();
        git(&repository_path, &["add", "evidence.txt"]);
        git(&repository_path, &["commit", "-qm", "frozen evidence"]);
        let commit = git(&repository_path, &["rev-parse", "HEAD"]);

        let data_root = directory.path().join("data");
        let _data_root_override = DataRootOverride::set(&data_root);
        let store = LocalWorkspaceStore::from_environment().unwrap();
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Upgrade fixture".into(),
                repository_display_name: "repository".into(),
                repository_path: repository_path.to_string_lossy().into_owned(),
                product_brief: "Prove that real saved decisions survive upgrades.".into(),
                context: ProjectContext::default(),
            })
            .unwrap();
        store.set_provider_preference("codex").unwrap();
        let goal = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "upgrade-preservation".into(),
                    title: sentinel.clone(),
                    business_outcome: "Saved decisions remain available across versions.".into(),
                    criteria: vec![
                        "A real encrypted workspace, report, history, configuration, and immutable evidence survive upgrade and rollback transactions."
                            .into(),
                    ],
                    priority: 5,
                    position: 1,
                    rubric_dimensions: vec!["Reliability".into()],
                },
            )
            .unwrap();
        let repository = LocalRepository::attach("attached-repository", &repository_path).unwrap();
        let evidence = repository
            .evidence(&commit, "evidence.txt", 1, 1, EvidenceKind::Test)
            .unwrap();
        let report = Report {
            id: "cross-version-report".into(),
            completed_at: OffsetDateTime::UNIX_EPOCH,
            repositories: vec![FrozenRepository {
                repository_id: "attached-repository".into(),
                commit_sha: commit.clone(),
            }],
            goal_version_ids: vec![goal.id.clone()],
            goal_set_hash: blake3::hash(&serde_json::to_vec(&vec![goal.clone()]).unwrap())
                .to_hex()
                .to_string(),
            provider: "test".into(),
            provider_version: "v1-fixture".into(),
            origin: ReportOrigin::Scan,
            assessments: vec![GoalAssessment {
                goal_version_id: goal.id.clone(),
                verdict: Verdict::Supported,
                summary: "The fixture binds a report to immutable evidence.".into(),
                architecture_narrative: String::new(),
                related_component_ids: vec![],
                criteria: vec![CriterionAssessment {
                    criterion_id: goal.criteria[0].id.clone(),
                    verdict: Verdict::Supported,
                    rationale: "The saved coordinate resolves at the frozen commit.".into(),
                    confidence: 1.0,
                    evidence: vec![evidence.clone()],
                }],
            }],
            architecture: vec![],
            recommendations: vec![],
            coverage: Some(1.0),
            unverified_criteria: 0,
            partial: false,
            analysis_warnings: vec![],
            codebase_map_id: None,
            codebase_map_hash: None,
        };
        store
            .record_report(&workspace.workspace_id, report)
            .unwrap();
        drop(store);

        let assert_workspace = || {
            let reopened = LocalWorkspaceStore::from_environment().unwrap();
            assert_eq!(
                reopened.provider_preference().unwrap().as_deref(),
                Some("codex")
            );
            let recent = reopened.recent_workspace().unwrap().unwrap();
            assert_eq!(recent.workspace_id, workspace.workspace_id);
            assert_eq!(recent.approved_goals[0].title, sentinel);
            assert_eq!(recent.report_heatmap.len(), 1);
            let saved = recent.latest_report.unwrap();
            assert_eq!(saved.id, "cross-version-report");
            assert_eq!(saved.repositories[0].commit_sha, commit);
            assert_eq!(saved.assessments[0].criteria[0].evidence[0], evidence);
            repository
                .verify_evidence(&saved.assessments[0].criteria[0].evidence[0])
                .unwrap();
            assert_private_state_is_encrypted(&data_root, sentinel.as_bytes());
        };

        let applications = directory.path().join("Applications");
        fs::create_dir(&applications).unwrap();
        let target = applications.join("CodeCaddie.app");
        let marker = |path: &Path, version: &str| {
            fs::create_dir(path).unwrap();
            fs::write(path.join("version.txt"), version).unwrap();
        };
        marker(&target, &prior_identity);

        // A paused promotion performs no replacement. Model the next launch by
        // reopening the real encrypted store from a fresh handle while the
        // current installed application remains byte-for-byte on v1.
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            prior_identity
        );
        assert_workspace();

        let failed_v2 = applications.join("replacement-v2-bad.app");
        marker(&failed_v2, &format!("{current_identity}-bad"));
        let mut reopened_after_failed_upgrade = None;
        let error = replace_application_transaction(
            &target,
            &failed_v2,
            &applications.join("backup-v1-failed.app"),
            &applications.join("failed-v2.app"),
            |_| anyhow::bail!("simulated v2 health failure"),
            |installed| {
                reopened_after_failed_upgrade =
                    Some(fs::read_to_string(installed.join("version.txt")).unwrap());
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("rolled back"));
        recover_failed_update_with(
            codecaddie_core::update::UpdaterResultCode::InstallFailed,
            true,
            |_| Ok(()),
            || {
                reopened_after_failed_upgrade =
                    Some(fs::read_to_string(target.join("version.txt")).unwrap());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            reopened_after_failed_upgrade.as_deref(),
            Some(prior_identity.as_str())
        );
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            prior_identity
        );
        assert_workspace();

        let healthy_v2 = applications.join("replacement-v2.app");
        marker(&healthy_v2, &current_identity);
        replace_application_transaction(
            &target,
            &healthy_v2,
            &applications.join("backup-v1-success.app"),
            &applications.join("failed-v2-success.app"),
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            current_identity
        );
        assert_workspace();

        let rollback_v1 = applications.join("replacement-v1-rollback.app");
        marker(&rollback_v1, &prior_identity);
        replace_application_transaction(
            &target,
            &rollback_v1,
            &applications.join("backup-v2-rollback.app"),
            &applications.join("failed-v1-rollback.app"),
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            prior_identity
        );
        assert_workspace();
    }
}
