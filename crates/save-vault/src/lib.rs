use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "https://example.invalid/trimui-save-vault-v1.schema.json";
pub const FORMAT: &str = "brickpro-save-vault";
pub const MAX_GENERATIONS: usize = 8;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const COMMIT_MARKER: &[u8] = b"brickpro-save-vault-commit-v1\n";
type ObjectBytes = Vec<(String, Vec<u8>)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreMode {
    Production,
    Simulator,
}
impl StoreMode {
    fn dir_mode(self) -> u32 {
        if matches!(self, Self::Production) {
            0o700
        } else {
            0o777
        }
    }
    fn file_mode(self) -> u32 {
        if matches!(self, Self::Production) {
            0o600
        } else {
            0o644
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SaveKind {
    Sram,
    Save,
    State,
    DeclaredState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotReason {
    NormalExit,
    PrePackage,
    PreUpdate,
    PreCoreChange,
    PreRecipe,
    PreSync,
    PreRestore,
}
impl SnapshotReason {
    fn protected(self) -> bool {
        !matches!(self, Self::NormalExit)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnomalyStatus {
    Valid,
    Quarantined,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetentionClass {
    Recent,
    Protected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity<'a> {
    pub content_version: &'a str,
    pub runner_version: &'a str,
    pub core_version: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub struct SnapshotFile {
    pub kind: SaveKind,
    pub relative: String,
    pub source: PathBuf,
}

#[derive(Clone, Debug)]
pub struct SaveKindPolicy {
    pub allow_empty: bool,
    pub max_shrink_percent: u8,
}
impl Default for SaveKindPolicy {
    fn default() -> Self {
        Self {
            allow_empty: false,
            max_shrink_percent: 50,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Catalog {
    policies: BTreeMap<SaveKind, SaveKindPolicy>,
}
impl Default for Catalog {
    fn default() -> Self {
        let mut policies = BTreeMap::new();
        for kind in [
            SaveKind::Sram,
            SaveKind::Save,
            SaveKind::State,
            SaveKind::DeclaredState,
        ] {
            policies.insert(kind, SaveKindPolicy::default());
        }
        Self { policies }
    }
}
impl Catalog {
    pub fn with_policy(mut self, kind: SaveKind, policy: SaveKindPolicy) -> Self {
        self.policies.insert(kind, policy);
        self
    }
    fn policy(&self, kind: SaveKind) -> SaveKindPolicy {
        self.policies.get(&kind).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOutcome {
    pub generation: u64,
    pub status: AnomalyStatus,
    pub committed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    pub generation: u64,
    pub content_version: String,
    pub runner_version: String,
    pub core_version: Option<String>,
    pub old_size: u64,
    pub new_size: u64,
    pub old_hash_status: String,
    pub new_hash_status: String,
    pub old_hash_prefix: String,
    pub new_hash_prefix: String,
    pub affected_kinds: Vec<SaveKind>,
    pub reason: SnapshotReason,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveVaultError(String);
impl std::fmt::Display for SaveVaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SaveVaultError {}
fn err(message: impl Into<String>) -> SaveVaultError {
    SaveVaultError(message.into())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub content_id: String,
    pub kind: SaveKind,
    pub source_relative: String,
    pub size: u64,
    pub sha256: String,
    pub object: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GenerationManifest {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    pub schema_version: u8,
    pub generation: u64,
    pub parent_generation: Option<u64>,
    pub content_version: String,
    pub runner_version: String,
    pub core_version: Option<String>,
    pub reason: SnapshotReason,
    pub timestamp_ms: u64,
    pub anomaly: AnomalyStatus,
    pub retention: RetentionClass,
    pub artifacts: Vec<ArtifactRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CurrentPointer {
    schema: String,
    generation: u64,
    checksum: String,
}

pub struct SaveVault {
    root: PathBuf,
    source_root: PathBuf,
    catalog: Catalog,
    mode: StoreMode,
}
impl SaveVault {
    pub fn new(
        root: impl Into<PathBuf>,
        source_root: impl Into<PathBuf>,
        catalog: Catalog,
    ) -> Result<Self, SaveVaultError> {
        Self::open(
            root.into(),
            source_root.into(),
            catalog,
            StoreMode::Production,
        )
    }
    pub fn for_simulator(
        root: impl Into<PathBuf>,
        source_root: impl Into<PathBuf>,
        catalog: Catalog,
    ) -> Result<Self, SaveVaultError> {
        Self::open(
            root.into(),
            source_root.into(),
            catalog,
            StoreMode::Simulator,
        )
    }
    fn open(
        root: PathBuf,
        source_root: PathBuf,
        catalog: Catalog,
        mode: StoreMode,
    ) -> Result<Self, SaveVaultError> {
        for path in [&root, &source_root] {
            if !path.is_absolute() || *path == Path::new("/") {
                return Err(err("vault roots must be absolute and non-root"));
            }
            reject_symlink_components(path)?;
        }
        if !source_root.is_dir() {
            return Err(err("save source root is unavailable"));
        }
        reject_source_tree(&source_root)?;
        let vault = Self {
            root,
            source_root,
            catalog,
            mode,
        };
        vault.ensure_layout()?;
        vault.validate_layout()?;
        Ok(vault)
    }
    fn ensure_layout(&self) -> Result<(), SaveVaultError> {
        if self.root.exists() && (!self.root.is_dir() || symlink(&self.root)) {
            return Err(err("vault root boundary is invalid"));
        }
        if !self.root.exists() {
            fs::create_dir_all(&self.root).map_err(error)?;
        }
        for name in ["objects", "generations", "quarantine", ".staging"] {
            let path = self.root.join(name);
            if path.exists() {
                if !path.is_dir() || symlink(&path) {
                    return Err(err("vault directory boundary is invalid"));
                }
            } else {
                fs::create_dir(&path).map_err(error)?;
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(self.mode.dir_mode()))
                .map_err(error)?;
        }
        fs::set_permissions(&self.root, fs::Permissions::from_mode(self.mode.dir_mode()))
            .map_err(error)?;
        let lock = self.root.join("operation.lock");
        if symlink(&lock) {
            return Err(err("vault operation lock boundary is invalid"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(self.mode.file_mode())
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lock)
            .map_err(error)?;
        fs::set_permissions(&lock, fs::Permissions::from_mode(self.mode.file_mode()))
            .map_err(error)?;
        file.sync_all().map_err(error)
    }
    fn validate_layout(&self) -> Result<(), SaveVaultError> {
        for name in ["objects", "generations", "quarantine", ".staging"] {
            let path = self.root.join(name);
            let m = fs::symlink_metadata(path).map_err(error)?;
            if !m.is_dir()
                || m.file_type().is_symlink()
                || m.permissions().mode() & 0o7777 != self.mode.dir_mode()
            {
                return Err(err("vault directory mode or boundary is invalid"));
            }
        }
        let lock = fs::symlink_metadata(self.root.join("operation.lock")).map_err(error)?;
        if lock.file_type().is_symlink()
            || !lock.is_file()
            || lock.permissions().mode() & 0o7777 != self.mode.file_mode()
        {
            return Err(err("vault operation lock mode or boundary is invalid"));
        }
        Ok(())
    }

    fn acquire_lock(&self) -> Result<OperationLock, SaveVaultError> {
        OperationLock::acquire(&self.root, self.mode.file_mode())
    }

    pub fn snapshot(
        &self,
        identity: Identity<'_>,
        files: &[SnapshotFile],
        reason: SnapshotReason,
    ) -> Result<SnapshotOutcome, SaveVaultError> {
        let _lock = self.acquire_lock()?;
        self.snapshot_locked(identity, files, reason)
    }

    fn snapshot_locked(
        &self,
        identity: Identity<'_>,
        files: &[SnapshotFile],
        reason: SnapshotReason,
    ) -> Result<SnapshotOutcome, SaveVaultError> {
        self.validate_layout()?;
        reject_source_tree(&self.source_root)?;
        validate_token(identity.content_version)?;
        validate_token(identity.runner_version)?;
        if let Some(core) = identity.core_version {
            validate_token(core)?;
        }
        if files.len() > 64 {
            return Err(err("save file count exceeds bound"));
        }
        let generation = self.next_generation()?;
        let stage = self
            .root
            .join(".staging")
            .join(format!("generation-{generation}"));
        let target = self
            .root
            .join("generations")
            .join(format!("generation-{generation}"));
        if symlink(&stage) || symlink(&target) {
            return Err(err("snapshot destination boundary is invalid"));
        }
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(error)?;
        }
        if target.exists() {
            return Err(err("snapshot generation already exists"));
        }
        fs::create_dir(&stage).map_err(error)?;
        fs::set_permissions(&stage, fs::Permissions::from_mode(self.mode.dir_mode()))
            .map_err(error)?;
        let result = (|| {
            let (manifest, objects) = self.snapshot_inner(generation, identity, files, reason)?;
            for (hash, bytes) in objects {
                self.publish_object(&hash, &bytes)?;
            }
            reject_source_tree(&self.source_root)?;
            self.verify_snapshot_sources(&manifest, files)?;
            self.verify_manifest_objects(&manifest)?;
            write_json(
                &stage.join("manifest.json"),
                &manifest,
                self.mode.file_mode(),
            )?;
            write_file(
                &stage.join("commit.marker"),
                COMMIT_MARKER,
                self.mode.file_mode(),
            )?;
            sync_dir(&stage)?;
            let quarantine = manifest.anomaly == AnomalyStatus::Quarantined;
            let destination = if quarantine {
                self.root
                    .join("quarantine")
                    .join(format!("generation-{generation}"))
            } else {
                target
            };
            if symlink(&destination) || destination.exists() {
                return Err(err("snapshot destination already exists"));
            }
            fs::rename(&stage, &destination).map_err(error)?;
            sync_dir(
                destination
                    .parent()
                    .ok_or_else(|| err("vault destination has no parent"))?,
            )?;
            if !quarantine {
                publish_current(&self.root, generation, self.mode.file_mode())?;
                self.prune(generation)?;
            }
            Ok(SnapshotOutcome {
                generation,
                status: manifest.anomaly,
                committed: !quarantine,
            })
        })();
        if result.is_err() && stage.exists() && !symlink(&stage) {
            let _ = fs::remove_dir_all(&stage);
        }
        result
    }
    fn snapshot_inner(
        &self,
        generation: u64,
        identity: Identity<'_>,
        files: &[SnapshotFile],
        reason: SnapshotReason,
    ) -> Result<(GenerationManifest, ObjectBytes), SaveVaultError> {
        let current = self.current_generation();
        let parent = self.parent_generation(current);
        let mut artifacts = Vec::new();
        let mut objects = Vec::new();
        let mut anomaly = AnomalyStatus::Valid;
        let prior = current.and_then(|number| self.read_manifest(number));
        let mut relatives = BTreeSet::new();
        for item in files {
            validate_source_relative(item.kind, &item.relative)?;
            if !relatives.insert(item.relative.clone())
                || item.source != self.source_root.join(&item.relative)
            {
                return Err(err("save source identity is invalid"));
            }
            let (bytes, size, hash) = read_stable(&item.source)?;
            if size > MAX_FILE_BYTES {
                return Err(err("save file exceeds bound"));
            }
            let policy = self.catalog.policy(item.kind);
            let old = prior.as_ref().and_then(|manifest| {
                manifest
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.source_relative == item.relative)
            });
            if (!policy.allow_empty && size == 0)
                || old.is_some_and(|old| {
                    old.size > 0
                        && size.saturating_mul(100)
                            < old
                                .size
                                .saturating_mul(u64::from(100 - policy.max_shrink_percent))
                })
            {
                anomaly = AnomalyStatus::Quarantined;
            }
            let object = format!("{hash}.bin");
            objects.push((hash.clone(), bytes));
            artifacts.push(ArtifactRecord {
                content_id: format!("sha256:{hash}"),
                kind: item.kind,
                source_relative: item.relative.clone(),
                size,
                sha256: hash.clone(),
                object,
            });
        }
        let manifest = GenerationManifest {
            schema: SCHEMA.into(),
            format: FORMAT.into(),
            schema_version: 1,
            generation,
            parent_generation: parent,
            content_version: identity.content_version.into(),
            runner_version: identity.runner_version.into(),
            core_version: identity.core_version.map(str::to_owned),
            reason,
            timestamp_ms: now_ms(),
            anomaly,
            retention: if reason.protected() {
                RetentionClass::Protected
            } else {
                RetentionClass::Recent
            },
            artifacts,
        };
        Ok((manifest, objects))
    }
    fn parent_generation(&self, current: Option<u64>) -> Option<u64> {
        self.history()
            .into_iter()
            .find(|manifest| manifest.parent_generation.is_none())
            .map(|manifest| manifest.generation)
            .or(current)
    }
    fn verify_snapshot_sources(
        &self,
        manifest: &GenerationManifest,
        files: &[SnapshotFile],
    ) -> Result<(), SaveVaultError> {
        for item in files {
            let artifact = manifest
                .artifacts
                .iter()
                .find(|artifact| artifact.source_relative == item.relative)
                .ok_or_else(|| err("snapshot source is missing from manifest"))?;
            let (_, size, hash) = read_stable(&item.source)?;
            if size != artifact.size || hash != artifact.sha256 {
                return Err(err("save source changed before publication"));
            }
        }
        Ok(())
    }
    fn verify_object(&self, hash: &str, size: u64) -> Result<(), SaveVaultError> {
        let object = self.root.join("objects").join(format!("{hash}.bin"));
        let metadata = fs::symlink_metadata(&object).map_err(error)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != size
            || digest(&fs::read(&object).map_err(error)?) != hash
        {
            return Err(err("content object verification failed"));
        }
        Ok(())
    }
    fn verify_manifest_objects(&self, manifest: &GenerationManifest) -> Result<(), SaveVaultError> {
        for artifact in &manifest.artifacts {
            self.verify_object(&artifact.sha256, artifact.size)?;
        }
        Ok(())
    }
    fn publish_object(&self, hash: &str, bytes: &[u8]) -> Result<(), SaveVaultError> {
        validate_hash(hash)?;
        let path = self.root.join("objects").join(format!("{hash}.bin"));
        if path.exists() {
            let m = fs::symlink_metadata(&path).map_err(error)?;
            if m.file_type().is_symlink()
                || !m.is_file()
                || m.len() != bytes.len() as u64
                || digest(&fs::read(&path).map_err(error)?) != hash
            {
                return Err(err("content object is corrupt"));
            }
            return Ok(());
        }
        let temp = self.root.join("objects").join(format!(".{hash}.tmp"));
        if symlink(&temp) {
            return Err(err("content object staging boundary is invalid"));
        }
        if temp.exists() {
            fs::remove_file(&temp).map_err(error)?;
        }
        write_file(&temp, bytes, self.mode.file_mode())?;
        fs::rename(&temp, &path).map_err(error)?;
        self.verify_object(hash, bytes.len() as u64)?;
        sync_dir(&self.root.join("objects"))
    }
    fn next_generation(&self) -> Result<u64, SaveVaultError> {
        let mut max: u64 = 0;
        for tree in ["generations", "quarantine"] {
            for entry in fs::read_dir(self.root.join(tree)).map_err(error)? {
                let name = entry
                    .map_err(error)?
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                if let Some(value) = name
                    .strip_prefix("generation-")
                    .and_then(|x| x.parse().ok())
                {
                    max = max.max(value);
                }
            }
        }
        Ok(max.saturating_add(1))
    }
    pub fn current_generation(&self) -> Option<u64> {
        let path = self.root.join("current.json");
        if symlink(&path) {
            return None;
        }
        let bytes = fs::read(path).ok()?;
        let pointer: CurrentPointer = serde_json::from_slice(&bytes).ok()?;
        if pointer.schema != "brickpro-save-vault-current/v1"
            || pointer.checksum != pointer_checksum(pointer.generation)
        {
            return None;
        }
        self.read_manifest(pointer.generation)
            .filter(|manifest| manifest.anomaly == AnomalyStatus::Valid)
            .map(|_| pointer.generation)
    }
    fn read_manifest(&self, generation: u64) -> Option<GenerationManifest> {
        let path = self
            .root
            .join("generations")
            .join(format!("generation-{generation}"));
        if !committed(&path) {
            return None;
        }
        let manifest: GenerationManifest =
            serde_json::from_slice(&fs::read(path.join("manifest.json")).ok()?).ok()?;
        if manifest.generation != generation || manifest.anomaly != AnomalyStatus::Valid {
            return None;
        }
        validate_manifest(&manifest).ok()?;
        self.verify_manifest_objects(&manifest).ok()?;
        Some(manifest)
    }
    pub fn history(&self) -> Vec<GenerationManifest> {
        let mut result = fs::read_dir(self.root.join("generations"))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .strip_prefix("generation-")
                    .and_then(|x| x.parse().ok())
            })
            .filter_map(|number| self.read_manifest(number))
            .collect::<Vec<_>>();
        result.sort_by_key(|manifest| manifest.generation);
        result
    }
    pub fn preview(&self, generation: u64) -> Result<RestorePreview, SaveVaultError> {
        let manifest = self
            .read_manifest(generation)
            .ok_or_else(|| err("recovery generation is unavailable"))?;
        let current = manifest
            .artifacts
            .iter()
            .map(|artifact| read_stable(&self.source_root.join(&artifact.source_relative)).ok())
            .collect::<Vec<_>>();
        let old_size = manifest
            .artifacts
            .iter()
            .map(|artifact| artifact.size)
            .sum();
        let new_size = current.iter().flatten().map(|(_, size, _)| *size).sum();
        let new_hash_status = if current.iter().all(Option::is_some)
            && current
                .iter()
                .zip(&manifest.artifacts)
                .all(|(current, artifact)| {
                    current
                        .as_ref()
                        .is_some_and(|value| value.2 == artifact.sha256)
                }) {
            "matches"
        } else if current.iter().any(Option::is_none) {
            "unavailable"
        } else {
            "different"
        };
        let affected_kinds = manifest
            .artifacts
            .iter()
            .map(|artifact| artifact.kind)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if manifest.artifacts.is_empty() {
            return Err(err("recovery generation has no data"));
        }
        let old_hash_prefix = hash_prefix(&aggregate_digest(
            manifest
                .artifacts
                .iter()
                .map(|artifact| artifact.sha256.clone()),
        ));
        let new_hash_prefix = if current.iter().all(Option::is_some) {
            hash_prefix(&aggregate_digest(
                current
                    .iter()
                    .filter_map(|value| value.as_ref().map(|value| value.2.clone())),
            ))
        } else {
            "unavailable".into()
        };
        Ok(RestorePreview {
            generation,
            content_version: manifest.content_version,
            runner_version: manifest.runner_version,
            core_version: manifest.core_version,
            old_size,
            new_size,
            old_hash_status: "verified".into(),
            new_hash_status: new_hash_status.into(),
            old_hash_prefix,
            new_hash_prefix,
            affected_kinds,
            reason: manifest.reason,
            timestamp_ms: manifest.timestamp_ms,
        })
    }
    pub fn restore(
        &self,
        generation: u64,
        confirmed: bool,
    ) -> Result<RestorePreview, SaveVaultError> {
        let _lock = self.acquire_lock()?;
        let preview = self.preview(generation)?;
        if !confirmed {
            return Err(err("restore requires explicit confirmation"));
        }
        let manifest = self
            .read_manifest(generation)
            .ok_or_else(|| err("recovery generation is unavailable"))?;
        let files = manifest
            .artifacts
            .iter()
            .filter(|artifact| self.source_root.join(&artifact.source_relative).is_file())
            .map(|artifact| SnapshotFile {
                kind: artifact.kind,
                relative: artifact.source_relative.clone(),
                source: self.source_root.join(&artifact.source_relative),
            })
            .collect::<Vec<_>>();
        let before_restore = self.snapshot_locked(
            Identity {
                content_version: &manifest.content_version,
                runner_version: &manifest.runner_version,
                core_version: manifest.core_version.as_deref(),
            },
            &files,
            SnapshotReason::PreRestore,
        )?;
        if !before_restore.committed {
            return Err(err("pre-restore snapshot was quarantined"));
        }
        let mut backups = Vec::new();
        let result = (|| {
            for (index, artifact) in manifest.artifacts.iter().enumerate() {
                let source = self.source_root.join(&artifact.source_relative);
                let original = match fs::symlink_metadata(&source) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                        return Err(err("restore source is not a regular file"));
                    }
                    Ok(_) => Some(fs::read(&source).map_err(error)?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(io_error) => return Err(error(io_error)),
                };
                backups.push((source.clone(), original));
                let object = self.root.join("objects").join(&artifact.object);
                let temp = source.with_file_name(format!(".restore-{generation}-{index}.tmp"));
                if symlink(&temp) {
                    return Err(err("restore staging boundary is invalid"));
                }
                if temp.exists() {
                    fs::remove_file(&temp).map_err(error)?;
                }
                write_file(
                    &temp,
                    &fs::read(&object).map_err(error)?,
                    self.mode.file_mode(),
                )?;
                if read_stable(&temp)?.2 != artifact.sha256 {
                    return Err(err("restore verification failed"));
                }
                fs::rename(&temp, &source).map_err(error)?;
                sync_dir(
                    source
                        .parent()
                        .ok_or_else(|| err("restore source has no parent"))?,
                )?;
            }
            Ok(())
        })();
        if let Err(original_error) = result {
            if let Err(rollback_error) = restore_backups(&backups, self.mode.file_mode()) {
                return Err(err(format!(
                    "restore failed and rollback failed: {rollback_error}"
                )));
            }
            return Err(original_error);
        }
        Ok(preview)
    }
    fn prune(&self, current: u64) -> Result<(), SaveVaultError> {
        let manifests = self.history();
        if manifests.len() <= MAX_GENERATIONS {
            return Ok(());
        }
        let by_generation = manifests
            .iter()
            .map(|manifest| (manifest.generation, manifest))
            .collect::<BTreeMap<_, _>>();
        let mut keep = manifests
            .iter()
            .rev()
            .take(MAX_GENERATIONS)
            .map(|manifest| manifest.generation)
            .collect::<BTreeSet<_>>();
        keep.insert(current);
        keep.extend(
            manifests
                .iter()
                .filter(|manifest| manifest.retention == RetentionClass::Protected)
                .map(|manifest| manifest.generation),
        );
        let mut changed = true;
        while changed {
            changed = false;
            for generation in keep.clone() {
                if let Some(Some(parent)) = by_generation
                    .get(&generation)
                    .map(|manifest| manifest.parent_generation)
                {
                    changed |= keep.insert(parent);
                }
            }
        }
        for manifest in &manifests {
            if !keep.contains(&manifest.generation) {
                let path = self
                    .root
                    .join("generations")
                    .join(format!("generation-{}", manifest.generation));
                if symlink(&path) {
                    return Err(err("retention generation boundary is invalid"));
                }
                fs::remove_dir_all(path).map_err(error)?;
            }
        }
        let mut referenced = self
            .history()
            .into_iter()
            .flat_map(|m| m.artifacts.into_iter().map(|a| a.object))
            .collect::<BTreeSet<_>>();
        for entry in fs::read_dir(self.root.join("quarantine")).map_err(error)? {
            let entry = entry.map_err(error)?;
            let path = entry.path();
            if committed(&path) {
                if let Ok(manifest) = serde_json::from_slice::<GenerationManifest>(
                    &fs::read(path.join("manifest.json")).map_err(error)?,
                ) {
                    if validate_manifest(&manifest).is_ok()
                        && manifest.anomaly == AnomalyStatus::Quarantined
                    {
                        referenced.extend(
                            manifest
                                .artifacts
                                .into_iter()
                                .map(|artifact| artifact.object),
                        );
                    }
                }
            }
        }
        for entry in fs::read_dir(self.root.join("objects")).map_err(error)? {
            let entry = entry.map_err(error)?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| !referenced.contains(name))
            {
                if symlink(&entry.path()) {
                    return Err(err("object retention boundary is invalid"));
                }
                fs::remove_file(entry.path()).map_err(error)?;
            }
        }
        sync_dir(&self.root.join("generations"))
    }

    pub fn snapshot_standard(
        root: &Path,
        reason: SnapshotReason,
    ) -> Result<SnapshotOutcome, SaveVaultError> {
        let source = root.join("data");
        let vault = Self::new(
            root.join(".brickpro/save-vault"),
            &source,
            Catalog::default(),
        )?;
        let mut files = Vec::new();
        for (directory, kind) in [("saves", SaveKind::Save), ("states", SaveKind::State)] {
            collect_standard(
                &source.join(directory),
                Path::new(directory),
                kind,
                &mut files,
            )?;
        }
        let outcome = vault.snapshot(
            Identity {
                content_version: "standard",
                runner_version: "broker",
                core_version: None,
            },
            &files,
            reason,
        )?;
        if reason.protected() && !outcome.committed {
            return Err(err("protected save snapshot was quarantined"));
        }
        Ok(outcome)
    }
    pub fn standard_integrity(root: &Path) -> Result<String, SaveVaultError> {
        let vault = Self::new(
            root.join(".brickpro/save-vault"),
            root.join("data"),
            Catalog::default(),
        )?;
        vault.integrity_digest()
    }

    pub fn integrity_digest(&self) -> Result<String, SaveVaultError> {
        let _lock = self.acquire_lock()?;
        let mut files = Vec::new();
        collect_all(&self.root, Path::new(""), &mut files)?;
        files.sort_by(|left, right| left.0.cmp(&right.0));
        let mut hash = Sha256::new();
        for (path, bytes) in files {
            hash.update(path.as_bytes());
            hash.update([0]);
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        Ok(format!("{:x}", hash.finalize()))
    }
}

fn restore_backups(
    backups: &[(PathBuf, Option<Vec<u8>>)],
    mode: u32,
) -> Result<(), SaveVaultError> {
    for (index, (source, bytes)) in backups.iter().enumerate() {
        match bytes {
            Some(bytes) => {
                let temp = source.with_file_name(format!(".restore-revert-{index}.tmp"));
                if symlink(&temp) {
                    return Err(err("restore rollback staging boundary is invalid"));
                }
                if temp.exists() {
                    fs::remove_file(&temp).map_err(error)?;
                }
                write_file(&temp, bytes, mode)?;
                fs::rename(&temp, source).map_err(error)?;
            }
            None => {
                if symlink(source) {
                    return Err(err("restore rollback source boundary is invalid"));
                }
                if source.exists() {
                    fs::remove_file(source).map_err(error)?;
                }
            }
        }
        sync_dir(
            source
                .parent()
                .ok_or_else(|| err("restore rollback source has no parent"))?,
        )?;
    }
    Ok(())
}

struct OperationLock {
    file: File,
}
impl OperationLock {
    fn acquire(root: &Path, mode: u32) -> Result<Self, SaveVaultError> {
        let path = root.join("operation.lock");
        if symlink(&path) {
            return Err(err("vault operation lock boundary is invalid"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(error)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(Self { file })
    }
}
impl Drop for OperationLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn collect_all(
    path: &Path,
    relative: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), SaveVaultError> {
    for entry in fs::read_dir(path).map_err(error)? {
        let entry = entry.map_err(error)?;
        let child = entry.path();
        let child_relative = relative.join(entry.file_name());
        let metadata = fs::symlink_metadata(&child).map_err(error)?;
        if metadata.file_type().is_symlink() {
            return Err(err("vault integrity contains a symlink"));
        }
        if metadata.is_dir() {
            collect_all(&child, &child_relative, files)?;
        } else if metadata.is_file() {
            files.push((
                child_relative.to_string_lossy().into_owned(),
                fs::read(child).map_err(error)?,
            ));
        } else {
            return Err(err("vault integrity contains an unsupported object"));
        }
    }
    Ok(())
}

fn reject_source_tree(path: &Path) -> Result<(), SaveVaultError> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(err("save source root is invalid"));
    }
    for entry in fs::read_dir(path).map_err(error)? {
        let child = entry.map_err(error)?.path();
        let metadata = fs::symlink_metadata(&child).map_err(error)?;
        if metadata.file_type().is_symlink() {
            return Err(err("save source symlink is forbidden"));
        }
        if metadata.is_dir() {
            reject_source_tree(&child)?;
        }
    }
    Ok(())
}
fn collect_standard(
    path: &Path,
    relative: &Path,
    kind: SaveKind,
    files: &mut Vec<SnapshotFile>,
) -> Result<(), SaveVaultError> {
    if !path.is_dir() || symlink(path) {
        return Err(err("standard save boundary is invalid"));
    }
    for entry in fs::read_dir(path).map_err(error)? {
        let entry = entry.map_err(error)?;
        let child = entry.path();
        let child_rel = relative.join(entry.file_name());
        let metadata = fs::symlink_metadata(&child).map_err(error)?;
        if metadata.file_type().is_symlink() {
            return Err(err("save source symlink is forbidden"));
        }
        if metadata.is_dir() {
            collect_standard(&child, &child_rel, kind, files)?;
        } else if metadata.is_file() {
            files.push(SnapshotFile {
                kind,
                relative: child_rel.to_string_lossy().into_owned(),
                source: child,
            });
        } else {
            return Err(err("save source object is unsupported"));
        }
    }
    Ok(())
}
fn validate_source_relative(kind: SaveKind, value: &str) -> Result<(), SaveVaultError> {
    validate_relative(value)?;
    let prefix = match kind {
        SaveKind::Sram | SaveKind::Save => "saves/",
        SaveKind::State => "states/",
        SaveKind::DeclaredState => "declared/",
    };
    if !value.starts_with(prefix) {
        return Err(err("save source is outside its declared kind boundary"));
    }
    if value.to_ascii_lowercase().contains("rom") || value.to_ascii_lowercase().contains("bios") {
        return Err(err("ROM and BIOS data are outside the save boundary"));
    }
    Ok(())
}
fn validate_relative(value: &str) -> Result<(), SaveVaultError> {
    if value.is_empty()
        || value.len() > 255
        || value.contains('\\')
        || Path::new(value).is_absolute()
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(err("save path is not a sanitized relative path"));
    }
    Ok(())
}
fn validate_token(value: &str) -> Result<(), SaveVaultError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(err("save identity is invalid"));
    }
    Ok(())
}
fn validate_hash(value: &str) -> Result<(), SaveVaultError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(err("save hash is invalid"));
    }
    Ok(())
}
fn validate_manifest(manifest: &GenerationManifest) -> Result<(), SaveVaultError> {
    if manifest.schema != SCHEMA
        || manifest.format != FORMAT
        || manifest.schema_version != 1
        || manifest.generation == 0
        || manifest.timestamp_ms == 0
        || manifest
            .parent_generation
            .is_some_and(|parent| parent >= manifest.generation)
        || manifest.artifacts.len() > 64
    {
        return Err(err("save manifest identity is invalid"));
    }
    validate_token(&manifest.content_version)?;
    validate_token(&manifest.runner_version)?;
    if let Some(core) = &manifest.core_version {
        validate_token(core)?;
    }
    let mut relatives = BTreeSet::new();
    for artifact in &manifest.artifacts {
        validate_source_relative(artifact.kind, &artifact.source_relative)?;
        if !relatives.insert(artifact.source_relative.clone()) || artifact.size > MAX_FILE_BYTES {
            return Err(err("save artifact identity is invalid"));
        }
        validate_hash(&artifact.sha256)?;
        if artifact.content_id != format!("sha256:{}", artifact.sha256)
            || artifact.object != format!("{}.bin", artifact.sha256)
        {
            return Err(err("save artifact identity is invalid"));
        }
    }
    Ok(())
}
fn committed(path: &Path) -> bool {
    !symlink(path)
        && path.is_dir()
        && !symlink(&path.join("commit.marker"))
        && !symlink(&path.join("manifest.json"))
        && fs::read(path.join("commit.marker")).ok().as_deref() == Some(COMMIT_MARKER)
        && path.join("manifest.json").is_file()
}
fn read_stable(path: &Path) -> Result<(Vec<u8>, u64, String), SaveVaultError> {
    let before = fs::symlink_metadata(path).map_err(error)?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(err("save source is not a regular file"));
    }
    let identity = (
        before.dev(),
        before.ino(),
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
    );
    let mut file = File::open(path).map_err(error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(error)?;
    let after = fs::symlink_metadata(path).map_err(error)?;
    if after.file_type().is_symlink()
        || !after.is_file()
        || (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        ) != identity
        || bytes.len() as u64 != identity.2
    {
        return Err(err("save source changed during snapshot"));
    }
    let hash = digest(&bytes);
    Ok((bytes, identity.2, hash))
}
fn publish_current(root: &Path, generation: u64, mode: u32) -> Result<(), SaveVaultError> {
    let path = root.join("current.json");
    let temp = root.join(".current.json.tmp");
    if symlink(&temp) || symlink(&path) {
        return Err(err("current pointer boundary is invalid"));
    }
    if temp.exists() {
        fs::remove_file(&temp).map_err(error)?;
    }
    let bytes = serde_json::to_vec(&CurrentPointer {
        schema: "brickpro-save-vault-current/v1".into(),
        generation,
        checksum: pointer_checksum(generation),
    })
    .map_err(error)?;
    write_file(&temp, &bytes, mode)?;
    fs::rename(temp, path).map_err(error)?;
    sync_dir(root)
}
fn pointer_checksum(generation: u64) -> String {
    digest(format!("brickpro-save-vault-current/v1:{generation}").as_bytes())
}
fn write_json<T: Serialize>(path: &Path, value: &T, mode: u32) -> Result<(), SaveVaultError> {
    write_file(
        path,
        &serde_json::to_vec_pretty(value).map_err(error)?,
        mode,
    )
}
fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), SaveVaultError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.sync_all().map_err(error)
}
fn sync_dir(path: &Path) -> Result<(), SaveVaultError> {
    File::open(path).map_err(error)?.sync_all().map_err(error)
}
fn reject_symlink_components(path: &Path) -> Result<(), SaveVaultError> {
    let mut current = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current.push(name);
                if let Ok(metadata) = fs::symlink_metadata(&current) {
                    if metadata.file_type().is_symlink() {
                        return Err(err("vault root symlink is forbidden"));
                    }
                    if !metadata.is_dir() {
                        return Err(err("vault root component is not a directory"));
                    }
                }
            }
            _ => return Err(err("vault root path is not normalized")),
        }
    }
    Ok(())
}
fn symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn aggregate_digest(hashes: impl IntoIterator<Item = String>) -> String {
    let mut digest = Sha256::new();
    for hash in hashes {
        digest.update(hash.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}
fn hash_prefix(hash: &str) -> String {
    hash[..12].to_owned()
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn error(_: impl std::fmt::Display) -> SaveVaultError {
    err("save vault storage failure")
}
