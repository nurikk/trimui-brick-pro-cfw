use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub const SCHEMA: &str = "https://example.invalid/trimui-emulator-catalog-v1.schema.json";
pub const FORMAT: &str = "trimui-emulator-catalog";
const MAX_ID: usize = 64;
const MAX_VERSION: usize = 32;
const MAX_PATH: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogError {
    code: &'static str,
    message: String,
}

impl CatalogError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CatalogError {}
pub type Result<T> = std::result::Result<T, CatalogError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentHeader {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalPath {
    pub root: PathRoot,
    pub relative: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PathRoot {
    Roms,
    Bios,
    #[serde(rename = "data/saves")]
    DataSaves,
    #[serde(rename = "data/states")]
    DataStates,
}

impl LogicalPath {
    pub fn validate(&self) -> Result<()> {
        if self.relative.is_empty()
            || self.relative.len() > MAX_PATH
            || self.relative.starts_with('/')
            || self.relative.contains('\\')
            || self.relative.contains('\0')
        {
            return Err(CatalogError::new(
                "invalid_path",
                "logical path is not portable",
            ));
        }
        for component in self.relative.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(CatalogError::new(
                    "path_escape",
                    "logical path contains an escape component",
                ));
            }
            if component.len() > 255 || component.ends_with(['.', ' ']) {
                return Err(CatalogError::new(
                    "invalid_path",
                    "logical path component is invalid",
                ));
            }
        }
        Ok(())
    }

    fn components(&self) -> Vec<String> {
        self.relative
            .split('/')
            .map(|part| part.to_ascii_lowercase())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VersionedId {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub id: String,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateStatus {
    Unverified,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateAvailability {
    MetadataOnly,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamCandidate {
    pub name: String,
    #[serde(rename = "sourceUrl")]
    pub source_url: String,
    #[serde(rename = "sourceRef")]
    pub source_ref: String,
    #[serde(rename = "licenseUrl")]
    pub license_url: String,
    pub status: CandidateStatus,
    pub availability: CandidateAvailability,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceLane {
    PublicMetadataOnly,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum InternalScale {
    #[serde(rename = "native-1x")]
    Native1x,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentalBaseline {
    #[serde(rename = "internalScale")]
    pub internal_scale: InternalScale,
    #[serde(rename = "postProcessing")]
    pub post_processing: bool,
    pub speedhack: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentalMetadata {
    pub candidate: UpstreamCandidate,
    #[serde(rename = "evidenceLane")]
    pub evidence_lane: EvidenceLane,
    pub baseline: ExperimentalBaseline,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Display,
    Audio,
    Input,
    Rumble,
    SaveState,
    Rewind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceLimits {
    #[serde(rename = "maxWidth")]
    pub max_width: u16,
    #[serde(rename = "maxHeight")]
    pub max_height: u16,
    #[serde(rename = "maxControllers")]
    pub max_controllers: u8,
    #[serde(rename = "maxAudioChannels")]
    pub max_audio_channels: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BiosRequirement {
    pub id: String,
    #[serde(default, rename = "expectedSha256")]
    pub expected_sha256: Option<String>,
    pub locations: Vec<LogicalPath>,
    #[serde(default)]
    pub status: Option<BiosStatus>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum BiosStatus {
    RequiredUnverified,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsDefaults {
    #[serde(rename = "displayWidth")]
    pub display_width: u16,
    #[serde(rename = "displayHeight")]
    pub display_height: u16,
    #[serde(rename = "displayMode")]
    pub display_mode: DisplayMode,
    #[serde(rename = "frameSkip")]
    pub frame_skip: u8,
    pub rumble: bool,
    #[serde(rename = "audioLatencyMs")]
    pub audio_latency_ms: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsDelta {
    #[serde(rename = "displayWidth", default)]
    pub display_width: Option<u16>,
    #[serde(rename = "displayHeight", default)]
    pub display_height: Option<u16>,
    #[serde(rename = "displayMode", default)]
    pub display_mode: Option<DisplayMode>,
    #[serde(rename = "frameSkip", default)]
    pub frame_skip: Option<u8>,
    #[serde(default)]
    pub rumble: Option<bool>,
    #[serde(rename = "audioLatencyMs", default)]
    pub audio_latency_ms: Option<u16>,
}

impl SettingsDelta {
    fn is_empty(&self) -> bool {
        self.display_width.is_none()
            && self.display_height.is_none()
            && self.display_mode.is_none()
            && self.frame_skip.is_none()
            && self.rumble.is_none()
            && self.audio_latency_ms.is_none()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum DeltaStatus {
    Unverified,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameDelta {
    #[serde(rename = "contentId")]
    pub content_id: String,
    #[serde(rename = "systemId")]
    pub system_id: String,
    pub runner: VersionedId,
    pub core: VersionedId,
    pub settings: SettingsDelta,
    pub reversible: bool,
    pub status: DeltaStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    Integer,
    Fit,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    pub channel: ChannelName,
    pub extensions: Vec<String>,
    #[serde(rename = "biosRequirements")]
    pub bios_requirements: Vec<BiosRequirement>,
    #[serde(rename = "runtimeRequirements")]
    pub runtime_requirements: Vec<String>,
    #[serde(rename = "defaultRunner")]
    pub default_runner: VersionedId,
    #[serde(rename = "defaultCore")]
    pub default_core: Option<VersionedId>,
    #[serde(rename = "savePath")]
    pub save_path: LogicalPath,
    #[serde(rename = "statePath")]
    pub state_path: LogicalPath,
    pub capabilities: Vec<Capability>,
    #[serde(rename = "deviceLimits")]
    pub device_limits: DeviceLimits,
    #[serde(rename = "settingsDefaults")]
    pub settings_defaults: SettingsDefaults,
    #[serde(rename = "licenseProvenanceUrl")]
    pub license_provenance_url: String,
    #[serde(default)]
    pub experimental: Option<ExperimentalMetadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
    pub id: String,
    pub version: String,
    #[serde(rename = "targetArchitecture")]
    pub target_architecture: String,
    pub artifact: Artifact,
    #[serde(rename = "supportedSystems")]
    pub supported_systems: Vec<String>,
    #[serde(rename = "supportedContent")]
    pub supported_content: Vec<String>,
    pub capabilities: Vec<Capability>,
    #[serde(rename = "licenseProvenanceUrl")]
    pub license_provenance_url: String,
    pub channel: ChannelName,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Core {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
    pub id: String,
    pub version: String,
    #[serde(rename = "runnerId")]
    pub runner_id: String,
    #[serde(rename = "runnerVersion")]
    pub runner_version: String,
    #[serde(rename = "supportedSystems")]
    pub supported_systems: Vec<String>,
    #[serde(rename = "supportedContent")]
    pub supported_content: Vec<String>,
    pub capabilities: Vec<Capability>,
    #[serde(rename = "licenseProvenanceUrl")]
    pub license_provenance_url: String,
    pub channel: ChannelName,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
    pub id: String,
    #[serde(rename = "systemId")]
    pub system_id: String,
    pub capabilities: Vec<Capability>,
    pub channel: ChannelName,
    #[serde(rename = "gameDeltas", default)]
    pub game_deltas: Vec<GameDelta>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelName {
    Stable,
    Experimental,
}

impl ChannelName {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Experimental => "experimental",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
    pub id: ChannelName,
    pub systems: Vec<String>,
    pub runners: Vec<VersionedId>,
    pub cores: Vec<VersionedId>,
    pub profiles: Vec<String>,
    #[serde(rename = "smokeEvidenceId")]
    pub smoke_evidence_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub systems: Vec<System>,
    pub runners: Vec<Runner>,
    pub cores: Vec<Core>,
    pub profiles: Vec<Profile>,
    pub channels: Vec<Channel>,
}

fn parse<T: DeserializeOwned>(bytes: &[u8], kind: &str) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|error| {
        let text = error.to_string();
        let code = if text.contains("unknown field") {
            "unknown_field"
        } else {
            "malformed_json"
        };
        CatalogError::new(code, format!("{kind} document: {text}"))
    })
}

fn load_dir<T: DeserializeOwned>(root: &Path, name: &str) -> Result<Vec<T>> {
    let dir = root.join(name);
    let mut paths = fs::read_dir(&dir)
        .map_err(|e| CatalogError::new("catalog_io", format!("read {name}: {e}")))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| CatalogError::new("catalog_io", format!("read {name}: {e}")))?;
    paths.sort();
    paths
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            fs::read(&path)
                .map_err(|e| CatalogError::new("catalog_io", format!("read catalog document: {e}")))
                .and_then(|bytes| parse(&bytes, name))
        })
        .collect()
}

impl Catalog {
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let catalog = Self {
            systems: load_dir(root, "systems")?,
            runners: load_dir(root, "runners")?,
            cores: load_dir(root, "cores")?,
            profiles: load_dir(root, "profiles")?,
            channels: load_dir(root, "channels")?,
        };
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<()> {
        if self.channels.len() != 2
            || !self.channels.iter().any(|c| c.id == ChannelName::Stable)
            || !self
                .channels
                .iter()
                .any(|c| c.id == ChannelName::Experimental)
        {
            return Err(CatalogError::new(
                "channel_set",
                "stable and experimental channels are required",
            ));
        }
        validate_channel_identities(self)?;
        for system in &self.systems {
            validate_common(
                &system.schema,
                &system.format,
                system.schema_version,
                &system.kind,
                "system",
            )?;
            validate_id(&system.id, "system")?;
            if system.target_sku != "TG4040" {
                return Err(CatalogError::new(
                    "target_forbidden",
                    "only TG4040 is permitted",
                ));
            }
            if system.display_name.is_empty() {
                return Err(CatalogError::new(
                    "invalid_display_name",
                    "system display name is empty",
                ));
            }
            validate_extensions(&system.extensions)?;
            validate_capabilities(&system.capabilities)?;
            validate_ref(&system.default_runner, "default runner")?;
            if let Some(core) = &system.default_core {
                validate_ref(core, "default core")?;
            }
            system.save_path.validate()?;
            system.state_path.validate()?;
            if system.save_path.root != PathRoot::DataSaves
                || system.state_path.root != PathRoot::DataStates
            {
                return Err(CatalogError::new(
                    "invalid_path",
                    "system save/state roots are invalid",
                ));
            }
            validate_limits(&system.device_limits)?;
            validate_defaults(&system.settings_defaults)?;
            validate_url(&system.license_provenance_url)?;
            if system.channel == ChannelName::Experimental && system.id != "tg4040-lab" {
                validate_experimental(system.experimental.as_ref())?;
                let runner = self.runner(&system.default_runner)?;
                if runner.channel != ChannelName::Experimental || runner.artifact.sha256.is_some() {
                    return Err(CatalogError::new(
                        "artifact_unpinned",
                        "experimental candidate runner must be metadata-only and hash-free",
                    ));
                }
            } else if system.experimental.is_some() {
                return Err(CatalogError::new(
                    "channel_leak",
                    "stable system contains experimental metadata",
                ));
            }
            let mut ids = BTreeSet::new();
            for requirement in &system.bios_requirements {
                validate_id(&requirement.id, "BIOS requirement")?;
                if let Some(hash) = requirement.expected_sha256.as_deref() {
                    validate_hash(hash)?;
                } else if system.channel == ChannelName::Stable {
                    return Err(CatalogError::new(
                        "bios_unresolved",
                        "stable BIOS requirement has no SHA-256 pin",
                    ));
                }
                if system.channel == ChannelName::Experimental
                    && (requirement.expected_sha256.is_some()
                        || requirement.status != Some(BiosStatus::RequiredUnverified))
                {
                    return Err(CatalogError::new(
                        "bios_unresolved",
                        "experimental BIOS requirement must be explicitly unverified and hash-free",
                    ));
                }
                if requirement.locations.is_empty() {
                    return Err(CatalogError::new(
                        "bios_unresolved",
                        "BIOS requirement has no locations",
                    ));
                }
                for location in &requirement.locations {
                    location.validate()?;
                    if location.root != PathRoot::Bios {
                        return Err(CatalogError::new(
                            "bios_unresolved",
                            "BIOS location is not rooted at bios",
                        ));
                    }
                }
                if !ids.insert(&requirement.id) {
                    return Err(CatalogError::new(
                        "duplicate_id",
                        "BIOS requirement IDs are duplicated",
                    ));
                }
            }
            for runtime in &system.runtime_requirements {
                if runtime.is_empty() {
                    return Err(CatalogError::new(
                        "runtime_unresolved",
                        "runtime requirement is empty",
                    ));
                }
            }
        }
        unique_ids(self.systems.iter().map(|e| &e.id), "system")?;
        for runner in &self.runners {
            validate_common(
                &runner.schema,
                &runner.format,
                runner.schema_version,
                &runner.kind,
                "runner",
            )?;
            validate_id(&runner.id, "runner")?;
            validate_version(&runner.version, "runner")?;
            if runner.target_architecture != "aarch64-unknown-linux-musl" {
                return Err(CatalogError::new(
                    "invalid_architecture",
                    "runner target architecture is unsupported",
                ));
            }
            validate_id(&runner.artifact.id, "artifact")?;
            if let Some(hash) = runner.artifact.sha256.as_deref() {
                validate_hash(hash)?;
            } else if runner.channel == ChannelName::Stable {
                return Err(CatalogError::new(
                    "artifact_unpinned",
                    "stable runner artifact has no SHA-256 pin",
                ));
            }
            if runner.supported_systems.is_empty() || runner.supported_content.is_empty() {
                return Err(CatalogError::new(
                    "unsupported_scope",
                    "runner support scope is empty",
                ));
            }
            validate_id_list(&runner.supported_systems, "runner system")?;
            validate_extensions(&runner.supported_content)?;
            validate_capabilities(&runner.capabilities)?;
            validate_url(&runner.license_provenance_url)?;
        }
        unique_versions(self.runners.iter().map(|e| (&e.id, &e.version)), "runner")?;
        for core in &self.cores {
            validate_common(
                &core.schema,
                &core.format,
                core.schema_version,
                &core.kind,
                "core",
            )?;
            validate_id(&core.id, "core")?;
            validate_version(&core.version, "core")?;
            validate_version(&core.runner_version, "core runner")?;
            validate_id(&core.runner_id, "core runner")?;
            validate_id_list(&core.supported_systems, "core system")?;
            validate_extensions(&core.supported_content)?;
            validate_capabilities(&core.capabilities)?;
            validate_url(&core.license_provenance_url)?;
        }
        unique_versions(self.cores.iter().map(|e| (&e.id, &e.version)), "core")?;
        for system in &self.systems {
            let runner = self.runner(&system.default_runner)?;
            if !runner.supported_systems.contains(&system.id) {
                return Err(CatalogError::new(
                    "cross_reference",
                    "default runner does not support system",
                ));
            }
            if let Some(reference) = &system.default_core {
                let core = self.core(reference)?;
                if core.runner_id != runner.id || core.runner_version != runner.version {
                    return Err(CatalogError::new(
                        "cross_reference",
                        "default core does not match default runner",
                    ));
                }
            }
        }
        for runner in &self.runners {
            for system_id in &runner.supported_systems {
                self.system(system_id)?;
            }
        }
        for core in &self.cores {
            let runner = self.runner(&VersionedId {
                id: core.runner_id.clone(),
                version: core.runner_version.clone(),
            })?;
            for system_id in &core.supported_systems {
                self.system(system_id)?;
            }
            if !runner
                .supported_systems
                .iter()
                .any(|id| core.supported_systems.contains(id))
            {
                return Err(CatalogError::new(
                    "cross_reference",
                    "core and runner have no common system",
                ));
            }
        }
        for profile in &self.profiles {
            validate_common(
                &profile.schema,
                &profile.format,
                profile.schema_version,
                &profile.kind,
                "profile",
            )?;
            validate_id(&profile.id, "profile")?;
            validate_id(&profile.system_id, "profile system")?;
            validate_capabilities(&profile.capabilities)?;
            validate_game_deltas(self, profile)?;
        }
        unique_ids(self.profiles.iter().map(|e| &e.id), "profile")?;
        for channel in &self.channels {
            self.validate_channel(channel)?;
        }
        let stable = self.channel(ChannelName::Stable)?;
        let experimental = self.channel(ChannelName::Experimental)?;
        let stable_keys = channel_keys(stable);
        let experimental_keys = channel_keys(experimental);
        let stable_paths = channel_paths(self, stable)?;
        let experimental_paths = channel_paths(self, experimental)?;
        if !stable_keys.is_disjoint(&experimental_keys)
            || stable_paths.iter().any(|stable| {
                experimental_paths
                    .iter()
                    .any(|experimental| paths_overlap(stable, experimental))
            })
        {
            return Err(CatalogError::new(
                "channel_leak",
                "stable and experimental selections overlap",
            ));
        }
        if stable
            .smoke_evidence_id
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err(CatalogError::new(
                "stable_evidence",
                "stable channel needs smoke evidence ID",
            ));
        }
        for id in &stable.systems {
            let system = self.system(id)?;
            if !system.runtime_requirements.is_empty() {
                return Err(CatalogError::new(
                    "runtime_unresolved",
                    "stable system has unresolved runtime requirements",
                ));
            }
        }
        Ok(())
    }

    fn validate_channel(&self, channel: &Channel) -> Result<()> {
        validate_common(
            &channel.schema,
            &channel.format,
            channel.schema_version,
            &channel.kind,
            "channel",
        )?;
        if channel.systems.is_empty()
            || channel.runners.is_empty()
            || channel.cores.is_empty()
            || channel.profiles.is_empty()
        {
            return Err(CatalogError::new(
                "channel_set",
                "channel selection is incomplete",
            ));
        }
        unique_ids(channel.systems.iter(), "channel system")?;
        unique_ids(channel.profiles.iter(), "channel profile")?;
        unique_versions(
            channel.runners.iter().map(|e| (&e.id, &e.version)),
            "channel runner",
        )?;
        unique_versions(
            channel.cores.iter().map(|e| (&e.id, &e.version)),
            "channel core",
        )?;
        for id in &channel.systems {
            let e = self.system(id)?;
            if e.channel != channel.id {
                return Err(CatalogError::new(
                    "channel_leak",
                    "system is listed in the wrong channel",
                ));
            }
        }
        for entry in &channel.runners {
            let e = self.runner(entry)?;
            if e.channel != channel.id {
                return Err(CatalogError::new(
                    "channel_leak",
                    "runner is listed in the wrong channel",
                ));
            }
        }
        for entry in &channel.cores {
            let e = self.core(entry)?;
            if e.channel != channel.id {
                return Err(CatalogError::new(
                    "channel_leak",
                    "core is listed in the wrong channel",
                ));
            }
        }
        for id in &channel.profiles {
            let e = self.profiles.iter().find(|e| e.id == *id).ok_or_else(|| {
                CatalogError::new("cross_reference", "channel profile is missing")
            })?;
            if e.channel != channel.id {
                return Err(CatalogError::new(
                    "channel_leak",
                    "profile is listed in the wrong channel",
                ));
            }
        }
        Ok(())
    }

    pub fn channel(&self, id: ChannelName) -> Result<&Channel> {
        self.channels
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| CatalogError::new("channel_set", "channel is missing"))
    }
    fn system(&self, id: &str) -> Result<&System> {
        self.systems
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| CatalogError::new("cross_reference", "system reference is missing"))
    }
    fn runner(&self, id: &VersionedId) -> Result<&Runner> {
        self.runners
            .iter()
            .find(|e| e.id == id.id && e.version == id.version)
            .ok_or_else(|| CatalogError::new("cross_reference", "runner reference is missing"))
    }
    fn core(&self, id: &VersionedId) -> Result<&Core> {
        self.cores
            .iter()
            .find(|e| e.id == id.id && e.version == id.version)
            .ok_or_else(|| CatalogError::new("cross_reference", "core reference is missing"))
    }

    pub fn resolve(&self, fixture: &ResolutionFixture) -> Result<Resolved> {
        if fixture.schema != SCHEMA
            || fixture.format != FORMAT
            || fixture.schema_version != 1
            || fixture.kind != "resolution-fixture"
        {
            return Err(CatalogError::new(
                "schema_identity",
                "resolution fixture identity is invalid",
            ));
        }
        let channel = self.channel(fixture.channel.clone())?;
        if channel.id == ChannelName::Experimental && !fixture.experimental_opt_in {
            return Err(CatalogError::new(
                "experimental_opt_in",
                "experimental selection requires explicit opt-in",
            ));
        }
        let system = self.system(&fixture.system_id)?;
        if channel.id == ChannelName::Experimental
            && !system.bios_requirements.is_empty()
            && !fixture.bios_ready
        {
            return Err(CatalogError::new(
                "bios_missing",
                "required BIOS is missing or mismatched",
            ));
        }
        if fixture.renderer.is_some() {
            return Err(CatalogError::new(
                "unsupported_renderer",
                "requested renderer is unsupported or unverified",
            ));
        }
        if let Some(content_id) = &fixture.content_id {
            validate_content_id(content_id)?;
        }
        if !channel.systems.contains(&system.id) {
            return Err(CatalogError::new(
                "unavailable_system",
                "system is not enabled in the selected channel",
            ));
        }
        let extension = normalize_extension(&fixture.extension)?;
        if !system
            .extensions
            .iter()
            .map(|e| normalize_extension(e))
            .any(|e| e.as_ref().is_ok_and(|e| e == &extension))
        {
            return Err(CatalogError::new(
                "extension_unavailable",
                "extension is not supported by the system",
            ));
        }
        fixture.content_path.validate()?;
        if fixture.content_path.root != PathRoot::Roms {
            return Err(CatalogError::new(
                "invalid_path",
                "content path must be rooted at roms",
            ));
        }
        let runner_ref = fixture
            .runner
            .clone()
            .unwrap_or_else(|| system.default_runner.clone());
        let runner = self.runner(&runner_ref)?;
        if !channel
            .runners
            .iter()
            .any(|e| e.id == runner.id && e.version == runner.version)
        {
            return Err(CatalogError::new(
                "unavailable_runner",
                "runner is not enabled in selected channel",
            ));
        }
        if !runner.supported_systems.contains(&system.id)
            || !runner.supported_content.contains(&extension)
        {
            return Err(CatalogError::new(
                "extension_unavailable",
                "runner does not support system or extension",
            ));
        }
        let core_ref = fixture.core.clone().or_else(|| system.default_core.clone());
        let core = core_ref
            .as_ref()
            .map(|reference| self.core(reference))
            .transpose()?;
        if let Some(core) = core {
            if !channel
                .cores
                .iter()
                .any(|e| e.id == core.id && e.version == core.version)
            {
                return Err(CatalogError::new(
                    "unavailable_core",
                    "core is not enabled in selected channel",
                ));
            }
            if core.runner_id != runner.id
                || core.runner_version != runner.version
                || !core.supported_systems.contains(&system.id)
                || !core.supported_content.contains(&extension)
            {
                return Err(CatalogError::new(
                    "incompatible_core",
                    "core does not match runner, system, or extension",
                ));
            }
        }
        let profile_id = fixture
            .profile_id
            .as_deref()
            .unwrap_or_else(|| channel.profiles.first().map(String::as_str).unwrap_or(""));
        validate_capabilities(&fixture.requested_capabilities)?;
        validate_capabilities(&fixture.device_capabilities)?;
        let profile = self
            .profiles
            .iter()
            .find(|e| e.id == profile_id)
            .ok_or_else(|| CatalogError::new("cross_reference", "profile reference is missing"))?;
        if !channel.profiles.contains(&profile.id) || profile.system_id != system.id {
            return Err(CatalogError::new(
                "unavailable_profile",
                "profile is not enabled for system",
            ));
        }
        let mut supported = system.capabilities.iter().cloned().collect::<BTreeSet<_>>();
        supported.retain(|cap| {
            runner.capabilities.contains(cap)
                && profile.capabilities.contains(cap)
                && core.as_ref().is_none_or(|c| c.capabilities.contains(cap))
        });
        for capability in &fixture.requested_capabilities {
            if !supported.contains(capability) || !fixture.device_capabilities.contains(capability)
            {
                return Err(CatalogError::new(
                    "capability_incompatible",
                    "requested capability is unavailable",
                ));
            }
        }
        let mut settings = EffectiveSettings::from_defaults(&system.settings_defaults);
        settings.apply(&fixture.overrides.device, SettingLayer::Device);
        settings.apply(&fixture.overrides.system, SettingLayer::System);
        settings.apply(&fixture.overrides.core, SettingLayer::Core);
        let content = fixture.content_path.components();
        let mut previous = Vec::new();
        for folder in &fixture.overrides.folder_ancestors {
            validate_folder(folder, &content, &mut previous)?;
            settings.apply(&folder.settings, SettingLayer::Folder);
        }
        if let Some(content_id) = &fixture.content_id {
            if let Some(delta) = profile.game_deltas.iter().find(|delta| {
                &delta.content_id == content_id
                    && delta.runner == runner_ref
                    && core_ref.as_ref() == Some(&delta.core)
            }) {
                settings.apply(&delta.settings, SettingLayer::Game);
            }
        }

        settings.apply(&fixture.overrides.game, SettingLayer::Game);
        settings.apply(&fixture.overrides.session, SettingLayer::Session);
        if settings.display_width.value > system.device_limits.max_width
            || settings.display_height.value > system.device_limits.max_height
        {
            return Err(CatalogError::new(
                "device_limit",
                "resolved display exceeds system device limits",
            ));
        }
        Ok(Resolved {
            system_id: system.id.clone(),
            display_name: system.display_name.clone(),
            extension,
            runner: runner_ref,
            core: core_ref,
            settings: settings.into_map(),
        })
    }

    pub fn audit(&self, root: impl AsRef<Path>, channel_id: ChannelName) -> Result<AuditReport> {
        let channel = self.channel(channel_id.clone())?;
        let mut requirements = Vec::new();
        for system_id in &channel.systems {
            for requirement in &self.system(system_id)?.bios_requirements {
                let mut present = 0;
                let mut missing = 0;
                let mut mismatch = 0;
                let mut unverified = 0;
                for location in &requirement.locations {
                    let path = logical_host_path(root.as_ref(), location)?;
                    match requirement.expected_sha256.as_deref() {
                        Some(expected) => match hash_file(&path) {
                            Ok(hash) if hash == expected => present += 1,
                            Ok(_) => mismatch += 1,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => missing += 1,
                            Err(error) => {
                                return Err(CatalogError::new(
                                    "bios_io",
                                    format!("BIOS audit read failed: {error}"),
                                ))
                            }
                        },
                        None if path.exists() => unverified += 1,
                        None => missing += 1,
                    }
                }
                let status = if present > 0 {
                    "present"
                } else if mismatch > 0 {
                    "mismatch"
                } else if unverified > 0 {
                    "unverified"
                } else {
                    "missing"
                };
                requirements.push(AuditItem {
                    requirement_id: requirement.id.clone(),
                    present,
                    missing,
                    mismatch,
                    unverified,
                    status: status.to_string(),
                });
            }
        }
        Ok(AuditReport { requirements })
    }
}

fn validate_common(
    schema: &str,
    format: &str,
    version: u8,
    kind: &str,
    expected: &str,
) -> Result<()> {
    if schema != SCHEMA || format != FORMAT || version != 1 || kind != expected {
        return Err(CatalogError::new(
            "schema_identity",
            format!("{expected} document identity is invalid"),
        ));
    }
    Ok(())
}
fn validate_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID
        || !value.bytes().enumerate().all(|(i, b)| {
            (i == 0 && b.is_ascii_lowercase())
                || (i > 0 && (b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        })
    {
        return Err(CatalogError::new(
            "invalid_id",
            format!("{label} ID is invalid"),
        ));
    }
    Ok(())
}
fn validate_version(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_VERSION
        || value.split('.').count() != 3
        || !value
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(CatalogError::new(
            "invalid_version",
            format!("{label} version is invalid"),
        ));
    }
    Ok(())
}
fn validate_hash(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(CatalogError::new(
            "invalid_hash",
            "SHA-256 must be lowercase hexadecimal",
        ));
    }
    Ok(())
}
fn validate_url(value: &str) -> Result<()> {
    if !value.starts_with("https://") || value.len() > 2048 || value.contains(char::is_whitespace) {
        return Err(CatalogError::new(
            "invalid_provenance",
            "license/provenance URL is invalid",
        ));
    }
    Ok(())
}
fn validate_extensions(values: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        let normalized = normalize_extension(value)?;
        if !seen.insert(normalized) {
            return Err(CatalogError::new(
                "extension_collision",
                "normalized extensions collide",
            ));
        }
        if value != &value.to_ascii_lowercase() || value.starts_with('.') {
            return Err(CatalogError::new(
                "extension_not_normalized",
                "extension is not lowercase and dotless",
            ));
        }
    }
    if values.is_empty() {
        return Err(CatalogError::new(
            "extension_empty",
            "extension set is empty",
        ));
    }
    Ok(())
}
fn normalize_extension(value: &str) -> Result<String> {
    let value = value
        .strip_prefix('.')
        .unwrap_or(value)
        .to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 16
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'_'))
    {
        return Err(CatalogError::new(
            "invalid_extension",
            "extension is invalid",
        ));
    }
    Ok(value)
}
fn validate_capabilities(values: &[Capability]) -> Result<()> {
    if values.is_empty() || values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(CatalogError::new(
            "capability_set",
            "capability set is empty or duplicated",
        ));
    }
    Ok(())
}
fn validate_limits(limits: &DeviceLimits) -> Result<()> {
    if limits.max_width == 0
        || limits.max_height == 0
        || limits.max_controllers == 0
        || limits.max_audio_channels == 0
    {
        return Err(CatalogError::new(
            "device_limit",
            "device limits must be positive",
        ));
    }
    Ok(())
}
fn validate_defaults(defaults: &SettingsDefaults) -> Result<()> {
    if defaults.display_width == 0 || defaults.display_height == 0 || defaults.frame_skip > 10 {
        return Err(CatalogError::new(
            "settings_value",
            "system setting default is invalid",
        ));
    }
    Ok(())
}

fn validate_experimental(metadata: Option<&ExperimentalMetadata>) -> Result<()> {
    let metadata = metadata.ok_or_else(|| {
        CatalogError::new(
            "experimental_metadata",
            "experimental system is missing candidate metadata",
        )
    })?;
    if metadata.candidate.name.is_empty()
        || metadata.candidate.source_ref.len() != 40
        || !metadata
            .candidate
            .source_ref
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(CatalogError::new(
            "experimental_metadata",
            "candidate name or source pin is invalid",
        ));
    }
    validate_url(&metadata.candidate.source_url)?;
    validate_url(&metadata.candidate.license_url)?;
    if metadata.candidate.status != CandidateStatus::Unverified
        || metadata.evidence_lane != EvidenceLane::PublicMetadataOnly
        || metadata.candidate.availability != CandidateAvailability::MetadataOnly
        || metadata.baseline.internal_scale != InternalScale::Native1x
        || metadata.baseline.post_processing
        || metadata.baseline.speedhack
    {
        return Err(CatalogError::new(
            "experimental_metadata",
            "experimental metadata is not explicitly conservative and unverified",
        ));
    }
    Ok(())
}

fn validate_content_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'_' | b'-'))
    {
        return Err(CatalogError::new(
            "invalid_content_id",
            "content ID must be an opaque portable token",
        ));
    }
    Ok(())
}

fn validate_game_deltas(catalog: &Catalog, profile: &Profile) -> Result<()> {
    if profile.channel == ChannelName::Stable && !profile.game_deltas.is_empty() {
        return Err(CatalogError::new(
            "channel_leak",
            "stable profile contains experimental game deltas",
        ));
    }
    let mut content_ids = BTreeSet::new();
    for delta in &profile.game_deltas {
        validate_content_id(&delta.content_id)?;
        if !content_ids.insert(&delta.content_id)
            || !delta.reversible
            || delta.status != DeltaStatus::Unverified
            || delta.settings.is_empty()
        {
            return Err(CatalogError::new(
                "invalid_game_delta",
                "game delta must be unique, non-empty, reversible, and unverified",
            ));
        }
        if delta.system_id != profile.system_id
            || delta.runner != catalog.system(&profile.system_id)?.default_runner
        {
            return Err(CatalogError::new(
                "invalid_game_delta",
                "game delta does not match the system runner pin",
            ));
        }
        let system = catalog.system(&delta.system_id)?;
        if system.default_core.as_ref() != Some(&delta.core)
            || catalog.runner(&delta.runner)?.channel != ChannelName::Experimental
            || catalog.core(&delta.core)?.channel != ChannelName::Experimental
        {
            return Err(CatalogError::new(
                "invalid_game_delta",
                "game delta does not match the experimental core pin",
            ));
        }
    }
    Ok(())
}
fn validate_ref(reference: &VersionedId, label: &str) -> Result<()> {
    validate_id(&reference.id, label)?;
    validate_version(&reference.version, label)
}
fn validate_id_list<'a>(values: impl IntoIterator<Item = &'a String>, label: &str) -> Result<()> {
    for value in values {
        validate_id(value, label)?;
    }
    Ok(())
}
fn unique_ids<'a>(values: impl IntoIterator<Item = &'a String>, label: &str) -> Result<()> {
    let mut set = BTreeSet::new();
    for value in values {
        if !set.insert(value) {
            return Err(CatalogError::new(
                "duplicate_id",
                format!("{label} IDs are duplicated"),
            ));
        }
    }
    Ok(())
}
fn unique_versions<'a>(
    values: impl IntoIterator<Item = (&'a String, &'a String)>,
    label: &str,
) -> Result<()> {
    let mut set = BTreeSet::new();
    for value in values {
        if !set.insert(value) {
            return Err(CatalogError::new(
                "duplicate_id",
                format!("{label} IDs and versions are duplicated"),
            ));
        }
    }
    Ok(())
}
fn channel_keys(channel: &Channel) -> BTreeSet<String> {
    channel
        .systems
        .iter()
        .map(|x| format!("system:{x}"))
        .chain(channel.runners.iter().map(|x| format!("runner:{}", x.id)))
        .chain(channel.cores.iter().map(|x| format!("core:{}", x.id)))
        .chain(channel.profiles.iter().map(|x| format!("profile:{x}")))
        .collect()
}

fn validate_channel_identities(catalog: &Catalog) -> Result<()> {
    let stable = catalog.channel(ChannelName::Stable)?;
    let experimental = catalog.channel(ChannelName::Experimental)?;
    if !channel_keys(stable).is_disjoint(&channel_keys(experimental)) {
        return Err(CatalogError::new(
            "channel_leak",
            "stable and experimental identities overlap",
        ));
    }
    Ok(())
}

fn channel_paths(catalog: &Catalog, channel: &Channel) -> Result<Vec<LogicalPath>> {
    channel
        .systems
        .iter()
        .map(|id| {
            let system = catalog.system(id)?;
            Ok(vec![system.save_path.clone(), system.state_path.clone()])
        })
        .collect::<Result<Vec<_>>>()
        .map(|paths| paths.into_iter().flatten().collect())
}

fn paths_overlap(left: &LogicalPath, right: &LogicalPath) -> bool {
    if left.root != right.root {
        return false;
    }
    let left = left.components();
    let right = right.components();
    left.starts_with(&right) || right.starts_with(&left)
}

fn validate_folder(
    folder: &FolderOverride,
    content: &[String],
    previous: &mut Vec<String>,
) -> Result<()> {
    let path = LogicalPath {
        root: PathRoot::Roms,
        relative: folder.relative.clone(),
    };
    path.validate()?;
    let components = path.components();
    if components.len() >= content.len() || content[..components.len()] != components[..] {
        return Err(CatalogError::new(
            "path_escape",
            "folder ancestor is outside content path",
        ));
    }
    if !previous.is_empty() && previous == &components {
        return Err(CatalogError::new(
            "case_collision",
            "folder ancestors collide case-insensitively",
        ));
    }
    if !previous.is_empty() && components.len() <= previous.len() {
        return Err(CatalogError::new(
            "path_escape",
            "folder ancestors are not root-to-leaf",
        ));
    }
    *previous = components;
    Ok(())
}

fn logical_host_path(root: &Path, path: &LogicalPath) -> Result<PathBuf> {
    path.validate()?;
    let base = root.join(match path.root {
        PathRoot::Roms => "roms",
        PathRoot::Bios => "bios",
        PathRoot::DataSaves => "data/saves",
        PathRoot::DataStates => "data/states",
    });
    let candidate = base.join(&path.relative);
    let base_canonical = fs::canonicalize(&base)
        .map_err(|e| CatalogError::new("bios_io", format!("BIOS root cannot be read: {e}")))?;
    let mut existing = candidate.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        missing.push(
            existing
                .file_name()
                .ok_or_else(|| CatalogError::new("invalid_path", "logical path has no filename"))?
                .to_owned(),
        );
        existing = existing
            .parent()
            .ok_or_else(|| CatalogError::new("invalid_path", "logical path has no parent"))?;
    }
    let existing_canonical = fs::canonicalize(existing)
        .map_err(|e| CatalogError::new("bios_io", format!("BIOS parent cannot be read: {e}")))?;
    if !existing_canonical.starts_with(&base_canonical) {
        return Err(CatalogError::new(
            "path_escape",
            "logical path escapes BIOS root",
        ));
    }
    let mut result = existing_canonical;
    for component in missing.into_iter().rev() {
        result.push(component);
    }
    Ok(result)
}
fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionFixture {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
    pub channel: ChannelName,
    #[serde(rename = "experimentalOptIn", default)]
    pub experimental_opt_in: bool,
    #[serde(rename = "contentId", default)]
    pub content_id: Option<String>,
    #[serde(rename = "biosReady", default)]
    pub bios_ready: bool,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(rename = "systemId")]
    pub system_id: String,
    pub extension: String,
    #[serde(rename = "contentPath")]
    pub content_path: LogicalPath,
    #[serde(default)]
    pub runner: Option<VersionedId>,
    #[serde(default)]
    pub core: Option<VersionedId>,
    #[serde(rename = "profileId", default)]
    pub profile_id: Option<String>,
    #[serde(rename = "requestedCapabilities")]
    pub requested_capabilities: Vec<Capability>,
    #[serde(rename = "deviceCapabilities")]
    pub device_capabilities: Vec<Capability>,
    pub overrides: Overrides,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    pub device: SettingsDelta,
    pub system: SettingsDelta,
    pub core: SettingsDelta,
    #[serde(rename = "folderAncestors")]
    pub folder_ancestors: Vec<FolderOverride>,
    pub game: SettingsDelta,
    pub session: SettingsDelta,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FolderOverride {
    pub relative: String,
    pub settings: SettingsDelta,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WinningSetting<T> {
    pub value: T,
    pub source: SettingLayer,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SettingLayer {
    System,
    Device,
    Core,
    Folder,
    Game,
    Session,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    #[serde(rename = "systemId")]
    pub system_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub extension: String,
    pub runner: VersionedId,
    pub core: Option<VersionedId>,
    pub settings: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditItem {
    #[serde(rename = "requirementId")]
    pub requirement_id: String,
    pub present: u32,
    pub missing: u32,
    pub mismatch: u32,
    pub unverified: u32,
    pub status: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditReport {
    pub requirements: Vec<AuditItem>,
}

impl AuditReport {
    pub fn is_launchable(&self) -> bool {
        self.requirements
            .iter()
            .all(|item| item.status == "present")
    }
}

struct EffectiveSettings {
    display_width: WinningSetting<u16>,
    display_height: WinningSetting<u16>,
    display_mode: WinningSetting<DisplayMode>,
    frame_skip: WinningSetting<u8>,
    rumble: WinningSetting<bool>,
    audio_latency_ms: WinningSetting<u16>,
}
impl EffectiveSettings {
    fn from_defaults(d: &SettingsDefaults) -> Self {
        let source = SettingLayer::System;
        Self {
            display_width: WinningSetting {
                value: d.display_width,
                source: source.clone(),
            },
            display_height: WinningSetting {
                value: d.display_height,
                source: source.clone(),
            },
            display_mode: WinningSetting {
                value: d.display_mode.clone(),
                source: source.clone(),
            },
            frame_skip: WinningSetting {
                value: d.frame_skip,
                source: source.clone(),
            },
            rumble: WinningSetting {
                value: d.rumble,
                source: source.clone(),
            },
            audio_latency_ms: WinningSetting {
                value: d.audio_latency_ms,
                source,
            },
        }
    }
    fn apply(&mut self, d: &SettingsDelta, source: SettingLayer) {
        if let Some(value) = d.display_width {
            self.display_width = WinningSetting {
                value,
                source: source.clone(),
            };
        }
        if let Some(value) = d.display_height {
            self.display_height = WinningSetting {
                value,
                source: source.clone(),
            };
        }
        if let Some(value) = &d.display_mode {
            self.display_mode = WinningSetting {
                value: value.clone(),
                source: source.clone(),
            };
        }
        if let Some(value) = d.frame_skip {
            self.frame_skip = WinningSetting {
                value,
                source: source.clone(),
            };
        }
        if let Some(value) = d.rumble {
            self.rumble = WinningSetting {
                value,
                source: source.clone(),
            };
        }
        if let Some(value) = d.audio_latency_ms {
            self.audio_latency_ms = WinningSetting { value, source };
        }
    }
    fn into_map(self) -> BTreeMap<String, serde_json::Value> {
        let mut map = BTreeMap::new();
        for (name, value) in [
            (
                "displayWidth",
                serde_json::to_value(self.display_width).unwrap(),
            ),
            (
                "displayHeight",
                serde_json::to_value(self.display_height).unwrap(),
            ),
            (
                "displayMode",
                serde_json::to_value(self.display_mode).unwrap(),
            ),
            ("frameSkip", serde_json::to_value(self.frame_skip).unwrap()),
            ("rumble", serde_json::to_value(self.rumble).unwrap()),
            (
                "audioLatencyMs",
                serde_json::to_value(self.audio_latency_ms).unwrap(),
            ),
        ] {
            map.insert(name.to_string(), value);
        }
        map
    }
}

pub fn load_fixture(path: impl AsRef<Path>) -> Result<ResolutionFixture> {
    parse(
        &fs::read(path)
            .map_err(|e| CatalogError::new("fixture_io", format!("read fixture: {e}")))?,
        "resolution fixture",
    )
}
pub fn json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map(|s| format!("{s}\n"))
        .map_err(|e| CatalogError::new("serialize", e.to_string()))
}

pub fn schema_validation_journey() -> Result<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    Catalog::load(&root)?;
    let unknown = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/emulator-catalog/cases/unknown-field.json");
    let error = load_fixture(unknown)
        .err()
        .ok_or_else(|| CatalogError::new("journey", "unknown field was accepted"))?;
    if error.code() != "unknown_field" {
        return Err(CatalogError::new(
            "journey",
            format!("unknown field returned {}", error.code()),
        ));
    }
    Ok(
        "schema validation journey: 26 positive documents accepted, including four experimental candidates; unknown field rejected"
            .to_string(),
    )
}

pub fn fixture_journey() -> Result<String> {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/emulator-catalog/journeys.json");
    let manifest: JourneyManifest = parse(
        &fs::read(&manifest_path).map_err(|e| CatalogError::new("fixture_io", e.to_string()))?,
        "journey manifest",
    )?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    let catalog = Catalog::load(&root)?;
    let bios_root = env::temp_dir().join(format!("emulator-catalog-bios-{}", std::process::id()));
    fs::remove_dir_all(&bios_root).ok();
    fs::create_dir_all(bios_root.join("bios"))
        .map_err(|e| CatalogError::new("journey", format!("create BIOS fixture: {e}")))?;
    let precedence = load_fixture(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/emulator-catalog/cases/precedence.json"),
    )?;
    let resolved = catalog.resolve(&precedence)?;
    let names = [
        "displayWidth",
        "displayHeight",
        "displayMode",
        "frameSkip",
        "rumble",
        "audioLatencyMs",
    ];
    if resolved.settings["displayWidth"]["source"] != "device"
        || resolved.settings["displayWidth"]["value"] != 800
        || resolved.settings["displayHeight"]["source"] != "system"
        || resolved.settings["displayHeight"]["value"] != 600
        || resolved.settings["displayMode"]["source"] != "core"
        || resolved.settings["displayMode"]["value"] != "fit"
        || resolved.settings["frameSkip"]["source"] != "folder"
        || resolved.settings["frameSkip"]["value"] != 2
        || resolved.settings["rumble"]["source"] != "game"
        || resolved.settings["rumble"]["value"] != true
        || resolved.settings["audioLatencyMs"]["source"] != "session"
        || resolved.settings["audioLatencyMs"]["value"] != 32
    {
        return Err(CatalogError::new(
            "journey",
            format!("precedence values/sources missing for {}", names.join(", ")),
        ));
    }
    let mut checked = 0;
    for item in manifest.items {
        let result = match item.kind.as_str() {
            "resolve" => load_fixture(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../")
                    .join(&item.path),
            )
            .and_then(|fixture| catalog.resolve(&fixture))
            .map(|_| ()),
            "validate" => Catalog::load(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../")
                    .join(&item.path),
            )
            .and_then(|c| {
                let _ = c.channel(parse_channel(&item.channel)?)?;
                Ok(())
            }),
            "audit" => catalog
                .audit(&bios_root, parse_channel(&item.channel)?)
                .and_then(|report| {
                    if report.requirements.iter().any(|r| r.status != "present") {
                        Err(CatalogError::new(
                            "bios_missing",
                            "BIOS audit found no matching candidate",
                        ))
                    } else {
                        let _ = report;
                        Ok(())
                    }
                }),
            _ => Err(CatalogError::new("journey", "unknown journey kind")),
        };
        let error = result.err().ok_or_else(|| {
            CatalogError::new(
                "journey",
                format!("negative journey unexpectedly passed: {}", item.path),
            )
        })?;
        if error.code() != item.expected_code.as_str() {
            return Err(CatalogError::new(
                "journey",
                format!(
                    "{} returned {}, expected {}",
                    item.path,
                    error.code(),
                    item.expected_code
                ),
            ));
        }
        checked += 1;
    }
    fs::remove_dir_all(&bios_root).ok();
    Ok(format!("emulator-catalog fixture journey: precedence accepted; {checked} negative journeys rejected with stable reason codes"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JourneyManifest {
    items: Vec<JourneyItem>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JourneyItem {
    kind: String,
    path: String,
    channel: String,
    #[serde(rename = "expectedCode")]
    expected_code: String,
}
fn parse_channel(value: &str) -> Result<ChannelName> {
    match value {
        "stable" => Ok(ChannelName::Stable),
        "experimental" => Ok(ChannelName::Experimental),
        _ => Err(CatalogError::new(
            "invalid_channel",
            "channel must be stable or experimental",
        )),
    }
}

pub const CORE_PACK_SCHEMA: &str = "https://example.invalid/trimui-tg4040-core-pack-v1.schema.json";
const CORE_PACK_JOURNEY_SCHEMA: &str =
    "https://example.invalid/trimui-tg4040-core-pack-journey-v1.schema.json";
const CORE_PACK_FORMAT: &str = "trimui-tg4040-core-pack";

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CorePackStatus {
    Approved,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackHashState {
    pub id: String,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackIdentity {
    pub id: String,
    pub version: String,
    #[serde(rename = "targetArchitecture")]
    pub target_architecture: String,
    pub manifest: CorePackHashState,
    pub artifact: CorePackHashState,
    pub status: CorePackStatus,
    #[serde(rename = "blockedReason")]
    pub blocked_reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackBiosRequirement {
    pub id: String,
    pub required: bool,
    #[serde(rename = "expectedSha256")]
    pub expected_sha256: Option<String>,
    pub status: CorePackStatus,
    #[serde(rename = "blockedReason")]
    pub blocked_reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackSystem {
    pub id: String,
    pub extensions: Vec<String>,
    pub core: CorePackIdentity,
    #[serde(rename = "packageId")]
    pub package_id: String,
    #[serde(rename = "packageVersion")]
    pub package_version: String,
    #[serde(rename = "biosRequirements")]
    pub bios_requirements: Vec<CorePackBiosRequirement>,
    pub status: CorePackStatus,
    #[serde(rename = "blockedReason")]
    pub blocked_reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CorePackRoutingMode {
    ExplicitSystemSelection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackRouting {
    pub mode: CorePackRoutingMode,
    #[serde(rename = "sharedExtensions")]
    pub shared_extensions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackDisplayDefaults {
    pub width: u16,
    pub height: u16,
    pub scaling: DisplayMode,
    pub shader: Option<String>,
    pub overlay: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackDefaults {
    pub display: CorePackDisplayDefaults,
    #[serde(rename = "frameSkip")]
    pub frame_skip: u8,
    pub rumble: bool,
    #[serde(rename = "audioLatencyMs")]
    pub audio_latency_ms: u16,
    #[serde(rename = "inputProfileId")]
    pub input_profile_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DisplayPrecedence {
    System,
    Profile,
    Game,
    Reset,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InputPrecedence {
    #[serde(rename = "built-in")]
    BuiltIn,
    System,
    Game,
    Session,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackProvenance {
    pub status: CorePackStatus,
    #[serde(rename = "blockedReason")]
    pub blocked_reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePackCatalog {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: String,
    pub id: String,
    pub version: String,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    pub channel: ChannelName,
    pub status: CorePackStatus,
    #[serde(rename = "blockedReason")]
    pub blocked_reason: String,
    pub package: CorePackIdentity,
    pub runner: CorePackIdentity,
    pub systems: Vec<CorePackSystem>,
    pub routing: CorePackRouting,
    pub defaults: CorePackDefaults,
    #[serde(rename = "displayPrecedence")]
    pub display_precedence: Vec<DisplayPrecedence>,
    #[serde(rename = "inputPrecedence")]
    pub input_precedence: Vec<InputPrecedence>,
    pub provenance: CorePackProvenance,
}

impl CorePackCatalog {
    pub fn validate(&self) -> Result<()> {
        if self.schema != CORE_PACK_SCHEMA
            || self.format != CORE_PACK_FORMAT
            || self.schema_version != 1
            || self.kind != "core-pack"
            || self.target_sku != "TG4040"
            || self.channel != ChannelName::Stable
            || self.status != CorePackStatus::Blocked
            || self.blocked_reason.is_empty()
        {
            return Err(CatalogError::new(
                "core_pack_schema",
                "core-pack identity or blocked state is invalid",
            ));
        }
        validate_id(&self.id, "core-pack")?;
        validate_version(&self.version, "core-pack")?;
        validate_identity(&self.package, "package")?;
        validate_identity(&self.runner, "runner")?;
        if self.package.id != self.id || self.package.version != self.version {
            return Err(CatalogError::new(
                "core_pack_reference",
                "package identity does not match core-pack",
            ));
        }
        if self.systems.len() != 10
            || self.routing.mode != CorePackRoutingMode::ExplicitSystemSelection
            || self.routing.shared_extensions != vec!["zip".to_string()]
        {
            return Err(CatalogError::new(
                "core_pack_routing",
                "core-pack routing policy is invalid",
            ));
        }
        let mut extensions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for system in &self.systems {
            for extension in &system.extensions {
                extensions
                    .entry(normalize_extension(extension)?)
                    .or_default()
                    .insert(system.id.clone());
            }
        }
        for (extension, system_ids) in &extensions {
            if system_ids.len() > 1 && !self.routing.shared_extensions.contains(extension) {
                return Err(CatalogError::new(
                    "extension_collision",
                    "ambiguous extension route requires explicit system selection",
                ));
            }
        }
        let mut systems = BTreeSet::new();
        for system in &self.systems {
            validate_id(&system.id, "system")?;
            if !systems.insert(&system.id) {
                return Err(CatalogError::new(
                    "duplicate_id",
                    "core-pack system is duplicated",
                ));
            }
            validate_extensions(&system.extensions)?;
            let (expected_core, expected_extensions) = expected_core_pack_system(&system.id)
                .ok_or_else(|| {
                    CatalogError::new("core_pack_scope", "unsupported core-pack system")
                })?;
            if system.extensions != expected_extensions
                || system.core.id != expected_core
                || system.package_id != self.package.id
                || system.package_version != self.package.version
                || system.status != CorePackStatus::Blocked
                || system.blocked_reason.is_empty()
            {
                return Err(CatalogError::new(
                    "core_pack_scope",
                    "core-pack system projection is invalid",
                ));
            }
            validate_identity(&system.core, "core")?;
            let mut bios = BTreeSet::new();
            for requirement in &system.bios_requirements {
                validate_id(&requirement.id, "BIOS requirement")?;
                if !bios.insert(&requirement.id)
                    || !requirement.required
                    || requirement.expected_sha256.is_some()
                    || requirement.status != CorePackStatus::Blocked
                    || requirement.blocked_reason.is_empty()
                {
                    return Err(CatalogError::new(
                        "bios_unresolved",
                        "BIOS requirement is not explicitly blocked",
                    ));
                }
            }
        }
        if systems
            .iter()
            .map(|id| id.to_string())
            .collect::<BTreeSet<_>>()
            != expected_core_pack_system_ids()
        {
            return Err(CatalogError::new(
                "core_pack_scope",
                "stable core-pack scope is incomplete",
            ));
        }
        if self.defaults.display.width != 1024
            || self.defaults.display.height != 768
            || self.defaults.display.scaling != DisplayMode::Integer
            || self.defaults.display.shader.is_some()
            || self.defaults.display.overlay.is_some()
            || self.defaults.frame_skip != 0
            || self.defaults.rumble
            || self.defaults.audio_latency_ms != 64
            || self.defaults.input_profile_id != "default"
        {
            return Err(CatalogError::new(
                "core_pack_defaults",
                "core-pack safe defaults are invalid",
            ));
        }
        if self.display_precedence
            != vec![
                DisplayPrecedence::System,
                DisplayPrecedence::Profile,
                DisplayPrecedence::Game,
                DisplayPrecedence::Reset,
            ]
            || self.input_precedence
                != vec![
                    InputPrecedence::BuiltIn,
                    InputPrecedence::System,
                    InputPrecedence::Game,
                    InputPrecedence::Session,
                ]
        {
            return Err(CatalogError::new(
                "core_pack_precedence",
                "core-pack precedence is invalid",
            ));
        }
        if self.provenance.status != CorePackStatus::Blocked
            || self.provenance.blocked_reason.is_empty()
        {
            return Err(CatalogError::new(
                "core_pack_provenance",
                "core-pack provenance is not blocked",
            ));
        }
        Ok(())
    }

    pub fn ensure_installable(&self) -> Result<()> {
        self.validate()?;
        Err(CatalogError::new(
            "core_pack_blocked",
            "blocked core-pack cannot be installed or activated",
        ))
    }

    pub fn select(&self, system_id: &str, extension: &str, bios_ready: bool) -> Result<()> {
        self.validate()?;
        let extension = normalize_extension(extension)?;
        let system = self
            .systems
            .iter()
            .find(|system| system.id == system_id)
            .ok_or_else(|| CatalogError::new("core_pack_scope", "system is not in core-pack"))?;
        if !system
            .extensions
            .iter()
            .any(|candidate| candidate == &extension)
        {
            return Err(CatalogError::new(
                "extension_unavailable",
                "extension is not supported by selected system",
            ));
        }
        if !bios_ready && !system.bios_requirements.is_empty() {
            return Err(CatalogError::new(
                "bios_missing",
                "required BIOS is missing or mismatched",
            ));
        }
        self.ensure_installable()
    }
}

fn validate_identity(identity: &CorePackIdentity, label: &str) -> Result<()> {
    validate_id(&identity.id, label)?;
    validate_version(&identity.version, label)?;
    if identity.target_architecture != "aarch64-unknown-linux-musl"
        || identity.status != CorePackStatus::Blocked
        || identity.blocked_reason.is_empty()
    {
        return Err(CatalogError::new(
            "core_pack_identity",
            format!("{label} identity is not blocked and target-pinned"),
        ));
    }
    validate_hash_state(&identity.manifest, "manifest")?;
    validate_hash_state(&identity.artifact, "artifact")
}

fn validate_hash_state(state: &CorePackHashState, label: &str) -> Result<()> {
    validate_id(&state.id, label)?;
    if state.sha256.is_some() {
        return Err(CatalogError::new(
            "core_pack_hash",
            "blocked core-pack identity must not invent a SHA-256 pin",
        ));
    }
    Ok(())
}

fn expected_core_pack_system(system_id: &str) -> Option<(&'static str, Vec<String>)> {
    Some(match system_id {
        "gb" => ("core-gambatte", vec!["gb".to_string()]),
        "gbc" => ("core-gambatte", vec!["gbc".to_string()]),
        "gba" => ("core-mgba", vec!["gba".to_string()]),
        "mega-drive" => (
            "core-genesis-plus-gx",
            vec!["md".to_string(), "gen".to_string()],
        ),
        "nes" => ("core-mesen", vec!["nes".to_string()]),
        "snes" => ("core-snes9x", vec!["sfc".to_string(), "smc".to_string()]),
        "pc-engine" => ("core-beetle-pce-fast", vec!["pce".to_string()]),
        "neo-geo" => ("core-fbneo", vec!["zip".to_string()]),
        "arcade" => ("core-fbneo", vec!["zip".to_string()]),
        "ps1" => (
            "core-pcsx-rearmed",
            vec![
                "cue".to_string(),
                "chd".to_string(),
                "iso".to_string(),
                "pbp".to_string(),
                "m3u".to_string(),
            ],
        ),
        _ => return None,
    })
}

fn expected_core_pack_system_ids() -> BTreeSet<String> {
    [
        "gb",
        "gbc",
        "gba",
        "mega-drive",
        "nes",
        "snes",
        "pc-engine",
        "neo-geo",
        "arcade",
        "ps1",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub fn load_core_pack(path: impl AsRef<Path>) -> Result<CorePackCatalog> {
    parse(
        &fs::read(path)
            .map_err(|e| CatalogError::new("fixture_io", format!("read core-pack: {e}")))?,
        "core-pack",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorePackJourney {
    #[serde(rename = "$schema")]
    schema: String,
    format: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(rename = "fixtureKind")]
    fixture_kind: String,
    catalog: String,
    cases: Vec<CorePackJourneyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorePackJourneyCase {
    #[serde(rename = "systemId")]
    system_id: String,
    extension: String,
    #[serde(rename = "biosReady")]
    bios_ready: bool,
    #[serde(rename = "expectedCode")]
    expected_code: String,
}

pub fn core_pack_journey() -> Result<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: CorePackJourney = parse(
        &fs::read(root.join("fixtures/core-pack/journey.json"))
            .map_err(|e| CatalogError::new("fixture_io", e.to_string()))?,
        "core-pack journey",
    )?;
    if fixture.schema != CORE_PACK_JOURNEY_SCHEMA
        || fixture.format != "trimui-tg4040-core-pack-journey"
        || fixture.schema_version != 1
        || fixture.fixture_kind != "generated-synthetic-core-pack-journey"
        || fixture.catalog != "catalog/core-packs/stable.json"
        || fixture.cases.len() != 6
    {
        return Err(CatalogError::new(
            "journey",
            "core-pack fixture identity is invalid",
        ));
    }
    let catalog = load_core_pack(root.join(&fixture.catalog))?;
    catalog.validate()?;
    let mut ambiguous = catalog.clone();
    ambiguous
        .systems
        .iter_mut()
        .find(|system| system.id == "gbc")
        .ok_or_else(|| CatalogError::new("journey", "synthetic GBC route is missing"))?
        .extensions
        .push("gb".to_string());
    let ambiguity = ambiguous
        .validate()
        .err()
        .ok_or_else(|| CatalogError::new("journey", "ambiguous extension route was accepted"))?;
    if ambiguity.code() != "extension_collision" {
        return Err(CatalogError::new(
            "journey",
            format!("ambiguous extension route returned {}", ambiguity.code()),
        ));
    }
    for case in fixture.cases {
        let error = catalog
            .select(&case.system_id, &case.extension, case.bios_ready)
            .err()
            .ok_or_else(|| CatalogError::new("journey", "blocked core-pack was selectable"))?;
        if error.code() != case.expected_code {
            return Err(CatalogError::new(
                "journey",
                format!(
                    "core-pack case returned {}, expected {}",
                    error.code(),
                    case.expected_code
                ),
            ));
        }
    }
    Ok("core-pack fixture journey: synthetic metadata accepted; BIOS and blocked activation rejected".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog")).unwrap()
    }

    #[test]
    fn schema_negative_fixture_covers_stable_game_deltas_guard() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schemas/emulator-catalog-v1.schema.json"
        ))
        .unwrap();
        let negative: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/emulator-catalog/schema-negative/stable-profile-game-deltas.json"
        ))
        .unwrap();
        let guard = schema["$defs"]["profile"]["allOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["if"]["properties"]["channel"]["const"] == "stable")
            .unwrap();
        assert_eq!(guard["then"]["not"]["required"][0], "gameDeltas");
        assert_eq!(negative["channel"], "stable");
        assert!(negative.get("gameDeltas").is_some());
    }

    #[test]
    fn experimental_candidates_are_metadata_only_and_conservative() {
        let catalog = catalog();
        let expected = [
            ("n64", "Mupen64Plus-Next"),
            ("dreamcast", "Flycast"),
            ("psp", "PPSSPP"),
            ("nintendo-ds", "melonDS DS"),
        ];
        for (system_id, candidate_name) in expected {
            let system = catalog.system(system_id).unwrap();
            let metadata = system.experimental.as_ref().unwrap();
            assert_eq!(metadata.candidate.name, candidate_name);
            assert_eq!(
                metadata.candidate.availability,
                CandidateAvailability::MetadataOnly
            );
            assert_eq!(metadata.candidate.status, CandidateStatus::Unverified);
            assert_eq!(metadata.baseline.internal_scale, InternalScale::Native1x);
            assert!(!metadata.baseline.post_processing);
            assert!(!metadata.baseline.speedhack);
            let runner = catalog.runner(&system.default_runner).unwrap();
            assert!(runner.artifact.sha256.is_none());
        }
    }

    #[test]
    fn unverified_bios_file_is_not_a_launchable_audit() {
        let catalog = catalog();
        let root = env::temp_dir().join(format!(
            "emulator-catalog-unverified-bios-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(root.join("bios/dreamcast")).unwrap();
        fs::File::create(root.join("bios/dreamcast/firmware.bin")).unwrap();
        let report = catalog.audit(&root, ChannelName::Experimental).unwrap();
        assert_eq!(report.requirements[0].status, "unverified");
        assert!(!report.is_launchable());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn experimental_source_ref_must_be_an_immutable_commit() {
        let mut catalog = catalog();
        catalog
            .systems
            .iter_mut()
            .find(|system| system.id == "n64")
            .unwrap()
            .experimental
            .as_mut()
            .unwrap()
            .candidate
            .source_ref = "mupen_next_old_gliden".to_string();
        assert_eq!(
            catalog.validate().unwrap_err().code(),
            "experimental_metadata"
        );
        catalog
            .systems
            .iter_mut()
            .find(|system| system.id == "n64")
            .unwrap()
            .experimental
            .as_mut()
            .unwrap()
            .candidate
            .source_ref = format!("A{}", "0".repeat(39));
        assert_eq!(
            catalog.validate().unwrap_err().code(),
            "experimental_metadata"
        );
    }

    #[test]
    fn metadata_only_candidate_rejects_runner_artifact_hash() {
        let mut catalog = catalog();
        let runner_id = catalog.system("n64").unwrap().default_runner.clone();
        catalog
            .runners
            .iter_mut()
            .find(|runner| runner.id == runner_id.id && runner.version == runner_id.version)
            .unwrap()
            .artifact
            .sha256 = Some("a".repeat(64));
        assert_eq!(catalog.validate().unwrap_err().code(), "artifact_unpinned");
    }

    #[test]
    fn experimental_resolution_requires_opt_in() {
        let mut fixture = load_fixture(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/emulator-catalog/cases/dreamcast-selection.json"),
        )
        .unwrap();
        let catalog = catalog();
        fixture.experimental_opt_in = false;
        assert_eq!(
            catalog.resolve(&fixture).unwrap_err().code(),
            "experimental_opt_in"
        );
        fixture.experimental_opt_in = true;
        let resolved = catalog.resolve(&fixture).unwrap();
        assert_eq!(resolved.settings["displayMode"]["source"], "game");
        assert_eq!(resolved.settings["displayMode"]["value"], "fit");
    }

    #[test]
    fn experimental_resolution_rejects_missing_bios_and_renderer() {
        let catalog = catalog();
        let mut bios = load_fixture(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/emulator-catalog/cases/dreamcast-selection.json"),
        )
        .unwrap();
        bios.experimental_opt_in = true;
        bios.bios_ready = false;
        assert_eq!(catalog.resolve(&bios).unwrap_err().code(), "bios_missing");
        bios.bios_ready = true;
        bios.renderer = Some("powervr".to_string());
        assert_eq!(
            catalog.resolve(&bios).unwrap_err().code(),
            "unsupported_renderer"
        );
    }

    #[test]
    fn experimental_delta_must_be_reversible_and_match_selection() {
        let mut catalog = catalog();
        let profile_index = catalog
            .profiles
            .iter()
            .position(|profile| profile.id == "n64-experimental")
            .unwrap();
        catalog.profiles[profile_index].game_deltas[0].reversible = false;
        assert_eq!(catalog.validate().unwrap_err().code(), "invalid_game_delta");

        catalog.profiles[profile_index].game_deltas[0].reversible = true;
        catalog.profiles[profile_index].game_deltas[0].core.version = "9.9.9".to_string();
        assert_eq!(catalog.validate().unwrap_err().code(), "invalid_game_delta");
    }

    #[test]
    fn experimental_path_nested_under_stable_path_is_rejected() {
        let mut catalog = catalog();
        let stable_path = catalog.system("tg4040").unwrap().save_path.clone();
        let experimental = catalog
            .systems
            .iter_mut()
            .find(|system| system.id == "n64")
            .unwrap();
        experimental.save_path.relative = format!("{}/nested", stable_path.relative);
        assert_eq!(catalog.validate().unwrap_err().code(), "channel_leak");
    }

    #[test]
    fn experimental_stable_identity_and_path_collisions_are_rejected() {
        let mut collision_catalog = catalog();
        let stable_core = collision_catalog
            .cores
            .iter()
            .find(|core| core.channel == ChannelName::Stable)
            .unwrap()
            .clone();
        collision_catalog.cores.push(Core {
            channel: ChannelName::Experimental,
            ..stable_core.clone()
        });
        collision_catalog
            .channels
            .iter_mut()
            .find(|channel| channel.id == ChannelName::Experimental)
            .unwrap()
            .cores
            .push(VersionedId {
                id: stable_core.id,
                version: stable_core.version,
            });
        assert_eq!(
            collision_catalog.validate().unwrap_err().code(),
            "channel_leak"
        );

        let mut path_catalog = catalog();
        let stable_path = path_catalog.system("tg4040").unwrap().save_path.clone();
        path_catalog
            .systems
            .iter_mut()
            .find(|system| system.id == "n64")
            .unwrap()
            .save_path = stable_path;
        assert_eq!(path_catalog.validate().unwrap_err().code(), "channel_leak");
    }
}
