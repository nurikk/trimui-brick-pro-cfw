use serde::{Deserialize, Serialize};
use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
};

pub const REQUEST_SCHEMA: &str = "https://example.invalid/trimui-launch-request-v1.schema.json";
pub const CATALOG_SCHEMA: &str = "https://example.invalid/trimui-launch-catalog-v1.schema.json";
const FORMAT: &str = "brickpro-launch-request";
const CATALOG_FORMAT: &str = "brickpro-launch-catalog";
const MAX_ID: usize = 64;
const MAX_VERSION: usize = 32;
const MAX_PATH_BYTES: usize = 4096;
const MAX_COMPONENT_BYTES: usize = 255;

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

/// Stable, UI-safe reasons for a rejected or failed launch.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchDiagnosticCode {
    UnsupportedFormat,
    MissingBios,
    MissingData,
    MissingRuntime,
    LaunchCrash,
}

impl LaunchDiagnosticCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnsupportedFormat => "unsupported-format",
            Self::MissingBios => "missing-bios",
            Self::MissingData => "missing-data",
            Self::MissingRuntime => "missing-runtime",
            Self::LaunchCrash => "launch-crash",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LaunchDiagnostic {
    pub code: LaunchDiagnosticCode,
    pub reason: String,
}

impl LaunchDiagnostic {
    pub fn new(code: LaunchDiagnosticCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, ContractError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRequest {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub kind: LaunchKind,
    #[serde(rename = "contentId")]
    pub content_id: String,
    #[serde(rename = "contentSha256")]
    pub content_sha256: String,
    #[serde(rename = "contentPath")]
    pub content_path: LogicalPath,
    #[serde(rename = "savePath")]
    pub save_path: LogicalPath,
    #[serde(rename = "statePath")]
    pub state_path: LogicalPath,
    pub runner: VersionedId,
    #[serde(default)]
    pub package: Option<VersionedId>,
    pub core: Option<VersionedId>,
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "resumeMode")]
    pub resume_mode: ResumeMode,
    pub display: DisplaySettings,
    pub input: InputSettings,
    pub power: PowerSettings,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalPath {
    pub root: PathRoot,
    pub relative: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PathRoot {
    Roms,
    #[serde(rename = "data/saves")]
    DataSaves,
    #[serde(rename = "data/states")]
    DataStates,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LaunchKind {
    Libretro,
    Standalone,
    Portmaster,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResumeMode {
    Fresh,
    Resume,
    Auto,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VersionedId {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DisplaySettings {
    pub width: u16,
    pub height: u16,
    #[serde(rename = "refreshHz")]
    pub refresh_hz: u16,
    pub scaling: Scaling,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Scaling {
    Integer,
    Fit,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputSettings {
    pub layout: InputLayout,
    pub rumble: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InputLayout {
    Standard,
    Arcade,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PowerSettings {
    pub suspend: SuspendMode,
    #[serde(rename = "batterySaver")]
    pub battery_saver: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SuspendMode {
    Allowed,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub runners: Vec<RunnerEntry>,
    pub cores: Vec<CoreEntry>,
    pub profiles: Vec<ProfileEntry>,
}

pub type LaunchCatalog = Catalog;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerEntry {
    pub id: String,
    pub version: String,
    pub kinds: Vec<LaunchKind>,
    pub capabilities: Vec<Capability>,
    pub status: CatalogEntryStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoreEntry {
    pub id: String,
    pub version: String,
    #[serde(rename = "runnerId")]
    pub runner_id: String,
    #[serde(rename = "runnerVersion")]
    pub runner_version: String,
    pub kind: LaunchKind,
    pub status: CatalogEntryStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileEntry {
    pub id: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogEntryStatus {
    Approved,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    DisplayBasic,
    InputStandard,
    InputArcade,
    Rumble,
    Suspend,
    BatterySaver,
}

pub fn parse_request_json(bytes: &[u8]) -> Result<LaunchRequest> {
    serde_json::from_slice(bytes)
        .map_err(|error| ContractError::new(format!("request JSON: {error}")))
}

pub fn parse_catalog_json(bytes: &[u8]) -> Result<Catalog> {
    serde_json::from_slice(bytes)
        .map_err(|error| ContractError::new(format!("catalog JSON: {error}")))
}

pub fn request_json(request: &LaunchRequest) -> Result<String> {
    serde_json::to_string_pretty(request)
        .map(|json| format!("{json}\n"))
        .map_err(|error| ContractError::new(format!("serialize request: {error}")))
}

pub fn validate(request: &LaunchRequest, catalog: &Catalog) -> Result<()> {
    if request.schema != REQUEST_SCHEMA || request.format != FORMAT || request.schema_version != 1 {
        return Err(ContractError::new(
            "request schema, format, or version is unsupported",
        ));
    }
    if catalog.schema != CATALOG_SCHEMA
        || catalog.format != CATALOG_FORMAT
        || catalog.schema_version != 1
    {
        return Err(ContractError::new(
            "catalog schema, format, or version is unsupported",
        ));
    }
    validate_catalog_projection(catalog)?;
    validate_request_fields(request)?;

    let runner = catalog
        .runners
        .iter()
        .find(|entry| entry.id == request.runner.id && entry.version == request.runner.version)
        .ok_or_else(|| ContractError::new("runner is not in the installed catalog"))?;
    if !runner.kinds.contains(&request.kind) {
        return Err(ContractError::new("runner does not support request kind"));
    }

    match (&request.kind, &request.core) {
        (LaunchKind::Libretro, Some(core)) => {
            let entry = catalog
                .cores
                .iter()
                .find(|entry| entry.id == core.id && entry.version == core.version)
                .ok_or_else(|| ContractError::new("core is not in the installed catalog"))?;
            if entry.kind != request.kind
                || entry.runner_id != runner.id
                || entry.runner_version != runner.version
            {
                return Err(ContractError::new(
                    "core is incompatible with kind or runner",
                ));
            }
        }
        (LaunchKind::Libretro, None) => return Err(ContractError::new("libretro requires a core")),
        (_, Some(_)) => {
            return Err(ContractError::new(
                "standalone and portmaster requests cannot name a core",
            ))
        }
        (_, None) => {}
    }

    let profile = catalog
        .profiles
        .iter()
        .find(|entry| entry.id == request.profile_id)
        .ok_or_else(|| ContractError::new("profile is not in the installed catalog"))?;
    for capability in required_capabilities(request) {
        if !runner.capabilities.contains(&capability) || !profile.capabilities.contains(&capability)
        {
            return Err(ContractError::new(
                "runner/profile capability combination is not allowlisted",
            ));
        }
    }
    Ok(())
}

pub fn validate_host_fixture(
    request: &LaunchRequest,
    catalog: &Catalog,
    fixture_root: &Path,
) -> Result<()> {
    validate(request, catalog)?;
    let root = fs::canonicalize(fixture_root)
        .map_err(|error| ContractError::new(format!("fixture root cannot be resolved: {error}")))?;
    if !root.is_dir() {
        return Err(ContractError::new("fixture root is not a directory"));
    }
    resolve_path(&root, &request.content_path, PathRoot::Roms, true)?;
    resolve_path(&root, &request.save_path, PathRoot::DataSaves, false)?;
    resolve_path(&root, &request.state_path, PathRoot::DataStates, false)?;
    Ok(())
}

pub fn validate_catalog_projection(catalog: &Catalog) -> Result<()> {
    if catalog.schema != CATALOG_SCHEMA
        || catalog.format != CATALOG_FORMAT
        || catalog.schema_version != 1
    {
        return Err(ContractError::new(
            "catalog schema, format, or version is unsupported",
        ));
    }
    validate_catalog_entries(catalog)
}

fn validate_catalog_entries(catalog: &Catalog) -> Result<()> {
    if catalog.runners.is_empty()
        || catalog.runners.len() > 64
        || catalog.cores.len() > 128
        || catalog.profiles.is_empty()
        || catalog.profiles.len() > 64
    {
        return Err(ContractError::new("catalog entry bounds are invalid"));
    }
    let mut runner_keys = Vec::new();
    for runner in &catalog.runners {
        validate_identifier(&runner.id, "runner identifier")?;
        validate_version(&runner.version, "runner version")?;
        if runner.status != CatalogEntryStatus::Approved
            || runner.kinds.is_empty()
            || runner.kinds.len() > 3
            || runner.capabilities.is_empty()
            || runner.capabilities.len() > 6
            || has_duplicates(&runner.kinds)
            || has_duplicates(&runner.capabilities)
        {
            return Err(ContractError::new("runner kinds/capabilities are invalid"));
        }
        if runner_keys
            .iter()
            .any(|key: &(String, String)| key == &(runner.id.clone(), runner.version.clone()))
        {
            return Err(ContractError::new("runner versions are duplicated"));
        }
        runner_keys.push((runner.id.clone(), runner.version.clone()));
    }
    let mut core_keys = Vec::new();
    for core in &catalog.cores {
        validate_identifier(&core.id, "core identifier")?;
        validate_version(&core.version, "core version")?;
        validate_version(&core.runner_version, "core runner version")?;
        if core.status != CatalogEntryStatus::Approved
            || core.kind != LaunchKind::Libretro
            || !catalog.runners.iter().any(|runner| {
                runner.id == core.runner_id
                    && runner.version == core.runner_version
                    && runner.kinds.contains(&LaunchKind::Libretro)
            })
            || core_keys
                .iter()
                .any(|key: &(String, String)| key == &(core.id.clone(), core.version.clone()))
        {
            return Err(ContractError::new("core entry is invalid or duplicated"));
        }
        core_keys.push((core.id.clone(), core.version.clone()));
    }
    let mut profile_ids = Vec::new();
    for profile in &catalog.profiles {
        validate_identifier(&profile.id, "profile identifier")?;
        if profile.capabilities.is_empty()
            || profile.capabilities.len() > 6
            || has_duplicates(&profile.capabilities)
        {
            return Err(ContractError::new("profile capabilities are invalid"));
        }
        if profile_ids.iter().any(|id| id == &profile.id) {
            return Err(ContractError::new("profile identifiers are duplicated"));
        }
        profile_ids.push(profile.id.clone());
    }
    Ok(())
}

fn validate_request_fields(request: &LaunchRequest) -> Result<()> {
    validate_token(&request.request_id, "request ID")?;
    validate_token(&request.content_id, "content ID")?;
    if !is_lower_hex_64(&request.content_sha256) {
        return Err(ContractError::new(
            "contentSha256 must be lowercase SHA-256",
        ));
    }
    validate_identifier(&request.runner.id, "runner identifier")?;
    validate_version(&request.runner.version, "runner version")?;
    validate_identifier(&request.profile_id, "profile identifier")?;
    if let Some(core) = &request.core {
        validate_identifier(&core.id, "core identifier")?;
        validate_version(&core.version, "core version")?;
    }
    if let Some(package) = &request.package {
        validate_identifier(&package.id, "package identifier")?;
        validate_version(&package.version, "package version")?;
    }
    if request.kind == LaunchKind::Portmaster && request.package.is_none() {
        return Err(ContractError::new("portmaster requires a package identity"));
    }
    if request.kind != LaunchKind::Portmaster && request.package.is_some() {
        return Err(ContractError::new(
            "package identity is only valid for portmaster",
        ));
    }
    if !(320..=4096).contains(&request.display.width)
        || !(240..=2160).contains(&request.display.height)
        || !(1..=240).contains(&request.display.refresh_hz)
    {
        return Err(ContractError::new(
            "display settings are outside contract bounds",
        ));
    }
    validate_logical_path(&request.content_path, PathRoot::Roms)?;
    validate_logical_path(&request.save_path, PathRoot::DataSaves)?;
    validate_logical_path(&request.state_path, PathRoot::DataStates)?;
    Ok(())
}

fn required_capabilities(request: &LaunchRequest) -> Vec<Capability> {
    let mut capabilities = vec![Capability::DisplayBasic];
    capabilities.push(match request.input.layout {
        InputLayout::Standard => Capability::InputStandard,
        InputLayout::Arcade => Capability::InputArcade,
    });
    if request.input.rumble {
        capabilities.push(Capability::Rumble);
    }
    if request.power.suspend == SuspendMode::Allowed {
        capabilities.push(Capability::Suspend);
    }
    if request.power.battery_saver {
        capabilities.push(Capability::BatterySaver);
    }
    capabilities
}

fn validate_logical_path(path: &LogicalPath, expected_root: PathRoot) -> Result<()> {
    if path.root != expected_root {
        return Err(ContractError::new("logical path root does not match field"));
    }
    let relative = &path.relative;
    if relative.is_empty()
        || relative.len() > MAX_PATH_BYTES
        || relative.contains('\0')
        || relative.contains('\\')
        || relative.starts_with('/')
    {
        return Err(ContractError::new(
            "logical path relative is not bounded and portable",
        ));
    }
    for component in relative.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > MAX_COMPONENT_BYTES
            || forbidden_component(component)
        {
            return Err(ContractError::new(
                "logical path contains an invalid component",
            ));
        }
    }
    Ok(())
}

fn resolve_path(
    root: &Path,
    logical: &LogicalPath,
    expected_root: PathRoot,
    must_exist: bool,
) -> Result<PathBuf> {
    validate_logical_path(logical, expected_root.clone())?;
    let root_name = match expected_root {
        PathRoot::Roms => "roms",
        PathRoot::DataSaves => "data/saves",
        PathRoot::DataStates => "data/states",
    };
    let base = root.join(root_name);
    let mut current = fs::canonicalize(&base).map_err(|error| {
        ContractError::new(format!(
            "logical path root cannot be canonicalized: {error}"
        ))
    })?;
    ensure_inside(root, &current)?;
    let components: Vec<&str> = logical.relative.split('/').collect();
    for (index, component) in components.iter().enumerate() {
        let matches = fs::read_dir(&current)
            .map_err(|error| {
                ContractError::new(format!("logical path parent cannot be read: {error}"))
            })?
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry.file_name().to_string_lossy().to_lowercase() == component.to_lowercase()
            })
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(ContractError::new(
                "case-colliding path aliases are forbidden",
            ));
        }
        if let Some(entry) = matches.into_iter().next() {
            current = fs::canonicalize(entry.path()).map_err(|error| {
                ContractError::new(format!("logical path cannot be canonicalized: {error}"))
            })?;
            ensure_inside(root, &current)?;
        } else if index + 1 == components.len() && !must_exist {
            current.push(component);
        } else {
            return Err(ContractError::new("logical path does not exist in fixture"));
        }
        if index + 1 < components.len() && !metadata_is_dir(&current)? {
            return Err(ContractError::new(
                "logical path component is not a directory",
            ));
        }
    }
    let resolved = match fs::symlink_metadata(&current) {
        Ok(_) => fs::canonicalize(&current).map_err(|error| {
            ContractError::new(format!("logical path cannot be canonicalized: {error}"))
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = current
                .parent()
                .ok_or_else(|| ContractError::new("logical path has no parent"))?;
            let canonical_parent = fs::canonicalize(parent).map_err(|error| {
                ContractError::new(format!(
                    "logical path parent cannot be canonicalized: {error}"
                ))
            })?;
            canonical_parent.join(
                current
                    .file_name()
                    .ok_or_else(|| ContractError::new("logical path has no filename"))?,
            )
        }
        Err(error) => {
            return Err(ContractError::new(format!(
                "logical path metadata cannot be read: {error}"
            )))
        }
    };
    ensure_inside(root, &resolved)?;
    if must_exist && !metadata_is_file(&resolved)? {
        return Err(ContractError::new("content path is not a regular file"));
    }
    Ok(resolved)
}

fn ensure_inside(root: &Path, candidate: &Path) -> Result<()> {
    if !candidate.starts_with(root) {
        return Err(ContractError::new(
            "logical path escapes supplied fixture root",
        ));
    }
    Ok(())
}

fn metadata_is_dir(path: &Path) -> Result<bool> {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir())
        .map_err(|error| {
            ContractError::new(format!("logical path metadata cannot be read: {error}"))
        })
}

fn metadata_is_file(path: &Path) -> Result<bool> {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file())
        .map_err(|error| {
            ContractError::new(format!("logical path metadata cannot be read: {error}"))
        })
}

fn validate_token(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ContractError::new(format!(
            "{label} is not an opaque bounded identifier"
        )));
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_ID
        || !value.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && byte.is_ascii_lowercase())
                || (index > 0
                    && (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
        })
    {
        return Err(ContractError::new(format!(
            "{label} is not a valid lower-case identifier"
        )));
    }
    Ok(())
}

fn validate_version(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_VERSION
        || value.split('.').count() != 3
        || !value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(ContractError::new(format!(
            "{label} is not a bounded semantic version"
        )));
    }
    Ok(())
}

fn forbidden_component(component: &str) -> bool {
    if component.ends_with(['.', ' '])
        || component.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
    {
        return true;
    }
    let stem = component
        .trim_end_matches(['.', ' '])
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[..index].contains(value))
}

pub fn fixture_journey() -> Result<String> {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/launch-contract/generated-v1");
    let catalog_bytes = fs::read(fixture_dir.join("catalog.synthetic.json"))
        .map_err(|error| ContractError::new(format!("read synthetic catalog: {error}")))?;
    let catalog = parse_catalog_json(&catalog_bytes)?;
    validate_catalog_projection(&catalog)?;
    let request_path = fixture_dir.join("requests/canonical.synthetic.json");
    let request_bytes = fs::read(&request_path)
        .map_err(|error| ContractError::new(format!("read canonical request: {error}")))?;
    let request = parse_request_json(&request_bytes)?;
    if request_json(&request)?.as_bytes() != request_bytes.as_slice() {
        return Err(ContractError::new(
            "canonical request JSON is not deterministic",
        ));
    }
    validate_host_fixture(&request, &catalog, &fixture_dir.join("host-root"))?;

    let mut rejected = 0;
    let mut entries = fs::read_dir(fixture_dir.join("requests/negative"))
        .map_err(|error| ContractError::new(format!("read synthetic negatives: {error}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| ContractError::new(format!("read synthetic negative entry: {error}")))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name == "symlink-canonical-escape.synthetic.json"
            || name == "case-collision.synthetic.json"
        {
            continue;
        }
        let bytes = fs::read(entry.path())
            .map_err(|error| ContractError::new(format!("read synthetic negative: {error}")))?;
        if parse_request_json(&bytes)
            .and_then(|candidate| {
                validate_host_fixture(&candidate, &catalog, &fixture_dir.join("host-root"))
            })
            .is_err()
        {
            rejected += 1;
        } else {
            return Err(ContractError::new(format!(
                "synthetic negative {} was accepted",
                entry.file_name().to_string_lossy()
            )));
        }
    }

    let root = unique_temp_root();
    make_escape_fixture(&root)?;
    let escape_path = fixture_dir.join("requests/negative/symlink-canonical-escape.synthetic.json");
    let escape = parse_request_json(
        &fs::read(escape_path)
            .map_err(|error| ContractError::new(format!("read escape fixture: {error}")))?,
    )?;
    if validate_host_fixture(&escape, &catalog, &root).is_ok() {
        return Err(ContractError::new("synthetic symlink escape was accepted"));
    }
    rejected += 1;
    let collision_path = fixture_dir.join("requests/negative/case-collision.synthetic.json");
    let collision = parse_request_json(
        &fs::read(collision_path)
            .map_err(|error| ContractError::new(format!("read collision fixture: {error}")))?,
    )?;
    if validate_host_fixture(&collision, &catalog, &root).is_ok() {
        return Err(ContractError::new("synthetic case collision was accepted"));
    }
    rejected += 1;
    if rejected != 14 {
        return Err(ContractError::new(
            "synthetic negative fixture coverage is incomplete",
        ));
    }
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(root.with_file_name("launch-contract-generated-outside.bin"));
    Ok(format!(
        "launch-contract fixture journey: canonical accepted; {rejected} synthetic negatives rejected"
    ))
}

fn unique_temp_root() -> PathBuf {
    env::temp_dir().join(format!("launch-contract-generated-{}", std::process::id()))
}

fn make_escape_fixture(root: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("roms/generated"))
        .and_then(|_| fs::create_dir_all(root.join("data/saves/generated")))
        .and_then(|_| fs::create_dir_all(root.join("data/states/generated")))
        .map_err(|error| ContractError::new(format!("create synthetic escape fixture: {error}")))?;
    fs::write(root.join("roms/generated/content.bin"), b"")
        .and_then(|_| fs::write(root.join("roms/generated/Case.bin"), b""))
        .and_then(|_| fs::write(root.join("roms/generated/case.bin"), b""))
        .and_then(|_| fs::write(root.join("data/saves/generated/content.sav"), b""))
        .and_then(|_| fs::write(root.join("data/states/generated/content.state"), b""))
        .and_then(|_| {
            fs::write(
                root.with_file_name("launch-contract-generated-outside.bin"),
                b"",
            )
        })
        .map_err(|error| ContractError::new(format!("write synthetic escape fixture: {error}")))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.with_file_name("launch-contract-generated-outside.bin"),
        root.join("roms/generated/synthetic-escape.bin"),
    )
    .map_err(|error| ContractError::new(format!("create synthetic escape symlink: {error}")))?;
    #[cfg(not(unix))]
    return Err(ContractError::new(
        "symlink escape journey requires a Unix host",
    ));
    Ok(())
}
