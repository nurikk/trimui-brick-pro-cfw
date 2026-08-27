use std::{
    fmt,
    fmt::Write as FmtWrite,
    fs,
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "trimui-input-profile-catalog";
pub const SCHEMA_VERSION: u32 = 1;
const EXPECTED_IDENTITY: &str = "synthetic-hall-v1";
const REQUIRED_CONTROLS: [RawControl; 14] = [
    RawControl::Up,
    RawControl::Down,
    RawControl::Left,
    RawControl::Right,
    RawControl::A,
    RawControl::B,
    RawControl::Start,
    RawControl::Select,
    RawControl::L3,
    RawControl::R3,
    RawControl::F1,
    RawControl::F2,
    RawControl::Fn,
    RawControl::Home,
];
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileKind {
    BuiltIn,
    Southpaw,
    #[serde(rename = "d-pad-to-stick")]
    DpadToStick,
    #[serde(rename = "stick-to-d-pad")]
    StickToDpad,
    External,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawControl {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    Start,
    Select,
    L3,
    R3,
    F1,
    F2,
    Fn,
    Home,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Primary,
    Secondary,
    Start,
    Select,
    LeftStickClick,
    RightStickClick,
    F1,
    F2,
    Fn,
    Home,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Curve {
    Linear,
    Smooth,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AxisSettings {
    pub deadzone: f32,
    pub curve: Curve,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Mapping {
    pub control: RawControl,
    pub action: Action,
    pub axis: AxisSettings,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExternalController {
    pub sdl_guid: String,
    pub required_capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub kind: ProfileKind,
    pub transform: CurveTransform,
    pub mappings: Vec<Mapping>,
    pub external_controller: Option<ExternalController>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CurveTransform {
    Standard,
    Southpaw,
    #[serde(rename = "d-pad-to-stick")]
    DpadToStick,
    #[serde(rename = "stick-to-d-pad")]
    StickToDpad,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SystemSelection {
    pub system_id: String,
    pub profile_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GameSelection {
    pub system_id: String,
    pub game_id: String,
    pub profile_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Selections {
    pub built_in: String,
    pub systems: Vec<SystemSelection>,
    pub games: Vec<GameSelection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Catalog {
    pub schema: String,
    pub schema_version: u32,
    pub profiles: Vec<Profile>,
    pub selections: Selections,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionScope {
    BuiltIn,
    System,
    Game,
    Session,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProfile {
    pub profile_id: String,
    pub scope: ResolutionScope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileError {
    Json(String),
    Invalid(String),
    UnknownProfile(String),
    UnknownSystem(String),
    UnknownGame(String),
    ExternalMismatch,
    ExternalAmbiguous(Vec<String>),
    ExternalUnknownProfile(String),
    Calibration(String),
    Persistence(String),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(value)
            | Self::Invalid(value)
            | Self::Calibration(value)
            | Self::Persistence(value) => f.write_str(value),
            Self::UnknownProfile(value) => write!(f, "unknown profile: {value}"),
            Self::UnknownSystem(value) => write!(f, "unknown system: {value}"),
            Self::UnknownGame(value) => write!(f, "unknown game: {value}"),
            Self::ExternalMismatch => f.write_str("external controller GUID/capabilities mismatch"),
            Self::ExternalAmbiguous(ids) => {
                write!(f, "ambiguous external profiles: {}", ids.join(","))
            }
            Self::ExternalUnknownProfile(value) => write!(f, "unknown external profile: {value}"),
        }
    }
}
impl std::error::Error for ProfileError {}

fn mapping_action(profile: &Profile, control: RawControl) -> Result<Action, ProfileError> {
    profile
        .mappings
        .iter()
        .find(|mapping| mapping.control == control)
        .map(|mapping| mapping.action)
        .ok_or_else(|| ProfileError::Invalid(format!("missing mapping for {control:?}")))
}

impl Catalog {
    pub fn from_json(bytes: &[u8]) -> Result<Self, ProfileError> {
        let catalog: Self = serde_json::from_slice(bytes)
            .map_err(|e| ProfileError::Json(format!("malformed catalog: {e}")))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.schema != SCHEMA
            || self.schema_version != SCHEMA_VERSION
            || self.profiles.is_empty()
        {
            return Err(ProfileError::Invalid(
                "catalog schema or version is invalid".into(),
            ));
        }
        let mut ids = Vec::new();
        for profile in &self.profiles {
            if profile.id.is_empty() || ids.iter().any(|id| id == &profile.id) {
                return Err(ProfileError::Invalid(
                    "duplicate or empty profile ID".into(),
                ));
            }
            ids.push(profile.id.clone());
            if profile.kind == ProfileKind::External {
                let external = profile.external_controller.as_ref().ok_or_else(|| {
                    ProfileError::Invalid(format!(
                        "external profile {} has no controller match",
                        profile.id
                    ))
                })?;
                if external.sdl_guid.len() != 32
                    || !external.sdl_guid.bytes().all(|b| b.is_ascii_hexdigit())
                    || external.required_capabilities.is_empty()
                {
                    return Err(ProfileError::Invalid(format!(
                        "external profile {} has invalid match",
                        profile.id
                    )));
                }
                if external
                    .required_capabilities
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                {
                    return Err(ProfileError::Invalid(format!(
                        "external profile {} capabilities are not sorted and unique",
                        profile.id
                    )));
                }
            } else if profile.external_controller.is_some() {
                return Err(ProfileError::Invalid(format!(
                    "non-external profile {} has a controller match",
                    profile.id
                )));
            }
            if profile.mappings.len() != REQUIRED_CONTROLS.len() {
                return Err(ProfileError::Invalid(format!(
                    "profile {} has incomplete mappings",
                    profile.id
                )));
            }
            for required in REQUIRED_CONTROLS {
                if !profile
                    .mappings
                    .iter()
                    .any(|mapping| mapping.control == required)
                {
                    return Err(ProfileError::Invalid(format!(
                        "profile {} is missing a control",
                        profile.id
                    )));
                }
            }
            if profile
                .mappings
                .iter()
                .map(|mapping| mapping.control)
                .collect::<std::collections::HashSet<_>>()
                .len()
                != profile.mappings.len()
            {
                return Err(ProfileError::Invalid(format!(
                    "profile {} has duplicate mapping controls",
                    profile.id
                )));
            }
            for mapping in &profile.mappings {
                if !mapping.axis.deadzone.is_finite()
                    || !(0.0..=1.0).contains(&mapping.axis.deadzone)
                {
                    return Err(ProfileError::Invalid(format!(
                        "profile {} has invalid deadzone",
                        profile.id
                    )));
                }
            }
            let distinct_raw = [
                RawControl::L3,
                RawControl::R3,
                RawControl::F1,
                RawControl::F2,
            ]
            .into_iter()
            .map(|control| mapping_action(profile, control))
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
            if distinct_raw.len() != 4 {
                return Err(ProfileError::Invalid(format!(
                    "profile {} aliases L3/R3/F1/F2",
                    profile.id
                )));
            }
            if mapping_action(profile, RawControl::Fn)?
                == mapping_action(profile, RawControl::Home)?
            {
                return Err(ProfileError::Invalid(format!(
                    "profile {} aliases Fn/Home",
                    profile.id
                )));
            }
        }
        let profile_exists = |id: &str| ids.iter().any(|known| known == id);
        if !profile_exists(&self.selections.built_in) {
            return Err(ProfileError::UnknownProfile(
                self.selections.built_in.clone(),
            ));
        }
        let mut systems = Vec::new();
        for selection in &self.selections.systems {
            if selection.system_id.is_empty() || systems.iter().any(|id| id == &selection.system_id)
            {
                return Err(ProfileError::Invalid(
                    "duplicate or empty system selection".into(),
                ));
            }
            if !profile_exists(&selection.profile_id) {
                return Err(ProfileError::UnknownProfile(selection.profile_id.clone()));
            }
            systems.push(selection.system_id.clone());
        }
        let mut games = Vec::new();
        for selection in &self.selections.games {
            if selection.system_id.is_empty()
                || selection.game_id.is_empty()
                || games.iter().any(|key: &(String, String)| {
                    key.0 == selection.system_id && key.1 == selection.game_id
                })
            {
                return Err(ProfileError::Invalid(
                    "duplicate or empty game selection".into(),
                ));
            }
            if !profile_exists(&selection.profile_id) {
                return Err(ProfileError::UnknownProfile(selection.profile_id.clone()));
            }
            games.push((selection.system_id.clone(), selection.game_id.clone()));
        }
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<Vec<u8>, ProfileError> {
        serde_json::to_vec_pretty(self)
            .map(|mut bytes| {
                bytes.push(b'\n');
                bytes
            })
            .map_err(|e| ProfileError::Json(e.to_string()))
    }

    pub fn resolve(
        &self,
        system_id: Option<&str>,
        game_id: Option<&str>,
        session_profile: Option<&str>,
    ) -> Result<ResolvedProfile, ProfileError> {
        self.validate()?;
        let mut selected = (self.selections.built_in.clone(), ResolutionScope::BuiltIn);
        if let Some(system) = system_id {
            if let Some(entry) = self
                .selections
                .systems
                .iter()
                .find(|entry| entry.system_id == system)
            {
                selected = (entry.profile_id.clone(), ResolutionScope::System);
            }
            if let Some(game) = game_id {
                if let Some(entry) = self
                    .selections
                    .games
                    .iter()
                    .find(|entry| entry.system_id == system && entry.game_id == game)
                {
                    selected = (entry.profile_id.clone(), ResolutionScope::Game);
                }
            }
        } else if game_id.is_some() {
            return Err(ProfileError::Invalid("game scope requires a system".into()));
        }
        if let Some(session) = session_profile {
            if !self.profiles.iter().any(|profile| profile.id == session) {
                return Err(ProfileError::UnknownProfile(session.into()));
            }
            selected = (session.into(), ResolutionScope::Session);
        }
        Ok(ResolvedProfile {
            profile_id: selected.0,
            scope: selected.1,
        })
    }

    pub fn select_external(
        &self,
        guid: &str,
        capabilities: &[String],
        explicit_profile: Option<&str>,
    ) -> Result<ResolvedProfile, ProfileError> {
        self.validate()?;
        let compatible: Vec<&Profile> = self
            .profiles
            .iter()
            .filter(|profile| profile.kind == ProfileKind::External)
            .filter(|profile| {
                let Some(match_data) = profile.external_controller.as_ref() else {
                    return false;
                };
                match_data.sdl_guid == guid
                    && match_data.required_capabilities.iter().all(|capability| {
                        capabilities.iter().any(|available| available == capability)
                    })
            })
            .collect();
        if let Some(id) = explicit_profile {
            let profile = self
                .profiles
                .iter()
                .find(|profile| profile.id == id)
                .ok_or_else(|| ProfileError::ExternalUnknownProfile(id.into()))?;
            let match_data = profile
                .external_controller
                .as_ref()
                .ok_or(ProfileError::ExternalMismatch)?;
            if match_data.sdl_guid != guid
                || !match_data
                    .required_capabilities
                    .iter()
                    .all(|capability| capabilities.iter().any(|available| available == capability))
            {
                return Err(ProfileError::ExternalMismatch);
            }
            return Ok(ResolvedProfile {
                profile_id: profile.id.clone(),
                scope: ResolutionScope::Session,
            });
        }
        match compatible.as_slice() {
            [] => Err(ProfileError::ExternalMismatch),
            [profile] => Ok(ResolvedProfile {
                profile_id: profile.id.clone(),
                scope: ResolutionScope::Session,
            }),
            _ => Err(ProfileError::ExternalAmbiguous(
                compatible
                    .iter()
                    .map(|profile| profile.id.clone())
                    .collect(),
            )),
        }
    }

    pub fn profile(&self, id: &str) -> Result<&Profile, ProfileError> {
        self.profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| ProfileError::UnknownProfile(id.into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickPair {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DpadPair {
    pub x: i8,
    pub y: i8,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformOutput {
    Sticks { left: StickPair, right: StickPair },
    Dpad { value: DpadPair },
}

pub fn apply_transform(
    transform: CurveTransform,
    dpad: DpadPair,
    left: StickPair,
    right: StickPair,
) -> TransformOutput {
    match transform {
        CurveTransform::Standard | CurveTransform::Southpaw => TransformOutput::Sticks {
            left: if transform == CurveTransform::Southpaw {
                right
            } else {
                left
            },
            right: if transform == CurveTransform::Southpaw {
                left
            } else {
                right
            },
        },
        CurveTransform::DpadToStick => TransformOutput::Sticks {
            left: StickPair {
                x: dpad.x as f32,
                y: dpad.y as f32,
            },
            right,
        },
        CurveTransform::StickToDpad => TransformOutput::Dpad {
            value: DpadPair {
                x: if left.x.abs() >= 0.5 {
                    left.x.signum() as i8
                } else {
                    0
                },
                y: if left.y.abs() >= 0.5 {
                    left.y.signum() as i8
                } else {
                    0
                },
            },
        },
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
}
const EXPECTED_AXES: [RawAxis; 4] = [
    RawAxis::LeftX,
    RawAxis::LeftY,
    RawAxis::RightX,
    RawAxis::RightY,
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SamplePhase {
    Center,
    Minimum,
    Maximum,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SyntheticIdentity {
    pub id: String,
    pub axes: Vec<RawAxis>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RawSample {
    pub sequence: u64,
    pub axis: RawAxis,
    pub phase: SamplePhase,
    pub value: f64,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Capture {
    pub identity: SyntheticIdentity,
    pub samples: Vec<RawSample>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AxisCalibration {
    pub minimum: f64,
    pub center: f64,
    pub maximum: f64,
    pub deadzone: f64,
    pub curve: Curve,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Calibration {
    pub identity: SyntheticIdentity,
    pub axes: Vec<AxisCalibration>,
}

pub fn calibrate(
    expected: &SyntheticIdentity,
    capture: &Capture,
) -> Result<Calibration, ProfileError> {
    if expected.id != EXPECTED_IDENTITY
        || expected.axes.as_slice() != EXPECTED_AXES.as_slice()
        || capture.identity != *expected
    {
        return Err(ProfileError::Calibration(
            "unknown or mismatching synthetic identity".into(),
        ));
    }
    if capture.samples.len() < 36
        || capture
            .samples
            .iter()
            .enumerate()
            .any(|(index, sample)| sample.sequence != index as u64 || !sample.value.is_finite())
    {
        return Err(ProfileError::Calibration(
            "dropped, insufficient, or non-finite samples".into(),
        ));
    }
    let mut axes = Vec::new();
    for axis in [
        RawAxis::LeftX,
        RawAxis::LeftY,
        RawAxis::RightX,
        RawAxis::RightY,
    ] {
        let values: Vec<&RawSample> = capture
            .samples
            .iter()
            .filter(|sample| sample.axis == axis)
            .collect();
        let centers: Vec<f64> = values
            .iter()
            .filter(|sample| sample.phase == SamplePhase::Center)
            .map(|sample| sample.value)
            .collect();
        let minimums: Vec<f64> = values
            .iter()
            .filter(|sample| sample.phase == SamplePhase::Minimum)
            .map(|sample| sample.value)
            .collect();
        let maximums: Vec<f64> = values
            .iter()
            .filter(|sample| sample.phase == SamplePhase::Maximum)
            .map(|sample| sample.value)
            .collect();
        if centers.len() < 3 || minimums.len() < 2 || maximums.len() < 2 {
            return Err(ProfileError::Calibration(
                "insufficient phase samples".into(),
            ));
        }
        let center = centers.iter().sum::<f64>() / centers.len() as f64;
        if centers.iter().any(|value| (value - center).abs() > 0.05) {
            return Err(ProfileError::Calibration("noisy center samples".into()));
        }
        let minimum = minimums.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = maximums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if minimum >= center
            || maximum <= center
            || center - minimum < 0.25
            || maximum - center < 0.25
        {
            return Err(ProfileError::Calibration(
                "invalid or degenerate range".into(),
            ));
        }
        axes.push(AxisCalibration {
            minimum,
            center,
            maximum,
            deadzone: 0.05,
            curve: Curve::Linear,
        });
    }
    Ok(Calibration {
        identity: expected.clone(),
        axes,
    })
}

fn valid_axis_range(axis: &AxisCalibration) -> bool {
    axis.minimum.is_finite()
        && axis.center.is_finite()
        && axis.maximum.is_finite()
        && (0.0..1.0).contains(&axis.deadzone)
        && axis.minimum < axis.center
        && axis.center < axis.maximum
}

pub fn normalize(axis: &AxisCalibration, raw: f64) -> Result<f64, ProfileError> {
    if !raw.is_finite() || !valid_axis_range(axis) {
        return Err(ProfileError::Calibration(
            "invalid normalization input".into(),
        ));
    }
    let value = if raw >= axis.center {
        (raw - axis.center) / (axis.maximum - axis.center)
    } else {
        (raw - axis.center) / (axis.center - axis.minimum)
    };
    let value = value.clamp(-1.0, 1.0);
    let value = if value.abs() <= axis.deadzone {
        0.0
    } else {
        value.signum() * ((value.abs() - axis.deadzone) / (1.0 - axis.deadzone))
    };
    Ok(match axis.curve {
        Curve::Linear => value,
        Curve::Smooth => value.signum() * value.abs().powi(2),
    })
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CalibrationPayload {
    schema: String,
    schema_version: u32,
    identity: SyntheticIdentity,
    axes: Vec<AxisCalibration>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PersistedCalibration {
    schema: String,
    schema_version: u32,
    identity: SyntheticIdentity,
    axes: Vec<AxisCalibration>,
    sha256: String,
}

fn validate_calibration(calibration: &Calibration) -> Result<(), ProfileError> {
    if calibration.identity.id != EXPECTED_IDENTITY
        || calibration.identity.axes.as_slice() != EXPECTED_AXES.as_slice()
        || calibration.axes.len() != 4
    {
        return Err(ProfileError::Calibration(
            "calibration identity or axis count is invalid".into(),
        ));
    }
    for axis in &calibration.axes {
        if !valid_axis_range(axis) {
            return Err(ProfileError::Calibration(
                "calibration range is invalid".into(),
            ));
        }
    }
    Ok(())
}

fn persisted_bytes(calibration: &Calibration) -> Result<(Vec<u8>, String), ProfileError> {
    validate_calibration(calibration)?;
    let payload = CalibrationPayload {
        schema: "trimui-hall-calibration".into(),
        schema_version: SCHEMA_VERSION,
        identity: calibration.identity.clone(),
        axes: calibration.axes.clone(),
    };
    let bytes =
        serde_json::to_vec(&payload).map_err(|e| ProfileError::Persistence(e.to_string()))?;
    let digest = Sha256::digest(&bytes);
    let mut checksum = String::with_capacity(64);
    for byte in digest {
        write!(&mut checksum, "{byte:02x}")
            .map_err(|_| ProfileError::Persistence("checksum formatting failed".into()))?;
    }
    let record = PersistedCalibration {
        schema: payload.schema,
        schema_version: payload.schema_version,
        identity: payload.identity,
        axes: payload.axes,
        sha256: checksum.clone(),
    };
    let mut output =
        serde_json::to_vec_pretty(&record).map_err(|e| ProfileError::Persistence(e.to_string()))?;
    output.push(b'\n');
    Ok((output, checksum))
}

fn decode(bytes: &[u8], expected: &SyntheticIdentity) -> Result<Calibration, ProfileError> {
    let record: PersistedCalibration = serde_json::from_slice(bytes)
        .map_err(|e| ProfileError::Persistence(format!("malformed calibration: {e}")))?;
    if record.schema != "trimui-hall-calibration"
        || record.schema_version != SCHEMA_VERSION
        || record.identity != *expected
    {
        return Err(ProfileError::Persistence(
            "calibration schema or identity rejected".into(),
        ));
    }
    let calibration = Calibration {
        identity: record.identity.clone(),
        axes: record.axes.clone(),
    };
    validate_calibration(&calibration)?;
    let (canonical, checksum) = persisted_bytes(&calibration)?;
    if record.sha256 != checksum || bytes != canonical {
        return Err(ProfileError::Persistence(
            "calibration checksum or canonical bytes rejected".into(),
        ));
    }
    Ok(calibration)
}

fn read_regular(path: &Path) -> Result<Vec<u8>, ProfileError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| ProfileError::Persistence(format!("read calibration metadata: {e}")))?;
    if !metadata.file_type().is_file() {
        return Err(ProfileError::Persistence(
            "calibration path is not a regular file".into(),
        ));
    }
    fs::read(path).map_err(|e| ProfileError::Persistence(format!("read calibration: {e}")))
}

pub fn load(path: &Path, expected: &SyntheticIdentity) -> Result<Calibration, ProfileError> {
    decode(&read_regular(path)?, expected)
}

pub fn save(
    path: &Path,
    calibration: &Calibration,
    inject_publication_failure: bool,
) -> Result<(), ProfileError> {
    let (bytes, _) = persisted_bytes(calibration)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(ProfileError::Persistence(
                "calibration path is not a regular file".into(),
            ));
        }
        Ok(_) => {
            let _ = load(path, &calibration.identity)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ProfileError::Persistence(format!(
                "read calibration metadata: {error}"
            )))
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| ProfileError::Persistence("calibration path has no parent".into()))?;
    fs::create_dir_all(parent)
        .map_err(|e| ProfileError::Persistence(format!("create calibration parent: {e}")))?;
    let name = path
        .file_name()
        .ok_or_else(|| ProfileError::Persistence("calibration path has no file name".into()))?
        .to_string_lossy();
    let temporary = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|e| ProfileError::Persistence(format!("create calibration temporary: {e}")))?;
        file.write_all(&bytes)
            .map_err(|e| ProfileError::Persistence(format!("write calibration temporary: {e}")))?;
        file.sync_all()
            .map_err(|e| ProfileError::Persistence(format!("sync calibration temporary: {e}")))?;
        if inject_publication_failure {
            return Err(ProfileError::Persistence(
                "injected publication failure".into(),
            ));
        }
        fs::rename(&temporary, path).map_err(|e| {
            ProfileError::Persistence(format!("atomic calibration replacement: {e}"))
        })?;
        sync_parent(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_parent(parent: &Path) -> Result<(), ProfileError> {
    match fs::File::open(parent).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(()),
        Err(error) => Err(ProfileError::Persistence(format!(
            "sync calibration parent: {error}"
        ))),
    }
}
