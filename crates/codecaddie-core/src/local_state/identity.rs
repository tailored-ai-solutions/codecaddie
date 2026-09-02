//! Device-local workspace state, identities, and recovery payloads stored as
//! authenticated encrypted JSON plus an owner-only local key in the configured data root.

use codecaddie_domain::{DeviceIdentity, Role};
use ed25519_dalek::{SECRET_KEY_LENGTH, SigningKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

const LOCAL_STATE_FORMAT_V2: &str = "codecaddie-local-state-v2";
const LOCAL_STATE_FORMAT: &str = "codecaddie-local-state-v3";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalDeviceSecret {
    pub actor_id: String,
    pub device_id: String,
    pub label: String,
    pub signing_seed: Vec<u8>,
}

impl LocalDeviceSecret {
    fn random() -> anyhow::Result<Self> {
        let mut signing_seed = [0_u8; SECRET_KEY_LENGTH];
        crate::at_rest::fill_random_bytes(&mut signing_seed)?;
        Ok(Self {
            actor_id: format!("actor-{}", Uuid::new_v4()),
            device_id: format!("device-{}", Uuid::new_v4()),
            label: "This device".into(),
            signing_seed: signing_seed.to_vec(),
        })
    }

    pub fn signing_key(&self) -> anyhow::Result<SigningKey> {
        let seed: [u8; SECRET_KEY_LENGTH] = self
            .signing_seed
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("local signing identity is malformed"))?;
        Ok(SigningKey::from_bytes(&seed))
    }

    pub fn public_identity(&self) -> anyhow::Result<DeviceIdentity> {
        let signing_key = self.signing_key()?;
        Ok(DeviceIdentity {
            actor_id: self.actor_id.clone(),
            device_id: self.device_id.clone(),
            signing_public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            label: self.label.clone(),
        })
    }
}

/// The structured project context behind the flattened `product_brief`.
/// Device-local like `repository_path`: it names local files and setup
/// choices, so it lives in local state rather than the event log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContext {
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub website: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub context_file_names: Vec<String>,
    /// Structured, device-local references for files whose contents may be
    /// sent to the selected provider during goal generation. The legacy
    /// `context_file_names` field remains readable because old builds threw
    /// the original paths away.
    #[serde(default)]
    pub context_files: Vec<crate::context_documents::ContextFileReference>,
    /// Transient input accepted from the trusted desktop host. Store writes
    /// inspect these paths into `context_files` and clear this field before
    /// persistence, so unverified references never become workspace state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_file_paths: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalWorkspaceAccess {
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_fingerprint: String,
    pub role: Role,
    pub repository_path: String,
    pub product_brief: String,
    #[serde(default)]
    pub project_context: ProjectContext,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalState {
    format: String,
    pub(super) device: LocalDeviceSecret,
    pub(super) workspaces: BTreeMap<String, LocalWorkspaceAccess>,
}

impl LocalState {
    pub(super) fn new() -> anyhow::Result<Self> {
        Ok(Self {
            format: LOCAL_STATE_FORMAT.into(),
            device: LocalDeviceSecret::random()?,
            workspaces: BTreeMap::new(),
        })
    }

    pub(super) fn validate(&self) -> anyhow::Result<()> {
        if self.format != LOCAL_STATE_FORMAT && self.format != LOCAL_STATE_FORMAT_V2 {
            anyhow::bail!("local state format is unsupported; start a new local workspace");
        }
        let _ = self.device.public_identity()?;
        Ok(())
    }

    pub(super) fn upgrade_format(&mut self) {
        self.format = LOCAL_STATE_FORMAT.into();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecoveryPayload {
    pub(super) format: String,
    pub(super) workspace_id: String,
    pub(super) workspace_fingerprint: String,
    pub(super) device: LocalDeviceSecret,
    pub(super) role: Role,
    pub(super) events: Vec<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{FailOnce, LocalStateFile, PersistenceBoundary};

    #[test]
    fn local_state_without_project_context_deserializes_with_defaults() {
        let json = r#"{
            "workspaceId":"w1","workspaceName":"N","workspaceFingerprint":"f",
            "role":"editor","repositoryPath":"/local/x","productBrief":"brief"
        }"#;
        let access: LocalWorkspaceAccess = serde_json::from_str(json).unwrap();
        assert_eq!(access.project_context, ProjectContext::default());
    }

    #[test]
    fn v2_local_state_is_readable_and_atomically_advances_to_v3_on_save() {
        let state = LocalState::new().unwrap();
        let mut value = serde_json::to_value(&state).unwrap();
        value["format"] = serde_json::Value::String(LOCAL_STATE_FORMAT_V2.into());
        let mut migrated: LocalState = serde_json::from_value(value).unwrap();
        migrated.validate().unwrap();
        migrated.upgrade_format();
        let serialized = serde_json::to_string(&migrated).unwrap();
        assert!(serialized.contains(LOCAL_STATE_FORMAT));
        assert!(!serialized.contains(LOCAL_STATE_FORMAT_V2));
    }

    #[test]
    fn interrupted_local_state_migrations_converge_before_and_after_rename() {
        let directory = tempfile::tempdir().unwrap();
        let state_file = LocalStateFile::for_data_root(
            directory.path(),
            crate::at_rest::ContentCipher::for_tests(),
        )
        .unwrap();
        let mut v2 = LocalState::new().unwrap();
        v2.format = LOCAL_STATE_FORMAT_V2.into();
        state_file.save(&v2).unwrap();

        let mut migration: LocalState = state_file.load().unwrap();
        migration.upgrade_format();
        let before_rename = FailOnce::new(PersistenceBoundary::TemporaryFileSynced);
        assert!(
            state_file
                .save_with_fault(&migration, &before_rename)
                .is_err()
        );
        let reopened_v2: LocalState = state_file.load().unwrap();
        assert_eq!(reopened_v2.format, LOCAL_STATE_FORMAT_V2);

        let mut retry = reopened_v2;
        retry.upgrade_format();
        state_file.save(&retry).unwrap();
        let migrated: LocalState = state_file.load().unwrap();
        assert_eq!(migrated.format, LOCAL_STATE_FORMAT);

        v2.format = LOCAL_STATE_FORMAT_V2.into();
        state_file.save(&v2).unwrap();
        let mut migration: LocalState = state_file.load().unwrap();
        migration.upgrade_format();
        let after_rename = FailOnce::new(PersistenceBoundary::DestinationRenamed);
        assert!(
            state_file
                .save_with_fault(&migration, &after_rename)
                .is_err()
        );
        let committed_despite_interruption: LocalState = state_file.load().unwrap();
        assert_eq!(committed_despite_interruption.format, LOCAL_STATE_FORMAT);
        state_file.save(&committed_despite_interruption).unwrap();
        assert!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    !name.ends_with(".tmp") && !name.ends_with(".quarantined")
                })
        );
    }
}
