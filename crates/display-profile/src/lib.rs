use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

pub const SCHEMA: &str = "https://example.invalid/trimui-display-profile-v1.schema.json";
pub const FORMAT: &str = "trimui-display-profile";
pub const TARGET_SKU: &str = "TG4040";
pub const LOGICAL_WIDTH: u16 = 1024;
pub const LOGICAL_HEIGHT: u16 = 768;
const MAX_ID: usize = 64;
const MAX_CATALOG_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractError(String);

impl ContractError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ContractError {}
pub type Result<T> = std::result::Result<T, ContractError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: CatalogKind,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    #[serde(rename = "logicalOutput")]
    pub logical_output: LogicalOutput,
    pub systems: Vec<System>,
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogKind {
    Catalog,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LogicalOutput {
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub id: String,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    pub channel: Channel,
    #[serde(rename = "logicalOutput")]
    pub logical_output: LogicalOutput,
    #[serde(rename = "defaultSelection")]
    pub default_selection: Selection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    pub channel: Channel,
    #[serde(rename = "systemIds")]
    pub system_ids: Vec<String>,
    #[serde(rename = "defaultSelection")]
    pub default_selection: Selection,
    #[serde(rename = "gameOverrides")]
    pub game_overrides: Vec<GameOverride>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Experimental,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub scaling: Scaling,
    #[serde(rename = "overlaySelection")]
    pub overlay_selection: Option<IdentifierSelection>,
    #[serde(rename = "shaderSelection")]
    pub shader_selection: Option<IdentifierSelection>,
    pub warnings: Vec<Warning>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Scaling {
    Integer,
    OriginalAspect,
    Crop,
    Fullscreen,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IdentifierSelection {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Warning {
    pub code: WarningCode,
    pub severity: WarningSeverity,
    #[serde(rename = "messageKey")]
    pub message_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WarningCode {
    NonDefaultScaling,
    CropPresentation,
    FullscreenPresentation,
    OverlaySelected,
    ShaderSelected,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WarningSeverity {
    Warning,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "action", rename_all = "kebab-case")]
pub enum GameOverride {
    Set {
        #[serde(rename = "gameId")]
        game_id: String,
        selection: Selection,
    },
    Reset {
        #[serde(rename = "gameId")]
        game_id: String,
    },
}

impl GameOverride {
    fn game_id(&self) -> &String {
        match self {
            Self::Set { game_id, .. } | Self::Reset { game_id } => game_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRequest {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: RequestKind,
    pub channel: Channel,
    #[serde(rename = "systemId")]
    pub system_id: String,
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "gameId")]
    pub game_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RequestKind {
    ResolutionRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedProfile {
    pub channel: Channel,
    #[serde(rename = "systemId")]
    pub system_id: String,
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "logicalOutput")]
    pub logical_output: LogicalOutput,
    pub selection: Selection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub kind: FixtureKind,
    pub synthetic: bool,
    pub cases: Vec<FixtureCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FixtureKind {
    FixtureJourney,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureCase {
    pub id: String,
    pub channel: Channel,
    #[serde(rename = "systemId")]
    pub system_id: String,
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "gameId")]
    pub game_id: Option<String>,
    #[serde(rename = "expectedScaling")]
    pub expected_scaling: Scaling,
    #[serde(rename = "expectedOverlay")]
    pub expected_overlay: Option<String>,
    #[serde(rename = "expectedShader")]
    pub expected_shader: Option<String>,
}

pub fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(ContractError::new("fixture exceeds size budget"));
    }
    serde_json::from_slice(bytes).map_err(|error| ContractError::new(error.to_string()))
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map(|json| format!("{json}\n"))
        .map_err(|error| ContractError::new(format!("serialize contract: {error}")))
}

impl Catalog {
    pub fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA
            || self.format != FORMAT
            || self.schema_version != 1
            || self.kind != CatalogKind::Catalog
            || self.target_sku != TARGET_SKU
            || self.logical_output != logical_output()
        {
            return Err(ContractError::new(
                "catalog identity or logical output is invalid",
            ));
        }
        if self.systems.is_empty() || self.profiles.is_empty() {
            return Err(ContractError::new(
                "catalog must contain a system and profile",
            ));
        }
        unique_ids(self.systems.iter().map(|system| &system.id), "system")?;
        unique_ids(self.profiles.iter().map(|profile| &profile.id), "profile")?;
        for system in &self.systems {
            validate_id(&system.id, "system")?;
            validate_target(&system.target_sku, &system.logical_output)?;
            validate_selection(&system.default_selection, &system.default_selection.scaling)?;
            if system.default_selection.overlay_selection.is_some()
                || system.default_selection.shader_selection.is_some()
            {
                return Err(ContractError::new(
                    "system defaults must select no overlay or shader",
                ));
            }
        }
        for profile in &self.profiles {
            validate_id(&profile.id, "profile")?;
            validate_target(&profile.target_sku, &self.logical_output)?;
            if profile.system_ids.is_empty() {
                return Err(ContractError::new("profile system selection is empty"));
            }
            unique_ids(profile.system_ids.iter(), "profile system")?;
            for system_id in &profile.system_ids {
                validate_id(system_id, "profile system")?;
                let system = self.system(system_id)?;
                if system.channel != profile.channel {
                    return Err(ContractError::new(
                        "profile crosses stable and experimental channels",
                    ));
                }
                if system.target_sku != TARGET_SKU || system.logical_output != logical_output() {
                    return Err(ContractError::new("profile system target is invalid"));
                }
            }
            let system = self.system(&profile.system_ids[0])?;
            if profile.channel != system.channel {
                return Err(ContractError::new("profile channel does not match system"));
            }
            validate_selection(
                &profile.default_selection,
                &system.default_selection.scaling,
            )?;
            let mut game_ids = BTreeSet::new();
            for game_override in &profile.game_overrides {
                validate_id(game_override.game_id(), "game")?;
                if !game_ids.insert(game_override.game_id()) {
                    return Err(ContractError::new("duplicate game override identifier"));
                }
                if let GameOverride::Set { selection, .. } = game_override {
                    validate_selection(selection, &profile.default_selection.scaling)?;
                }
            }
        }
        Ok(())
    }

    pub fn resolve(&self, request: &ResolutionRequest) -> Result<ResolvedProfile> {
        self.validate()?;
        if request.schema != SCHEMA
            || request.format != FORMAT
            || request.schema_version != 1
            || request.kind != RequestKind::ResolutionRequest
        {
            return Err(ContractError::new("resolution request identity is invalid"));
        }
        let system = self.system(&request.system_id)?;
        if system.channel != request.channel {
            return Err(ContractError::new(
                "requested channel does not contain system",
            ));
        }
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == request.profile_id)
            .ok_or_else(|| ContractError::new("profile reference is missing"))?;
        if profile.channel != request.channel || !profile.system_ids.contains(&system.id) {
            return Err(ContractError::new(
                "profile is not enabled for requested system/channel",
            ));
        }
        let selection = request
            .game_id
            .as_ref()
            .and_then(|game_id| {
                profile
                    .game_overrides
                    .iter()
                    .find(|item| item.game_id() == game_id)
            })
            .map(|item| match item {
                GameOverride::Set { selection, .. } => selection.clone(),
                GameOverride::Reset { .. } => profile.default_selection.clone(),
            })
            .unwrap_or_else(|| profile.default_selection.clone());
        Ok(ResolvedProfile {
            channel: request.channel.clone(),
            system_id: system.id.clone(),
            profile_id: profile.id.clone(),
            logical_output: logical_output(),
            selection,
        })
    }

    fn system(&self, id: &str) -> Result<&System> {
        self.systems
            .iter()
            .find(|system| system.id == id)
            .ok_or_else(|| ContractError::new("system reference is missing"))
    }
}

fn logical_output() -> LogicalOutput {
    LogicalOutput {
        width: LOGICAL_WIDTH,
        height: LOGICAL_HEIGHT,
    }
}

fn validate_target(target_sku: &str, output: &LogicalOutput) -> Result<()> {
    if target_sku != TARGET_SKU || output != &logical_output() {
        return Err(ContractError::new(
            "only TG4040 at logical 1024x768 is permitted",
        ));
    }
    Ok(())
}

fn validate_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID
        || !value.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && byte.is_ascii_lowercase())
                || (index > 0
                    && (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
        })
    {
        return Err(ContractError::new(format!("{label} identifier is invalid")));
    }
    Ok(())
}

fn validate_selection(selection: &Selection, baseline_scaling: &Scaling) -> Result<()> {
    if let Some(overlay) = &selection.overlay_selection {
        validate_id(&overlay.id, "overlay")?;
    }
    if let Some(shader) = &selection.shader_selection {
        validate_id(&shader.id, "shader")?;
    }
    let mut expected = Vec::new();
    if &selection.scaling != baseline_scaling {
        expected.push(Warning::new(
            WarningCode::NonDefaultScaling,
            "display.warning.scaling.non-default",
        ));
    }
    if selection.scaling == Scaling::Crop {
        expected.push(Warning::new(
            WarningCode::CropPresentation,
            "display.warning.presentation.crop",
        ));
    }
    if selection.scaling == Scaling::Fullscreen {
        expected.push(Warning::new(
            WarningCode::FullscreenPresentation,
            "display.warning.presentation.fullscreen",
        ));
    }
    if selection.overlay_selection.is_some() {
        expected.push(Warning::new(
            WarningCode::OverlaySelected,
            "display.warning.overlay.selected",
        ));
    }
    if selection.shader_selection.is_some() {
        expected.push(Warning::new(
            WarningCode::ShaderSelected,
            "display.warning.shader.selected",
        ));
    }
    if selection.warnings != expected {
        return Err(ContractError::new(
            "selection warning projection is incomplete or inconsistent",
        ));
    }
    Ok(())
}

impl Warning {
    fn new(code: WarningCode, message_key: &str) -> Self {
        Self {
            code,
            severity: WarningSeverity::Warning,
            message_key: message_key.to_string(),
        }
    }
}

fn unique_ids<'a>(values: impl IntoIterator<Item = &'a String>, label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(ContractError::new(format!("duplicate {label} identifier")));
        }
    }
    Ok(())
}

pub fn validate_fixture_manifest(manifest: &FixtureManifest) -> Result<()> {
    if manifest.schema != SCHEMA
        || manifest.format != FORMAT
        || manifest.schema_version != 1
        || manifest.kind != FixtureKind::FixtureJourney
        || !manifest.synthetic
        || manifest.cases.is_empty()
    {
        return Err(ContractError::new("fixture manifest identity is invalid"));
    }
    unique_ids(manifest.cases.iter().map(|case| &case.id), "fixture case")?;
    for case in &manifest.cases {
        validate_id(&case.id, "fixture case")?;
        validate_id(case.system_id.as_str(), "fixture system")?;
        validate_id(case.profile_id.as_str(), "fixture profile")?;
        if let Some(game_id) = &case.game_id {
            validate_id(game_id, "game")?;
        }
        if let Some(overlay_id) = &case.expected_overlay {
            validate_id(overlay_id, "fixture overlay")?;
        }
        if let Some(shader_id) = &case.expected_shader {
            validate_id(shader_id, "fixture shader")?;
        }
    }
    Ok(())
}
