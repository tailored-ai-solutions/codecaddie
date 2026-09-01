use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChannel {
    Stable,
    Development,
}

impl RuntimeChannel {
    pub fn detect() -> Self {
        if let Ok(value) = std::env::var("CODECADDIE_RELEASE_CHANNEL") {
            return Self::from_name(&value).unwrap_or(Self::Stable);
        }
        Self::from_runtime_context(
            std::env::current_exe().ok().as_deref(),
            cfg!(debug_assertions),
        )
    }

    fn from_runtime_context(executable: Option<&Path>, debug_build: bool) -> Self {
        // `native dev` launches the Rust helper from `target/debug`, not from
        // a directory named `CodeCaddie Dev.app`. Treating that helper as the
        // stable channel makes it share stable application state.
        if debug_build {
            return Self::Development;
        }
        executable
            .map(Self::from_executable_path)
            .unwrap_or(Self::Stable)
    }

    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "stable" => Some(Self::Stable),
            "dev" | "development" => Some(Self::Development),
            _ => None,
        }
    }

    pub fn from_executable_path(path: &Path) -> Self {
        // Split both separators explicitly so tests and packaging checks can
        // validate Windows paths on non-Windows CI runners.
        let development = path.to_string_lossy().split(['/', '\\']).any(|component| {
            component.eq_ignore_ascii_case("CodeCaddie Dev.app")
                || component.eq_ignore_ascii_case("CodeCaddie Dev")
        });
        if development {
            Self::Development
        } else {
            Self::Stable
        }
    }

    pub fn data_root(self) -> anyhow::Result<PathBuf> {
        if let Some(configured) = std::env::var_os("CODECADDIE_DATA_DIR") {
            return Ok(PathBuf::from(configured));
        }
        let directory_name = match self {
            Self::Stable => "CodeCaddie",
            Self::Development => "CodeCaddie Dev",
        };
        if cfg!(target_os = "macos") {
            return Ok(PathBuf::from(std::env::var_os("HOME").ok_or_else(|| {
                anyhow::anyhow!("HOME is unavailable; set CODECADDIE_DATA_DIR")
            })?)
            .join("Library/Application Support")
            .join(directory_name));
        }
        if cfg!(target_os = "windows") {
            return Ok(PathBuf::from(std::env::var_os("APPDATA").ok_or_else(|| {
                anyhow::anyhow!("APPDATA is unavailable; set CODECADDIE_DATA_DIR")
            })?)
            .join(directory_name));
        }
        linux_data_root(
            self,
            std::env::var_os("XDG_DATA_HOME"),
            std::env::var_os("HOME"),
        )
    }

    /// Native SDK writes an actual panic marker before terminating. The core
    /// checks only this fixed, channel-owned path and never reads its contents.
    /// A missing platform environment returns `None` so reliability recording
    /// cannot break the application startup it is meant to observe.
    pub fn native_panic_marker_path(self) -> Option<PathBuf> {
        let app_identifier = match self {
            Self::Stable => "org.codecaddie.desktop",
            Self::Development => "org.codecaddie.desktop.dev",
        };
        if cfg!(target_os = "macos") {
            return std::env::var_os("HOME").map(|home| {
                PathBuf::from(home)
                    .join("Library/Logs")
                    .join(app_identifier)
                    .join("last-panic.txt")
            });
        }
        if cfg!(target_os = "windows") {
            return std::env::var_os("LOCALAPPDATA").map(|root| {
                PathBuf::from(root)
                    .join(app_identifier)
                    .join("Logs")
                    .join("last-panic.txt")
            });
        }
        let root = std::env::var_os("XDG_STATE_HOME")
            .filter(|value| !value.is_empty() && Path::new(value).is_absolute())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join(".local/state"))
            })?;
        Some(
            root.join(app_identifier)
                .join("logs")
                .join("last-panic.txt"),
        )
    }

    fn linux_directory_name(self) -> &'static str {
        match self {
            Self::Stable => "codecaddie",
            Self::Development => "codecaddie-dev",
        }
    }
}

/// Resolves the Linux data root per the XDG Base Directory Specification:
/// `XDG_DATA_HOME` is used when it is set to an absolute path, and an unset,
/// empty, or relative value falls back to `$HOME/.local/share`.
fn linux_data_root(
    channel: RuntimeChannel,
    xdg_data_home: Option<OsString>,
    home: Option<OsString>,
) -> anyhow::Result<PathBuf> {
    let directory_name = channel.linux_directory_name();
    if let Some(configured) =
        xdg_data_home.filter(|value| !value.is_empty() && Path::new(value).is_absolute())
    {
        return Ok(PathBuf::from(configured).join(directory_name));
    }
    let home = home.filter(|value| !value.is_empty()).ok_or_else(|| {
        anyhow::anyhow!("XDG_DATA_HOME and HOME are both unavailable; set CODECADDIE_DATA_DIR")
    })?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join(directory_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_developer_paths_are_isolated() {
        assert_eq!(
            RuntimeChannel::from_executable_path(Path::new(
                "/Users/example/Applications/CodeCaddie Dev.app/Contents/MacOS/codecaddie"
            )),
            RuntimeChannel::Development
        );
        assert_eq!(
            RuntimeChannel::from_executable_path(Path::new(
                r"C:\Users\example\AppData\Local\Programs\CodeCaddie Dev\bin\codecaddie.exe"
            )),
            RuntimeChannel::Development
        );
        assert_eq!(
            RuntimeChannel::from_executable_path(Path::new(
                "/Applications/CodeCaddie.app/Contents/MacOS/codecaddie"
            )),
            RuntimeChannel::Stable
        );
    }

    #[test]
    fn panic_markers_are_fixed_to_the_native_channel_log_directory() {
        let stable = RuntimeChannel::Stable.native_panic_marker_path();
        let development = RuntimeChannel::Development.native_panic_marker_path();
        if let (Some(stable), Some(development)) = (stable, development) {
            let log_directory = if cfg!(target_os = "macos") {
                None
            } else if cfg!(target_os = "windows") {
                Some("Logs")
            } else {
                Some("logs")
            };
            let suffix = |identifier: &str| {
                let mut path = PathBuf::from(identifier);
                if let Some(log_directory) = log_directory {
                    path.push(log_directory);
                }
                path.push("last-panic.txt");
                path
            };
            assert!(stable.ends_with(suffix("org.codecaddie.desktop")));
            assert!(development.ends_with(suffix("org.codecaddie.desktop.dev")));
        }
    }

    #[test]
    fn debug_helpers_use_the_development_channel_outside_an_app_bundle() {
        assert_eq!(
            RuntimeChannel::from_runtime_context(
                Some(Path::new("/checkout/target/debug/codecaddie-core")),
                true,
            ),
            RuntimeChannel::Development
        );
        assert_eq!(
            RuntimeChannel::from_runtime_context(
                Some(Path::new(
                    "/Applications/CodeCaddie.app/Contents/MacOS/codecaddie-core"
                )),
                false,
            ),
            RuntimeChannel::Stable
        );
    }

    #[cfg(unix)]
    #[test]
    fn linux_data_root_prefers_an_absolute_xdg_data_home() {
        assert_eq!(
            linux_data_root(
                RuntimeChannel::Stable,
                Some("/srv/xdg-data".into()),
                Some("/home/example".into()),
            )
            .unwrap(),
            PathBuf::from("/srv/xdg-data/codecaddie")
        );
        assert_eq!(
            linux_data_root(
                RuntimeChannel::Development,
                Some("/srv/xdg-data".into()),
                Some("/home/example".into()),
            )
            .unwrap(),
            PathBuf::from("/srv/xdg-data/codecaddie-dev")
        );
    }

    #[test]
    fn linux_data_root_falls_back_to_home_when_xdg_data_home_is_unusable() {
        let home = || Some(OsString::from("/home/example"));
        let expected = PathBuf::from("/home/example")
            .join(".local")
            .join("share")
            .join("codecaddie");
        // Unset, empty, and relative values are all ignored per the XDG spec.
        assert_eq!(
            linux_data_root(RuntimeChannel::Stable, None, home()).unwrap(),
            expected
        );
        assert_eq!(
            linux_data_root(RuntimeChannel::Stable, Some(OsString::new()), home()).unwrap(),
            expected
        );
        assert_eq!(
            linux_data_root(RuntimeChannel::Stable, Some("relative/data".into()), home()).unwrap(),
            expected
        );
        assert_eq!(
            linux_data_root(RuntimeChannel::Development, None, home()).unwrap(),
            PathBuf::from("/home/example")
                .join(".local")
                .join("share")
                .join("codecaddie-dev")
        );
    }

    #[test]
    fn linux_data_root_without_home_or_xdg_names_the_override() {
        let error = linux_data_root(RuntimeChannel::Stable, None, None).unwrap_err();
        assert!(error.to_string().contains("CODECADDIE_DATA_DIR"));
        let error =
            linux_data_root(RuntimeChannel::Stable, None, Some(OsString::new())).unwrap_err();
        assert!(error.to_string().contains("CODECADDIE_DATA_DIR"));
    }
}
