use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use serde::{de, Deserialize, Deserializer, Serialize};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const MAX_REGISTRY_BYTES: usize = 512 * 1024;
pub const MAX_SETTINGS: usize = 512;
pub const MAX_OPTIONS: usize = 128;
pub const MAX_PREDICATE_DEPTH: usize = 8;
pub const MAX_PREDICATE_NODES: usize = 64;
const FORMAT: &str = "brickpro-settings-registry";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Registry {
    pub format: String,
    pub schema_version: u32,
    pub allowed_namespaces: Vec<String>,
    pub sections: Vec<Section>,
    pub settings: Vec<SettingDescriptor>,
    pub migrations: Vec<MigrationMetadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub label_key: String,
    pub description_key: String,
    pub order: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettingDescriptor {
    pub id: String,
    pub namespace: String,
    pub section: String,
    pub group: String,
    pub order: i32,
    pub label_key: String,
    pub description_key: String,
    pub kind: FieldKind,
    pub default: Option<SettingValue>,
    pub current: Option<SettingValue>,
    pub pending: Option<SettingValue>,
    pub constraints: Option<Constraints>,
    pub units: Option<String>,
    pub display: Option<DisplayHints>,
    pub scope: Scope,
    pub apply: Vec<ApplyMode>,
    pub requires_capabilities: Vec<String>,
    pub unsupported_reason: Option<String>,
    pub visibility: Option<Predicate>,
    pub enabled_if: Option<Predicate>,
    pub redacted: bool,
    pub validation: ValidationMetadata,
    pub migration: Option<SettingMigration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderMetadata {
    pub id: String,
    pub enabled: bool,
    pub requires_credentials: bool,
    pub credential_configured: bool,
    pub priority: u8,
    pub max_concurrency: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldKind {
    Boolean,
    Integer,
    Decimal,
    Text,
    Secret,
    EnumSingle,
    EnumMulti,
    Action,
    ReadOnly,
    Status,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "kind",
    content = "value",
    rename_all = "kebab-case"
)]
pub enum SettingValue {
    Boolean(bool),
    Integer(i64),
    Decimal(f64),
    Text(String),
    Secret(CredentialReference),
    EnumSingle(String),
    EnumMulti(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CredentialReference {
    pub credential_ref: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Constraints {
    pub range: Option<NumericRange>,
    pub text: Option<TextConstraints>,
    pub options: Vec<OptionDescriptor>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NumericRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TextConstraints {
    pub min_length: usize,
    pub max_length: usize,
    pub pattern: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OptionDescriptor {
    pub value: String,
    pub label_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DisplayHints {
    pub unit: Option<String>,
    pub format: Option<String>,
    pub redact: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    System,
    User,
    EmulatorProfile,
    Core,
    Game,
    Theme,
    Provider,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplyMode {
    Immediate,
    OnConfirm,
    RestartLauncher,
    RebootCandidate,
    ExternalOperation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ValidationMetadata {
    pub required: bool,
    pub trim: bool,
    pub allow_empty: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettingMigration {
    pub key: String,
    pub introduced_in: u32,
    pub removed_in: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MigrationMetadata {
    pub id: String,
    pub from_version: u32,
    pub to_version: u32,
    pub changes: Vec<MigrationChange>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MigrationChange {
    pub setting_id: String,
    pub operation: MigrationOperation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MigrationOperation {
    Rename,
    Replace,
    Remove,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "kebab-case")]
pub enum Predicate {
    All {
        predicates: Vec<Predicate>,
    },
    Any {
        predicates: Vec<Predicate>,
    },
    Not {
        predicate: Box<Predicate>,
    },
    Equals {
        setting: String,
        value: SettingValue,
    },
    Present {
        setting: String,
    },
    Capability {
        capability: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectionContext {
    pub values: HashMap<String, SettingValue>,
    pub capabilities: HashSet<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MenuModel {
    pub sections: Vec<MenuSection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MenuSection {
    pub id: String,
    pub label_key: String,
    pub description_key: String,
    pub groups: Vec<MenuGroup>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MenuGroup {
    pub id: String,
    pub controls: Vec<FormControl>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FormControl {
    pub setting_id: String,
    pub kind: FieldKind,
    pub label_key: String,
    pub description_key: String,
    pub value: Option<SettingValue>,
    pub default: Option<SettingValue>,
    pub pending: Option<SettingValue>,
    pub constraints: Option<Constraints>,
    pub units: Option<String>,
    pub display: Option<DisplayHints>,
    pub scope: Scope,
    pub apply: ApplyMode,
    pub enabled: bool,
    pub unsupported_reason: Option<String>,
    pub redacted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    Json(String),
    Invalid(String),
    UnsupportedSchemaVersion(u32),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(message) => write!(formatter, "invalid registry JSON: {message}"),
            Self::Invalid(message) => write!(formatter, "invalid settings registry: {message}"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(formatter, "unsupported settings schema version: {version}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

impl Registry {
    pub fn from_json(bytes: &[u8]) -> Result<Self, RegistryError> {
        if bytes.len() > MAX_REGISTRY_BYTES {
            return invalid("registry exceeds size budget");
        }
        reject_duplicate_keys(bytes)?;
        let registry: Self = serde_json::from_slice(bytes)
            .map_err(|error| RegistryError::Json(error.to_string()))?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn validate(&self) -> Result<(), RegistryError> {
        if self.format != FORMAT {
            return invalid("format must be brickpro-settings-registry");
        }
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(RegistryError::UnsupportedSchemaVersion(self.schema_version));
        }
        if self.allowed_namespaces.is_empty()
            || !self.allowed_namespaces.iter().any(|n| n == "core")
        {
            return invalid("allowedNamespaces must include core");
        }
        ensure_unique(&self.allowed_namespaces, "namespace")?;
        for namespace in &self.allowed_namespaces {
            if !valid_name(namespace) {
                return invalid(format!("invalid namespace: {namespace}"));
            }
        }

        if self.settings.len() > MAX_SETTINGS {
            return invalid("registry exceeds setting count budget");
        }
        let mut section_ids = HashSet::new();
        for section in &self.sections {
            check_key(&section.id, "section id")?;
            check_key(&section.label_key, "section label key")?;
            check_key(&section.description_key, "section description key")?;
            if !section_ids.insert(section.id.as_str()) {
                return invalid(format!("duplicate section id: {}", section.id));
            }
        }
        let section_ids: HashSet<&str> = section_ids;

        let mut setting_ids = HashSet::new();
        for setting in &self.settings {
            check_key(&setting.id, "setting id")?;
            if !valid_name(&setting.id) {
                return invalid(format!("invalid setting id: {}", setting.id));
            }
            if !setting_ids.insert(setting.id.as_str()) {
                return invalid(format!("duplicate setting id: {}", setting.id));
            }
            if !self
                .allowed_namespaces
                .iter()
                .any(|n| n == &setting.namespace)
            {
                return invalid(format!(
                    "namespace is not allowlisted: {}",
                    setting.namespace
                ));
            }
            if setting.namespace != "core" && setting.id.starts_with("core.") {
                return invalid(format!("provider cannot override core id: {}", setting.id));
            }
            if !setting.id.starts_with(&(setting.namespace.clone() + ".")) {
                return invalid(format!("setting id is outside namespace: {}", setting.id));
            }
            if !section_ids.contains(setting.section.as_str()) {
                return invalid(format!("unknown section: {}", setting.section));
            }
            check_key(&setting.group, "setting group")?;
            check_key(&setting.label_key, "setting label key")?;
            check_key(&setting.description_key, "setting description key")?;
            if setting.apply.len() != 1 {
                return invalid(format!(
                    "setting {} must have exactly one apply mode",
                    setting.id
                ));
            }
            if setting.kind == FieldKind::Action && setting.apply[0] != ApplyMode::ExternalOperation
            {
                return invalid(format!("action {} must use external-operation", setting.id));
            }
            if matches!(setting.kind, FieldKind::ReadOnly | FieldKind::Status)
                && setting.apply[0] != ApplyMode::Immediate
            {
                return invalid(format!(
                    "read-only/status {} must use immediate",
                    setting.id
                ));
            }
            if setting.requires_capabilities.is_empty() != setting.unsupported_reason.is_none() {
                return invalid(format!("capability reason mismatch for {}", setting.id));
            }
            ensure_unique(&setting.requires_capabilities, "capability")?;
            for capability in &setting.requires_capabilities {
                check_key(capability, "capability")?;
            }
            validate_value_shape(setting)?;
            validate_constraints(setting)?;
            if setting.kind == FieldKind::Secret && !setting.redacted {
                return invalid(format!("secret {} must be redacted", setting.id));
            }
            if let Some(display) = &setting.display {
                if display.redact && !setting.redacted {
                    return invalid(format!(
                        "display redaction requires redacted: {}",
                        setting.id
                    ));
                }
            }
            if let Some(migration) = &setting.migration {
                check_key(&migration.key, "migration key")?;
                if migration.introduced_in > self.schema_version
                    || migration
                        .removed_in
                        .is_some_and(|version| version <= migration.introduced_in)
                {
                    return invalid(format!("invalid migration metadata for {}", setting.id));
                }
            }
        }
        validate_predicates(self, &setting_ids)?;
        validate_migrations(self, &setting_ids)
    }

    pub fn with_provider_metadata(
        mut self,
        providers: &[ProviderMetadata],
    ) -> Result<Self, RegistryError> {
        let mut ids = HashSet::new();
        for provider in providers {
            if !valid_name(&provider.id)
                || !ids.insert(provider.id.as_str())
                || !matches!(provider.priority, 1..=3)
                || !matches!(provider.max_concurrency, 1 | 2 | 4)
                || (!provider.requires_credentials && provider.credential_configured)
            {
                return invalid("invalid provider metadata");
            }
        }
        if providers.is_empty() || providers.len() > 3 {
            return invalid("provider metadata is outside bounds");
        }
        let options = providers
            .iter()
            .map(|provider| OptionDescriptor {
                value: provider.id.clone(),
                label_key: "settings.scraper.provider.name".into(),
            })
            .collect::<Vec<_>>();
        let mut ordered_providers: Vec<_> = providers.iter().collect();
        ordered_providers.sort_by_key(|provider| provider.priority);
        let ordered_ids = ordered_providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect::<Vec<_>>();
        self.settings.push(SettingDescriptor {
            id: "provider.scraper.priority".into(),
            namespace: "provider.scraper".into(),
            section: "scraper".into(),
            group: "providers".into(),
            order: 40,
            label_key: "settings.scraper.priority.label".into(),
            description_key: "settings.scraper.priority.description".into(),
            kind: FieldKind::EnumMulti,
            default: Some(SettingValue::EnumMulti(ordered_ids.clone())),
            current: Some(SettingValue::EnumMulti(ordered_ids)),
            pending: None,
            constraints: Some(Constraints {
                range: None,
                text: None,
                options,
            }),
            units: None,
            display: Some(DisplayHints {
                unit: None,
                format: Some("ordered-list".into()),
                redact: false,
            }),
            scope: Scope::Provider,
            apply: vec![ApplyMode::OnConfirm],
            requires_capabilities: Vec::new(),
            unsupported_reason: None,
            visibility: None,
            enabled_if: None,
            redacted: false,
            validation: ValidationMetadata {
                required: true,
                trim: false,
                allow_empty: false,
            },
            migration: None,
        });
        for provider in providers {
            let prefix = format!("provider.scraper.{}", provider.id);
            let mut enabled = provider_setting(
                format!("{prefix}-enabled"),
                30 + i32::from(provider.priority),
                FieldKind::Boolean,
                SettingValue::Boolean(provider.enabled),
                "settings.scraper.provider.enabled",
                "settings.scraper.provider.toggle",
                None,
            );
            enabled.apply = vec![ApplyMode::OnConfirm];
            self.settings.push(enabled);
            let credential_status = if provider.requires_credentials {
                if provider.credential_configured {
                    "required; configured"
                } else {
                    "required; not configured"
                }
            } else {
                "anonymous; configured"
            };
            self.settings.push(provider_setting(
                format!("{prefix}-credentials"),
                50 + i32::from(provider.priority),
                FieldKind::Status,
                SettingValue::Text(credential_status.into()),
                "settings.scraper.credentials.status",
                "settings.scraper.credentials.status",
                None,
            ));
            self.settings.push(provider_setting(
                format!("{prefix}-limit"),
                60 + i32::from(provider.priority),
                FieldKind::Status,
                SettingValue::Text(provider.max_concurrency.to_string()),
                "settings.scraper.provider.limit",
                "settings.scraper.provider.limit.status",
                Some("requests".into()),
            ));
        }
        self.validate()?;
        Ok(self)
    }

    pub fn canonicalized(&self) -> Self {
        let mut result = self.clone();
        result.allowed_namespaces.sort();
        result
            .sections
            .sort_by_key(|section| (section.order, section.id.clone()));
        result.settings.sort_by_key(|setting| {
            (
                setting.section.clone(),
                setting.group.clone(),
                setting.order,
                setting.id.clone(),
            )
        });
        result
            .migrations
            .sort_by_key(|migration| migration.id.clone());
        result
    }

    pub fn to_canonical_json(&self) -> Result<String, RegistryError> {
        self.validate()?;
        serde_json::to_string(&self.canonicalized())
            .map_err(|error| RegistryError::Json(error.to_string()))
    }

    pub fn project(&self, context: &ProjectionContext) -> Result<MenuModel, RegistryError> {
        self.validate()?;
        let registry = self.canonicalized();
        let mut values = HashMap::new();
        for setting in &registry.settings {
            if let Some(value) = setting.current.clone().or_else(|| setting.default.clone()) {
                values.insert(setting.id.clone(), value);
            }
        }
        values.extend(context.values.clone());
        let mut sections = Vec::new();
        for section in &registry.sections {
            let mut groups: Vec<MenuGroup> = Vec::new();
            for setting in registry
                .settings
                .iter()
                .filter(|setting| setting.section == section.id)
            {
                if !predicate_matches(setting.visibility.as_ref(), &values, &context.capabilities) {
                    continue;
                }
                let missing_capability = setting
                    .requires_capabilities
                    .iter()
                    .find(|capability| !context.capabilities.contains(*capability));
                let enabled = missing_capability.is_none()
                    && predicate_matches(
                        setting.enabled_if.as_ref(),
                        &values,
                        &context.capabilities,
                    );
                let group = groups.iter_mut().find(|group| group.id == setting.group);
                let control = FormControl {
                    setting_id: setting.id.clone(),
                    kind: setting.kind,
                    label_key: setting.label_key.clone(),
                    description_key: setting.description_key.clone(),
                    value: if setting.redacted {
                        None
                    } else {
                        values.get(&setting.id).cloned()
                    },
                    default: if setting.redacted {
                        None
                    } else {
                        setting.default.clone()
                    },
                    pending: if setting.redacted {
                        None
                    } else {
                        setting.pending.clone()
                    },
                    constraints: setting.constraints.clone(),
                    units: setting.units.clone(),
                    display: setting.display.clone(),
                    scope: setting.scope,
                    apply: setting.apply[0],
                    enabled,
                    unsupported_reason: missing_capability
                        .and_then(|_| setting.unsupported_reason.clone()),
                    redacted: setting.redacted,
                };
                if let Some(group) = group {
                    group.controls.push(control);
                } else {
                    groups.push(MenuGroup {
                        id: setting.group.clone(),
                        controls: vec![control],
                    });
                }
            }
            if !groups.is_empty() {
                sections.push(MenuSection {
                    id: section.id.clone(),
                    label_key: section.label_key.clone(),
                    description_key: section.description_key.clone(),
                    groups,
                });
            }
        }
        Ok(MenuModel { sections })
    }
}

impl MenuModel {
    pub fn to_canonical_json(&self) -> Result<String, RegistryError> {
        serde_json::to_string(self).map_err(|error| RegistryError::Json(error.to_string()))
    }
}

fn provider_setting(
    id: String,
    order: i32,
    kind: FieldKind,
    value: SettingValue,
    label_key: &str,
    description_key: &str,
    units: Option<String>,
) -> SettingDescriptor {
    SettingDescriptor {
        id,
        namespace: "provider.scraper".into(),
        section: "scraper".into(),
        group: "providers".into(),
        order,
        label_key: label_key.into(),
        description_key: description_key.into(),
        kind,
        default: None,
        current: Some(value),
        pending: None,
        constraints: None,
        units,
        display: Some(DisplayHints {
            unit: None,
            format: Some(if kind == FieldKind::Boolean {
                "toggle".into()
            } else {
                "status".into()
            }),
            redact: false,
        }),
        scope: Scope::Provider,
        apply: vec![ApplyMode::Immediate],
        requires_capabilities: Vec::new(),
        unsupported_reason: None,
        visibility: None,
        enabled_if: None,
        redacted: false,
        validation: ValidationMetadata {
            required: true,
            trim: false,
            allow_empty: false,
        },
        migration: None,
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, RegistryError> {
    Err(RegistryError::Invalid(message.into()))
}

fn check_key(value: &str, kind: &str) -> Result<(), RegistryError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_whitespace) {
        return invalid(format!("invalid {kind}: {value}"));
    }
    Ok(())
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            let mut characters = part.chars();
            matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
                && characters.all(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
                })
        })
}

fn ensure_unique(values: &[String], kind: &str) -> Result<(), RegistryError> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return invalid(format!("duplicate {kind}: {value}"));
        }
    }
    Ok(())
}

fn validate_value_shape(setting: &SettingDescriptor) -> Result<(), RegistryError> {
    for (name, value) in [
        ("default", &setting.default),
        ("current", &setting.current),
        ("pending", &setting.pending),
    ] {
        if let Some(value) = value {
            if !value_matches_kind(value, setting.kind) {
                return invalid(format!("{name} has the wrong type for {}", setting.id));
            }
            if let SettingValue::Secret(reference) = value {
                check_key(&reference.credential_ref, "credential reference")?;
            }
        }
    }
    if matches!(setting.kind, FieldKind::Action)
        && (setting.default.is_some() || setting.current.is_some() || setting.pending.is_some())
    {
        return invalid(format!("action {} cannot carry a value", setting.id));
    }
    Ok(())
}

fn value_matches_kind(value: &SettingValue, kind: FieldKind) -> bool {
    matches!(
        (kind, value),
        (FieldKind::Boolean, SettingValue::Boolean(_))
            | (FieldKind::Integer, SettingValue::Integer(_))
            | (FieldKind::Decimal, SettingValue::Decimal(_))
            | (FieldKind::Text, SettingValue::Text(_))
            | (FieldKind::Secret, SettingValue::Secret(_))
            | (FieldKind::EnumSingle, SettingValue::EnumSingle(_))
            | (FieldKind::EnumMulti, SettingValue::EnumMulti(_))
            | (FieldKind::ReadOnly, SettingValue::Text(_))
            | (FieldKind::Status, SettingValue::Text(_))
    )
}

fn validate_constraints(setting: &SettingDescriptor) -> Result<(), RegistryError> {
    let Some(constraints) = &setting.constraints else {
        if matches!(setting.kind, FieldKind::EnumSingle | FieldKind::EnumMulti) {
            return invalid(format!("enum {} must declare options", setting.id));
        }
        return Ok(());
    };
    if let Some(range) = &constraints.range {
        if !range.min.is_finite()
            || !range.max.is_finite()
            || !range.step.is_finite()
            || range.min > range.max
            || range.step <= 0.0
            || matches!(
                setting.kind,
                FieldKind::Boolean
                    | FieldKind::Text
                    | FieldKind::Secret
                    | FieldKind::EnumSingle
                    | FieldKind::EnumMulti
                    | FieldKind::Action
                    | FieldKind::ReadOnly
                    | FieldKind::Status
            )
            || (setting.kind == FieldKind::Integer
                && (range.min.fract() != 0.0
                    || range.max.fract() != 0.0
                    || range.step.fract() != 0.0))
        {
            return invalid(format!("invalid numeric range for {}", setting.id));
        }
        for value in [&setting.default, &setting.current, &setting.pending]
            .into_iter()
            .flatten()
        {
            let number = match value {
                SettingValue::Integer(value) => *value as f64,
                SettingValue::Decimal(value) => *value,
                _ => continue,
            };
            let quotient = (number - range.min) / range.step;
            if number < range.min
                || number > range.max
                || (quotient - quotient.round()).abs() > 1e-9
            {
                return invalid(format!("value outside range for {}", setting.id));
            }
        }
    }
    if let Some(text) = &constraints.text {
        if !matches!(
            setting.kind,
            FieldKind::Text | FieldKind::Secret | FieldKind::ReadOnly | FieldKind::Status
        ) || text.min_length > text.max_length
            || text.max_length > 4096
            || text
                .pattern
                .as_deref()
                .is_some_and(|pattern| !safe_pattern(pattern))
        {
            return invalid(format!("invalid text constraints for {}", setting.id));
        }
        for value in [&setting.default, &setting.current, &setting.pending]
            .into_iter()
            .flatten()
        {
            if let SettingValue::Text(value) = value {
                if value.chars().count() < text.min_length
                    || value.chars().count() > text.max_length
                {
                    return invalid(format!(
                        "text length outside constraints for {}",
                        setting.id
                    ));
                }
            }
        }
    }
    if matches!(setting.kind, FieldKind::EnumSingle | FieldKind::EnumMulti)
        && constraints.options.is_empty()
    {
        return invalid(format!("enum {} must declare options", setting.id));
    }
    if constraints.options.len() > MAX_OPTIONS {
        return invalid(format!("options exceed count budget for {}", setting.id));
    }
    if !constraints.options.is_empty() {
        if !matches!(setting.kind, FieldKind::EnumSingle | FieldKind::EnumMulti) {
            return invalid(format!("options are only valid for enums: {}", setting.id));
        }
        let options: HashSet<&str> = constraints
            .options
            .iter()
            .map(|option| option.value.as_str())
            .collect();
        if options.len() != constraints.options.len() || options.is_empty() {
            return invalid(format!("duplicate or empty options for {}", setting.id));
        }
        for option in &constraints.options {
            check_key(&option.value, "option")?;
            check_key(&option.label_key, "option label key")?;
        }
        for value in [&setting.default, &setting.current, &setting.pending]
            .into_iter()
            .flatten()
        {
            let valid = match value {
                SettingValue::EnumSingle(value) => options.contains(value.as_str()),
                SettingValue::EnumMulti(values) => {
                    let unique: HashSet<&str> = values.iter().map(String::as_str).collect();
                    unique.len() == values.len()
                        && values.iter().all(|value| options.contains(value.as_str()))
                }
                _ => true,
            };
            if !valid {
                return invalid(format!("enum value outside options for {}", setting.id));
            }
        }
    }
    Ok(())
}

fn safe_pattern(pattern: &str) -> bool {
    if pattern.is_empty()
        || pattern.len() > 256
        || pattern.contains("(?")
        || pattern.contains("\\1")
        || pattern.contains("\\2")
    {
        return false;
    }
    let mut escaped = false;
    let mut class = false;
    for character in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '[' if !class => class = true,
            ']' if class => class = false,
            '(' | ')' | '{' | '}' if !class => return false,
            _ => {}
        }
    }
    !escaped && !class
}

fn validate_predicates(
    registry: &Registry,
    setting_ids: &HashSet<&str>,
) -> Result<(), RegistryError> {
    let kinds: HashMap<&str, FieldKind> = registry
        .settings
        .iter()
        .map(|setting| (setting.id.as_str(), setting.kind))
        .collect();
    for setting in &registry.settings {
        for predicate in [&setting.visibility, &setting.enabled_if]
            .into_iter()
            .flatten()
        {
            let mut nodes = 0;
            validate_predicate(predicate, &kinds, 0, &mut nodes)?;
        }
    }
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for setting in &registry.settings {
        let mut references = Vec::new();
        for predicate in [&setting.visibility, &setting.enabled_if]
            .into_iter()
            .flatten()
        {
            collect_references(predicate, &mut references);
        }
        edges.insert(setting.id.as_str(), references);
    }
    let mut colors = HashMap::new();
    for id in setting_ids {
        visit_cycle(id, &edges, &mut colors)?;
    }
    Ok(())
}

fn validate_predicate(
    predicate: &Predicate,
    kinds: &HashMap<&str, FieldKind>,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), RegistryError> {
    *nodes += 1;
    if depth > MAX_PREDICATE_DEPTH || *nodes > MAX_PREDICATE_NODES {
        return invalid("predicate exceeds bounds");
    }
    match predicate {
        Predicate::All { predicates } | Predicate::Any { predicates } => {
            if predicates.is_empty() {
                return invalid("all/any predicate cannot be empty");
            }
            for child in predicates {
                validate_predicate(child, kinds, depth + 1, nodes)?
            }
        }
        Predicate::Not { predicate } => validate_predicate(predicate, kinds, depth + 1, nodes)?,
        Predicate::Equals { setting, value } => {
            let Some(kind) = kinds.get(setting.as_str()) else {
                return invalid(format!("unknown predicate setting: {setting}"));
            };
            if !value_matches_kind(value, *kind) {
                return invalid(format!("predicate value has the wrong type for {setting}"));
            }
        }
        Predicate::Present { setting } => {
            if !kinds.contains_key(setting.as_str()) {
                return invalid(format!("unknown predicate setting: {setting}"));
            }
        }
        Predicate::Capability { capability } => check_key(capability, "predicate capability")?,
    }
    Ok(())
}

fn collect_references<'a>(predicate: &'a Predicate, references: &mut Vec<&'a str>) {
    match predicate {
        Predicate::All { predicates } | Predicate::Any { predicates } => {
            for child in predicates {
                collect_references(child, references);
            }
        }
        Predicate::Not { predicate } => collect_references(predicate, references),
        Predicate::Equals { setting, .. } | Predicate::Present { setting } => {
            references.push(setting)
        }
        Predicate::Capability { .. } => {}
    }
}

fn visit_cycle<'a>(
    id: &'a str,
    edges: &HashMap<&'a str, Vec<&'a str>>,
    colors: &mut HashMap<&'a str, u8>,
) -> Result<(), RegistryError> {
    match colors.get(id).copied() {
        Some(1) => return invalid(format!("circular predicate reference at {id}")),
        Some(2) => return Ok(()),
        _ => {}
    }
    colors.insert(id, 1);
    if let Some(references) = edges.get(id) {
        for reference in references {
            visit_cycle(reference, edges, colors)?;
        }
    }
    colors.insert(id, 2);
    Ok(())
}

fn validate_migrations(
    registry: &Registry,
    setting_ids: &HashSet<&str>,
) -> Result<(), RegistryError> {
    let mut ids = HashSet::new();
    for migration in &registry.migrations {
        check_key(&migration.id, "migration id")?;
        if !ids.insert(&migration.id) {
            return invalid(format!("duplicate migration id: {}", migration.id));
        }
        if migration.from_version >= migration.to_version
            || migration.to_version > registry.schema_version
        {
            return invalid(format!("invalid migration versions: {}", migration.id));
        }
        let mut changed = HashSet::new();
        for change in &migration.changes {
            if !setting_ids.contains(change.setting_id.as_str()) {
                return invalid(format!("unknown migrated setting: {}", change.setting_id));
            }
            if !changed.insert(&change.setting_id) {
                return invalid(format!("duplicate migration change: {}", change.setting_id));
            }
        }
    }
    Ok(())
}

fn predicate_matches(
    predicate: Option<&Predicate>,
    values: &HashMap<String, SettingValue>,
    capabilities: &HashSet<String>,
) -> bool {
    let Some(predicate) = predicate else {
        return true;
    };
    match predicate {
        Predicate::All { predicates } => predicates
            .iter()
            .all(|child| predicate_matches(Some(child), values, capabilities)),
        Predicate::Any { predicates } => predicates
            .iter()
            .any(|child| predicate_matches(Some(child), values, capabilities)),
        Predicate::Not { predicate } => !predicate_matches(Some(predicate), values, capabilities),
        Predicate::Equals { setting, value } => values.get(setting) == Some(value),
        Predicate::Present { setting } => values.contains_key(setting),
        Predicate::Capability { capability } => capabilities.contains(capability),
    }
}

fn reject_duplicate_keys(bytes: &[u8]) -> Result<(), RegistryError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer
        .deserialize_any(RejectVisitor)
        .map_err(|error| {
            RegistryError::Json(format!("malformed JSON or duplicate key: {error}"))
        })?;
    deserializer
        .end()
        .map_err(|error| RegistryError::Json(error.to_string()))
}

struct RejectSeed;

impl<'de> de::DeserializeSeed<'de> for RejectSeed {
    type Value = ();
    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RejectVisitor)
    }
}

struct RejectVisitor;

impl<'de> de::Visitor<'de> for RejectVisitor {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_borrowed_str<E>(self, _: &'de str) -> Result<(), E> {
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RejectVisitor)
    }
    fn visit_seq<A>(self, mut sequence: A) -> Result<(), A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        while sequence.next_element_seed(RejectSeed)?.is_some() {}
        Ok(())
    }
    fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate named key: {key}")));
            }
            map.next_value_seed(RejectSeed)?;
        }
        Ok(())
    }
}
