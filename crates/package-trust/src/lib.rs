use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore_tuf::{DelegatedRole, Error as TufError, TrustedMetadataSet};

pub const TRUST_STATE_FORMAT: &str = "brickpro-trusted-metadata-state";
pub const TRUST_STATE_VERSION: u8 = 1;
pub const MAX_CLOCK_UNCERTAINTY_SECONDS: u64 = 300;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryStatus {
    SignatureFailure,
    Expired,
    Rollback,
    Freeze,
    ClockUncertain,
    CorruptTrustedState,
    Unsupported,
}

impl RecoveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SignatureFailure => "signature-failure",
            Self::Expired => "expired",
            Self::Rollback => "rollback",
            Self::Freeze => "freeze",
            Self::ClockUncertain => "clock-uncertainty",
            Self::CorruptTrustedState => "corrupt-trusted-state",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug)]
pub struct TrustError {
    pub status: RecoveryStatus,
    pub message: String,
}

impl TrustError {
    fn new(status: RecoveryStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TrustError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.status.as_str(), self.message)
    }
}

impl std::error::Error for TrustError {}

pub type Result<T> = std::result::Result<T, TrustError>;

#[derive(Clone, Copy, Debug)]
pub struct VerificationTime<'a> {
    pub now_rfc3339: &'a str,
    pub uncertainty_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedMetadataState {
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "rootVersion")]
    pub root_version: u64,
    #[serde(rename = "timestampVersion")]
    pub timestamp_version: u64,
    #[serde(rename = "snapshotVersion")]
    pub snapshot_version: u64,
    #[serde(rename = "targetsVersion")]
    pub targets_version: u64,
    pub delegated: BTreeMap<String, u64>,
}

impl Default for TrustedMetadataState {
    fn default() -> Self {
        Self {
            format: TRUST_STATE_FORMAT.to_string(),
            schema_version: TRUST_STATE_VERSION,
            root_version: 0,
            timestamp_version: 0,
            snapshot_version: 0,
            targets_version: 0,
            delegated: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTarget {
    pub path: String,
    pub length: u64,
    pub sha256: String,
    pub delegated_role: String,
}

#[derive(Clone, Debug)]
pub struct VerificationReport {
    pub root_version: u64,
    pub timestamp_version: u64,
    pub snapshot_version: u64,
    pub targets_version: u64,
    pub target: VerifiedTarget,
}

pub struct RepositoryMetadata<'a> {
    pub root_bytes: &'a [u8],
    pub root_updates: &'a [&'a [u8]],
    pub timestamp_bytes: &'a [u8],
    pub snapshot_bytes: &'a [u8],
    pub targets_bytes: &'a [u8],
    pub delegated_role: &'a str,
    pub delegated_bytes: &'a [u8],
    pub target_bytes: &'a [u8],
}

pub struct TrustStore<'a> {
    state_path: &'a Path,
    temp_path: Option<PathBuf>,
    fail_before_publish: bool,
}

impl<'a> TrustStore<'a> {
    pub fn new(state_path: &'a Path) -> Self {
        Self {
            state_path,
            temp_path: None,
            fail_before_publish: false,
        }
    }

    pub fn with_temp_path(mut self, temp_path: &Path) -> Self {
        self.temp_path = Some(temp_path.to_path_buf());
        self
    }

    pub fn with_publication_failure(mut self) -> Self {
        self.fail_before_publish = true;
        self
    }

    pub fn verify_repository(
        &self,
        metadata: RepositoryMetadata<'_>,
        target_path: &str,
        time: VerificationTime<'_>,
    ) -> Result<VerificationReport> {
        validate_time(time)?;
        validate_target_path(target_path)?;
        if metadata.delegated_role.is_empty()
            || matches!(
                metadata.delegated_role,
                "root" | "timestamp" | "snapshot" | "targets"
            )
        {
            return Err(TrustError::new(
                RecoveryStatus::Unsupported,
                "delegated role is not package-scoped",
            ));
        }
        let previous = self.load_state()?;
        let now = time
            .now_rfc3339
            .parse::<jiff::Timestamp>()
            .map_err(|error| {
                TrustError::new(
                    RecoveryStatus::ClockUncertain,
                    format!("invalid verification time: {error}"),
                )
            })?;

        let mut trusted =
            TrustedMetadataSet::from_root(metadata.root_bytes).map_err(map_tuf_error)?;
        validate_root_policy(trusted.root())?;
        if trusted.root().version < previous.root_version {
            return Err(TrustError::new(
                RecoveryStatus::Rollback,
                "bootstrap root is older than persisted trusted root",
            ));
        }
        for update in metadata.root_updates {
            trusted.update_root(update).map_err(map_tuf_error)?;
            validate_root_policy(trusted.root())?;
        }
        trusted.check_root_expired(now).map_err(map_tuf_error)?;
        let root_version = trusted.root().version;
        if root_version < previous.root_version {
            return Err(TrustError::new(
                RecoveryStatus::Rollback,
                "root version moved backwards",
            ));
        }

        trusted
            .update_timestamp(metadata.timestamp_bytes, now)
            .map_err(map_tuf_error)?;
        trusted
            .update_snapshot(metadata.snapshot_bytes, now)
            .map_err(map_tuf_error)?;
        let top_targets = trusted
            .update_targets(metadata.targets_bytes, now)
            .map_err(map_tuf_error)?
            .clone();
        trusted
            .check_timestamp_expired(now)
            .map_err(map_tuf_error)?;
        trusted.check_snapshot_expired(now).map_err(map_tuf_error)?;
        let delegation = top_targets
            .delegations
            .as_ref()
            .and_then(|delegations| {
                delegations
                    .roles
                    .iter()
                    .find(|role| role.name == metadata.delegated_role)
            })
            .cloned()
            .ok_or_else(|| {
                TrustError::new(
                    RecoveryStatus::SignatureFailure,
                    "delegated role is not authorized by targets",
                )
            })?;
        if !delegation_matches(&delegation, target_path)? {
            return Err(TrustError::new(
                RecoveryStatus::SignatureFailure,
                "target is outside delegated path scope",
            ));
        }
        let delegated = trusted
            .update_delegated_targets(
                metadata.delegated_bytes,
                metadata.delegated_role,
                "targets",
                now,
            )
            .map_err(map_tuf_error)?
            .clone();
        let target = delegated.target(target_path).ok_or_else(|| {
            TrustError::new(
                RecoveryStatus::CorruptTrustedState,
                "delegated metadata does not list target",
            )
        })?;
        let sha256 = target.hashes.get("sha256").ok_or_else(|| {
            TrustError::new(
                RecoveryStatus::Unsupported,
                "target has no SHA-256 integrity pin",
            )
        })?;
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(TrustError::new(
                RecoveryStatus::CorruptTrustedState,
                "target SHA-256 is malformed",
            ));
        }
        let report = VerificationReport {
            root_version,
            timestamp_version: trusted
                .timestamp()
                .ok_or_else(|| missing_role("timestamp"))?
                .version,
            snapshot_version: trusted
                .snapshot()
                .ok_or_else(|| missing_role("snapshot"))?
                .version,
            targets_version: top_targets.version,
            target: VerifiedTarget {
                path: target_path.to_string(),
                length: target.length,
                sha256: sha256.clone(),
                delegated_role: metadata.delegated_role.to_string(),
            },
        };
        if report.timestamp_version < previous.timestamp_version
            || report.snapshot_version < previous.snapshot_version
            || report.targets_version < previous.targets_version
            || delegated.version
                < previous
                    .delegated
                    .get(metadata.delegated_role)
                    .copied()
                    .unwrap_or(0)
        {
            return Err(TrustError::new(
                RecoveryStatus::Rollback,
                "metadata version moved backwards",
            ));
        }
        self.verify_target_bytes(&report.target, metadata.target_bytes)?;
        let mut state = previous;
        state.root_version = report.root_version;
        state.timestamp_version = report.timestamp_version;
        state.snapshot_version = report.snapshot_version;
        state.targets_version = report.targets_version;
        state
            .delegated
            .insert(metadata.delegated_role.to_string(), delegated.version);
        self.save_state(&state)?;
        Ok(report)
    }

    pub fn verify_target_bytes(&self, target: &VerifiedTarget, bytes: &[u8]) -> Result<()> {
        if bytes.len() as u64 != target.length {
            return Err(TrustError::new(
                RecoveryStatus::CorruptTrustedState,
                "target length differs from signed metadata",
            ));
        }
        let actual = hex_digest(bytes);
        if actual != target.sha256 {
            return Err(TrustError::new(
                RecoveryStatus::CorruptTrustedState,
                "target SHA-256 differs from signed metadata",
            ));
        }
        Ok(())
    }

    fn load_state(&self) -> Result<TrustedMetadataState> {
        reject_symlink(self.state_path, "trusted state")?;
        if !self.state_path.exists() {
            return Ok(TrustedMetadataState::default());
        }
        let bytes = fs::read(self.state_path).map_err(|error| corrupt_state(error.to_string()))?;
        let state: TrustedMetadataState =
            serde_json::from_slice(&bytes).map_err(|error| corrupt_state(error.to_string()))?;
        if state.format != TRUST_STATE_FORMAT
            || state.schema_version != TRUST_STATE_VERSION
            || state.root_version == 0
                && (state.timestamp_version != 0
                    || state.snapshot_version != 0
                    || state.targets_version != 0)
            || state
                .delegated
                .keys()
                .any(|role| role.is_empty() || role.contains('/'))
        {
            return Err(corrupt_state(
                "trusted state has unsupported fields or versions",
            ));
        }
        Ok(state)
    }

    fn save_state(&self, state: &TrustedMetadataState) -> Result<()> {
        let parent = self
            .state_path
            .parent()
            .ok_or_else(|| corrupt_state("trusted state has no parent"))?;
        fs::create_dir_all(parent).map_err(|error| corrupt_state(error.to_string()))?;
        reject_symlink(parent, "trusted state parent")?;
        reject_symlink(self.state_path, "trusted state")?;
        let (temp, mut file) = self.create_temp(parent)?;
        let bytes =
            serde_json::to_vec_pretty(state).map_err(|error| corrupt_state(error.to_string()))?;
        let result = (|| {
            file.write_all(&bytes)
                .map_err(|error| corrupt_state(error.to_string()))?;
            file.sync_all()
                .map_err(|error| corrupt_state(error.to_string()))?;
            if self.fail_before_publish {
                return Err(corrupt_state("simulated interrupted state publication"));
            }
            fs::rename(&temp, self.state_path).map_err(|error| corrupt_state(error.to_string()))?;
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| corrupt_state(error.to_string()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    fn create_temp(&self, parent: &Path) -> Result<(PathBuf, File)> {
        if let Some(path) = &self.temp_path {
            if path.parent() != Some(parent) {
                return Err(corrupt_state("state temp path is not same-directory"));
            }
            reject_symlink(path, "trusted state temp")?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| corrupt_state(error.to_string()))?;
            return Ok((path.clone(), file));
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..16u32 {
            let path = parent.join(format!(
                ".trusted-state.tmp.{}.{}.{}",
                std::process::id(),
                stamp,
                attempt
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(corrupt_state(error.to_string())),
            }
        }
        Err(corrupt_state(
            "could not allocate collision-safe trusted state temp",
        ))
    }
}

fn validate_time(time: VerificationTime<'_>) -> Result<()> {
    if time.uncertainty_seconds > MAX_CLOCK_UNCERTAINTY_SECONDS {
        return Err(TrustError::new(
            RecoveryStatus::ClockUncertain,
            "clock uncertainty exceeds 300 seconds",
        ));
    }
    Ok(())
}

fn validate_root_policy(root: &sigstore_tuf::Root) -> Result<()> {
    if root.type_ != "root"
        || root.spec_version.split('.').next() != Some("1")
        || !root.consistent_snapshot
    {
        return Err(TrustError::new(
            RecoveryStatus::Unsupported,
            "root must use TUF 1.x consistent snapshots",
        ));
    }
    let expected = ["root", "timestamp", "snapshot", "targets"];
    if root.roles.len() != expected.len()
        || expected.iter().any(|role| !root.roles.contains_key(*role))
    {
        return Err(TrustError::new(
            RecoveryStatus::Unsupported,
            "root must declare exactly the four top-level roles",
        ));
    }
    if root
        .roles
        .values()
        .any(|role| role.threshold == 0 || role.keyids.is_empty())
    {
        return Err(TrustError::new(
            RecoveryStatus::SignatureFailure,
            "top-level role threshold is empty",
        ));
    }
    Ok(())
}

fn delegation_matches(role: &DelegatedRole, path: &str) -> Result<bool> {
    role.matches_path(path)
        .map_err(|error| TrustError::new(RecoveryStatus::Unsupported, error.to_string()))
}

fn validate_target_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err(TrustError::new(
            RecoveryStatus::CorruptTrustedState,
            "target path is not relative POSIX",
        ));
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(TrustError::new(
            RecoveryStatus::CorruptTrustedState,
            "target path is not normalized",
        ));
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn missing_role(role: &str) -> TrustError {
    TrustError::new(
        RecoveryStatus::CorruptTrustedState,
        format!("trusted {role} metadata is missing"),
    )
}

fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(corrupt_state(format!("{label} is a symlink")));
    }
    Ok(())
}

fn corrupt_state(message: impl Into<String>) -> TrustError {
    TrustError::new(RecoveryStatus::CorruptTrustedState, message)
}

fn map_tuf_error(error: TufError) -> TrustError {
    let status = match error {
        TufError::Expired { .. } => RecoveryStatus::Expired,
        TufError::Rollback { .. }
        | TufError::EqualVersion { .. }
        | TufError::BadRootVersion { .. } => RecoveryStatus::Rollback,
        TufError::ThresholdNotMet { .. }
        | TufError::DuplicateSignature { .. }
        | TufError::InvalidSignatureEncoding { .. }
        | TufError::Crypto(_)
        | TufError::UnusableKey { .. } => RecoveryStatus::SignatureFailure,
        TufError::IntegrityMismatch(_) => RecoveryStatus::Freeze,
        TufError::UnsupportedScheme { .. } => RecoveryStatus::Unsupported,
        _ => RecoveryStatus::CorruptTrustedState,
    };
    TrustError::new(status, error.to_string())
}

impl From<io::Error> for TrustError {
    fn from(error: io::Error) -> Self {
        corrupt_state(error.to_string())
    }
}
