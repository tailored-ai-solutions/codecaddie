//! Passphrase-encrypted, authenticated workspace backup bundles.
//!
//! Portable backup keys are derived for each export with Argon2id. They are
//! never written to the local data root or an operating-system credential
//! manager. The encrypted payload remains subject to signed-event validation
//! before import.

use super::identity::{LocalDeviceSecret, LocalWorkspaceAccess};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use codecaddie_domain::{EventEnvelope, Role, WorkspaceProjection};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use zeroize::Zeroizing;

pub(super) const PORTABLE_BACKUP_FORMAT: &str = "codecaddie-portable-backup-v1";
const PAYLOAD_FORMAT: &str = "codecaddie-portable-backup-payload-v1";
const MANIFEST_SCHEMA_VERSION: u16 = 1;
const ALGORITHM: &str = "XChaCha20-Poly1305";
const KDF: &str = "Argon2id";
const MEMORY_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;
const PARALLELISM: u32 = 1;
const KEY_BYTES: usize = 32;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;
pub(super) const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;
const MIN_PASSPHRASE_BYTES: usize = 12;
const MAX_PASSPHRASE_BYTES: usize = 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableBackupEnvelope {
    format: String,
    algorithm: String,
    kdf: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PortableBackupPayload {
    format: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    product_version: String,
    pub(super) workspace: LocalWorkspaceAccess,
    pub(super) device: LocalDeviceSecret,
    pub(super) events: Vec<EventEnvelope>,
    manifest: PortableBackupManifest,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableBackupManifest {
    /// These fields were added to the v1 payload after the first portable
    /// backups shipped. `None` is accepted only as the complete legacy shape;
    /// partially populated or future manifests fail closed.
    #[serde(default)]
    schema_version: Option<u16>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    created_at: Option<OffsetDateTime>,
    #[serde(default)]
    encryption: Option<PortableEncryptionMetadata>,
    workspace_id: String,
    workspace_fingerprint: String,
    event_count: usize,
    events_blake3: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableEncryptionMetadata {
    algorithm: String,
    kdf: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl PortableEncryptionMetadata {
    fn current() -> Self {
        Self {
            algorithm: ALGORITHM.into(),
            kdf: KDF.into(),
            memory_kib: MEMORY_KIB,
            iterations: ITERATIONS,
            parallelism: PARALLELISM,
        }
    }

    fn is_current(&self) -> bool {
        self.algorithm == ALGORITHM
            && self.kdf == KDF
            && self.memory_kib == MEMORY_KIB
            && self.iterations == ITERATIONS
            && self.parallelism == PARALLELISM
    }
}

impl PortableBackupPayload {
    pub(super) fn new(
        workspace: LocalWorkspaceAccess,
        device: LocalDeviceSecret,
        events: Vec<EventEnvelope>,
    ) -> anyhow::Result<Self> {
        let events_blake3 = events_digest(&events)?;
        let created_at = OffsetDateTime::now_utc();
        let manifest = PortableBackupManifest {
            schema_version: Some(MANIFEST_SCHEMA_VERSION),
            created_at: Some(created_at),
            encryption: Some(PortableEncryptionMetadata::current()),
            workspace_id: workspace.workspace_id.clone(),
            workspace_fingerprint: workspace.workspace_fingerprint.clone(),
            event_count: events.len(),
            events_blake3,
        };
        let payload = Self {
            format: PAYLOAD_FORMAT.into(),
            created_at,
            product_version: env!("CARGO_PKG_VERSION").into(),
            workspace,
            device,
            events,
            manifest,
        };
        payload.validate()?;
        Ok(payload)
    }

    pub(super) fn validate(&self) -> anyhow::Result<WorkspaceProjection> {
        if self.format != PAYLOAD_FORMAT {
            anyhow::bail!("portable backup payload format is unsupported");
        }
        if self.product_version.trim().is_empty() || self.product_version.len() > 32 {
            anyhow::bail!("portable backup product version is invalid");
        }
        if self.events.is_empty() || self.events.len() > MAX_EVENTS {
            anyhow::bail!("portable backup event count is outside the supported range");
        }
        match (
            self.manifest.schema_version,
            self.manifest.created_at,
            self.manifest.encryption.as_ref(),
        ) {
            (None, None, None) => {}
            (Some(MANIFEST_SCHEMA_VERSION), Some(created_at), Some(encryption))
                if created_at == self.created_at && encryption.is_current() => {}
            _ => anyhow::bail!("portable backup manifest metadata is unsupported"),
        }
        if self.manifest.workspace_id != self.workspace.workspace_id
            || self.manifest.workspace_fingerprint != self.workspace.workspace_fingerprint
            || self.manifest.event_count != self.events.len()
            || self.manifest.events_blake3 != events_digest(&self.events)?
        {
            anyhow::bail!("portable backup manifest does not match its payload");
        }
        if self.workspace.role != Role::Editor {
            anyhow::bail!("portable backup does not contain editing authority");
        }
        let projection = WorkspaceProjection::rebuild(&self.events)
            .map_err(|error| anyhow::anyhow!("portable backup event validation failed: {error}"))?;
        if projection.workspace_id != self.workspace.workspace_id
            || projection.workspace_fingerprint != self.workspace.workspace_fingerprint
        {
            anyhow::bail!("portable backup workspace identity is inconsistent");
        }
        let device = self.device.public_identity()?;
        let Some(access) = projection.devices.get(&device.device_id) else {
            anyhow::bail!("portable backup signing device is not registered");
        };
        if access.role != Role::Editor
            || access.identity.actor_id != device.actor_id
            || access.identity.signing_public_key != device.signing_public_key
        {
            anyhow::bail!("portable backup signing authority is invalid");
        }
        Ok(projection)
    }

    pub(super) fn manifest_digest(&self) -> anyhow::Result<String> {
        Ok(blake3::hash(&serde_json::to_vec(&self.manifest)?)
            .to_hex()
            .to_string())
    }
}

pub(super) fn seal(payload: &PortableBackupPayload, passphrase: &str) -> anyhow::Result<Vec<u8>> {
    seal_inner(payload, passphrase, true)
}

#[cfg(test)]
pub(super) fn seal_without_validation_for_test(
    payload: &PortableBackupPayload,
    passphrase: &str,
) -> anyhow::Result<Vec<u8>> {
    seal_inner(payload, passphrase, false)
}

fn seal_inner(
    payload: &PortableBackupPayload,
    passphrase: &str,
    validate: bool,
) -> anyhow::Result<Vec<u8>> {
    validate_passphrase(passphrase)?;
    if validate {
        payload.validate()?;
    }
    let plaintext = Zeroizing::new(serde_json::to_vec(payload)?);
    if plaintext.len() > MAX_BACKUP_BYTES {
        anyhow::bail!("portable backup payload exceeds the 64 MiB limit");
    }
    let mut salt = [0_u8; SALT_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let key = derive_key(passphrase, &salt)?;
    let mut nonce = [0_u8; NONCE_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("portable backup key derivation failed"))?;
    let ciphertext = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext.as_ref(),
                aad: PORTABLE_BACKUP_FORMAT.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("portable backup encryption failed"))?;
    let encoded = serde_json::to_vec_pretty(&PortableBackupEnvelope {
        format: PORTABLE_BACKUP_FORMAT.into(),
        algorithm: ALGORITHM.into(),
        kdf: KDF.into(),
        memory_kib: MEMORY_KIB,
        iterations: ITERATIONS,
        parallelism: PARALLELISM,
        salt: STANDARD_NO_PAD.encode(salt),
        nonce: STANDARD_NO_PAD.encode(nonce),
        ciphertext: STANDARD_NO_PAD.encode(ciphertext),
    })?;
    if encoded.len() > MAX_BACKUP_BYTES {
        anyhow::bail!("portable backup exceeds the 64 MiB limit");
    }
    Ok(encoded)
}

pub(super) fn open(bytes: &[u8], passphrase: &str) -> anyhow::Result<PortableBackupPayload> {
    validate_passphrase(passphrase)?;
    if bytes.len() > MAX_BACKUP_BYTES {
        anyhow::bail!("portable backup exceeds the 64 MiB limit");
    }
    let envelope: PortableBackupEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("portable backup envelope is invalid"))?;
    if envelope.format != PORTABLE_BACKUP_FORMAT
        || envelope.algorithm != ALGORITHM
        || envelope.kdf != KDF
        || envelope.memory_kib != MEMORY_KIB
        || envelope.iterations != ITERATIONS
        || envelope.parallelism != PARALLELISM
    {
        anyhow::bail!("portable backup cryptographic parameters are unsupported");
    }
    let salt = decode_fixed::<SALT_BYTES>(&envelope.salt, "salt")?;
    let nonce = decode_fixed::<NONCE_BYTES>(&envelope.nonce, "nonce")?;
    let ciphertext = STANDARD_NO_PAD
        .decode(envelope.ciphertext)
        .map_err(|_| anyhow::anyhow!("portable backup ciphertext is invalid"))?;
    let key = derive_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("portable backup key derivation failed"))?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &ciphertext,
                    aad: PORTABLE_BACKUP_FORMAT.as_bytes(),
                },
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "portable backup could not be authenticated; check the passphrase and file"
                )
            })?,
    );
    let payload: PortableBackupPayload = serde_json::from_slice(plaintext.as_ref())
        .map_err(|_| anyhow::anyhow!("portable backup payload is invalid"))?;
    payload.validate()?;
    Ok(payload)
}

pub(super) fn validate_passphrase(passphrase: &str) -> anyhow::Result<()> {
    if !(MIN_PASSPHRASE_BYTES..=MAX_PASSPHRASE_BYTES).contains(&passphrase.len()) {
        anyhow::bail!("portable backup passphrase must be 12 to 1024 bytes");
    }
    Ok(())
}

fn derive_key(
    passphrase: &str,
    salt: &[u8; SALT_BYTES],
) -> anyhow::Result<Zeroizing<[u8; KEY_BYTES]>> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, Some(KEY_BYTES))
        .map_err(|_| anyhow::anyhow!("portable backup KDF parameters are invalid"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|_| anyhow::anyhow!("portable backup key derivation failed"))?;
    Ok(key)
}

fn decode_fixed<const N: usize>(encoded: &str, name: &str) -> anyhow::Result<[u8; N]> {
    STANDARD_NO_PAD
        .decode(encoded)
        .map_err(|_| anyhow::anyhow!("portable backup {name} is invalid"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("portable backup {name} length is invalid"))
}

fn events_digest(events: &[EventEnvelope]) -> anyhow::Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(events)?)
        .to_hex()
        .to_string())
}
