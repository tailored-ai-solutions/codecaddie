use crate::runtime_channel::RuntimeChannel;
use std::{path::PathBuf, process::Command};

pub fn is_enabled() -> anyhow::Result<bool> {
    let (name, _) = identity()?;
    if cfg!(target_os = "macos") {
        let script =
            "tell application \"System Events\" to return exists login item named (item 1 of argv)";
        let output = Command::new("/usr/bin/osascript")
            .args(["-e", "on run argv", "-e", script, "-e", "end run", &name])
            .output()?;
        if !output.status.success() {
            anyhow::bail!("macOS could not read the CodeCaddie login item")
        }
        return Ok(String::from_utf8_lossy(&output.stdout).trim() == "true");
    }
    if cfg!(target_os = "windows") {
        return Ok(Command::new("reg.exe")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                &name,
            ])
            .status()?
            .success());
    }
    anyhow::bail!("Launch at Login is supported only on macOS and Windows")
}

pub fn set_enabled(enabled: bool) -> anyhow::Result<bool> {
    let (name, executable) = identity()?;
    if cfg!(target_os = "macos") {
        let script = if enabled {
            "tell application \"System Events\" to if not (exists login item named (item 1 of argv)) then make login item at end with properties {name:(item 1 of argv), path:(item 2 of argv), hidden:false}"
        } else {
            "tell application \"System Events\" to if exists login item named (item 1 of argv) then delete login item named (item 1 of argv)"
        };
        let status = Command::new("/usr/bin/osascript")
            .args(["-e", "on run argv", "-e", script, "-e", "end run", &name])
            .arg(&executable)
            .status()?;
        if !status.success() {
            anyhow::bail!("macOS could not change the CodeCaddie login item")
        }
        return Ok(enabled);
    }
    if cfg!(target_os = "windows") {
        let status = if enabled {
            Command::new("reg.exe")
                .args([
                    "add",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    &name,
                    "/t",
                    "REG_SZ",
                    "/d",
                ])
                .arg(format!("\"{}\"", executable.display()))
                .args(["/f"])
                .status()?
        } else {
            Command::new("reg.exe")
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    &name,
                    "/f",
                ])
                .status()?
        };
        if !status.success() {
            anyhow::bail!("Windows could not change the CodeCaddie startup entry")
        }
        return Ok(enabled);
    }
    anyhow::bail!("Launch at Login is supported only on macOS and Windows")
}

fn identity() -> anyhow::Result<(String, PathBuf)> {
    let channel = RuntimeChannel::detect();
    let name = match channel {
        RuntimeChannel::Stable => "CodeCaddie",
        RuntimeChannel::Development => "CodeCaddie Dev",
    };
    let core = std::env::current_exe()?;
    let desktop = core.with_file_name(if cfg!(target_os = "windows") {
        "codecaddie.exe"
    } else {
        "codecaddie"
    });
    if !desktop.is_file() {
        anyhow::bail!("Launch at Login is available from an installed CodeCaddie application")
    }
    Ok((name.into(), desktop))
}
