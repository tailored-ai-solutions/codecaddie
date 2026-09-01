//! Restart-safe, device-local containment controls.
//!
//! The shipped defaults are version-controlled. An operator may persist the
//! same closed structure as an encrypted owner-only override in the existing
//! data root. No credential manager or second store is involved.

#[cfg(test)]
use crate::persistence::write_encrypted_replace;
use crate::{
    at_rest::ContentCipher, persistence::read_encrypted_migrating, runtime_channel::RuntimeChannel,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

const CONTROL_FILE: &str = "runtime-feature-controls-v1.json";
const CONTROL_PURPOSE: &str = "runtime-feature-controls-v1.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureState {
    Enabled,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeFeatureControls {
    pub schema_version: u16,
    pub owner: String,
    pub features: FeatureControls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureControls {
    pub provider_execution: FeatureState,
    pub portable_backup_import: FeatureState,
    pub report_export: FeatureState,
    pub recommendation_prompt_copy: FeatureState,
}

impl RuntimeFeatureControls {
    fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != 1 || self.owner.trim().is_empty() {
            anyhow::bail!("runtime feature controls are incomplete");
        }
        Ok(())
    }

    fn state_for_method(&self, method: &str) -> Option<FeatureState> {
        match method {
            "goals.generate" | "scan.run" | "map.generate" => {
                Some(self.features.provider_execution)
            }
            "workspace.backup.import" => Some(self.features.portable_backup_import),
            "reports.export_word" => Some(self.features.report_export),
            "recommendations.copy_prompt" => Some(self.features.recommendation_prompt_copy),
            _ => None,
        }
    }
}

pub fn shipped_controls() -> anyhow::Result<RuntimeFeatureControls> {
    let controls: RuntimeFeatureControls = serde_json::from_str(include_str!(
        "../../../config/runtime-feature-controls.json"
    ))?;
    controls.validate()?;
    Ok(controls)
}

pub fn controls_for_environment() -> anyhow::Result<RuntimeFeatureControls> {
    let root = RuntimeChannel::detect().data_root()?;
    load_from_root(&root)
}

fn load_from_root(root: &Path) -> anyhow::Result<RuntimeFeatureControls> {
    let path = root.join(CONTROL_FILE);
    if !path.exists() {
        return shipped_controls();
    }
    let cipher = ContentCipher::from_local_key_file(root)?;
    let plaintext = read_encrypted_migrating(&path, &cipher, CONTROL_PURPOSE)?;
    let controls: RuntimeFeatureControls = serde_json::from_slice(&plaintext)?;
    controls.validate()?;
    Ok(controls)
}

#[cfg(test)]
fn save_to_root(root: &Path, controls: &RuntimeFeatureControls) -> anyhow::Result<()> {
    controls.validate()?;
    let cipher = ContentCipher::from_local_key_file(root)?;
    write_encrypted_replace(
        &root.join(CONTROL_FILE),
        &serde_json::to_vec(controls)?,
        &cipher,
        CONTROL_PURPOSE,
    )
}

pub fn method_is_paused(method: &str) -> anyhow::Result<bool> {
    Ok(controls_for_environment()?.state_for_method(method) == Some(FeatureState::Paused))
}

pub fn method_is_controlled(method: &str) -> bool {
    matches!(
        method,
        "goals.generate"
            | "scan.run"
            | "map.generate"
            | "workspace.backup.import"
            | "reports.export_word"
            | "recommendations.copy_prompt"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_provider_execution_survives_restart_without_touching_customer_state() {
        let directory = tempfile::tempdir().unwrap();
        let customer_state = directory.path().join("customer-state.sentinel");
        std::fs::write(&customer_state, "preserve").unwrap();
        let mut controls = shipped_controls().unwrap();
        controls.features.provider_execution = FeatureState::Paused;
        save_to_root(directory.path(), &controls).unwrap();

        let reopened = load_from_root(directory.path()).unwrap();
        assert_eq!(
            reopened.state_for_method("scan.run"),
            Some(FeatureState::Paused)
        );
        assert_eq!(
            reopened.state_for_method("goals.generate"),
            Some(FeatureState::Paused)
        );
        assert_eq!(
            reopened.state_for_method("workspace.recent"),
            None,
            "reading existing customer state is never disabled"
        );
        assert_eq!(
            std::fs::read_to_string(&customer_state).unwrap(),
            "preserve"
        );

        let encrypted = std::fs::read(directory.path().join(CONTROL_FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&encrypted).contains("providerExecution"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.path().join(CONTROL_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        controls.features.provider_execution = FeatureState::Enabled;
        save_to_root(directory.path(), &controls).unwrap();
        assert_eq!(
            load_from_root(directory.path())
                .unwrap()
                .state_for_method("scan.run"),
            Some(FeatureState::Enabled)
        );
        assert_eq!(std::fs::read_to_string(customer_state).unwrap(), "preserve");
    }

    #[test]
    fn shipped_controls_are_closed_owned_and_enabled() {
        let controls = shipped_controls().unwrap();
        assert_eq!(controls.schema_version, 1);
        assert!(!controls.owner.is_empty());
        for method in [
            "scan.run",
            "workspace.backup.import",
            "reports.export_word",
            "recommendations.copy_prompt",
        ] {
            assert_eq!(
                controls.state_for_method(method),
                Some(FeatureState::Enabled)
            );
        }
    }
}
