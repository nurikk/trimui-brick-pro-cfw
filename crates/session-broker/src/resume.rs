use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use launch_contract::{LaunchKind, LaunchRequest, VersionedId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "https://example.invalid/trimui-resume-record-v1.schema.json";
pub const CONFIG_SCHEMA: &str = "https://example.invalid/trimui-resume-capabilities-v1.schema.json";
const FORMAT: &str = "brickpro-resume-record";
const CONFIG_FORMAT: &str = "brickpro-resume-capabilities";
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const MAX_STATE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SRAM_BYTES: usize = 2 * 1024 * 1024;
const MAX_SCREENSHOT_BYTES: usize = 4 * 1024 * 1024;
const MAX_HISTORY: usize = 4;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResumeCapability {
    None,
    SramOnly,
    NativeState,
    BrokerState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckpointReason {
    NormalExit,
    PreSuspend,
    LowBattery,
    Periodic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitFault {
    None,
    Artifact,
    Metadata,
    Promotion,
    Pointer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResumeCapabilityEntry {
    pub content_id: String,
    pub kind: LaunchKind,
    pub capability: ResumeCapability,
    pub retained_core: Option<VersionedId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResumeCapabilityConfig {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    pub schema_version: u8,
    pub entries: Vec<ResumeCapabilityEntry>,
}

impl ResumeCapabilityConfig {
    pub fn parse(bytes: &[u8]) -> Result<Self, ResumeError> {
        let config: Self = serde_json::from_slice(bytes).map_err(error)?;
        if config.schema != CONFIG_SCHEMA
            || config.format != CONFIG_FORMAT
            || config.schema_version != 1
            || config.entries.len() > 32
        {
            return Err(ResumeError::new(
                "resume capability configuration is invalid",
            ));
        }
        for entry in &config.entries {
            valid_id(&entry.content_id)?;
            if entry.capability == ResumeCapability::None {
                return Err(ResumeError::new(
                    "declared resume capability cannot be none",
                ));
            }
            if let Some(core) = &entry.retained_core {
                valid_id(&core.id)?;
                valid_version(&core.version)?;
                if entry.kind != LaunchKind::Libretro
                    || entry.capability == ResumeCapability::SramOnly
                {
                    return Err(ResumeError::new(
                        "retained core is invalid for this resume capability",
                    ));
                }
            }
        }
        for (index, entry) in config.entries.iter().enumerate() {
            if config.entries[..index]
                .iter()
                .any(|other| other.content_id == entry.content_id && other.kind == entry.kind)
            {
                return Err(ResumeError::new("resume capabilities are duplicated"));
            }
        }
        Ok(config)
    }

    fn entry(&self, request: &LaunchRequest) -> Option<&ResumeCapabilityEntry> {
        self.entries
            .iter()
            .find(|entry| entry.content_id == request.content_id && entry.kind == request.kind)
    }

    fn capability(&self, request: &LaunchRequest) -> Option<ResumeCapability> {
        self.entry(request).map(|entry| entry.capability)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactIntegrity {
    pub relative: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResumeRecord {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    pub schema_version: u8,
    pub generation: u64,
    pub content_id: String,
    pub content_sha256: String,
    pub runner: VersionedId,
    pub core: Option<VersionedId>,
    pub capability: ResumeCapability,
    pub reason: CheckpointReason,
    pub timestamp_ms: u64,
    pub state: ArtifactIntegrity,
    pub sram: ArtifactIntegrity,
    pub screenshot: ArtifactIntegrity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CurrentPointer {
    schema: String,
    generation: u64,
    checksum: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResumeSummary {
    pub content_id: String,
    pub generation: u64,
    pub capability: ResumeCapability,
    pub reason: CheckpointReason,
    pub timestamp_ms: u64,
    pub screenshot: ArtifactIntegrity,
    pub status: String,
    pub choices: Vec<ResumeDecision>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResumeDecision {
    Resume,
    RetainedMatchingCore,
    ColdStartSram,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeResult {
    pub decision: ResumeDecision,
    pub content_id: String,
    pub generation: Option<u64>,
    pub used_sram: bool,
    pub effective_core: Option<VersionedId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeError(String);

impl ResumeError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ResumeError {}

#[derive(Clone, Debug)]
pub struct ResumeStore {
    root: PathBuf,
    config: ResumeCapabilityConfig,
}

impl ResumeStore {
    pub fn new(
        root: impl Into<PathBuf>,
        config: ResumeCapabilityConfig,
    ) -> Result<Self, ResumeError> {
        let root = root.into();
        fs::create_dir_all(root.join("generations")).map_err(error)?;
        fs::create_dir_all(root.join(".staging")).map_err(error)?;
        for directory in [
            root.clone(),
            root.join("generations"),
            root.join(".staging"),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o777)).map_err(error)?;
        }
        Ok(Self { root, config })
    }

    pub fn checkpoint(
        &self,
        request: &LaunchRequest,
        reason: CheckpointReason,
        state: &[u8],
        sram: &[u8],
        screenshot: &[u8],
        fault: CommitFault,
    ) -> Result<ResumeRecord, ResumeError> {
        let capability = self
            .config
            .capability(request)
            .ok_or_else(|| ResumeError::new("resume capability is undeclared"))?;
        if capability == ResumeCapability::None {
            return Err(ResumeError::new("resume is disabled for this adapter"));
        }
        if state.len() > MAX_STATE_BYTES
            || sram.len() > MAX_SRAM_BYTES
            || screenshot.len() > MAX_SCREENSHOT_BYTES
        {
            return Err(ResumeError::new("checkpoint artifact exceeds its bound"));
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
        let _ = fs::remove_dir_all(&stage);
        fs::create_dir(&stage).map_err(error)?;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o777)).map_err(error)?;
        let result = (|| {
            write_file(&stage.join("state.bin"), state)?;
            write_file(&stage.join("sram.bin"), sram)?;
            write_file(&stage.join("screenshot.png"), screenshot)?;
            if fault == CommitFault::Artifact {
                return Err(ResumeError::new("injected artifact commit failure"));
            }
            let record = ResumeRecord {
                schema: SCHEMA.into(),
                format: FORMAT.into(),
                schema_version: 1,
                generation,
                content_id: request.content_id.clone(),
                content_sha256: request.content_sha256.clone(),
                runner: request.runner.clone(),
                core: request.core.clone(),
                capability,
                reason,
                timestamp_ms: now_ms(),
                state: integrity("state.bin", state),
                sram: integrity("sram.bin", sram),
                screenshot: integrity("screenshot.png", screenshot),
            };
            let bytes = serde_json::to_vec_pretty(&record).map_err(error)?;
            if bytes.len() as u64 > MAX_RECORD_BYTES {
                return Err(ResumeError::new("resume record exceeds its bound"));
            }
            write_file(&stage.join("record.json"), &bytes)?;
            sync_dir(&stage)?;
            if fault == CommitFault::Metadata {
                return Err(ResumeError::new("injected metadata commit failure"));
            }
            if fault == CommitFault::Promotion {
                return Err(ResumeError::new("injected generation promotion failure"));
            }
            fs::rename(&stage, &target).map_err(error)?;
            sync_dir(&self.root.join("generations"))?;
            if fault == CommitFault::Pointer {
                return Err(ResumeError::new("injected current pointer failure"));
            }
            publish_current(&self.root, generation)?;
            self.prune(generation)?;
            Ok(record)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage);
        }
        result
    }

    pub fn list(&self, requests: &[LaunchRequest]) -> Vec<ResumeSummary> {
        let Some(current) = read_current(&self.root) else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(self.root.join("generations")) else {
            return Vec::new();
        };
        let mut records = entries
            .flatten()
            .filter_map(|entry| read_record(&entry.path()))
            .filter(|record| record.generation <= current)
            .map(|record| {
                let choices = requests
                    .iter()
                    .find(|request| request.content_id == record.content_id)
                    .map(|request| self.choices(request))
                    .unwrap_or_else(|| vec![ResumeDecision::Cancel]);
                ResumeSummary {
                    content_id: record.content_id,
                    generation: record.generation,
                    capability: record.capability,
                    reason: record.reason,
                    timestamp_ms: record.timestamp_ms,
                    screenshot: record.screenshot,
                    status: if choices == [ResumeDecision::Cancel] {
                        "unavailable".into()
                    } else {
                        "available".into()
                    },
                    choices,
                }
            })
            .collect::<Vec<_>>();
        entries_valid(&mut records);
        records
    }

    pub fn choices(&self, request: &LaunchRequest) -> Vec<ResumeDecision> {
        let Some(current) = read_current(&self.root) else {
            return vec![ResumeDecision::Cancel];
        };
        let Some(record) = read_record_for(&self.root, current, &request.content_id) else {
            return vec![ResumeDecision::Cancel];
        };
        let exact = record.content_sha256 == request.content_sha256
            && record.runner == request.runner
            && record.core == request.core;
        let mut choices = Vec::new();
        if exact && record.capability != ResumeCapability::SramOnly {
            choices.push(ResumeDecision::Resume);
        } else if self.retained_core(request, &record).is_some() {
            choices.push(ResumeDecision::RetainedMatchingCore);
        }
        if record.sram.size > 0 {
            choices.push(ResumeDecision::ColdStartSram);
        }
        choices.push(ResumeDecision::Cancel);
        choices
    }

    pub fn decide(
        &self,
        request: &LaunchRequest,
        decision: ResumeDecision,
    ) -> Result<ResumeResult, ResumeError> {
        let current = read_current(&self.root);
        let record = current
            .and_then(|generation| read_record_for(&self.root, generation, &request.content_id));
        match decision {
            ResumeDecision::Cancel => Ok(ResumeResult {
                decision,
                content_id: request.content_id.clone(),
                generation: None,
                used_sram: false,
                effective_core: None,
            }),
            ResumeDecision::ColdStartSram => {
                let record =
                    record.ok_or_else(|| ResumeError::new("SRAM cold start is unavailable"))?;
                if record.content_sha256 != request.content_sha256 || record.sram.size == 0 {
                    return Err(ResumeError::new("SRAM cold start is incompatible"));
                }
                Ok(ResumeResult {
                    decision,
                    content_id: request.content_id.clone(),
                    generation: Some(record.generation),
                    used_sram: true,
                    effective_core: None,
                })
            }
            ResumeDecision::Resume | ResumeDecision::RetainedMatchingCore => {
                let record =
                    record.ok_or_else(|| ResumeError::new("resume checkpoint is unavailable"))?;
                let exact = record.content_sha256 == request.content_sha256
                    && record.runner == request.runner
                    && record.core == request.core;
                let retained_core = self.retained_core(request, &record);
                if record.capability == ResumeCapability::SramOnly
                    || (decision == ResumeDecision::Resume && !exact)
                    || (decision == ResumeDecision::RetainedMatchingCore && retained_core.is_none())
                {
                    return Err(ResumeError::new("resume identity is incompatible"));
                }
                Ok(ResumeResult {
                    decision,
                    content_id: request.content_id.clone(),
                    generation: Some(record.generation),
                    used_sram: false,
                    effective_core: retained_core,
                })
            }
        }
    }

    fn retained_core(&self, request: &LaunchRequest, record: &ResumeRecord) -> Option<VersionedId> {
        let retained = self.config.entry(request)?.retained_core.clone()?;
        if record.content_sha256 != request.content_sha256
            || record.runner != request.runner
            || record.core.is_none()
            || record.core == request.core
            || record.core == Some(retained.clone())
        {
            return None;
        }
        Some(retained)
    }

    fn next_generation(&self) -> Result<u64, ResumeError> {
        let mut highest = 0;
        for entry in fs::read_dir(self.root.join("generations")).map_err(error)? {
            let name = entry.map_err(error)?.file_name();
            let Some(number) = name
                .to_string_lossy()
                .strip_prefix("generation-")
                .and_then(|value| value.parse::<u64>().ok())
            else {
                continue;
            };
            highest = highest.max(number);
        }
        Ok(highest.saturating_add(1))
    }

    fn prune(&self, current: u64) -> Result<(), ResumeError> {
        let mut generations = fs::read_dir(self.root.join("generations"))
            .map_err(error)?
            .flatten()
            .filter_map(|entry| {
                let number = entry
                    .file_name()
                    .to_string_lossy()
                    .strip_prefix("generation-")
                    .and_then(|value| value.parse::<u64>().ok())?;
                Some((number, entry.path()))
            })
            .filter(|(number, _)| *number <= current)
            .collect::<Vec<_>>();
        generations.sort_by_key(|(number, _)| *number);
        while generations.len() > MAX_HISTORY {
            let (_, path) = generations.remove(0);
            fs::remove_dir_all(path).map_err(error)?;
        }
        sync_dir(&self.root.join("generations"))
    }
}

fn entries_valid(entries: &mut Vec<ResumeSummary>) {
    entries.sort_by(|left, right| {
        content_rank(&left.content_id)
            .cmp(&content_rank(&right.content_id))
            .then(right.generation.cmp(&left.generation))
    });
    entries.truncate(MAX_HISTORY);
}

fn content_rank(content_id: &str) -> usize {
    match content_id {
        "nebula-nes" => 0,
        "mirror-ps1" => 1,
        "orbit-garden" => 2,
        "signal-workshop" => 3,
        _ => usize::MAX,
    }
}

fn read_record_for(root: &Path, current: u64, content_id: &str) -> Option<ResumeRecord> {
    (1..=current).rev().find_map(|generation| {
        let record = read_record(
            &root
                .join("generations")
                .join(format!("generation-{generation}")),
        )?;
        (record.content_id == content_id).then_some(record)
    })
}

fn read_record(path: &Path) -> Option<ResumeRecord> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let bytes = fs::read(path.join("record.json")).ok()?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return None;
    }
    let record: ResumeRecord = serde_json::from_slice(&bytes).ok()?;
    validate_record(&record).ok()?;
    let directory_generation = path
        .file_name()?
        .to_string_lossy()
        .strip_prefix("generation-")?
        .parse::<u64>()
        .ok()?;
    if directory_generation != record.generation {
        return None;
    }
    for artifact in [&record.state, &record.sram, &record.screenshot] {
        let artifact_path = path.join(&artifact.relative);
        let metadata = fs::symlink_metadata(&artifact_path).ok()?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != artifact.size
        {
            return None;
        }
        let bytes = fs::read(artifact_path).ok()?;
        if digest(&bytes) != artifact.sha256 {
            return None;
        }
    }
    Some(record)
}

fn validate_record(record: &ResumeRecord) -> Result<(), ResumeError> {
    if record.schema != SCHEMA
        || record.format != FORMAT
        || record.schema_version != 1
        || record.generation == 0
        || record.capability == ResumeCapability::None
        || record.timestamp_ms == 0
    {
        return Err(ResumeError::new("resume record identity is invalid"));
    }
    valid_id(&record.content_id)?;
    valid_hash(&record.content_sha256)?;
    valid_id(&record.runner.id)?;
    valid_version(&record.runner.version)?;
    if let Some(core) = &record.core {
        valid_id(&core.id)?;
        valid_version(&core.version)?;
    }
    for artifact in [&record.state, &record.sram, &record.screenshot] {
        if artifact.relative.starts_with('/')
            || artifact.relative.contains('\\')
            || artifact
                .relative
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(ResumeError::new("resume artifact path is not relative"));
        }
        valid_hash(&artifact.sha256)?;
    }
    Ok(())
}

fn publish_current(root: &Path, generation: u64) -> Result<(), ResumeError> {
    let temporary = root.join(".current.json.tmp");
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema": "trimui-resume-current/v1",
        "generation": generation,
        "checksum": pointer_checksum(generation),
    }))
    .map_err(error)?;
    let _ = fs::remove_file(&temporary);
    write_file(&temporary, &bytes)?;
    fs::rename(temporary, root.join("current.json")).map_err(error)?;
    sync_dir(root)
}

fn read_current(root: &Path) -> Option<u64> {
    if fs::read_dir(root).ok()?.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("current") && name != "current.json"
    }) {
        return None;
    }
    let path = root.join("current.json");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 {
        return None;
    }
    let pointer: CurrentPointer = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (pointer.schema == "trimui-resume-current/v1"
        && pointer.generation > 0
        && pointer.checksum == pointer_checksum(pointer.generation))
    .then_some(pointer.generation)
}

fn pointer_checksum(generation: u64) -> String {
    digest(format!("trimui-resume-current/v1:{generation}").as_bytes())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), ResumeError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.sync_all().map_err(error)
}

fn sync_dir(path: &Path) -> Result<(), ResumeError> {
    File::open(path).map_err(error)?.sync_all().map_err(error)
}

fn integrity(relative: &str, bytes: &[u8]) -> ArtifactIntegrity {
    ArtifactIntegrity {
        relative: relative.into(),
        size: bytes.len() as u64,
        sha256: digest(bytes),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &str) -> Result<(), ResumeError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ResumeError::new("resume digest is invalid"));
    }
    Ok(())
}

fn valid_id(value: &str) -> Result<(), ResumeError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(ResumeError::new("resume identifier is invalid"));
    }
    Ok(())
}

fn valid_version(value: &str) -> Result<(), ResumeError> {
    if value.is_empty()
        || value.len() > 32
        || value.split('.').count() != 3
        || value
            .split('.')
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(ResumeError::new("resume version is invalid"));
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn error(error: impl std::fmt::Display) -> ResumeError {
    ResumeError::new(error.to_string())
}
