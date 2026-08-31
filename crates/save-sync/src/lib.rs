use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use save_vault::{Identity, MaterialFile, SaveKind, SaveVault, SnapshotReason};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod syncthing;
pub mod webdav;

pub const SCHEMA: &str = "https://example.invalid/trimui-save-sync-v1.schema.json";
pub const FORMAT: &str = "trimui-save-sync";
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ANCESTRY: usize = 32;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Lineage {
    pub parent_hash: Option<String>,
    pub ancestry: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateStatus {
    Candidate,
    Quarantined,
    Conflict,
    Canonical,
    Resolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Candidate {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    pub schema_version: u8,
    pub logical_id: String,
    pub content_id: String,
    pub device: Device,
    pub generation: u64,
    pub hash: String,
    pub lineage: Lineage,
    pub save_kind: SaveKind,
    pub timestamp_ms: u64,
    pub size: u64,
    pub validator: Option<String>,
    pub status: CandidateStatus,
    pub deleted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateView {
    pub logical_id: String,
    pub content_id: String,
    pub device_id: String,
    pub device_name: String,
    pub generation: u64,
    pub hash_prefix: String,
    pub parent_hash_prefix: Option<String>,
    pub ancestry: Vec<String>,
    pub save_kind: SaveKind,
    pub timestamp_ms: u64,
    pub size: u64,
    pub status: CandidateStatus,
    pub deleted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncGate {
    Ready,
    Gameplay,
    SaveFlush,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionAction {
    KeepLocal,
    KeepRemote,
    KeepBoth,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub local: CandidateView,
    pub remote: CandidateView,
    pub state: String,
    pub transport_outcome: String,
    pub actions: Vec<ResolutionAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionReceipt {
    pub action: ResolutionAction,
    pub state: String,
    pub canonical_generation: u64,
    pub preserved_hash_prefixes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveTarget {
    pub logical_id: String,
    pub content_id: String,
    pub relative: String,
    pub kind: SaveKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedCandidate {
    candidate: Candidate,
    payload: Vec<u8>,
    conflict_copy: bool,
}

impl StagedCandidate {
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn is_conflict_copy(&self) -> bool {
        self.conflict_copy
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncError(String);
impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SyncError {}
impl SyncError {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
fn error(message: impl Into<String>) -> SyncError {
    SyncError::message(message)
}

#[derive(Clone)]
pub struct Exchange {
    root: PathBuf,
    directory_mode: u32,
    file_mode: u32,
}

impl Exchange {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, SyncError> {
        Self::new_with_modes(root.into(), 0o700, 0o600)
    }

    pub fn for_simulator(root: impl Into<PathBuf>) -> Result<Self, SyncError> {
        Self::new_with_modes(root.into(), 0o777, 0o644)
    }

    fn new_with_modes(
        root: PathBuf,
        directory_mode: u32,
        file_mode: u32,
    ) -> Result<Self, SyncError> {
        if !root.is_absolute() || root == Path::new("/") {
            return Err(error("exchange root must be absolute and non-root"));
        }
        reject_symlink_components(&root)?;
        if root.exists() && (!root.is_dir() || symlink(&root)) {
            return Err(error("exchange root boundary is invalid"));
        }
        if !root.exists() {
            fs::create_dir_all(&root).map_err(storage_error)?;
        }
        for name in ["outgoing", "incoming", "quarantine", "pending"] {
            let path = root.join(name);
            if path.exists() && (!path.is_dir() || symlink(&path)) {
                return Err(error("exchange directory boundary is invalid"));
            }
            if !path.exists() {
                fs::create_dir(&path).map_err(storage_error)?;
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(directory_mode))
                .map_err(storage_error)?;
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(directory_mode))
            .map_err(storage_error)?;
        Ok(Self {
            root,
            directory_mode,
            file_mode,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn pending_count(&self) -> Result<usize, SyncError> {
        count_dirs(&self.root.join("pending"))
    }

    fn export(&self, candidate: &Candidate, payload: &[u8]) -> Result<PathBuf, SyncError> {
        validate_candidate(candidate)?;
        verify_payload(candidate, payload)?;
        self.write_record("outgoing", candidate, payload, false)
    }

    pub fn stage_remote(
        &self,
        mut candidate: Candidate,
        payload: &[u8],
        conflict_copy: bool,
    ) -> Result<StagedCandidate, SyncError> {
        validate_candidate(&candidate)?;
        verify_payload(&candidate, payload)?;
        candidate.status = CandidateStatus::Quarantined;
        self.write_record("quarantine", &candidate, payload, conflict_copy)?;
        Ok(StagedCandidate {
            candidate,
            payload: payload.to_vec(),
            conflict_copy,
        })
    }

    pub fn enqueue_pending(&self, candidate: &Candidate, payload: &[u8]) -> Result<(), SyncError> {
        validate_candidate(candidate)?;
        verify_payload(candidate, payload)?;
        let path = self
            .root
            .join("pending")
            .join(format!("{}-{}", candidate.hash, candidate.generation));
        if path.exists() {
            if symlink(&path) || !path.is_dir() {
                return Err(error("pending candidate boundary is invalid"));
            }
            return Ok(());
        }
        self.write_record("pending", candidate, payload, false)
            .map(|_| ())
    }

    pub fn quarantined(&self) -> Result<Vec<StagedCandidate>, SyncError> {
        self.read_records("quarantine")
    }

    fn write_record(
        &self,
        bucket: &str,
        candidate: &Candidate,
        payload: &[u8],
        conflict_copy: bool,
    ) -> Result<PathBuf, SyncError> {
        let name = format!("{}-{}", candidate.hash, candidate.generation);
        let directory = self.root.join(bucket).join(name);
        if symlink(&directory) || directory.exists() {
            return Err(error("exchange candidate already exists"));
        }
        fs::create_dir(&directory).map_err(storage_error)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(self.directory_mode))
            .map_err(storage_error)?;
        let result = (|| {
            write_new(
                &directory.join("candidate.json"),
                &serde_json::to_vec_pretty(candidate).map_err(storage_error)?,
                self.file_mode,
            )?;
            write_new(&directory.join("payload.bin"), payload, self.file_mode)?;
            if conflict_copy {
                write_new(
                    &directory.join("syncthing-conflict"),
                    b"conflict-copy\n",
                    self.file_mode,
                )?;
            }
            sync_dir(&directory)?;
            sync_dir(
                directory
                    .parent()
                    .ok_or_else(|| error("exchange candidate has no parent"))?,
            )?;
            Ok(directory.clone())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&directory);
        }
        result
    }

    fn read_records(&self, bucket: &str) -> Result<Vec<StagedCandidate>, SyncError> {
        let mut records = Vec::new();
        for entry in fs::read_dir(self.root.join(bucket)).map_err(storage_error)? {
            let entry = entry.map_err(storage_error)?;
            let directory = entry.path();
            let metadata = fs::symlink_metadata(&directory).map_err(storage_error)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(error("exchange candidate is not a regular directory"));
            }
            let candidate_path = directory.join("candidate.json");
            let payload_path = directory.join("payload.bin");
            for child in fs::read_dir(&directory).map_err(storage_error)? {
                let child = child.map_err(storage_error)?.path();
                let metadata = fs::symlink_metadata(&child).map_err(storage_error)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(error("exchange candidate contains unsafe data"));
                }
            }
            if symlink(&candidate_path) || symlink(&payload_path) {
                return Err(error("exchange candidate contains a symlink"));
            }
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(candidate_path).map_err(storage_error)?)
                    .map_err(|_| error("exchange candidate metadata is invalid"))?;
            let payload = fs::read(payload_path).map_err(storage_error)?;
            validate_candidate(&candidate)?;
            verify_payload(&candidate, &payload)?;
            records.push(StagedCandidate {
                candidate,
                payload,
                conflict_copy: directory.join("syncthing-conflict").is_file(),
            });
        }
        records.sort_by_key(|record| record.candidate.generation);
        Ok(records)
    }
}

pub struct SyncReconciler<'a> {
    vault: &'a SaveVault,
    exchange: Exchange,
    target: SaveTarget,
}

impl<'a> SyncReconciler<'a> {
    pub fn new(
        vault: &'a SaveVault,
        exchange: Exchange,
        target: SaveTarget,
    ) -> Result<Self, SyncError> {
        validate_target(&target)?;
        Ok(Self {
            vault,
            exchange,
            target,
        })
    }

    pub fn exchange(&self) -> &Exchange {
        &self.exchange
    }

    pub fn export_local(&self, device: Device) -> Result<PathBuf, SyncError> {
        let candidate = self.local_candidate(device)?;
        let payload = self
            .vault
            .material(candidate.generation)
            .map_err(vault_error)?
            .into_iter()
            .find(|file| file.relative == self.target.relative)
            .ok_or_else(|| error("canonical save target is unavailable"))?;
        self.exchange.export(&candidate, &payload.bytes)
    }

    pub fn local_candidate(&self, device: Device) -> Result<Candidate, SyncError> {
        let generation = self
            .vault
            .current_generation()
            .ok_or_else(|| error("save vault has no canonical generation"))?;
        let manifest = self
            .vault
            .manifest(generation)
            .ok_or_else(|| error("canonical generation is unavailable"))?;
        let payload = self
            .vault
            .material(generation)
            .map_err(vault_error)?
            .into_iter()
            .find(|file| file.relative == self.target.relative)
            .ok_or_else(|| error("canonical save target is unavailable"))?;
        let parent_hash = manifest.parent_generation.and_then(|parent| {
            self.vault
                .material(parent)
                .ok()?
                .into_iter()
                .find(|file| file.relative == self.target.relative)
                .map(|file| digest(&file.bytes))
        });
        let mut ancestry = Vec::new();
        let mut parent = manifest.parent_generation;
        while let Some(generation) = parent {
            if let Some(file) = self.vault.material(generation).ok().and_then(|files| {
                files
                    .into_iter()
                    .find(|file| file.relative == self.target.relative)
            }) {
                ancestry.push(digest(&file.bytes));
            }
            parent = self
                .vault
                .manifest(generation)
                .and_then(|m| m.parent_generation);
        }
        Ok(Candidate {
            schema: SCHEMA.into(),
            format: FORMAT.into(),
            schema_version: 1,
            logical_id: self.target.logical_id.clone(),
            content_id: self.target.content_id.clone(),
            device,
            generation,
            hash: digest(&payload.bytes),
            lineage: Lineage {
                parent_hash,
                ancestry,
            },
            save_kind: self.target.kind,
            timestamp_ms: manifest.timestamp_ms,
            size: payload.bytes.len() as u64,
            validator: None,
            status: CandidateStatus::Canonical,
            deleted: false,
        })
    }

    pub fn reconcile(
        &self,
        local: &Candidate,
        remote: &StagedCandidate,
        gate: SyncGate,
    ) -> Result<SyncStatus, SyncError> {
        validate_candidate(local)?;
        validate_candidate(&remote.candidate)?;
        self.validate_match(local, &remote.candidate)?;
        if gate != SyncGate::Ready {
            self.exchange
                .enqueue_pending(&remote.candidate, &remote.payload)?;
            return Ok(status(local, &remote.candidate, "paused", "queued-durable"));
        }
        let state = if remote.candidate.hash == local.hash
            || local
                .lineage
                .ancestry
                .iter()
                .any(|hash| hash == &remote.candidate.hash)
        {
            "already-current"
        } else if remote.candidate.deleted || local.deleted {
            "conflict"
        } else if remote.candidate.lineage.parent_hash.as_deref() == Some(&local.hash)
            || remote
                .candidate
                .lineage
                .ancestry
                .iter()
                .any(|hash| hash == &local.hash)
        {
            self.install_remote(local, remote)?;
            "fast-forwarded"
        } else {
            "conflict"
        };
        Ok(status(
            local,
            &remote.candidate,
            state,
            "deterministic-ancestry",
        ))
    }

    pub fn resolve(
        &self,
        local: &Candidate,
        remote: &StagedCandidate,
        action: ResolutionAction,
    ) -> Result<ResolutionReceipt, SyncError> {
        validate_candidate(local)?;
        validate_candidate(&remote.candidate)?;
        self.validate_match(local, &remote.candidate)?;
        if local.hash == remote.candidate.hash {
            return Err(error("resolution requires distinct candidates"));
        }
        let generation = self.commit_remote(&remote.candidate, &remote.payload)?;
        if action == ResolutionAction::KeepRemote {
            self.vault.restore(generation, true).map_err(vault_error)?;
            self.vault.promote(generation).map_err(vault_error)?;
        }
        let canonical_generation = self
            .vault
            .current_generation()
            .ok_or_else(|| error("canonical generation disappeared"))?;
        Ok(ResolutionReceipt {
            action,
            state: match action {
                ResolutionAction::KeepLocal => "local-canonical",
                ResolutionAction::KeepRemote => "remote-canonical",
                ResolutionAction::KeepBoth => "both-retained-local-canonical",
            }
            .into(),
            canonical_generation,
            preserved_hash_prefixes: vec![prefix(&local.hash), prefix(&remote.candidate.hash)],
        })
    }

    fn validate_match(&self, local: &Candidate, remote: &Candidate) -> Result<(), SyncError> {
        if local.logical_id != self.target.logical_id
            || local.content_id != self.target.content_id
            || local.save_kind != self.target.kind
            || remote.logical_id != self.target.logical_id
            || remote.content_id != self.target.content_id
            || remote.save_kind != self.target.kind
        {
            return Err(error("candidate does not match the selected save target"));
        }
        Ok(())
    }

    fn install_remote(&self, local: &Candidate, remote: &StagedCandidate) -> Result<(), SyncError> {
        let generation = self.commit_remote(&remote.candidate, &remote.payload)?;
        if local.hash != remote.candidate.hash {
            self.vault.restore(generation, true).map_err(vault_error)?;
            self.vault.promote(generation).map_err(vault_error)?;
        }
        Ok(())
    }

    fn commit_remote(&self, candidate: &Candidate, payload: &[u8]) -> Result<u64, SyncError> {
        let outcome = self
            .vault
            .commit_material(
                Identity {
                    content_version: &candidate.content_id,
                    runner_version: "save-sync",
                    core_version: None,
                },
                &[MaterialFile {
                    kind: self.target.kind,
                    relative: self.target.relative.clone(),
                    bytes: payload.to_vec(),
                }],
                SnapshotReason::PreSync,
            )
            .map_err(vault_error)?;
        Ok(outcome.generation)
    }
}

fn status(local: &Candidate, remote: &Candidate, state: &str, outcome: &str) -> SyncStatus {
    SyncStatus {
        local: view(local),
        remote: view(remote),
        state: state.into(),
        transport_outcome: outcome.into(),
        actions: vec![
            ResolutionAction::KeepLocal,
            ResolutionAction::KeepRemote,
            ResolutionAction::KeepBoth,
        ],
    }
}

fn view(candidate: &Candidate) -> CandidateView {
    CandidateView {
        logical_id: candidate.logical_id.clone(),
        content_id: candidate.content_id.clone(),
        device_id: candidate.device.id.clone(),
        device_name: candidate.device.name.clone(),
        generation: candidate.generation,
        hash_prefix: prefix(&candidate.hash),
        parent_hash_prefix: candidate.lineage.parent_hash.as_deref().map(prefix),
        ancestry: candidate
            .lineage
            .ancestry
            .iter()
            .map(|hash| prefix(hash))
            .collect(),
        save_kind: candidate.save_kind,
        timestamp_ms: candidate.timestamp_ms,
        size: candidate.size,
        status: candidate.status,
        deleted: candidate.deleted,
    }
}

fn validate_target(target: &SaveTarget) -> Result<(), SyncError> {
    validate_token(&target.logical_id)?;
    validate_token(&target.content_id)?;
    validate_relative(&target.relative)?;
    if target.relative.to_ascii_lowercase().contains("rom")
        || target.relative.to_ascii_lowercase().contains("bios")
    {
        return Err(error("ROM and BIOS paths are outside save synchronization"));
    }
    Ok(())
}

fn validate_candidate(candidate: &Candidate) -> Result<(), SyncError> {
    validate_target(&SaveTarget {
        logical_id: candidate.logical_id.clone(),
        content_id: candidate.content_id.clone(),
        relative: match candidate.save_kind {
            SaveKind::Sram | SaveKind::Save => "saves/sync.save".into(),
            SaveKind::State => "states/sync.state".into(),
            SaveKind::DeclaredState => "declared/sync.state".into(),
        },
        kind: candidate.save_kind,
    })?;
    validate_token(&candidate.device.id)?;
    validate_device_name(&candidate.device.name)?;
    validate_hash(&candidate.hash)?;
    if candidate.schema != SCHEMA
        || candidate.format != FORMAT
        || candidate.schema_version != 1
        || candidate.generation == 0
        || candidate.timestamp_ms == 0
        || candidate.size > MAX_BYTES
    {
        return Err(error("candidate bounds are invalid"));
    }
    if candidate.lineage.ancestry.len() > MAX_ANCESTRY {
        return Err(error("candidate ancestry exceeds bound"));
    }
    if let Some(parent) = &candidate.lineage.parent_hash {
        validate_hash(parent)?;
    }
    for hash in &candidate.lineage.ancestry {
        validate_hash(hash)?;
    }
    if let Some(validator) = &candidate.validator {
        validate_opaque(validator)?;
    }
    Ok(())
}

fn verify_payload(candidate: &Candidate, payload: &[u8]) -> Result<(), SyncError> {
    if candidate.deleted {
        if !payload.is_empty() || candidate.size != 0 || candidate.hash != digest(payload) {
            return Err(error("deleted candidate payload is invalid"));
        }
    } else if payload.len() as u64 != candidate.size || digest(payload) != candidate.hash {
        return Err(error("candidate payload hash or size is invalid"));
    }
    Ok(())
}

fn validate_token(value: &str) -> Result<(), SyncError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        || value.to_ascii_lowercase().contains("rom")
        || value.to_ascii_lowercase().contains("bios")
    {
        return Err(error("identifier is not bounded and sanitized"));
    }
    Ok(())
}

fn validate_device_name(value: &str) -> Result<(), SyncError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._- ".contains(&byte))
    {
        return Err(error("device name is not bounded and sanitized"));
    }
    Ok(())
}

fn validate_relative(value: &str) -> Result<(), SyncError> {
    if value.is_empty()
        || value.len() > 255
        || value.contains('\\')
        || Path::new(value).is_absolute()
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(error("save relative path is invalid"));
    }
    Ok(())
}

fn validate_opaque(value: &str) -> Result<(), SyncError> {
    if value.is_empty()
        || value.len() > 255
        || !value.is_ascii()
        || value.contains('\n')
        || value.to_ascii_lowercase().contains("password")
        || value.to_ascii_lowercase().contains("credential")
        || value.to_ascii_lowercase().contains("secret")
    {
        return Err(error("transport validator is untrustworthy"));
    }
    Ok(())
}

fn validate_hash(value: &str) -> Result<(), SyncError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(error("candidate hash is invalid"));
    }
    Ok(())
}

fn prefix(hash: &str) -> String {
    hash.get(..12).unwrap_or_default().to_owned()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn storage_error(_: impl std::fmt::Display) -> SyncError {
    error("sync exchange storage failure")
}
fn vault_error(error: impl std::fmt::Display) -> SyncError {
    SyncError(error.to_string())
}
fn symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}
fn reject_symlink_components(path: &Path) -> Result<(), SyncError> {
    let mut current = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current.push(name);
                if let Ok(metadata) = fs::symlink_metadata(&current) {
                    if metadata.file_type().is_symlink() {
                        return Err(error("exchange path contains a symlink"));
                    }
                    if !metadata.is_dir() {
                        return Err(error("exchange path component is not a directory"));
                    }
                }
            }
            _ => return Err(error("exchange path is not normalized")),
        }
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8], mode: u32) -> Result<(), SyncError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(storage_error)?;
    file.write_all(bytes).map_err(storage_error)?;
    file.sync_all().map_err(storage_error)
}
fn sync_dir(path: &Path) -> Result<(), SyncError> {
    File::open(path)
        .map_err(storage_error)?
        .sync_all()
        .map_err(storage_error)
}
fn count_dirs(path: &Path) -> Result<usize, SyncError> {
    Ok(fs::read_dir(path)
        .map_err(storage_error)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir() && std::ops::Not::not(symlink(&entry.path())))
        .count())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretRef(PathBuf);
impl SecretRef {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, SyncError> {
        let path = path.into();
        let base = Path::new("/data/secrets/save-sync");
        if !path.is_absolute()
            || path.parent() != Some(base)
            || path.file_name().is_none()
            || path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().is_empty())
            || symlink(&path)
        {
            return Err(error(
                "secret reference must be directly under /data/secrets/save-sync",
            ));
        }
        let reference = Self(path);
        if reference.0.exists() {
            reference.validate_file()?;
        }
        Ok(reference)
    }

    pub fn prepare(file_name: &str) -> Result<Self, SyncError> {
        if file_name.is_empty()
            || file_name.len() > 64
            || !file_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(error("secret file name is invalid"));
        }
        let base = Path::new("/data/secrets/save-sync");
        if symlink(base) {
            return Err(error("secret directory is a symlink"));
        }
        if !base.exists() {
            fs::create_dir_all(base).map_err(storage_error)?;
        }
        fs::set_permissions(base, fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
        let path = base.join(file_name);
        if !path.exists() {
            write_new(&path, &[], 0o600)?;
        }
        Self::new(path)
    }

    pub fn validate_file(&self) -> Result<(), SyncError> {
        let metadata = fs::symlink_metadata(&self.0).map_err(storage_error)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.permissions().mode() & 0o7777 != 0o600
        {
            return Err(error("secret file must be regular and 0600"));
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

pub fn validate_secret_directory(path: &Path) -> Result<(), SyncError> {
    if path != Path::new("/data/secrets/save-sync") {
        return Err(error("secret directory policy is fixed"));
    }
    if path.exists() {
        let mode = fs::symlink_metadata(path).map_err(storage_error)?;
        if mode.file_type().is_symlink()
            || !mode.is_dir()
            || mode.permissions().mode() & 0o7777 != 0o700
        {
            return Err(error("secret directory must be 0700 and non-symlink"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefixes_never_expose_full_hash() {
        let candidate = Candidate {
            schema: SCHEMA.into(),
            format: FORMAT.into(),
            schema_version: 1,
            logical_id: "fixture-save".into(),
            content_id: "fixture-content".into(),
            device: Device {
                id: "device-a".into(),
                name: "Brick A".into(),
            },
            generation: 1,
            hash: digest("a".as_bytes()),
            lineage: Lineage {
                parent_hash: None,
                ancestry: vec![],
            },
            save_kind: SaveKind::Save,
            timestamp_ms: 1,
            size: 1,
            validator: None,
            status: CandidateStatus::Candidate,
            deleted: false,
        };
        assert_eq!(view(&candidate).hash_prefix.len(), 12);
        assert_ne!(view(&candidate).hash_prefix, candidate.hash);
    }
}
