use crate::runtime_channel::RuntimeChannel;
use futures_util::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore_tuf::{FileStore, HttpRepository, StoreRepository, Updater};
use sigstore_verify::{
    VerificationPolicy,
    trust_root::{
        PRODUCTION_TUF_ROOT, SIGSTORE_PRODUCTION_TRUSTED_ROOT, TRUSTED_ROOT_TARGET, TrustedRoot,
    },
    types::{
        Bundle as SigstoreBundle, MediaType as SigstoreBundleMediaType,
        SignatureContent as SigstoreSignatureContent,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use thiserror::Error;
use x509_cert::{
    Certificate,
    der::{Decode, asn1::Utf8StringRef},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const MANIFEST_SCHEMA_VERSION: u32 = 2;
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
/// Bounds on update HTTP traffic so a stalled server can never hang the
/// core. Manifests are tiny, so the whole request gets one minute; artifact
/// downloads are large, so they get a generous whole-request ceiling plus a
/// per-read stall bound instead.
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const MANIFEST_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const MANIFEST_PAIR_ATTEMPTS: u8 = 3;
const MANIFEST_PAIR_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
const SIGSTORE_TUF_REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const SIGSTORE_TUF_CACHE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const ARTIFACT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const ARTIFACT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
pub const DEFAULT_STABLE_MANIFEST_URL: &str =
    "https://github.com/tailored-ai-solutions/codecaddie/releases/latest/download/manifest.json";
const SIGSTORE_TUF_URL: &str = "https://tuf-repo-cdn.sigstore.dev";
const SIGSTORE_BUNDLE_MEDIA_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";
const SIGSTORE_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";
const SOURCE_REPOSITORY: &str = "tailored-ai-solutions/codecaddie";
const SOURCE_REPOSITORY_URI: &str = "https://github.com/tailored-ai-solutions/codecaddie";
const SOURCE_REPOSITORY_REF: &str = "refs/heads/main";
const RELEASE_WORKFLOW_IDENTITY: &str = "https://github.com/tailored-ai-solutions/codecaddie/.github/workflows/release.yml@refs/heads/main";
const RELEASE_BUILD_TRIGGERS: [&str; 2] = ["push", "workflow_dispatch"];
const STAGED_METADATA_FILE: &str = "staged-update.json";
const STAGED_MANIFEST_FILE: &str = "manifest.json";
const STAGED_SIGSTORE_BUNDLE_FILE: &str = "manifest.sigstore.json";
const STAGED_METADATA_SCHEMA_VERSION: u32 = 2;
const UPDATER_RESULT_FILE: &str = "last-updater-result-v1.json";
const MAX_UPDATER_RESULT_BYTES: u64 = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseManifestV2 {
    pub schema_version: u32,
    pub channel: String,
    pub version: String,
    pub build: u64,
    pub published_at: String,
    pub release_notes_url: String,
    pub minimum_supported_version: String,
    pub required: bool,
    pub source_repository: String,
    pub source_commit: String,
    pub artifacts: Vec<ReleaseArtifactV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseArtifactV1 {
    pub platform: String,
    pub architecture: String,
    pub format: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub current_build: u64,
    pub latest_version: String,
    pub latest_build: u64,
    pub channel: String,
    pub available: bool,
    pub required: bool,
    pub release_notes_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ReleaseArtifactV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedUpdate {
    pub version: String,
    pub build: u64,
    pub artifact_path: PathBuf,
    pub size: u64,
    pub sha256: String,
    #[serde(skip)]
    pub source_commit: String,
}

/// A deliberately content-free mailbox written by the external helper before
/// it reopens the application after a failed install. The next core handshake
/// consumes this fixed-code result, so repository text and raw OS errors can
/// never cross the updater-to-desktop boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdaterResultCode {
    InstallFailed,
    ReopenFailed,
    RestartRequired,
    ManualRepairRequired,
    ResultUnreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdaterResultStatus {
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdaterResultV1 {
    pub schema_version: u8,
    pub status: UpdaterResultStatus,
    pub code: UpdaterResultCode,
}

impl UpdaterResultV1 {
    pub fn failed(code: UpdaterResultCode) -> Self {
        Self {
            schema_version: 1,
            status: UpdaterResultStatus::Failed,
            code,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedMetadataV2 {
    schema_version: u32,
    artifact: ReleaseArtifactV1,
    version: String,
    build: u64,
    source_commit: String,
    artifact_file: String,
}

#[derive(Debug, Clone)]
pub struct VerifiedRelease {
    pub raw_manifest: Vec<u8>,
    pub raw_sigstore_bundle: Vec<u8>,
    pub manifest: ReleaseManifestV2,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("release manifest is too large")]
    ManifestTooLarge,
    #[error("release manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("release manifest Sigstore verification failed")]
    InvalidSignature,
    #[error("Sigstore trust metadata is unavailable")]
    TrustMetadataUnavailable,
    #[error("release manifest channel mismatch: expected {expected}, received {actual}")]
    ChannelMismatch { expected: String, actual: String },
    #[error("release manifest would downgrade {current} to {latest}")]
    Downgrade { current: String, latest: String },
    #[error("no update artifact exists for {platform}/{architecture}")]
    UnsupportedPlatform {
        platform: String,
        architecture: String,
    },
    #[error("update artifact URL must use HTTPS")]
    InsecureArtifactUrl,
    #[error("update artifact exceeds its declared size")]
    ArtifactTooLarge,
    #[error("update artifact size mismatch: expected {expected}, received {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("update artifact SHA-256 mismatch")]
    ChecksumMismatch,
    #[error("staged update path is outside CodeCaddie's update directory")]
    UnsafeStagedPath,
    #[error("no staged update metadata exists")]
    MissingStagedMetadata,
    #[error("update helper is unavailable at {0}")]
    MissingUpdater(PathBuf),
    #[error(
        "CodeCaddie is running from a mounted volume. Move CodeCaddie to Applications, reopen it there, and try the update again."
    )]
    MacAppOnMountedVolume,
    #[error(
        "CodeCaddie is running from a temporary App Translocation location. Move CodeCaddie to Applications, reopen it there, and try the update again."
    )]
    MacAppTranslocated,
    #[error(
        "CodeCaddie's containing Applications folder is not writable by this account. Move CodeCaddie to your Applications folder, reopen it there, and try the update again."
    )]
    MacDestinationNotWritable,
    #[error(
        "CodeCaddie's application bundle could not be located. Move CodeCaddie to Applications, reopen it there, and try the update again."
    )]
    MacAppBundleNotFound,
    #[error("the previous updater result is invalid")]
    InvalidUpdaterResult,
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SigstoreIdentityPolicy {
    issuer: String,
    identity: String,
    repository: String,
    repository_uri: String,
    repository_id: String,
    source_ref: String,
    source_commit: String,
    build_triggers: BTreeSet<String>,
}

impl SigstoreIdentityPolicy {
    fn production(source_commit: &str) -> Result<Self, UpdateError> {
        let repository_id = option_env!("CODECADDIE_GITHUB_REPOSITORY_ID")
            .filter(|value| valid_pinned_repository_id(value))
            .ok_or_else(|| {
                UpdateError::InvalidManifest(
                    "this build has no valid pinned GitHub repository ID".into(),
                )
            })?;
        Ok(Self {
            issuer: SIGSTORE_OIDC_ISSUER.into(),
            identity: RELEASE_WORKFLOW_IDENTITY.into(),
            repository: SOURCE_REPOSITORY.into(),
            repository_uri: SOURCE_REPOSITORY_URI.into(),
            repository_id: repository_id.into(),
            source_ref: SOURCE_REPOSITORY_REF.into(),
            source_commit: source_commit.into(),
            build_triggers: RELEASE_BUILD_TRIGGERS
                .into_iter()
                .map(str::to_owned)
                .collect(),
        })
    }
}

fn valid_pinned_repository_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| number > 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SigstoreCertificateClaims {
    issuer: String,
    workflow_sha: String,
    repository: String,
    source_ref: String,
    issuer_v2: String,
    build_signer_uri: String,
    build_signer_digest: String,
    repository_uri: String,
    source_commit: String,
    source_ref_v2: String,
    repository_id: String,
    build_config_uri: String,
    build_config_digest: String,
    build_trigger: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn current_build() -> u64 {
    option_env!("CODECADDIE_BUILD_NUMBER")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

pub fn current_commit() -> &'static str {
    option_env!("CODECADDIE_COMMIT_SHA").unwrap_or("development")
}

fn sigstore_tuf_cache_directory() -> Result<PathBuf, UpdateError> {
    let path = updates_directory()?.join("sigstore-tuf");
    fs::create_dir_all(&path)?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

async fn refresh_sigstore_trusted_root() -> Result<TrustedRoot, UpdateError> {
    let cache = sigstore_tuf_cache_directory()?;
    let repository =
        HttpRepository::new(SIGSTORE_TUF_URL).map_err(|_| UpdateError::TrustMetadataUnavailable)?;
    let store = FileStore::new(cache);
    let mut updater = Updater::new(repository, PRODUCTION_TUF_ROOT)
        .map_err(|_| UpdateError::TrustMetadataUnavailable)?
        .with_store(store);
    let now = jiff::Timestamp::now();
    updater
        .refresh(now)
        .await
        .map_err(|_| UpdateError::TrustMetadataUnavailable)?;
    let raw_root = updater
        .get_target(TRUSTED_ROOT_TARGET, now)
        .await
        .map_err(|_| UpdateError::TrustMetadataUnavailable)?;
    let raw_root =
        std::str::from_utf8(&raw_root).map_err(|_| UpdateError::TrustMetadataUnavailable)?;
    TrustedRoot::from_json(raw_root).map_err(|_| UpdateError::TrustMetadataUnavailable)
}

async fn cached_sigstore_trusted_root(cache: PathBuf) -> Result<TrustedRoot, UpdateError> {
    // Treat the cache as an untrusted repository: StoreRepository feeds every
    // cached role and target back through TUF verification from the embedded
    // bootstrap root, including expiry and anti-rollback checks.
    let store = FileStore::new(cache);
    let repository = StoreRepository::new(store.clone());
    let mut updater = Updater::new(repository, PRODUCTION_TUF_ROOT)
        .map_err(|_| UpdateError::TrustMetadataUnavailable)?
        .with_store(store);
    let now = jiff::Timestamp::now();
    updater
        .refresh(now)
        .await
        .map_err(|_| UpdateError::TrustMetadataUnavailable)?;
    let raw_root = updater
        .get_target(TRUSTED_ROOT_TARGET, now)
        .await
        .map_err(|_| UpdateError::TrustMetadataUnavailable)?;
    let raw_root =
        std::str::from_utf8(&raw_root).map_err(|_| UpdateError::TrustMetadataUnavailable)?;
    TrustedRoot::from_json(raw_root).map_err(|_| UpdateError::TrustMetadataUnavailable)
}

fn embedded_sigstore_trusted_root() -> Result<TrustedRoot, UpdateError> {
    TrustedRoot::from_json(SIGSTORE_PRODUCTION_TRUSTED_ROOT)
        .map_err(|_| UpdateError::TrustMetadataUnavailable)
}

fn select_sigstore_trusted_root(
    refreshed: Result<TrustedRoot, UpdateError>,
    cached: Result<TrustedRoot, UpdateError>,
) -> Result<TrustedRoot, UpdateError> {
    refreshed
        .or(cached)
        .or_else(|_| embedded_sigstore_trusted_root())
}

async fn sigstore_trusted_root_for_check() -> Result<TrustedRoot, UpdateError> {
    // Root rotation is best effort for an offline desktop. A stalled mirror is
    // bounded; a verified, unexpired cache is next; the embedded production
    // root is the final fail-safe and never turns network failure into trust of
    // unverified cache bytes.
    let refreshed = tokio::time::timeout(
        SIGSTORE_TUF_REFRESH_TIMEOUT,
        refresh_sigstore_trusted_root(),
    )
    .await
    .unwrap_or(Err(UpdateError::TrustMetadataUnavailable));
    if refreshed.is_ok() {
        return refreshed;
    }
    let cached = match sigstore_tuf_cache_directory() {
        Ok(cache) => tokio::time::timeout(
            SIGSTORE_TUF_CACHE_TIMEOUT,
            cached_sigstore_trusted_root(cache),
        )
        .await
        .unwrap_or(Err(UpdateError::TrustMetadataUnavailable)),
        Err(_) => Err(UpdateError::TrustMetadataUnavailable),
    };
    select_sigstore_trusted_root(refreshed, cached)
}

fn staged_sigstore_trusted_root() -> Result<TrustedRoot, UpdateError> {
    let cache = sigstore_tuf_cache_directory()?;
    let cached = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| UpdateError::TrustMetadataUnavailable)?;
        runtime.block_on(cached_sigstore_trusted_root(cache))
    })
    .join()
    .ok()
    .and_then(Result::ok);
    if let Some(root) = cached {
        return Ok(root);
    }
    embedded_sigstore_trusted_root()
}

pub fn verify_release(
    raw_manifest: &[u8],
    raw_sigstore_bundle: &[u8],
    expected_channel: &str,
    trusted_root: &TrustedRoot,
) -> Result<VerifiedRelease, UpdateError> {
    if raw_manifest.len() > MAX_MANIFEST_BYTES || raw_sigstore_bundle.len() > MAX_MANIFEST_BYTES {
        return Err(UpdateError::ManifestTooLarge);
    }
    let manifest: ReleaseManifestV2 = serde_json::from_slice(raw_manifest)?;
    validate_manifest(&manifest)?;
    if manifest.channel != expected_channel {
        return Err(UpdateError::ChannelMismatch {
            expected: expected_channel.into(),
            actual: manifest.channel.clone(),
        });
    }
    let identity = SigstoreIdentityPolicy::production(&manifest.source_commit)?;
    verify_release_with_policy(
        raw_manifest,
        raw_sigstore_bundle,
        manifest,
        trusted_root,
        &identity,
    )
}

fn verify_release_with_policy(
    raw_manifest: &[u8],
    raw_sigstore_bundle: &[u8],
    manifest: ReleaseManifestV2,
    trusted_root: &TrustedRoot,
    identity: &SigstoreIdentityPolicy,
) -> Result<VerifiedRelease, UpdateError> {
    if manifest.source_repository != identity.repository
        || manifest.source_commit != identity.source_commit
    {
        return Err(UpdateError::InvalidSignature);
    }
    let raw_bundle =
        std::str::from_utf8(raw_sigstore_bundle).map_err(|_| UpdateError::InvalidSignature)?;
    let bundle =
        SigstoreBundle::from_json(raw_bundle).map_err(|_| UpdateError::InvalidSignature)?;
    verify_sigstore_bundle_crypto(raw_manifest, &bundle, trusted_root, identity)?;
    let claims = sigstore_certificate_claims(&bundle)?;
    verify_sigstore_certificate_claims(&claims, identity)?;
    Ok(VerifiedRelease {
        raw_manifest: raw_manifest.to_vec(),
        raw_sigstore_bundle: raw_sigstore_bundle.to_vec(),
        manifest,
    })
}

fn verify_sigstore_bundle_crypto(
    raw_manifest: &[u8],
    bundle: &SigstoreBundle,
    trusted_root: &TrustedRoot,
    identity: &SigstoreIdentityPolicy,
) -> Result<(), UpdateError> {
    if !matches!(
        &bundle.content,
        SigstoreSignatureContent::MessageSignature(_)
    ) {
        return Err(UpdateError::InvalidSignature);
    }
    verify_sigstore_bundle_integrity(raw_manifest, bundle, trusted_root, identity)
}

fn verify_sigstore_bundle_integrity(
    raw_manifest: &[u8],
    bundle: &SigstoreBundle,
    trusted_root: &TrustedRoot,
    identity: &SigstoreIdentityPolicy,
) -> Result<(), UpdateError> {
    if bundle.media_type != SIGSTORE_BUNDLE_MEDIA_TYPE
        || bundle.version().ok() != Some(SigstoreBundleMediaType::Bundle0_3)
        || bundle.verification_material.tlog_entries.len() != 1
        || bundle
            .verification_material
            .tlog_entries
            .iter()
            .any(|entry| {
                entry.inclusion_proof.is_none()
                    || entry.inclusion_promise.is_none()
                    || entry.integrated_time <= 0
            })
    {
        return Err(UpdateError::InvalidSignature);
    }
    let policy = VerificationPolicy::default()
        .require_identity(&identity.identity)
        .require_issuer(&identity.issuer);
    let result = sigstore_verify::verify(raw_manifest, bundle, &policy, trusted_root)
        .map_err(|_| UpdateError::InvalidSignature)?;
    if result.identity.as_deref() != Some(identity.identity.as_str())
        || result.issuer.as_deref() != Some(identity.issuer.as_str())
        || result.integrated_time.is_none()
    {
        return Err(UpdateError::InvalidSignature);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum SigstoreExtensionEncoding {
    RawUtf8,
    DerUtf8,
}

fn sigstore_extension(
    certificate: &Certificate,
    oid: &str,
    encoding: SigstoreExtensionEncoding,
) -> Result<String, UpdateError> {
    let extensions = certificate
        .tbs_certificate
        .extensions
        .as_deref()
        .ok_or(UpdateError::InvalidSignature)?;
    let mut matching = extensions
        .iter()
        .filter(|extension| extension.extn_id.to_string() == oid);
    let extension = matching.next().ok_or(UpdateError::InvalidSignature)?;
    if matching.next().is_some() {
        return Err(UpdateError::InvalidSignature);
    }
    match encoding {
        SigstoreExtensionEncoding::RawUtf8 => std::str::from_utf8(extension.extn_value.as_bytes())
            .map(str::to_owned)
            .map_err(|_| UpdateError::InvalidSignature),
        SigstoreExtensionEncoding::DerUtf8 => {
            Utf8StringRef::from_der(extension.extn_value.as_bytes())
                .map(|value| value.to_string())
                .map_err(|_| UpdateError::InvalidSignature)
        }
    }
}

fn sigstore_certificate_claims(
    bundle: &SigstoreBundle,
) -> Result<SigstoreCertificateClaims, UpdateError> {
    let certificate = bundle
        .signing_certificate()
        .ok_or(UpdateError::InvalidSignature)?;
    sigstore_certificate_claims_from_der(certificate.as_bytes())
}

fn sigstore_certificate_claims_from_der(
    certificate: &[u8],
) -> Result<SigstoreCertificateClaims, UpdateError> {
    let certificate =
        Certificate::from_der(certificate).map_err(|_| UpdateError::InvalidSignature)?;
    let raw = SigstoreExtensionEncoding::RawUtf8;
    let der = SigstoreExtensionEncoding::DerUtf8;
    Ok(SigstoreCertificateClaims {
        issuer: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.1", raw)?,
        workflow_sha: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.3", raw)?,
        repository: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.5", raw)?,
        source_ref: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.6", raw)?,
        issuer_v2: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.8", der)?,
        build_signer_uri: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.9", der)?,
        build_signer_digest: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.10", der)?,
        repository_uri: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.12", der)?,
        source_commit: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.13", der)?,
        source_ref_v2: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.14", der)?,
        repository_id: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.15", der)?,
        build_config_uri: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.18", der)?,
        build_config_digest: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.19", der)?,
        build_trigger: sigstore_extension(&certificate, "1.3.6.1.4.1.57264.1.20", der)?,
    })
}

fn verify_sigstore_certificate_claims(
    claims: &SigstoreCertificateClaims,
    expected: &SigstoreIdentityPolicy,
) -> Result<(), UpdateError> {
    if claims.issuer != expected.issuer
        || claims.issuer_v2 != expected.issuer
        || claims.workflow_sha != expected.source_commit
        || claims.build_signer_digest != expected.source_commit
        || claims.source_commit != expected.source_commit
        || claims.build_config_digest != expected.source_commit
        || claims.repository != expected.repository
        || claims.repository_uri != expected.repository_uri
        || claims.repository_id != expected.repository_id
        || claims.source_ref != expected.source_ref
        || claims.source_ref_v2 != expected.source_ref
        || claims.build_signer_uri != expected.identity
        || claims.build_config_uri != expected.identity
        || !expected.build_triggers.contains(&claims.build_trigger)
    {
        return Err(UpdateError::InvalidSignature);
    }
    Ok(())
}

fn validate_manifest(manifest: &ReleaseManifestV2) -> Result<(), UpdateError> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(UpdateError::InvalidManifest(
            "unsupported release manifest schema".into(),
        ));
    }
    let version = Version::parse(&manifest.version)
        .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?;
    let minimum = Version::parse(&manifest.minimum_supported_version)
        .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?;
    if version.to_string() != manifest.version
        || minimum.to_string() != manifest.minimum_supported_version
        || !version.build.is_empty()
        || !minimum.build.is_empty()
        || manifest.build == 0
    {
        return Err(UpdateError::InvalidManifest(
            "release version or build identity is not canonical".into(),
        ));
    }
    if minimum > version {
        return Err(UpdateError::InvalidManifest(
            "minimum supported version cannot exceed the release version".into(),
        ));
    }
    if manifest.channel != "stable" && manifest.channel != "beta" {
        return Err(UpdateError::InvalidManifest(
            "channel must be stable or beta".into(),
        ));
    }
    if !manifest.release_notes_url.starts_with("https://")
        || time::OffsetDateTime::parse(
            &manifest.published_at,
            &time::format_description::well_known::Rfc3339,
        )
        .is_err()
        || manifest.source_repository != SOURCE_REPOSITORY
        || !is_lowercase_commit(&manifest.source_commit)
        || manifest.artifacts.is_empty()
    {
        return Err(UpdateError::InvalidManifest(
            "release metadata is incomplete or not HTTPS".into(),
        ));
    }
    let mut platform_artifacts = BTreeSet::new();
    for artifact in &manifest.artifacts {
        if !artifact.url.starts_with("https://") {
            return Err(UpdateError::InsecureArtifactUrl);
        }
        if artifact.size == 0
            || artifact.sha256.len() != 64
            || !artifact
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || artifact.platform.trim().is_empty()
            || artifact.architecture.trim().is_empty()
            || artifact.format.trim().is_empty()
        {
            return Err(UpdateError::InvalidManifest(
                "artifact size or SHA-256 is invalid".into(),
            ));
        }
        if !platform_artifacts.insert((
            artifact.platform.as_str(),
            artifact.architecture.as_str(),
            artifact.format.as_str(),
        )) {
            return Err(UpdateError::InvalidManifest(
                "duplicate platform artifact".into(),
            ));
        }
    }
    Ok(())
}

fn is_lowercase_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn select_update(
    release: &VerifiedRelease,
    current_version: &str,
    current_build: u64,
    platform: &str,
    architecture: &str,
) -> Result<UpdateCheckResult, UpdateError> {
    let current = Version::parse(current_version)
        .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?;
    let latest = Version::parse(&release.manifest.version)
        .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?;
    let minimum = Version::parse(&release.manifest.minimum_supported_version)
        .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?;
    if latest < current {
        return Err(UpdateError::Downgrade {
            current: current.to_string(),
            latest: latest.to_string(),
        });
    }
    let available =
        latest > current || (latest == current && release.manifest.build > current_build);
    let expected_format = if platform == "macos" { "zip" } else { "msi" };
    let artifact = release
        .manifest
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.platform == platform
                && artifact.architecture == architecture
                && artifact.format == expected_format
        })
        .cloned();
    if available && artifact.is_none() {
        return Err(UpdateError::UnsupportedPlatform {
            platform: platform.into(),
            architecture: architecture.into(),
        });
    }
    Ok(UpdateCheckResult {
        current_version: current.to_string(),
        current_build,
        latest_version: latest.to_string(),
        latest_build: release.manifest.build,
        channel: release.manifest.channel.clone(),
        available,
        required: available && (release.manifest.required || current < minimum),
        release_notes_url: release.manifest.release_notes_url.clone(),
        artifact: available.then_some(artifact).flatten(),
    })
}

pub fn host_platform() -> (&'static str, &'static str) {
    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unsupported"
    };
    let architecture = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "unsupported"
    };
    (platform, architecture)
}

pub fn manifest_url() -> String {
    std::env::var("CODECADDIE_UPDATE_MANIFEST_URL")
        .unwrap_or_else(|_| DEFAULT_STABLE_MANIFEST_URL.into())
}

pub async fn check() -> Result<UpdateCheckResult, UpdateError> {
    let release = fetch_release(&manifest_url(), "stable").await?;
    let (platform, architecture) = host_platform();
    select_update(
        &release,
        current_version(),
        current_build(),
        platform,
        architecture,
    )
}

pub async fn fetch_release(
    manifest_url: &str,
    expected_channel: &str,
) -> Result<VerifiedRelease, UpdateError> {
    if !update_url_is_allowed(manifest_url) {
        return Err(UpdateError::InvalidManifest(
            "manifest URL must use HTTPS".into(),
        ));
    }
    let manifest_request_url = reqwest::Url::parse(manifest_url)
        .map_err(|_| UpdateError::InvalidManifest("manifest URL is invalid".into()))?;
    let bundle_url = sibling_sigstore_bundle_url(&manifest_request_url)?;
    let trusted_root = sigstore_trusted_root_for_check().await?;
    let client = reqwest::Client::builder()
        .user_agent(format!("CodeCaddie/{}", current_version()))
        .https_only(!cfg!(debug_assertions))
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(MANIFEST_REQUEST_TIMEOUT)
        .build()?;
    for attempt in 1..=MANIFEST_PAIR_ATTEMPTS {
        let result = async {
            let raw_manifest = fetch_bounded(&client, manifest_url, MAX_MANIFEST_BYTES).await?;
            let raw_bundle =
                fetch_bounded(&client, bundle_url.as_str(), MAX_MANIFEST_BYTES).await?;
            verify_release(&raw_manifest, &raw_bundle, expected_channel, &trusted_root)
        }
        .await;
        match result {
            Ok(release) => return Ok(release),
            Err(error)
                if attempt < MANIFEST_PAIR_ATTEMPTS
                    && matches!(
                        &error,
                        UpdateError::InvalidSignature
                            | UpdateError::Json(_)
                            | UpdateError::Network(_)
                    ) =>
            {
                tokio::time::sleep(MANIFEST_PAIR_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded manifest pair loop always returns")
}

fn update_url_is_allowed(url: &str) -> bool {
    url.starts_with("https://")
        || (cfg!(debug_assertions)
            && (url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost")))
}

fn sibling_sigstore_bundle_url(manifest_url: &reqwest::Url) -> Result<reqwest::Url, UpdateError> {
    if !update_url_is_allowed(manifest_url.as_str())
        || !manifest_url.path().ends_with("/manifest.json")
    {
        return Err(UpdateError::InvalidManifest(
            "manifest request URL must end in /manifest.json".into(),
        ));
    }
    let mut bundle_url = manifest_url.clone();
    let path = manifest_url
        .path()
        .strip_suffix("manifest.json")
        .ok_or_else(|| UpdateError::InvalidManifest("manifest URL is invalid".into()))?;
    bundle_url.set_path(&format!("{path}manifest.sigstore.json"));
    bundle_url.set_query(None);
    bundle_url.set_fragment(None);
    Ok(bundle_url)
}

async fn fetch_bounded(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
) -> Result<Vec<u8>, UpdateError> {
    let response = client.get(url).send().await?.error_for_status()?;
    read_bounded_response(response, limit).await
}

async fn read_bounded_response(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, UpdateError> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(UpdateError::ManifestTooLarge);
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(limit as u64) as usize,
    );
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(UpdateError::ManifestTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn download() -> Result<StagedUpdate, UpdateError> {
    let release = fetch_release(&manifest_url(), "stable").await?;
    let (platform, architecture) = host_platform();
    let check = select_update(
        &release,
        current_version(),
        current_build(),
        platform,
        architecture,
    )?;
    let artifact = check.artifact.ok_or_else(|| {
        UpdateError::InvalidManifest("there is no newer update to download".into())
    })?;
    stage_release(&release, &artifact).await
}

async fn stage_release(
    release: &VerifiedRelease,
    artifact: &ReleaseArtifactV1,
) -> Result<StagedUpdate, UpdateError> {
    let staging = staging_directory()?;
    fs::create_dir_all(&staging)?;
    #[cfg(unix)]
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))?;
    let artifact_file = match artifact.format.as_str() {
        "zip" => "CodeCaddie-update.zip",
        "msi" => "CodeCaddie-update.msi",
        other => {
            return Err(UpdateError::InvalidManifest(format!(
                "unsupported artifact format {other}"
            )));
        }
    };
    let artifact_path = staging.join(artifact_file);
    let partial = staging.join(format!("{artifact_file}.partial"));
    if partial.exists() {
        fs::remove_file(&partial)?;
    }
    let client = reqwest::Client::builder()
        .user_agent(format!("CodeCaddie/{}", current_version()))
        .https_only(true)
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(ARTIFACT_REQUEST_TIMEOUT)
        .read_timeout(ARTIFACT_READ_TIMEOUT)
        .build()?;
    let response = client.get(&artifact.url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size != artifact.size)
    {
        return Err(UpdateError::SizeMismatch {
            expected: artifact.size,
            actual: response.content_length().unwrap_or_default(),
        });
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&partial)?;
    let mut stream = response.bytes_stream();
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size = size.saturating_add(chunk.len() as u64);
        if size > artifact.size {
            drop(file);
            let _ = fs::remove_file(&partial);
            return Err(UpdateError::ArtifactTooLarge);
        }
        hash.update(&chunk);
        file.write_all(&chunk)?;
    }
    file.sync_all()?;
    drop(file);
    if size != artifact.size {
        let _ = fs::remove_file(&partial);
        return Err(UpdateError::SizeMismatch {
            expected: artifact.size,
            actual: size,
        });
    }
    let digest = hex::encode(hash.finalize());
    if !digest.eq_ignore_ascii_case(&artifact.sha256) {
        let _ = fs::remove_file(&partial);
        return Err(UpdateError::ChecksumMismatch);
    }
    if artifact_path.exists() {
        fs::remove_file(&artifact_path)?;
    }
    fs::rename(&partial, &artifact_path)?;
    write_private(&staging.join(STAGED_MANIFEST_FILE), &release.raw_manifest)?;
    write_private(
        &staging.join(STAGED_SIGSTORE_BUNDLE_FILE),
        &release.raw_sigstore_bundle,
    )?;
    let metadata = StagedMetadataV2 {
        schema_version: STAGED_METADATA_SCHEMA_VERSION,
        artifact: artifact.clone(),
        version: release.manifest.version.clone(),
        build: release.manifest.build,
        source_commit: release.manifest.source_commit.clone(),
        artifact_file: artifact_file.into(),
    };
    write_private(
        &staging.join(STAGED_METADATA_FILE),
        &serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(StagedUpdate {
        version: metadata.version,
        build: metadata.build,
        artifact_path,
        size,
        sha256: digest,
        source_commit: metadata.source_commit,
    })
}

fn staging_directory() -> Result<PathBuf, UpdateError> {
    Ok(RuntimeChannel::detect()
        .data_root()
        .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?
        .join("updates/staging"))
}

fn updates_directory() -> Result<PathBuf, UpdateError> {
    Ok(RuntimeChannel::detect()
        .data_root()
        .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?
        .join("updates"))
}

fn prepare_private_updates_directory(path: &Path) -> Result<(), UpdateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err(UpdateError::InvalidUpdaterResult),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn updater_result_path(root: &Path) -> PathBuf {
    root.join("updates").join(UPDATER_RESULT_FILE)
}

fn remove_path_without_following(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn persistence_error(error: anyhow::Error) -> UpdateError {
    match error.downcast::<std::io::Error>() {
        Ok(error) => UpdateError::Io(error),
        Err(_) => UpdateError::InvalidUpdaterResult,
    }
}

fn record_updater_result_at_with(
    root: &Path,
    code: UpdaterResultCode,
    injector: &impl crate::persistence::PersistenceFaultInjector,
) -> Result<(), UpdateError> {
    let updates = root.join("updates");
    prepare_private_updates_directory(&updates)?;
    let bytes = serde_json::to_vec(&UpdaterResultV1::failed(code))?;
    if bytes.len() as u64 > MAX_UPDATER_RESULT_BYTES {
        return Err(UpdateError::InvalidUpdaterResult);
    }
    crate::persistence::write_private_replace_with(&updater_result_path(root), &bytes, injector)
        .map_err(persistence_error)
}

fn record_updater_result_at(root: &Path, code: UpdaterResultCode) -> Result<(), UpdateError> {
    record_updater_result_at_with(root, code, &crate::persistence::NoPersistenceFault)
}

/// Records a fixed-code failure under the existing channel data root. Raw
/// installer errors and paths remain on stderr only and are never persisted.
pub fn record_updater_result(code: UpdaterResultCode) -> Result<(), UpdateError> {
    let updates = updates_directory()?;
    let root = updates.parent().ok_or(UpdateError::InvalidUpdaterResult)?;
    record_updater_result_at(root, code)
}

fn take_updater_result_at(root: &Path) -> Result<Option<UpdaterResultV1>, UpdateError> {
    let path = updater_result_path(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_UPDATER_RESULT_BYTES {
        remove_path_without_following(&path)?;
        return Err(UpdateError::InvalidUpdaterResult);
    }
    let consuming = path.with_extension(format!("consuming-{}", std::process::id()));
    remove_path_without_following(&consuming)?;
    fs::rename(&path, &consuming)?;
    let bytes = fs::read(&consuming);
    let _ = fs::remove_file(&consuming);
    let bytes = bytes?;
    if bytes.len() as u64 > MAX_UPDATER_RESULT_BYTES {
        return Err(UpdateError::InvalidUpdaterResult);
    }
    let result: UpdaterResultV1 =
        serde_json::from_slice(&bytes).map_err(|_| UpdateError::InvalidUpdaterResult)?;
    if result.schema_version != 1 {
        return Err(UpdateError::InvalidUpdaterResult);
    }
    Ok(Some(result))
}

/// Atomically consumes the helper's one-shot result on the next startup.
pub fn take_updater_result() -> Result<Option<UpdaterResultV1>, UpdateError> {
    let updates = updates_directory()?;
    let root = updates.parent().ok_or(UpdateError::InvalidUpdaterResult)?;
    take_updater_result_at(root)
}

fn clear_updater_result() -> Result<(), UpdateError> {
    let path = updates_directory()?.join(UPDATER_RESULT_FILE);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn path_has_component(path: &Path, expected: &str) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(expected)
    })
}

#[cfg(any(target_os = "macos", test))]
fn macos_application_bundle(current_executable: &Path) -> Result<&Path, UpdateError> {
    current_executable
        .ancestors()
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
        .ok_or(UpdateError::MacAppBundleNotFound)
}

#[cfg(any(target_os = "macos", test))]
fn validate_macos_install_location_with<Probe>(
    current_executable: &Path,
    probe_parent: Probe,
) -> Result<(), UpdateError>
where
    Probe: FnOnce(&Path) -> std::io::Result<()>,
{
    let target = macos_application_bundle(current_executable)?;
    let canonical = fs::canonicalize(current_executable).ok();
    for candidate in std::iter::once(current_executable).chain(canonical.as_deref()) {
        if candidate.starts_with(Path::new("/Volumes")) {
            return Err(UpdateError::MacAppOnMountedVolume);
        }
        if path_has_component(candidate, "AppTranslocation") {
            return Err(UpdateError::MacAppTranslocated);
        }
    }
    let parent = target
        .parent()
        .ok_or(UpdateError::MacDestinationNotWritable)?;
    probe_parent(parent).map_err(|_| UpdateError::MacDestinationNotWritable)
}

#[cfg(target_os = "macos")]
fn probe_macos_parent(parent: &Path) -> std::io::Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let probe = parent.join(format!(
        ".codecaddie-update-write-test-{}-{nonce}",
        std::process::id(),
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(&probe)?;
    file.sync_all()?;
    drop(file);
    fs::remove_file(probe)
}

#[cfg(target_os = "macos")]
fn validate_macos_install_location(current_executable: &Path) -> Result<(), UpdateError> {
    validate_macos_install_location_with(current_executable, probe_macos_parent)
}

pub fn validate_staged(requested_path: &Path) -> Result<StagedUpdate, UpdateError> {
    let staging = staging_directory()?;
    let canonical_staging = staging.canonicalize()?;
    let canonical_artifact = requested_path.canonicalize()?;
    if canonical_artifact.parent() != Some(canonical_staging.as_path()) {
        return Err(UpdateError::UnsafeStagedPath);
    }
    let metadata_path = staging.join(STAGED_METADATA_FILE);
    if !metadata_path.exists() {
        return Err(UpdateError::MissingStagedMetadata);
    }
    let metadata: StagedMetadataV2 = serde_json::from_slice(&fs::read(metadata_path)?)?;
    if metadata.schema_version != STAGED_METADATA_SCHEMA_VERSION
        || canonical_artifact
            .file_name()
            .and_then(|value| value.to_str())
            != Some(metadata.artifact_file.as_str())
    {
        return Err(UpdateError::UnsafeStagedPath);
    }
    let raw_manifest = fs::read(staging.join(STAGED_MANIFEST_FILE))?;
    let raw_sigstore_bundle = fs::read(staging.join(STAGED_SIGSTORE_BUNDLE_FILE))?;
    let trusted_root = staged_sigstore_trusted_root()?;
    let verified = verify_release(&raw_manifest, &raw_sigstore_bundle, "stable", &trusted_root)?;
    if metadata.version != verified.manifest.version
        || metadata.build != verified.manifest.build
        || metadata.source_commit != verified.manifest.source_commit
    {
        return Err(UpdateError::InvalidManifest(
            "staged metadata does not match the signed release".into(),
        ));
    }
    let (platform, architecture) = host_platform();
    let selected = select_update(
        &verified,
        current_version(),
        current_build(),
        platform,
        architecture,
    )?
    .artifact
    .ok_or_else(|| UpdateError::InvalidManifest("the staged release is not an update".into()))?;
    if selected != metadata.artifact {
        return Err(UpdateError::InvalidManifest(
            "staged artifact does not match the signed platform selection".into(),
        ));
    }
    verify_file(&canonical_artifact, &selected)?;
    Ok(StagedUpdate {
        version: metadata.version,
        build: metadata.build,
        artifact_path: canonical_artifact,
        size: selected.size,
        sha256: selected.sha256,
        source_commit: metadata.source_commit,
    })
}

fn verify_file(path: &Path, artifact: &ReleaseArtifactV1) -> Result<(), UpdateError> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size.saturating_add(count as u64);
        if size > artifact.size {
            return Err(UpdateError::ArtifactTooLarge);
        }
        hash.update(&buffer[..count]);
    }
    if size != artifact.size {
        return Err(UpdateError::SizeMismatch {
            expected: artifact.size,
            actual: size,
        });
    }
    if !hex::encode(hash.finalize()).eq_ignore_ascii_case(&artifact.sha256) {
        return Err(UpdateError::ChecksumMismatch);
    }
    Ok(())
}

pub fn install(staged_path: &Path, parent_pid: u32) -> Result<StagedUpdate, UpdateError> {
    let staged = validate_staged(staged_path)?;
    let current_executable = std::env::current_exe()?;
    #[cfg(target_os = "macos")]
    validate_macos_install_location(&current_executable)?;
    // A new helper run owns the one-shot result mailbox. Failure to clear it
    // is reported while the desktop is still running, before any quit.
    clear_updater_result()?;
    let helper_name = if cfg!(target_os = "windows") {
        "codecaddie-updater.exe"
    } else {
        "codecaddie-updater"
    };
    let helper = current_executable.with_file_name(helper_name);
    if !helper.is_file() {
        return Err(UpdateError::MissingUpdater(helper));
    }
    // Windows Installer cannot replace an updater executable that is running
    // from the installation directory, so launch a verified copy beside the
    // staged artifact. The next update transaction replaces this fixed copy.
    let launch_helper = if cfg!(target_os = "windows") {
        let external = staging_directory()?.join("codecaddie-updater-external.exe");
        if external.exists() {
            fs::remove_file(&external)?;
        }
        fs::copy(&helper, &external)?;
        external
    } else {
        helper
    };
    let mut command = Command::new(launch_helper);
    command
        .arg("--artifact")
        .arg(&staged.artifact_path)
        .arg("--parent-pid")
        .arg(parent_pid.to_string())
        .arg("--current-executable")
        .arg(&current_executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn()?;
    Ok(staged)
}

pub fn update_summary() -> BTreeMap<&'static str, serde_json::Value> {
    let mut summary = BTreeMap::new();
    summary.insert("version", current_version().into());
    summary.insert("build", current_build().into());
    summary.insert("commit", current_commit().into());
    summary.insert(
        "channel",
        match RuntimeChannel::detect() {
            RuntimeChannel::Stable => "stable",
            RuntimeChannel::Development => "dev",
        }
        .into(),
    );
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    const PUBLIC_FIXTURE_COMMIT: &str = "193b5bd7d3985809503963ae400594ea16df31cf";
    const PUBLIC_FIXTURE_IDENTITY: &str = "https://github.com/prefix-dev/sigstore-example/.github/workflows/action.yaml@refs/heads/main";

    fn manifest_fixture() -> ReleaseManifestV2 {
        let artifact_bytes = b"signed update";
        ReleaseManifestV2 {
            schema_version: MANIFEST_SCHEMA_VERSION,
            channel: "stable".into(),
            version: "0.3.1".into(),
            build: 42,
            published_at: "2026-08-07T12:00:00Z".into(),
            release_notes_url: "https://github.com/tailored-ai-solutions/codecaddie/releases/tag/v0.3.1".into(),
            minimum_supported_version: "0.3.0".into(),
            required: false,
            source_repository: SOURCE_REPOSITORY.into(),
            source_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            artifacts: vec![ReleaseArtifactV1 {
                platform: "macos".into(),
                architecture: "arm64".into(),
                format: "zip".into(),
                url: "https://github.com/tailored-ai-solutions/codecaddie/releases/download/v0.3.1/CodeCaddie.zip".into(),
                size: artifact_bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(artifact_bytes)),
            }],
        }
    }

    fn verified_release_fixture() -> VerifiedRelease {
        let manifest = manifest_fixture();
        VerifiedRelease {
            raw_manifest: serde_json::to_vec(&manifest).unwrap(),
            raw_sigstore_bundle: Vec::new(),
            manifest,
        }
    }

    // Public upstream interop fixture from sigstore-verify 0.11.0, produced by
    // prefix-dev/sigstore-example's GitHub Actions workflow.
    fn public_sigstore_bundle() -> SigstoreBundle {
        SigstoreBundle::from_json(include_str!("testdata/public-github-actions.sigstore.json"))
            .unwrap()
    }

    fn public_sigstore_payload() -> Vec<u8> {
        STANDARD
            .decode(
                include_str!("testdata/public-github-actions-payload.b64")
                    .split_whitespace()
                    .collect::<String>(),
            )
            .unwrap()
    }

    fn public_identity_policy() -> SigstoreIdentityPolicy {
        SigstoreIdentityPolicy {
            issuer: SIGSTORE_OIDC_ISSUER.into(),
            identity: PUBLIC_FIXTURE_IDENTITY.into(),
            repository: "prefix-dev/sigstore-example".into(),
            repository_uri: "https://github.com/prefix-dev/sigstore-example".into(),
            repository_id: "1048392115".into(),
            source_ref: SOURCE_REPOSITORY_REF.into(),
            source_commit: PUBLIC_FIXTURE_COMMIT.into(),
            build_triggers: RELEASE_BUILD_TRIGGERS
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    fn production_root() -> TrustedRoot {
        TrustedRoot::from_json(SIGSTORE_PRODUCTION_TRUSTED_ROOT).unwrap()
    }

    fn assert_invalid_signature(result: Result<(), UpdateError>) {
        assert!(matches!(result, Err(UpdateError::InvalidSignature)));
    }

    #[test]
    fn sigstore_bundle_verifies_exact_payload_and_full_log_material() {
        let bundle = public_sigstore_bundle();
        let payload = public_sigstore_payload();
        let policy = public_identity_policy();
        let root = production_root();

        verify_sigstore_bundle_integrity(&payload, &bundle, &root, &policy).unwrap();
        // This public GitHub Actions fixture is a DSSE attestation. It proves
        // the production Fulcio/Rekor/TUF path, but the updater deliberately
        // accepts only cosign sign-blob message signatures for manifests.
        assert_invalid_signature(verify_sigstore_bundle_crypto(
            &payload, &bundle, &root, &policy,
        ));

        let mut tampered = payload.clone();
        tampered[0] ^= 1;
        assert_invalid_signature(verify_sigstore_bundle_integrity(
            &tampered, &bundle, &root, &policy,
        ));

        let mut no_proof = bundle.clone();
        no_proof.verification_material.tlog_entries[0].inclusion_proof = None;
        assert_invalid_signature(verify_sigstore_bundle_integrity(
            &payload, &no_proof, &root, &policy,
        ));

        let mut no_promise = bundle.clone();
        no_promise.verification_material.tlog_entries[0].inclusion_promise = None;
        assert_invalid_signature(verify_sigstore_bundle_integrity(
            &payload,
            &no_promise,
            &root,
            &policy,
        ));

        let mut alternate_media_type = bundle.clone();
        alternate_media_type.media_type =
            "application/vnd.dev.sigstore.bundle+json;version=0.3".into();
        assert_invalid_signature(verify_sigstore_bundle_integrity(
            &payload,
            &alternate_media_type,
            &root,
            &policy,
        ));

        let mut ambiguous_log = bundle.clone();
        let duplicate = ambiguous_log.verification_material.tlog_entries[0].clone();
        ambiguous_log
            .verification_material
            .tlog_entries
            .push(duplicate);
        assert_invalid_signature(verify_sigstore_bundle_integrity(
            &payload,
            &ambiguous_log,
            &root,
            &policy,
        ));

        let mut wrong_identity = policy.clone();
        wrong_identity.identity =
            "https://github.com/example/other/.github/workflows/release.yml@refs/heads/main".into();
        assert_invalid_signature(verify_sigstore_bundle_integrity(
            &payload,
            &bundle,
            &root,
            &wrong_identity,
        ));
    }

    #[test]
    fn github_certificate_claims_pin_every_repository_workflow_and_commit_dimension() {
        let claims = sigstore_certificate_claims(&public_sigstore_bundle()).unwrap();
        let policy = public_identity_policy();
        verify_sigstore_certificate_claims(&claims, &policy).unwrap();

        macro_rules! rejects_field {
            ($field:ident, $value:expr) => {{
                let mut altered = claims.clone();
                altered.$field = $value.into();
                assert_invalid_signature(verify_sigstore_certificate_claims(&altered, &policy));
            }};
        }

        rejects_field!(issuer, "https://issuer.example.invalid");
        rejects_field!(issuer_v2, "https://issuer.example.invalid");
        rejects_field!(workflow_sha, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        rejects_field!(
            build_signer_digest,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        rejects_field!(source_commit, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        rejects_field!(
            build_config_digest,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        rejects_field!(repository, "prefix-dev/sigstore-example-lookalike");
        rejects_field!(
            repository_uri,
            "https://github.com/prefix-dev/sigstore-example-lookalike"
        );
        rejects_field!(repository_id, "9999999999");
        rejects_field!(source_ref, "refs/heads/release");
        rejects_field!(source_ref_v2, "refs/heads/release");
        rejects_field!(
            build_signer_uri,
            "https://github.com/prefix-dev/sigstore-example/.github/workflows/other.yml@refs/heads/main"
        );
        rejects_field!(
            build_config_uri,
            "https://github.com/prefix-dev/sigstore-example/.github/workflows/other.yml@refs/heads/main"
        );
    }

    #[test]
    fn github_build_trigger_has_an_exact_two_value_allowlist() {
        let claims = sigstore_certificate_claims(&public_sigstore_bundle()).unwrap();
        let policy = public_identity_policy();
        assert_eq!(
            policy.build_triggers,
            BTreeSet::from(["push".to_owned(), "workflow_dispatch".to_owned()])
        );
        assert_eq!(claims.build_trigger, "push");
        verify_sigstore_certificate_claims(&claims, &policy).unwrap();

        let mut manual = claims.clone();
        manual.build_trigger = "workflow_dispatch".into();
        verify_sigstore_certificate_claims(&manual, &policy).unwrap();

        for untrusted in [
            "pull_request",
            "pull_request_target",
            "schedule",
            "workflow_call",
        ] {
            let mut altered = claims.clone();
            altered.build_trigger = untrusted.into();
            assert_invalid_signature(verify_sigstore_certificate_claims(&altered, &policy));
        }
    }

    #[test]
    fn embedded_identity_policy_matches_the_tracked_release_trust_config() {
        let tracked: serde_json::Value =
            serde_json::from_str(include_str!("../../../config/release-trust.json")).unwrap();
        let sigstore = &tracked["sigstore"];
        assert_eq!(tracked["schemaVersion"], MANIFEST_SCHEMA_VERSION);
        assert_eq!(sigstore["bundleMediaType"], SIGSTORE_BUNDLE_MEDIA_TYPE);
        assert_eq!(sigstore["oidcIssuer"], SIGSTORE_OIDC_ISSUER);
        assert_eq!(sigstore["certificateIdentity"], RELEASE_WORKFLOW_IDENTITY);
        assert_eq!(sigstore["repository"], SOURCE_REPOSITORY);
        assert_eq!(sigstore["workflowRef"], SOURCE_REPOSITORY_REF);
        assert_eq!(sigstore["tufMirror"], SIGSTORE_TUF_URL);
        let triggers: BTreeSet<_> = sigstore["allowedTriggers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(triggers, BTreeSet::from(RELEASE_BUILD_TRIGGERS));
    }

    #[test]
    fn repository_id_build_pin_rejects_placeholders_and_zero() {
        assert!(valid_pinned_repository_id("1048392115"));
        for invalid in [
            "",
            "0",
            "-1",
            "REPLACE_WITH_PUBLIC_REPOSITORY_ID",
            "1048392115 ",
            "1.5",
        ] {
            assert!(!valid_pinned_repository_id(invalid));
        }
    }

    #[tokio::test]
    async fn invalid_cached_tuf_metadata_is_rejected_before_embedded_root_fallback() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("timestamp.json"),
            b"not trusted metadata",
        )
        .unwrap();
        let cached = cached_sigstore_trusted_root(directory.path().to_path_buf()).await;
        assert!(matches!(cached, Err(UpdateError::TrustMetadataUnavailable)));

        let root = select_sigstore_trusted_root(Err(UpdateError::TrustMetadataUnavailable), cached)
            .unwrap();
        verify_sigstore_bundle_integrity(
            &public_sigstore_payload(),
            &public_sigstore_bundle(),
            &root,
            &public_identity_policy(),
        )
        .unwrap();
        assert!(SIGSTORE_TUF_REFRESH_TIMEOUT <= std::time::Duration::from_secs(5));
    }

    #[test]
    fn manifest_schema_two_requires_keyless_source_identity_and_rejects_hsm_fields() {
        let manifest = manifest_fixture();
        validate_manifest(&manifest).unwrap();

        let mut missing_source_commit = serde_json::to_value(&manifest).unwrap();
        missing_source_commit
            .as_object_mut()
            .unwrap()
            .remove("sourceCommit");
        assert!(serde_json::from_value::<ReleaseManifestV2>(missing_source_commit).is_err());

        for legacy_field in ["keyId", "trustPolicy", "signature"] {
            let mut legacy = serde_json::to_value(&manifest).unwrap();
            legacy[legacy_field] = serde_json::Value::String("legacy".into());
            assert!(serde_json::from_value::<ReleaseManifestV2>(legacy).is_err());
        }

        let mut wrong_schema = manifest.clone();
        wrong_schema.schema_version = 1;
        assert!(validate_manifest(&wrong_schema).is_err());

        let mut wrong_repository = manifest.clone();
        wrong_repository.source_repository = "tailored-ai-solutions/codecaddie-lookalike".into();
        assert!(validate_manifest(&wrong_repository).is_err());

        for invalid_commit in [
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "gggggggggggggggggggggggggggggggggggggggg",
        ] {
            let mut invalid = manifest.clone();
            invalid.source_commit = invalid_commit.into();
            assert!(validate_manifest(&invalid).is_err());
        }
    }

    #[test]
    fn manifests_reject_incoherent_versions_and_ambiguous_artifacts() {
        let mut minimum_too_new = manifest_fixture();
        minimum_too_new.minimum_supported_version = "0.4.0".into();
        assert!(validate_manifest(&minimum_too_new).is_err());

        let mut duplicate = manifest_fixture();
        duplicate.artifacts.push(duplicate.artifacts[0].clone());
        assert!(validate_manifest(&duplicate).is_err());

        let mut build_metadata = manifest_fixture();
        build_metadata.version = "0.3.1+unsigned".into();
        assert!(validate_manifest(&build_metadata).is_err());

        let mut noncanonical_sha = manifest_fixture();
        noncanonical_sha.artifacts[0].sha256 =
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into();
        assert!(validate_manifest(&noncanonical_sha).is_err());
    }

    #[test]
    fn bundle_url_is_derived_from_the_original_release_url_not_an_opaque_redirect() {
        let original = reqwest::Url::parse(DEFAULT_STABLE_MANIFEST_URL).unwrap();
        let opaque_final_url = reqwest::Url::parse(
            "https://objects.githubusercontent.com/github-production-release-asset/opaque-token?x=1",
        )
        .unwrap();
        assert!(!opaque_final_url.path().ends_with("/manifest.json"));

        assert_eq!(
            sibling_sigstore_bundle_url(&original).unwrap().as_str(),
            "https://github.com/tailored-ai-solutions/codecaddie/releases/latest/download/manifest.sigstore.json"
        );

        let with_query = reqwest::Url::parse(
            "https://github.com/tailored-ai-solutions/codecaddie/releases/latest/download/manifest.json?cache=ignored#fragment",
        )
        .unwrap();
        assert_eq!(
            sibling_sigstore_bundle_url(&with_query).unwrap().as_str(),
            "https://github.com/tailored-ai-solutions/codecaddie/releases/latest/download/manifest.sigstore.json"
        );
        assert!(sibling_sigstore_bundle_url(&opaque_final_url).is_err());
    }

    #[test]
    fn selects_exact_platform_rejects_downgrades_and_enforces_required_releases() {
        let release = verified_release_fixture();
        let update = select_update(&release, "0.3.0", 41, "macos", "arm64").unwrap();
        assert!(update.available);
        assert!(!update.required);
        assert_eq!(update.artifact.unwrap().format, "zip");

        assert!(matches!(
            select_update(&release, "0.4.0", 1, "macos", "arm64"),
            Err(UpdateError::Downgrade { .. })
        ));
        assert!(matches!(
            select_update(&release, "0.3.0", 1, "macos", "x64"),
            Err(UpdateError::UnsupportedPlatform { .. })
        ));

        let mut explicitly_required = verified_release_fixture();
        explicitly_required.manifest.required = true;
        assert!(
            select_update(&explicitly_required, "0.3.0", 41, "macos", "arm64")
                .unwrap()
                .required
        );
        assert!(
            !select_update(&explicitly_required, "0.3.1", 42, "macos", "arm64")
                .unwrap()
                .required
        );

        assert!(
            select_update(&release, "0.2.9", 99, "macos", "arm64")
                .unwrap()
                .required
        );
    }

    #[test]
    fn same_version_requires_a_monotonic_build() {
        let release = verified_release_fixture();
        assert!(
            !select_update(&release, "0.3.1", 42, "macos", "arm64")
                .unwrap()
                .available
        );
        assert!(
            select_update(&release, "0.3.1", 41, "macos", "arm64")
                .unwrap()
                .available
        );
    }

    #[test]
    fn artifact_tampering_fails_closed() {
        let release = verified_release_fixture();
        let directory = tempfile::tempdir().unwrap();
        let artifact_path = directory.path().join("CodeCaddie.zip");
        fs::write(&artifact_path, b"signed update").unwrap();
        verify_file(&artifact_path, &release.manifest.artifacts[0]).unwrap();
        fs::write(&artifact_path, b"tampered data").unwrap();
        assert!(matches!(
            verify_file(&artifact_path, &release.manifest.artifacts[0]),
            Err(UpdateError::ChecksumMismatch)
        ));
    }

    #[test]
    fn macos_preflight_rejects_mounted_translocated_and_unwritable_locations() {
        let never_probe = |_: &Path| -> std::io::Result<()> {
            panic!("unsafe locations must fail before the write probe")
        };
        assert!(matches!(
            validate_macos_install_location_with(
                Path::new("/Volumes/CodeCaddie/CodeCaddie.app/Contents/MacOS/codecaddie-core"),
                never_probe,
            ),
            Err(UpdateError::MacAppOnMountedVolume)
        ));
        assert!(matches!(
            validate_macos_install_location_with(
                Path::new(
                    "/private/var/folders/example/AppTranslocation/CodeCaddie.app/Contents/MacOS/codecaddie-core"
                ),
                never_probe,
            ),
            Err(UpdateError::MacAppTranslocated)
        ));
        assert!(matches!(
            validate_macos_install_location_with(
                Path::new("/Applications/CodeCaddie.app/Contents/MacOS/codecaddie-core"),
                |_| Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "test"
                )),
            ),
            Err(UpdateError::MacDestinationNotWritable)
        ));

        let mut probed = None;
        validate_macos_install_location_with(
            Path::new("/Applications/CodeCaddie.app/Contents/MacOS/codecaddie-core"),
            |parent| {
                probed = Some(parent.to_path_buf());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(probed.unwrap(), Path::new("/Applications"));
    }

    #[test]
    fn updater_result_is_bounded_private_and_consumed_once() {
        let directory = tempfile::tempdir().unwrap();
        record_updater_result_at(directory.path(), UpdaterResultCode::InstallFailed).unwrap();
        let path = updater_result_path(directory.path());
        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.len() <= MAX_UPDATER_RESULT_BYTES);
        #[cfg(unix)]
        {
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let stored = fs::read_to_string(&path).unwrap();
        assert!(!stored.contains("repository"));
        assert_eq!(
            take_updater_result_at(directory.path()).unwrap(),
            Some(UpdaterResultV1::failed(UpdaterResultCode::InstallFailed))
        );
        assert!(!path.exists());
        assert_eq!(take_updater_result_at(directory.path()).unwrap(), None);
    }

    #[test]
    fn updater_result_replacement_preserves_the_prior_code_across_a_write_fault() {
        use crate::persistence::{FailOnce, PersistenceBoundary};

        let directory = tempfile::tempdir().unwrap();
        record_updater_result_at(directory.path(), UpdaterResultCode::InstallFailed).unwrap();
        let interrupted = FailOnce::new(PersistenceBoundary::TemporaryFileSynced);
        assert!(
            record_updater_result_at_with(
                directory.path(),
                UpdaterResultCode::ReopenFailed,
                &interrupted,
            )
            .is_err()
        );
        let retained: UpdaterResultV1 =
            serde_json::from_slice(&fs::read(updater_result_path(directory.path())).unwrap())
                .unwrap();
        assert_eq!(
            retained,
            UpdaterResultV1::failed(UpdaterResultCode::InstallFailed)
        );

        record_updater_result_at(directory.path(), UpdaterResultCode::ReopenFailed).unwrap();
        assert_eq!(
            take_updater_result_at(directory.path()).unwrap(),
            Some(UpdaterResultV1::failed(UpdaterResultCode::ReopenFailed))
        );
    }

    #[test]
    fn malformed_updater_result_is_consumed_without_surfacing_its_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = updater_result_path(directory.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"{"schemaVersion":1,"status":"failed","code":"unknown","message":"PRIVATE SOURCE CANARY"}"#,
        )
        .unwrap();
        assert!(matches!(
            take_updater_result_at(directory.path()),
            Err(UpdateError::InvalidUpdaterResult)
        ));
        assert!(!path.exists());
    }

    #[test]
    fn invalid_updater_result_directory_is_removed_instead_of_warning_forever() {
        let directory = tempfile::tempdir().unwrap();
        let path = updater_result_path(directory.path());
        fs::create_dir_all(path.join("nested")).unwrap();
        fs::write(path.join("nested/private-source-canary"), b"untrusted").unwrap();
        assert!(matches!(
            take_updater_result_at(directory.path()),
            Err(UpdateError::InvalidUpdaterResult)
        ));
        assert!(!path.exists());
        assert_eq!(take_updater_result_at(directory.path()).unwrap(), None);
    }
}
