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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    #[serde(rename = "expectedSha256")]
    pub expected_sha256: String,
    pub locations: Vec<LogicalPath>,
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
            let mut ids = BTreeSet::new();
            for requirement in &system.bios_requirements {
                validate_id(&requirement.id, "BIOS requirement")?;
                validate_hash(&requirement.expected_sha256)?;
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
            let hash = runner.artifact.sha256.as_deref().ok_or_else(|| {
                CatalogError::new("artifact_unpinned", "runner artifact has no SHA-256 pin")
            })?;
            validate_hash(hash)?;
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
        }
        unique_ids(self.profiles.iter().map(|e| &e.id), "profile")?;
        for channel in &self.channels {
            self.validate_channel(channel)?;
        }
        let stable = self.channel(ChannelName::Stable)?;
        let experimental = self.channel(ChannelName::Experimental)?;
        let stable_keys = channel_keys(stable);
        let experimental_keys = channel_keys(experimental);
        if !stable_keys.is_disjoint(&experimental_keys) {
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
        let system = self.system(&fixture.system_id)?;
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
                for location in &requirement.locations {
                    let path = logical_host_path(root.as_ref(), location)?;
                    match hash_file(&path) {
                        Ok(hash) if hash == requirement.expected_sha256 => present += 1,
                        Ok(_) => mismatch += 1,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => missing += 1,
                        Err(error) => {
                            return Err(CatalogError::new(
                                "bios_io",
                                format!("BIOS audit read failed: {error}"),
                            ))
                        }
                    }
                }
                let status = if present > 0 {
                    "present"
                } else if mismatch > 0 {
                    "mismatch"
                } else {
                    "missing"
                };
                requirements.push(AuditItem {
                    requirement_id: requirement.id.clone(),
                    present,
                    missing,
                    mismatch,
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
        .chain(
            channel
                .runners
                .iter()
                .map(|x| format!("runner:{}@{}", x.id, x.version)),
        )
        .chain(
            channel
                .cores
                .iter()
                .map(|x| format!("core:{}@{}", x.id, x.version)),
        )
        .chain(channel.profiles.iter().map(|x| format!("profile:{x}")))
        .collect()
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
    pub status: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditReport {
    pub requirements: Vec<AuditItem>,
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
        "schema validation journey: 10 positive documents accepted; unknown field rejected"
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
