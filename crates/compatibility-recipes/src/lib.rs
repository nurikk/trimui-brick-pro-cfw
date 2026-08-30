use display_profile::{Catalog as DisplayCatalog, RequestKind, ResolutionRequest};
use emulator_catalog::{Catalog as EmulatorCatalog, ChannelName};
use input_profile::Catalog as InputCatalog;
use serde::{de, Deserialize, Deserializer, Serialize};
use settings_schema::{FieldKind, Registry, SettingValue};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub const SCHEMA: &str = "https://example.invalid/trimui-compat-recipe-v1.schema.json";
pub const FORMAT: &str = "trimui-compatibility-recipe";
pub const TARGET_SKU: &str = "TG4040";
const DEVICE_PROFILE: &[u8] = include_bytes!("../../../config/platform/tg4040/compatibility.json");
pub const API_VERSION: u8 = 1;
pub const MAX_RECIPE_BYTES: usize = 256 * 1024;
pub const MAX_DELTA: usize = 16;
const CONFIG_ROOT: &str = ".brickpro/config/compatibility-recipes";
const VAULT_ROOT: &str = ".brickpro/config/compatibility-recipes/rollback-metadata";
const SAFE_POWER_PROFILES: [&str; 2] = ["balanced", "battery-saver"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeError {
    pub code: &'static str,
    message: String,
}
impl RecipeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for RecipeError {}
pub type Result<T> = std::result::Result<T, RecipeError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "apiVersion")]
    pub api_version: u8,
    pub kind: RecipeKind,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    #[serde(rename = "targetId")]
    pub target_id: String,
    #[serde(rename = "romSha256")]
    pub rom_sha256: String,
    #[serde(rename = "systemId")]
    pub system_id: String,
    pub core: CoreConstraint,
    #[serde(rename = "configDelta")]
    pub config_delta: Vec<ConfigChange>,
    pub profiles: ProfileReferences,
    #[serde(rename = "knownIssues")]
    pub known_issues: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeKind {
    CompatibilityRecipe,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoreConstraint {
    pub id: String,
    #[serde(rename = "versionConstraint")]
    pub version_constraint: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigChange {
    pub key: String,
    pub value: SettingValue,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileReferences {
    #[serde(rename = "displayProfileId")]
    pub display_profile_id: String,
    #[serde(rename = "inputProfileId")]
    pub input_profile_id: String,
    #[serde(rename = "powerProfileId")]
    pub power_profile_id: String,
}
#[derive(Clone, Debug)]
pub struct ValidationContext {
    pub catalog_root: PathBuf,
    pub display_catalog: Vec<u8>,
    pub input_catalog: Vec<u8>,
    pub settings_registry: Vec<u8>,
}
impl ValidationContext {
    pub fn new(catalog_root: impl Into<PathBuf>) -> Self {
        Self {
            catalog_root: catalog_root.into(),
            display_catalog: include_bytes!(
                "../../../fixtures/display-profile/generated-v1/catalog.json"
            )
            .to_vec(),
            input_catalog: include_bytes!("../../../config/input/profiles.json").to_vec(),
            settings_registry: include_bytes!("../../../fixtures/settings-schema/registry-v1.json")
                .to_vec(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MatchedRecipe {
    recipe: Recipe,
}
impl MatchedRecipe {
    pub fn target_id(&self) -> &str {
        &self.recipe.target_id
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOverrides {
    pub device: BTreeMap<String, SettingValue>,
    pub system: BTreeMap<String, SettingValue>,
    pub folder: BTreeMap<String, SettingValue>,
    pub game: BTreeMap<String, SettingValue>,
    pub session: BTreeMap<String, SettingValue>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub local_overrides: LocalOverrides,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub source: String,
    #[serde(rename = "recipeTarget")]
    pub recipe_target: String,
    pub core: CoreConstraint,
    pub profiles: ProfileReferences,
    #[serde(rename = "settingChanges")]
    pub setting_changes: Vec<SettingChange>,
    #[serde(rename = "localOverrides")]
    pub local_overrides: Vec<LocalOverride>,
    pub collisions: Vec<Collision>,
    #[serde(rename = "knownIssues")]
    pub known_issues: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingChange {
    pub key: String,
    pub before: Option<SettingValue>,
    pub after: Option<SettingValue>,
    #[serde(rename = "beforeSource")]
    pub before_source: String,
    #[serde(rename = "afterSource")]
    pub after_source: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOverride {
    pub key: String,
    pub layer: String,
    pub value: SettingValue,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Collision {
    pub key: String,
    #[serde(rename = "localLayer")]
    pub local_layer: String,
    #[serde(rename = "localValue")]
    pub local_value: SettingValue,
    #[serde(rename = "recipeValue")]
    pub recipe_value: SettingValue,
    pub resolution: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePoint {
    AfterVault,
    AfterLayer,
}
#[derive(Clone, Debug, Default)]
pub struct ApplyOptions {
    pub replace_collisions: BTreeSet<String>,
    pub failure: Option<FailurePoint>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyReceipt {
    pub generation: u64,
    #[serde(rename = "vaultRecord")]
    pub vault_record: String,
    #[serde(rename = "recipeLayer")]
    pub recipe_layer: String,
}

#[derive(Clone, Debug)]
pub enum LauncherAction {
    Preview {
        local_overrides: LocalOverrides,
    },
    Apply {
        local_overrides: LocalOverrides,
        replace_collisions: BTreeSet<String>,
    },
    Rollback,
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum LauncherResponse {
    Preview(Preview),
    Applied(ApplyReceipt),
    RolledBack,
}

pub fn launcher_dispatch(
    root: &Path,
    authenticated: &MatchedRecipe,
    context: &ValidationContext,
    target_id: &str,
    action: LauncherAction,
) -> Result<LauncherResponse> {
    match action {
        LauncherAction::Preview { local_overrides } => Ok(LauncherResponse::Preview(preview(
            authenticated,
            context,
            &local_overrides,
        )?)),
        LauncherAction::Apply {
            local_overrides,
            replace_collisions,
        } => Ok(LauncherResponse::Applied(apply(
            root,
            authenticated,
            context,
            &local_overrides,
            ApplyOptions {
                replace_collisions,
                failure: None,
            },
        )?)),
        LauncherAction::Rollback => {
            rollback(root, target_id)?;
            Ok(LauncherResponse::RolledBack)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeLayer {
    format: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    generation: u64,
    #[serde(rename = "targetId")]
    target_id: String,
    core: CoreConstraint,
    profiles: ProfileReferences,
    #[serde(rename = "configDelta")]
    config_delta: Vec<ConfigChange>,
    #[serde(rename = "replacementKeys")]
    replacement_keys: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VaultRecord {
    format: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    generation: u64,
    name: String,
    #[serde(rename = "targetId")]
    target_id: String,
    #[serde(rename = "protectedData")]
    protected_data: String,
    #[serde(rename = "priorRecipeLayer")]
    prior_recipe_layer: Option<RecipeLayer>,
}

pub fn parse_recipe(bytes: &[u8]) -> Result<Recipe> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err(RecipeError::new(
            "oversized-input",
            "recipe exceeds size budget",
        ));
    }
    reject_duplicate_keys(bytes)?;
    let recipe: Recipe = serde_json::from_slice(bytes)
        .map_err(|_| RecipeError::new("malformed-recipe", "recipe JSON is invalid"))?;
    validate_recipe_shape(&recipe)?;
    Ok(recipe)
}

pub fn validate_recipe(recipe: &Recipe, context: &ValidationContext) -> Result<()> {
    validate_recipe_shape(recipe)?;
    let catalog = EmulatorCatalog::load(&context.catalog_root).map_err(|_| {
        RecipeError::new(
            "catalog-unavailable",
            "current emulator catalog is unavailable",
        )
    })?;
    let stable = catalog
        .channel(ChannelName::Stable)
        .map_err(|_| RecipeError::new("catalog-unavailable", "stable catalog is unavailable"))?;
    let version = exact_version(&recipe.core.version_constraint)?;
    let core = catalog
        .cores
        .iter()
        .find(|core| core.id == recipe.core.id && core.version == version)
        .ok_or_else(|| {
            RecipeError::new(
                "unavailable-core",
                "required core is not in the current catalog",
            )
        })?;
    let blocked = blocked_core_pack(&context.catalog_root, &core.id, &core.version)?;
    if !stable
        .cores
        .iter()
        .any(|entry| entry.id == core.id && entry.version == core.version)
        || blocked
    {
        return Err(RecipeError::new(
            "unavailable-core",
            "required core is blocked or not enabled",
        ));
    }
    let system = catalog
        .systems
        .iter()
        .find(|system| system.id == recipe.system_id)
        .ok_or_else(|| RecipeError::new("catalog-unavailable", "recipe system is missing"))?;
    if system.target_sku != TARGET_SKU || !core.supported_systems.contains(&system.id) {
        return Err(RecipeError::new(
            "wrong-device",
            "recipe system or core target is invalid",
        ));
    }
    let display: DisplayCatalog = serde_json::from_slice(&context.display_catalog)
        .map_err(|_| RecipeError::new("profile-invalid", "display catalog is invalid"))?;
    let device = device_profile::DeviceProfile::from_json(DEVICE_PROFILE)
        .map_err(|_| RecipeError::new("profile-invalid", "device profile is invalid"))?;
    display
        .validate(&device)
        .map_err(|_| RecipeError::new("profile-invalid", "display catalog validation failed"))?;
    display
        .resolve(
            &device,
            &ResolutionRequest {
                schema: display_profile::SCHEMA.into(),
                format: display_profile::FORMAT.into(),
                schema_version: 1,
                kind: RequestKind::ResolutionRequest,
                channel: display_profile::Channel::Stable,
                system_id: recipe.system_id.clone(),
                profile_id: recipe.profiles.display_profile_id.clone(),
                game_id: None,
            },
        )
        .map_err(|_| {
            RecipeError::new(
                "profile-invalid",
                "display profile is not referenced by TG4040",
            )
        })?;
    let input = InputCatalog::from_json(&context.input_catalog)
        .map_err(|_| RecipeError::new("profile-invalid", "input catalog is invalid"))?;
    input
        .resolve(
            Some(&recipe.system_id),
            None,
            Some(&recipe.profiles.input_profile_id),
        )
        .map_err(|_| {
            RecipeError::new(
                "profile-invalid",
                "input profile is not referenced by TG4040",
            )
        })?;
    if !SAFE_POWER_PROFILES.contains(&recipe.profiles.power_profile_id.as_str()) {
        return Err(RecipeError::new(
            "profile-invalid",
            "power profile is not allowlisted",
        ));
    }
    let registry = Registry::from_json(&context.settings_registry)
        .map_err(|_| RecipeError::new("settings-invalid", "settings registry is invalid"))?;
    validate_delta(&recipe.config_delta, &registry)?;
    Ok(())
}

pub fn match_recipe(
    repository: &Path,
    target_id: &str,
    rom_sha256: &str,
    context: &ValidationContext,
) -> Result<MatchedRecipe> {
    validate_id(target_id, "recipe target")?;
    validate_hash(rom_sha256)?;
    let target_bytes = read_repository_file(repository, &format!("recipes/{target_id}.json"))?;
    let parsed = parse_recipe(&target_bytes)?;
    validate_recipe(&parsed, context)?;
    if parsed.target_id != target_id || parsed.rom_sha256 != rom_sha256 {
        return Err(RecipeError::new(
            "no-match",
            "no recipe matches the supplied content",
        ));
    }
    Ok(MatchedRecipe { recipe: parsed })
}

pub fn preview(
    authenticated: &MatchedRecipe,
    context: &ValidationContext,
    local: &LocalOverrides,
) -> Result<Preview> {
    validate_recipe(&authenticated.recipe, context)?;
    let registry = Registry::from_json(&context.settings_registry)
        .map_err(|_| RecipeError::new("settings-invalid", "settings registry is invalid"))?;
    validate_local(local, &registry)?;
    let (before, before_sources) = effective(&registry, local, &[], &BTreeSet::new());
    let collisions = collisions(local, &authenticated.recipe.config_delta);
    let (after, after_sources) = effective(
        &registry,
        local,
        &authenticated.recipe.config_delta,
        &BTreeSet::new(),
    );
    Ok(make_preview(
        authenticated,
        &registry,
        before,
        before_sources,
        after,
        after_sources,
        local,
        collisions,
    ))
}

pub fn apply(
    root: &Path,
    authenticated: &MatchedRecipe,
    context: &ValidationContext,
    local: &LocalOverrides,
    options: ApplyOptions,
) -> Result<ApplyReceipt> {
    let initial = preview(authenticated, context, local)?;
    let collision_keys: BTreeSet<_> = initial
        .collisions
        .iter()
        .map(|collision| collision.key.clone())
        .collect();
    if options.replace_collisions != collision_keys {
        return Err(RecipeError::new(
            "collision-choice-required",
            "every local collision needs an explicit replacement choice",
        ));
    }
    validate_private_root(root)?;
    save_vault::SaveVault::snapshot_standard(root, save_vault::SnapshotReason::PreRecipe).map_err(
        |error| {
            RecipeError::new(
                "snapshot-failed",
                format!("pre-recipe save snapshot failed: {error}"),
            )
        },
    )?;
    let layer_path = private_path(
        root,
        &format!("{CONFIG_ROOT}/{}.json", authenticated.recipe.target_id),
    )?;
    let prior = read_optional_layer(&layer_path)?;
    if let Some(prior) = &prior {
        validate_layer_identity(prior, &authenticated.recipe.target_id)?;
    }
    let generation = prior
        .as_ref()
        .map_or(1, |layer| layer.generation.saturating_add(1));
    let layer = RecipeLayer {
        format: "brickpro-compatibility-recipe-layer".into(),
        schema_version: 1,
        generation,
        target_id: authenticated.recipe.target_id.clone(),
        core: authenticated.recipe.core.clone(),
        profiles: authenticated.recipe.profiles.clone(),
        config_delta: authenticated.recipe.config_delta.clone(),
        replacement_keys: options.replace_collisions.iter().cloned().collect(),
    };
    let vault = VaultRecord {
        format: "brickpro-compatibility-save-vault".into(),
        schema_version: 1,
        generation,
        name: format!(
            "compatibility-recipe-{}-generation-{generation}",
            authenticated.recipe.target_id
        ),
        target_id: authenticated.recipe.target_id.clone(),
        protected_data: "untouched".into(),
        prior_recipe_layer: prior.clone(),
    };
    let vault_path = private_path(
        root,
        &format!(
            "{VAULT_ROOT}/{}-generation-{generation}.json",
            authenticated.recipe.target_id
        ),
    )?;
    atomic_json(&vault_path, &vault)?;
    if options.failure == Some(FailurePoint::AfterVault) {
        return Err(RecipeError::new(
            "publication-failed",
            "deterministic publication failure",
        ));
    }
    atomic_json(&layer_path, &layer)?;
    if options.failure == Some(FailurePoint::AfterLayer) {
        restore_layer(&layer_path, prior.as_ref())?;
        return Err(RecipeError::new(
            "publication-failed",
            "deterministic publication failure",
        ));
    }
    Ok(ApplyReceipt {
        generation,
        vault_record: vault_path
            .strip_prefix(root)
            .unwrap_or(&vault_path)
            .display()
            .to_string(),
        recipe_layer: layer_path
            .strip_prefix(root)
            .unwrap_or(&layer_path)
            .display()
            .to_string(),
    })
}

pub fn rollback(root: &Path, target_id: &str) -> Result<()> {
    validate_id(target_id, "recipe target")?;
    validate_private_root(root)?;
    save_vault::SaveVault::snapshot_standard(root, save_vault::SnapshotReason::PreRecipe).map_err(
        |error| {
            RecipeError::new(
                "snapshot-failed",
                format!("pre-recipe save snapshot failed: {error}"),
            )
        },
    )?;
    let layer_path = private_path(root, &format!("{CONFIG_ROOT}/{target_id}.json"))?;
    let layer = read_optional_layer(&layer_path)?
        .ok_or_else(|| RecipeError::new("rollback-unavailable", "recipe layer is not active"))?;
    validate_layer_identity(&layer, target_id)?;
    let vault_dir = private_path(root, VAULT_ROOT)?;
    let mut records = fs::read_dir(&vault_dir)
        .map_err(|_| RecipeError::new("rollback-unavailable", "save vault record is unavailable"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    records.sort();
    let vault_path = records
        .into_iter()
        .rev()
        .find(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with(&format!("{target_id}-generation-{}", layer.generation))
            })
        })
        .ok_or_else(|| {
            RecipeError::new(
                "rollback-unavailable",
                "matching save vault record is unavailable",
            )
        })?;
    let vault: VaultRecord = read_json(&vault_path)?;
    if vault.target_id != target_id
        || vault.generation != layer.generation
        || vault.protected_data != "untouched"
    {
        return Err(RecipeError::new(
            "rollback-unavailable",
            "save vault record is invalid",
        ));
    }
    if let Some(prior) = &vault.prior_recipe_layer {
        validate_layer_identity(prior, target_id)?;
    }
    restore_layer(&layer_path, vault.prior_recipe_layer.as_ref())
}

#[allow(clippy::too_many_arguments)]
fn make_preview(
    auth: &MatchedRecipe,
    registry: &Registry,
    before: BTreeMap<String, SettingValue>,
    before_sources: BTreeMap<String, String>,
    after: BTreeMap<String, SettingValue>,
    after_sources: BTreeMap<String, String>,
    local: &LocalOverrides,
    collisions: Vec<Collision>,
) -> Preview {
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).cloned().collect();
    let setting_changes = keys
        .into_iter()
        .map(|key| SettingChange {
            key: key.clone(),
            before: before.get(&key).cloned(),
            after: after.get(&key).cloned(),
            before_source: before_sources
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "unset".into()),
            after_source: after_sources
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "unset".into()),
        })
        .collect();
    let _ = registry;
    Preview {
        source: "local".into(),
        recipe_target: auth.recipe.target_id.clone(),
        core: auth.recipe.core.clone(),
        profiles: auth.recipe.profiles.clone(),
        setting_changes,
        local_overrides: flatten_local(local),
        collisions,
        known_issues: auth.recipe.known_issues.clone(),
    }
}

fn effective(
    registry: &Registry,
    local: &LocalOverrides,
    delta: &[ConfigChange],
    replacements: &BTreeSet<String>,
) -> (BTreeMap<String, SettingValue>, BTreeMap<String, String>) {
    let mut values = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for setting in &registry.settings {
        if setting.redacted {
            continue;
        }
        if let Some(value) = setting.current.clone().or_else(|| setting.default.clone()) {
            values.insert(setting.id.clone(), value);
            sources.insert(setting.id.clone(), "system".into());
        }
    }
    for (name, map) in [
        ("device", &local.device),
        ("system", &local.system),
        ("folder", &local.folder),
        ("game", &local.game),
        ("session", &local.session),
    ] {
        for (key, value) in map {
            values.insert(key.clone(), value.clone());
            sources.insert(key.clone(), name.into());
        }
    }
    for change in delta {
        if replacements.contains(&change.key) || !has_late_local(local, &change.key) {
            values.insert(change.key.clone(), change.value.clone());
            sources.insert(change.key.clone(), "recipe".into());
        }
    }
    (values, sources)
}
fn has_late_local(local: &LocalOverrides, key: &str) -> bool {
    [&local.folder, &local.game, &local.session]
        .into_iter()
        .any(|map| map.contains_key(key))
}
fn collisions(local: &LocalOverrides, delta: &[ConfigChange]) -> Vec<Collision> {
    delta
        .iter()
        .flat_map(|change| {
            [
                (&local.folder, "folder"),
                (&local.game, "game"),
                (&local.session, "session"),
            ]
            .into_iter()
            .filter_map(move |(map, layer)| {
                map.get(&change.key).map(|value| Collision {
                    key: change.key.clone(),
                    local_layer: layer.into(),
                    local_value: value.clone(),
                    recipe_value: change.value.clone(),
                    resolution: "local-override-wins-unless-explicit-replacement".into(),
                })
            })
        })
        .collect()
}
fn flatten_local(local: &LocalOverrides) -> Vec<LocalOverride> {
    [
        ("device", &local.device),
        ("system", &local.system),
        ("folder", &local.folder),
        ("game", &local.game),
        ("session", &local.session),
    ]
    .into_iter()
    .flat_map(|(layer, map)| {
        map.iter().map(move |(key, value)| LocalOverride {
            key: key.clone(),
            layer: layer.into(),
            value: value.clone(),
        })
    })
    .collect()
}

fn validate_recipe_shape(recipe: &Recipe) -> Result<()> {
    if recipe.schema != SCHEMA
        || recipe.format != FORMAT
        || recipe.schema_version != 1
        || recipe.api_version != API_VERSION
        || recipe.kind != RecipeKind::CompatibilityRecipe
        || recipe.target_sku != TARGET_SKU
    {
        return Err(RecipeError::new(
            "identity-invalid",
            "recipe identity or API version is invalid",
        ));
    }
    validate_id(&recipe.target_id, "recipe target")?;
    validate_hash(&recipe.rom_sha256)?;
    validate_id(&recipe.system_id, "system")?;
    validate_id(&recipe.core.id, "core")?;
    let version = exact_version(&recipe.core.version_constraint)?;
    validate_version(version)?;
    if recipe.config_delta.is_empty() || recipe.config_delta.len() > MAX_DELTA {
        return Err(RecipeError::new(
            "delta-invalid",
            "config delta is outside bounds",
        ));
    }
    let mut keys = BTreeSet::new();
    for change in &recipe.config_delta {
        validate_key(&change.key)?;
        if !keys.insert(&change.key) {
            return Err(RecipeError::new(
                "delta-invalid",
                "config delta contains duplicate keys",
            ));
        }
    }
    validate_id(&recipe.profiles.display_profile_id, "display profile")?;
    validate_id(&recipe.profiles.input_profile_id, "input profile")?;
    validate_id(&recipe.profiles.power_profile_id, "power profile")?;
    if recipe.known_issues.len() > 16 || recipe.known_issues.iter().any(|issue| !safe_text(issue)) {
        return Err(RecipeError::new(
            "unsafe-value",
            "known issue text is unsafe or oversized",
        ));
    }
    Ok(())
}
fn validate_delta(delta: &[ConfigChange], registry: &Registry) -> Result<()> {
    for change in delta {
        let setting = registry
            .settings
            .iter()
            .find(|setting| setting.id == change.key)
            .ok_or_else(|| {
                RecipeError::new("invalid-config-key", "config key is not allowlisted")
            })?;
        if setting.namespace != "core"
            || !matches!(
                setting.kind,
                FieldKind::Boolean
                    | FieldKind::Integer
                    | FieldKind::Decimal
                    | FieldKind::EnumSingle
                    | FieldKind::EnumMulti
            )
        {
            return Err(RecipeError::new(
                "invalid-config-key",
                "config key is outside the recipe allowlist",
            ));
        }
        if !value_matches_kind(&change.value, setting.kind) || !value_is_safe(&change.value) {
            return Err(RecipeError::new(
                "unsafe-value",
                "config value is not a permitted shape",
            ));
        }
        if let Some(constraints) = &setting.constraints {
            if let Some(range) = &constraints.range {
                let number = match &change.value {
                    SettingValue::Integer(value) => *value as f64,
                    SettingValue::Decimal(value) => *value,
                    _ => 0.0,
                };
                if matches!(
                    change.value,
                    SettingValue::Integer(_) | SettingValue::Decimal(_)
                ) && (number < range.min
                    || number > range.max
                    || !number.is_finite()
                    || ((number - range.min) / range.step).fract().abs() > 1e-9)
                {
                    return Err(RecipeError::new(
                        "unsafe-value",
                        "numeric config value is outside its producer constraint",
                    ));
                }
            }
            if !constraints.options.is_empty() {
                let allowed: BTreeSet<_> = constraints
                    .options
                    .iter()
                    .map(|option| option.value.as_str())
                    .collect();
                let valid = match &change.value {
                    SettingValue::EnumSingle(value) => allowed.contains(value.as_str()),
                    SettingValue::EnumMulti(values) => {
                        values.iter().all(|item| allowed.contains(item.as_str()))
                            && values.iter().collect::<BTreeSet<_>>().len() == values.len()
                    }
                    _ => true,
                };
                if !valid {
                    return Err(RecipeError::new(
                        "unsafe-value",
                        "enum config value is outside its producer constraint",
                    ));
                }
            }
        }
    }
    Ok(())
}
fn validate_local(local: &LocalOverrides, registry: &Registry) -> Result<()> {
    for map in [
        &local.device,
        &local.system,
        &local.folder,
        &local.game,
        &local.session,
    ] {
        for (key, value) in map.iter() {
            let setting = registry
                .settings
                .iter()
                .find(|setting| setting.id == *key)
                .ok_or_else(|| {
                    RecipeError::new("invalid-config-key", "local config key is not allowlisted")
                })?;
            if setting.namespace != "core"
                || setting.redacted
                || !value_matches_kind(value, setting.kind)
                || !value_is_safe(value)
            {
                return Err(RecipeError::new(
                    "unsafe-value",
                    "local config value is not permitted",
                ));
            }
        }
    }
    Ok(())
}
fn value_matches_kind(value: &SettingValue, kind: FieldKind) -> bool {
    matches!(
        (kind, value),
        (FieldKind::Boolean, SettingValue::Boolean(_))
            | (FieldKind::Integer, SettingValue::Integer(_))
            | (FieldKind::Decimal, SettingValue::Decimal(_))
            | (FieldKind::EnumSingle, SettingValue::EnumSingle(_))
            | (FieldKind::EnumMulti, SettingValue::EnumMulti(_))
    )
}
fn value_is_safe(value: &SettingValue) -> bool {
    match value {
        SettingValue::Boolean(_) | SettingValue::Integer(_) => true,
        SettingValue::Decimal(v) => v.is_finite(),
        SettingValue::EnumSingle(v) => safe_token(v),
        SettingValue::EnumMulti(v) => v.iter().all(|item| safe_token(item)),
        SettingValue::Text(v) => safe_text(v),
        SettingValue::Secret(_) => false,
    }
}
fn validate_key(key: &str) -> Result<()> {
    if !key.starts_with("core.")
        || key.len() > 128
        || !key.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return Err(RecipeError::new(
            "invalid-config-key",
            "config key is not a safe namespaced key",
        ));
    }
    Ok(())
}
fn safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
fn safe_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.bytes().any(|byte| byte.is_ascii_control())
        && !value.contains("//")
        && !value.contains("\\")
        && !value.starts_with('/')
        && !value.contains("..")
        && ![
            "script", "command", "exec", "argv", "shell", "payload", "http:", "https:",
        ]
        .iter()
        .any(|word| value.to_ascii_lowercase().contains(word))
}
fn validate_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().enumerate().all(|(i, b)| {
            (i == 0 && b.is_ascii_lowercase())
                || (i > 0 && (b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        })
    {
        return Err(RecipeError::new(
            "invalid-identifier",
            format!("{label} identifier is invalid"),
        ));
    }
    Ok(())
}
fn validate_version(value: &str) -> Result<()> {
    if value.len() > 32
        || value.split('.').count() != 3
        || !value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(RecipeError::new("invalid-version", "version is invalid"));
    }
    Ok(())
}
fn exact_version(value: &str) -> Result<&str> {
    value
        .strip_prefix('=')
        .filter(|version| !version.is_empty())
        .ok_or_else(|| RecipeError::new("invalid-version", "core version constraint must be exact"))
}
fn validate_hash(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(RecipeError::new(
            "invalid-hash",
            "content hash is not lowercase SHA-256",
        ));
    }
    Ok(())
}
fn read_repository_file(root: &Path, relative: &str) -> Result<Vec<u8>> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains('\\')
        || relative.contains('\0')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(RecipeError::new(
            "repository-invalid",
            "repository path is unsafe",
        ));
    }
    if fs::symlink_metadata(root)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(RecipeError::new(
            "repository-invalid",
            "repository root is a symlink",
        ));
    }
    let path = root.join(relative);
    let mut current = root.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        if fs::symlink_metadata(&current)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(RecipeError::new(
                "repository-invalid",
                "repository path is a symlink",
            ));
        }
    }
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| RecipeError::new("repository-unavailable", "recipe file is unavailable"))?;
    if !metadata.file_type().is_file() {
        return Err(RecipeError::new(
            "repository-invalid",
            "repository object is not a regular file",
        ));
    }
    fs::read(path)
        .map_err(|_| RecipeError::new("repository-unavailable", "recipe file is unavailable"))
}
fn blocked_core_pack(root: &Path, id: &str, version: &str) -> Result<bool> {
    let path = root.join("core-packs/stable.json");
    let Ok(bytes) = fs::read(path) else {
        return Ok(false);
    };
    let value = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|_| RecipeError::new("catalog-unavailable", "core-pack catalog is malformed"))?;
    fn walk(value: &serde_json::Value, id: &str, version: &str) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                let match_core = map.get("id").and_then(serde_json::Value::as_str) == Some(id)
                    && map.get("version").and_then(serde_json::Value::as_str) == Some(version)
                    && map.get("status").and_then(serde_json::Value::as_str) == Some("blocked");
                match_core || map.values().any(|child| walk(child, id, version))
            }
            serde_json::Value::Array(items) => items.iter().any(|child| walk(child, id, version)),
            _ => false,
        }
    }
    Ok(walk(&value, id, version))
}
fn private_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    let mut current = root.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        if fs::symlink_metadata(&current)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(RecipeError::new(
                "unsafe-path",
                "private recipe path is a symlink",
            ));
        }
    }
    Ok(path)
}
fn validate_private_root(root: &Path) -> Result<()> {
    if root.as_os_str().is_empty()
        || root == Path::new("/")
        || root
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || fs::symlink_metadata(root)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Err(RecipeError::new(
            "unsafe-path",
            "private recipe root is invalid",
        ));
    }
    fs::create_dir_all(root)
        .map_err(|_| RecipeError::new("storage-failed", "private recipe root cannot be created"))?;
    Ok(())
}
fn read_optional_layer(path: &Path) -> Result<Option<RecipeLayer>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(RecipeError::new("unsafe-path", "recipe layer is a symlink"))
        }
        Ok(meta) if !meta.file_type().is_file() => Err(RecipeError::new(
            "storage-failed",
            "recipe layer is not a regular file",
        )),
        Ok(_) => Ok(Some(read_json(path)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(RecipeError::new(
            "storage-failed",
            "recipe layer cannot be read",
        )),
    }
}
fn validate_layer_identity(layer: &RecipeLayer, target_id: &str) -> Result<()> {
    if layer.format != "brickpro-compatibility-recipe-layer"
        || layer.schema_version != 1
        || layer.generation == 0
        || layer.target_id != target_id
        || layer.config_delta.is_empty()
        || layer.config_delta.len() > MAX_DELTA
        || layer
            .replacement_keys
            .iter()
            .any(|key| !layer.config_delta.iter().any(|change| change.key == *key))
    {
        return Err(RecipeError::new(
            "storage-failed",
            "recipe layer identity is invalid",
        ));
    }
    Ok(())
}
fn restore_layer(path: &Path, prior: Option<&RecipeLayer>) -> Result<()> {
    if let Some(prior) = prior {
        atomic_json(path, prior)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(RecipeError::new(
                "storage-failed",
                "recipe layer could not be restored",
            )),
        }
    }
}
fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RecipeError::new("storage-failed", "private path has no parent"))?;
    fs::create_dir_all(parent)
        .map_err(|_| RecipeError::new("storage-failed", "private directory cannot be created"))?;
    let temp = parent.join(format!(
        ".{}.tmp",
        path.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_file(&temp);
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| RecipeError::new("storage-failed", "private metadata cannot be serialized"))?;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|_| {
            RecipeError::new("storage-failed", "private temporary cannot be created")
        })?;
        file.write_all(&bytes).map_err(|_| {
            RecipeError::new("storage-failed", "private metadata cannot be written")
        })?;
        file.sync_all()
            .map_err(|_| RecipeError::new("storage-failed", "private metadata cannot be synced"))?;
        drop(file);
        fs::rename(&temp, path).map_err(|_| {
            RecipeError::new("storage-failed", "private metadata cannot be published")
        })?;
        OpenOptions::new()
            .read(true)
            .open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| {
                RecipeError::new("storage-failed", "private directory cannot be synced")
            })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)
        .map_err(|_| RecipeError::new("storage-failed", "private metadata cannot be read"))?;
    reject_duplicate_keys(&bytes)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| RecipeError::new("storage-failed", "private metadata is invalid"))
}

fn reject_duplicate_keys(bytes: &[u8]) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer.deserialize_any(RejectVisitor).map_err(|_| {
        RecipeError::new(
            "malformed-recipe",
            "JSON is malformed or contains duplicate keys",
        )
    })?;
    deserializer
        .end()
        .map_err(|_| RecipeError::new("malformed-recipe", "JSON has trailing data"))
}
struct RejectSeed;
impl<'de> de::DeserializeSeed<'de> for RejectSeed {
    type Value = ();
    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RejectVisitor)
    }
}
struct RejectVisitor;
impl<'de> de::Visitor<'de> for RejectVisitor {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_borrowed_str<E>(self, _: &'de str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_some<D>(self, d: D) -> std::result::Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        d.deserialize_any(RejectVisitor)
    }
    fn visit_seq<A>(self, mut s: A) -> std::result::Result<(), A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        while s.next_element_seed(RejectSeed)?.is_some() {}
        Ok(())
    }
    fn visit_map<A>(self, mut m: A) -> std::result::Result<(), A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = m.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(de::Error::custom("duplicate named key"));
            }
            m.next_value_seed(RejectSeed)?;
        }
        Ok(())
    }
}
