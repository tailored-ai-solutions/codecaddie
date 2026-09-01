//! Authenticated encryption for persisted local workspace state.
//!
//! The content key is generated locally and stored as an owner-only file in
//! the existing CodeCaddie data root. The application never calls an operating
//! system credential manager and this module does not introduce a second data
//! store.

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use fs2::FileExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::Arc,
};
use zeroize::Zeroizing;

pub(crate) const ENVELOPE_FORMAT: &str = "codecaddie-encrypted-v1";
const ENVELOPE_ALGORITHM: &str = "XChaCha20-Poly1305";
const KEY_BYTES: usize = 32;
pub(crate) const LOCAL_KEY_FILE: &str = "local-content-key-v1";
const LOCAL_KEY_PENDING_FILE: &str = "local-content-key-v1.pending";

#[derive(Clone)]
pub(crate) struct ContentCipher {
    key: Arc<Zeroizing<[u8; KEY_BYTES]>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EncryptedEnvelope {
    format: String,
    algorithm: String,
    purpose: String,
    nonce: String,
    ciphertext: String,
}

pub(crate) struct DecodedBytes {
    pub(crate) plaintext: Vec<u8>,
    pub(crate) encrypted: bool,
}

impl ContentCipher {
    pub(crate) fn from_local_key_file(root: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(root)?;
        let lock_directory = root.join("locks-v1");
        fs::create_dir_all(&lock_directory)?;
        #[cfg(unix)]
        for directory in [root, lock_directory.as_path()] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        let lock_path = lock_directory.join("content-key-v1.lock");
        let mut lock_options = OpenOptions::new();
        lock_options.create(true).read(true).write(true);
        #[cfg(unix)]
        lock_options.mode(0o600);
        let lock = lock_options.open(&lock_path)?;
        #[cfg(unix)]
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))?;
        lock.lock_exclusive()?;

        let result = load_or_create_local_key(root).map(Self::from_key);

        FileExt::unlock(&lock)?;
        result
    }

    fn from_key(key: [u8; KEY_BYTES]) -> Self {
        Self {
            key: Arc::new(Zeroizing::new(key)),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::from_key([0xC7; KEY_BYTES])
    }

    pub(crate) fn seal(&self, purpose: &str, plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
        validate_purpose(purpose)?;
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref().as_ref())
            .map_err(|_| anyhow::anyhow!("the local content-encryption key is invalid"))?;
        let mut nonce_bytes = [0_u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from(nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: purpose.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("local state encryption failed"))?;
        Ok(serde_json::to_vec(&EncryptedEnvelope {
            format: ENVELOPE_FORMAT.into(),
            algorithm: ENVELOPE_ALGORITHM.into(),
            purpose: purpose.into(),
            nonce: STANDARD_NO_PAD.encode(nonce),
            ciphertext: STANDARD_NO_PAD.encode(ciphertext),
        })?)
    }

    pub(crate) fn open_or_plain(
        &self,
        purpose: &str,
        bytes: &[u8],
    ) -> anyhow::Result<DecodedBytes> {
        validate_purpose(purpose)?;
        let envelope = match serde_json::from_slice::<EncryptedEnvelope>(bytes) {
            Ok(envelope) if envelope.format == ENVELOPE_FORMAT => envelope,
            Ok(envelope) => {
                anyhow::bail!("unsupported local encryption format {}", envelope.format)
            }
            Err(_) => {
                return Ok(DecodedBytes {
                    plaintext: bytes.to_vec(),
                    encrypted: false,
                });
            }
        };
        if envelope.algorithm != ENVELOPE_ALGORITHM {
            anyhow::bail!("unsupported local encryption algorithm");
        }
        if envelope.purpose != purpose {
            anyhow::bail!("encrypted local state belongs to a different storage purpose");
        }
        let nonce = STANDARD_NO_PAD
            .decode(envelope.nonce)
            .map_err(|_| anyhow::anyhow!("encrypted local state has an invalid nonce"))?;
        let nonce: [u8; 24] = nonce
            .try_into()
            .map_err(|_| anyhow::anyhow!("encrypted local state has an invalid nonce length"))?;
        let ciphertext = STANDARD_NO_PAD
            .decode(envelope.ciphertext)
            .map_err(|_| anyhow::anyhow!("encrypted local state has invalid ciphertext"))?;
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref().as_ref())
            .map_err(|_| anyhow::anyhow!("the local content-encryption key is invalid"))?;
        let plaintext = cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &ciphertext,
                    aad: purpose.as_bytes(),
                },
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "encrypted local state could not be authenticated; restore the matching owner-only local content-key file and retry"
                )
            })?;
        Ok(DecodedBytes {
            plaintext,
            encrypted: true,
        })
    }
}

fn validate_purpose(purpose: &str) -> anyhow::Result<()> {
    if purpose.is_empty()
        || purpose.len() > 128
        || !purpose
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        anyhow::bail!("local encryption purpose is invalid");
    }
    Ok(())
}

fn load_or_create_local_key(root: &Path) -> anyhow::Result<[u8; KEY_BYTES]> {
    let key_path = root.join(LOCAL_KEY_FILE);
    if key_path.exists() {
        return read_local_key(&key_path);
    }
    let pending_path = root.join(LOCAL_KEY_PENDING_FILE);
    if pending_path.exists() {
        let key = read_local_key(&pending_path)?;
        fs::rename(&pending_path, &key_path)?;
        sync_directory(root)?;
        return Ok(key);
    }
    let mut generated = [0_u8; KEY_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut generated);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&pending_path)?;
    file.write_all(&generated)?;
    file.sync_all()?;
    fs::rename(&pending_path, &key_path)?;
    #[cfg(unix)]
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
    sync_directory(root)?;
    Ok(generated)
}

fn read_local_key(path: &Path) -> anyhow::Result<[u8; KEY_BYTES]> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        anyhow::bail!("the local content-key path is not a regular file");
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    key_from_bytes(fs::read(path)?)
}

fn key_from_bytes(bytes: Vec<u8>) -> anyhow::Result<[u8; KEY_BYTES]> {
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("the stored local content key has an invalid length"))
}

fn sync_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_adversarial_authenticated_envelope_hides_source_canary_and_binds_purpose() {
        let cipher = ContentCipher::for_tests();
        let sealed = cipher
            .seal(
                "local-state-v2",
                crate::privacy_test_support::REPOSITORY_FIXTURE.as_bytes(),
            )
            .unwrap();
        crate::privacy_test_support::assert_private_payload_absent(&sealed);
        let opened = cipher.open_or_plain("local-state-v2", &sealed).unwrap();
        assert!(opened.encrypted);
        assert_eq!(
            opened.plaintext,
            crate::privacy_test_support::REPOSITORY_FIXTURE.as_bytes()
        );
        assert!(
            cipher
                .open_or_plain("workspace-events-v1", &sealed)
                .is_err()
        );
    }

    #[test]
    fn privacy_adversarial_local_key_is_owner_only_and_reused_without_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let first = ContentCipher::from_local_key_file(directory.path()).unwrap();
        let key_path = directory.path().join(LOCAL_KEY_FILE);
        assert_eq!(fs::read(&key_path).unwrap().len(), KEY_BYTES);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let sealed = first.seal("reuse-test", b"private").unwrap();
        let second = ContentCipher::from_local_key_file(directory.path()).unwrap();
        assert_eq!(
            second
                .open_or_plain("reuse-test", &sealed)
                .unwrap()
                .plaintext,
            b"private"
        );
    }

    #[test]
    fn interrupted_local_key_creation_promotes_the_complete_pending_file() {
        let directory = tempfile::tempdir().unwrap();
        let pending = directory.path().join(LOCAL_KEY_PENDING_FILE);
        fs::write(&pending, [0xA5; KEY_BYTES]).unwrap();
        let cipher = ContentCipher::from_local_key_file(directory.path()).unwrap();
        assert!(!pending.exists());
        assert_eq!(
            fs::read(directory.path().join(LOCAL_KEY_FILE)).unwrap(),
            [0xA5; KEY_BYTES]
        );
        let sealed = cipher.seal("pending-test", b"preserved").unwrap();
        assert_eq!(
            cipher
                .open_or_plain("pending-test", &sealed)
                .unwrap()
                .plaintext,
            b"preserved"
        );
    }

    #[test]
    fn ciphertext_tampering_fails_authentication_without_plaintext_fallback() {
        let cipher = ContentCipher::for_tests();
        let sealed = cipher
            .seal("local-state-v2", b"authenticated state")
            .unwrap();
        let mut envelope: serde_json::Value = serde_json::from_slice(&sealed).unwrap();
        let mut ciphertext = STANDARD_NO_PAD
            .decode(envelope["ciphertext"].as_str().unwrap())
            .unwrap();
        ciphertext[0] ^= 1;
        envelope["ciphertext"] = serde_json::Value::String(STANDARD_NO_PAD.encode(ciphertext));
        let tampered = serde_json::to_vec(&envelope).unwrap();
        assert!(cipher.open_or_plain("local-state-v2", &tampered).is_err());
    }

    #[test]
    fn plaintext_is_detected_only_for_one_way_migration() {
        let cipher = ContentCipher::for_tests();
        let decoded = cipher
            .open_or_plain("local-state-v2", br#"{"format":"legacy"}"#)
            .unwrap();
        assert!(!decoded.encrypted);
        assert_eq!(decoded.plaintext, br#"{"format":"legacy"}"#);
    }
}
