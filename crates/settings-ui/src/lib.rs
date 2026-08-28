use std::{collections::BTreeMap, fmt};

use regex_automata::meta::Regex;
use serde::{Deserialize, Serialize};
use settings_schema::{
    ApplyMode, Constraints, FieldKind, FormControl, ProjectionContext, Registry, SettingValue,
};
use ui_model::{Button as UiButton, ControllerHelpStrip, HelpBinding};
use virtual_keyboard::{AllowedChars, FieldPolicy, InputResult, Keyboard, TypedValue};

pub const SCENE_WIDTH: u16 = 1024;
pub const SCENE_HEIGHT: u16 = 768;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    SectionList,
    Form,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplyBadge {
    RestartLauncher,
    RebootCandidate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticValue {
    Boolean(bool),
    Integer(i64),
    Decimal(f64),
    Text(String),
    EnumSingle(String),
    EnumMulti(Vec<String>),
    Masked { length_scalars: usize },
    Empty,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingControl {
    pub setting_id: String,
    pub kind: FieldKind,
    pub label_key: String,
    pub description_key: String,
    pub value: SemanticValue,
    pub pending: Option<SemanticValue>,
    pub constraints: Option<Constraints>,
    pub units: Option<String>,
    pub display: Option<settings_schema::DisplayHints>,
    pub scope: settings_schema::Scope,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub apply: ApplyMode,
    pub badges: Vec<ApplyBadge>,
    pub redacted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingGroup {
    pub id: String,
    pub controls: Vec<SettingControl>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionScene {
    pub id: String,
    pub label_key: String,
    pub description_key: String,
    pub groups: Vec<SettingGroup>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormScene {
    pub section_id: String,
    pub selected_setting_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedHelp {
    pub setting_id: String,
    pub label_key: String,
    pub description_key: String,
    pub apply: ApplyMode,
    pub badges: Vec<ApplyBadge>,
    pub enabled: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChange {
    pub setting_id: String,
    pub value: SemanticValue,
    pub apply: ApplyMode,
    pub badges: Vec<ApplyBadge>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSummary {
    pub changes: Vec<PendingChange>,
    pub count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationError {
    pub setting_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyboardField {
    Text,
    Secret,
    Numeric,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardRequest {
    pub setting_id: String,
    pub kind: KeyboardField,
    pub masked: bool,
    pub length_scalars: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalOperationRequest {
    pub setting_id: String,
    pub operation: ApplyMode,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub width: u16,
    pub height: u16,
    pub surface: Surface,
    pub sections: Vec<SectionScene>,
    pub selected_section_id: Option<String>,
    pub form: Option<FormScene>,
    pub selected_help: Option<SelectedHelp>,
    pub controller_help: ControllerHelpStrip,
    pub pending: PendingSummary,
    pub validation_errors: Vec<ValidationError>,
    pub keyboard: Option<KeyboardRequest>,
    pub external_operations: Vec<ExternalOperationRequest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Navigation,
    ValueChanged,
    KeyboardRequested,
    ValidationFailed,
    Confirmed,
    Cancelled,
    Applied,
    Back,
    ExternalOperationRequested,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiEvent {
    pub kind: EventKind,
    pub setting_id: Option<String>,
    pub value: Option<SemanticValue>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ControllerAction {
    Press(virtual_keyboard::Button),
    SetValue {
        setting_id: String,
        value: SettingValue,
    },
    OpenKeyboard {
        setting_id: String,
    },
    Confirm,
    Cancel,
    Apply,
    Back,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiError {
    Registry(String),
    UnknownSetting(String),
    NotEditable(String),
    Disabled(String),
    InvalidValue(String),
    Keyboard(String),
}

impl fmt::Display for UiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(message) => write!(formatter, "registry projection failed: {message}"),
            Self::UnknownSetting(id) => write!(formatter, "unknown setting: {id}"),
            Self::NotEditable(id) => write!(formatter, "setting is not editable: {id}"),
            Self::Disabled(id) => write!(formatter, "setting is disabled: {id}"),
            Self::InvalidValue(id) => write!(formatter, "invalid value for setting: {id}"),
            Self::Keyboard(id) => write!(formatter, "keyboard unavailable for setting: {id}"),
        }
    }
}

impl std::error::Error for UiError {}

pub struct KeyboardSession {
    pub setting_id: String,
    keyboard: Keyboard,
}

impl KeyboardSession {
    pub fn press(&mut self, button: virtual_keyboard::Button) -> InputResult {
        self.keyboard.press(button)
    }

    pub fn scene(&self) -> virtual_keyboard::Scene {
        self.keyboard.scene()
    }

    pub fn semantic_snapshot(&self) -> virtual_keyboard::SemanticSnapshot {
        self.keyboard.semantic_snapshot()
    }

    pub fn drain_events(&mut self) -> Vec<virtual_keyboard::SemanticEvent> {
        self.keyboard.drain_events()
    }
}

impl fmt::Debug for KeyboardSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyboardSession")
            .field("setting_id", &self.setting_id)
            .field("keyboard", &self.keyboard)
            .finish()
    }
}

pub struct SettingsUi {
    registry: Registry,
    capabilities: std::collections::HashSet<String>,
    committed: BTreeMap<String, SettingValue>,
    pending: BTreeMap<String, SettingValue>,
    secret_lengths: BTreeMap<String, usize>,
    errors: BTreeMap<String, String>,
    external_operations: Vec<ExternalOperationRequest>,
    events: Vec<UiEvent>,
    surface: Surface,
    section_index: usize,
    row_index: usize,
    keyboard: Option<KeyboardRequest>,
}

impl Serialize for SettingsUi {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.scene()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl fmt::Debug for SettingsUi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SettingsUi")
            .field("capability_count", &self.capabilities.len())
            .field("committed", &self.committed)
            .field("pending", &self.pending)
            .field("secret_lengths", &self.secret_lengths)
            .field("errors", &self.errors)
            .field("external_operations", &self.external_operations)
            .field("surface", &self.surface)
            .field("section_index", &self.section_index)
            .field("row_index", &self.row_index)
            .field("keyboard", &self.keyboard)
            .finish()
    }
}

impl SettingsUi {
    pub fn new(registry: Registry, context: ProjectionContext) -> Result<Self, UiError> {
        registry
            .validate()
            .map_err(|error| UiError::Registry(error.to_string()))?;
        let mut committed = BTreeMap::new();
        for setting in &registry.settings {
            let value = context
                .values
                .get(&setting.id)
                .cloned()
                .or_else(|| setting.current.clone())
                .or_else(|| setting.default.clone());
            if let Some(value) = value.filter(|value| !matches!(value, SettingValue::Secret(_))) {
                committed.insert(setting.id.clone(), value);
            }
        }
        let pending = registry
            .settings
            .iter()
            .filter_map(|setting| {
                setting
                    .pending
                    .clone()
                    .filter(|value| !matches!(value, SettingValue::Secret(_)))
                    .map(|value| (setting.id.clone(), value))
            })
            .collect();
        Ok(Self {
            registry,
            capabilities: context.capabilities,
            committed,
            pending,
            secret_lengths: BTreeMap::new(),
            errors: BTreeMap::new(),
            external_operations: Vec::new(),
            events: Vec::new(),
            surface: Surface::SectionList,
            section_index: 0,
            row_index: 0,
            keyboard: None,
        })
    }

    pub fn scene(&self) -> Result<Scene, UiError> {
        let projection = self
            .registry
            .project(&self.projection_context())
            .map_err(|error| UiError::Registry(error.to_string()))?;
        let sections: Vec<SectionScene> = projection
            .sections
            .iter()
            .map(|section| SectionScene {
                id: section.id.clone(),
                label_key: section.label_key.clone(),
                description_key: section.description_key.clone(),
                groups: section
                    .groups
                    .iter()
                    .map(|group| SettingGroup {
                        id: group.id.clone(),
                        controls: group
                            .controls
                            .iter()
                            .map(|control| self.control(control))
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let selected_section_id = sections
            .get(self.section_index)
            .map(|section| section.id.clone());
        let selected_control = if self.surface == Surface::Form {
            selected_section_id
                .as_deref()
                .and_then(|id| sections.iter().find(|section| section.id == id))
                .and_then(|section| flattened_controls(section).get(self.row_index).copied())
        } else {
            None
        };
        let selected_help = selected_control.map(|control| SelectedHelp {
            setting_id: control.setting_id.clone(),
            label_key: control.label_key.clone(),
            description_key: control.description_key.clone(),
            apply: control.apply,
            badges: control.badges.clone(),
            enabled: control.enabled,
            reason: control.disabled_reason.clone(),
        });
        let form = (self.surface == Surface::Form).then(|| FormScene {
            section_id: selected_section_id.clone().unwrap_or_default(),
            selected_setting_id: selected_help.as_ref().map(|help| help.setting_id.clone()),
        });
        let mut pending = Vec::new();
        for (setting_id, value) in &self.pending {
            if self.committed.get(setting_id) == Some(value) {
                continue;
            }
            let Some(setting) = self
                .registry
                .settings
                .iter()
                .find(|setting| setting.id == *setting_id)
            else {
                continue;
            };
            pending.push(PendingChange {
                setting_id: setting_id.clone(),
                value: semantic_value(value, false),
                apply: setting.apply[0],
                badges: badges(setting.apply[0]),
            });
        }
        Ok(Scene {
            width: SCENE_WIDTH,
            height: SCENE_HEIGHT,
            surface: self.surface,
            sections,
            selected_section_id,
            form,
            selected_help,
            controller_help: controller_help(),
            pending: PendingSummary {
                count: pending.len(),
                changes: pending,
            },
            validation_errors: self
                .errors
                .iter()
                .map(|(setting_id, message)| ValidationError {
                    setting_id: setting_id.clone(),
                    message: message.clone(),
                })
                .collect(),
            keyboard: self.keyboard.clone(),
            external_operations: self.external_operations.clone(),
        })
    }

    pub fn semantic_snapshot(&self) -> Result<Scene, UiError> {
        self.scene()
    }

    pub fn press(&mut self, button: virtual_keyboard::Button) -> Result<(), UiError> {
        match button {
            virtual_keyboard::Button::Up => self.move_row(-1),
            virtual_keyboard::Button::Down => self.move_row(1),
            virtual_keyboard::Button::Left => self.change_selected(-1)?,
            virtual_keyboard::Button::Right => self.change_selected(1)?,
            virtual_keyboard::Button::Primary => self.activate_selected()?,
            virtual_keyboard::Button::Secondary => self.back(),
            virtual_keyboard::Button::Menu => self.cancel(),
            virtual_keyboard::Button::Start => self.apply()?,
            virtual_keyboard::Button::LeftShoulder
            | virtual_keyboard::Button::RightShoulder
            | virtual_keyboard::Button::Select => {}
        }
        Ok(())
    }

    pub fn dispatch(&mut self, action: ControllerAction) -> Result<(), UiError> {
        match action {
            ControllerAction::Press(button) => self.press(button),
            ControllerAction::SetValue { setting_id, value } => self.set_value(&setting_id, value),
            ControllerAction::OpenKeyboard { setting_id } => {
                self.open_keyboard(&setting_id).map(|_| ())
            }
            ControllerAction::Confirm => self.confirm(),
            ControllerAction::Cancel => {
                self.cancel();
                Ok(())
            }
            ControllerAction::Apply => self.apply(),
            ControllerAction::Back => {
                self.back();
                Ok(())
            }
        }
    }

    pub fn set_value(&mut self, setting_id: &str, value: SettingValue) -> Result<(), UiError> {
        let setting = self
            .registry
            .settings
            .iter()
            .find(|setting| setting.id == setting_id)
            .ok_or_else(|| UiError::UnknownSetting(setting_id.to_owned()))?;
        if matches!(
            setting.kind,
            FieldKind::Action | FieldKind::ReadOnly | FieldKind::Status
        ) {
            return self.reject(setting_id, UiError::NotEditable(setting_id.to_owned()));
        }
        if setting.kind == FieldKind::Secret {
            return self.reject(setting_id, UiError::Keyboard(setting_id.to_owned()));
        }
        if !self.is_enabled(setting_id)? {
            return self.reject(setting_id, UiError::Disabled(setting_id.to_owned()));
        }
        if !matches_kind(&value, setting.kind) || !valid_value(setting, &value) {
            return self.reject(setting_id, UiError::InvalidValue(setting_id.to_owned()));
        }
        self.errors.remove(setting_id);
        let apply = setting.apply[0];
        if apply == ApplyMode::Immediate {
            self.committed.insert(setting_id.to_owned(), value.clone());
            self.pending.remove(setting_id);
        } else {
            self.pending.insert(setting_id.to_owned(), value.clone());
        }
        self.events.push(UiEvent {
            kind: EventKind::ValueChanged,
            setting_id: Some(setting_id.to_owned()),
            value: Some(semantic_value(&value, false)),
            message: None,
        });
        Ok(())
    }

    pub fn open_keyboard(&mut self, setting_id: &str) -> Result<KeyboardSession, UiError> {
        let setting = self
            .registry
            .settings
            .iter()
            .find(|setting| setting.id == setting_id)
            .ok_or_else(|| UiError::UnknownSetting(setting_id.to_owned()))?;
        if !self.is_enabled(setting_id)? {
            return self
                .reject(setting_id, UiError::Disabled(setting_id.to_owned()))
                .map(|_| unreachable!());
        }
        let (kind, initial, max_bytes, max_scalars, allowed) = match setting.kind {
            FieldKind::Text => {
                let limits = text_limits(setting);
                (
                    virtual_keyboard::FieldKind::Text,
                    text_value(self.value_for(setting_id)),
                    limits.0,
                    limits.1,
                    AllowedChars::ascii(),
                )
            }
            FieldKind::Secret => (
                virtual_keyboard::FieldKind::Secret,
                String::new(),
                4096,
                4096,
                AllowedChars::ascii(),
            ),
            FieldKind::Integer => (
                virtual_keyboard::FieldKind::Numeric,
                numeric_text(self.value_for(setting_id), FieldKind::Integer),
                32,
                32,
                AllowedChars::numeric(),
            ),
            FieldKind::Decimal | FieldKind::Action | FieldKind::ReadOnly | FieldKind::Status => {
                return self
                    .reject(setting_id, UiError::Keyboard(setting_id.to_owned()))
                    .map(|_| unreachable!());
            }
            _ => {
                return self
                    .reject(setting_id, UiError::Keyboard(setting_id.to_owned()))
                    .map(|_| unreachable!())
            }
        };
        let policy = match kind {
            virtual_keyboard::FieldKind::Text => FieldPolicy::text(
                &initial,
                &setting.label_key,
                max_bytes,
                max_scalars,
                allowed,
            ),
            virtual_keyboard::FieldKind::Secret => FieldPolicy::secret(
                &initial,
                &setting.label_key,
                max_bytes,
                max_scalars,
                allowed,
            ),
            virtual_keyboard::FieldKind::Numeric => {
                FieldPolicy::numeric(&initial, &setting.label_key, max_bytes, max_scalars)
            }
        };
        let keyboard =
            Keyboard::new(policy).map_err(|_| UiError::Keyboard(setting_id.to_owned()))?;
        self.keyboard = Some(KeyboardRequest {
            setting_id: setting_id.to_owned(),
            kind: keyboard_field(kind),
            masked: kind == virtual_keyboard::FieldKind::Secret,
            length_scalars: if kind == virtual_keyboard::FieldKind::Secret {
                self.secret_lengths.get(setting_id).copied().unwrap_or(0)
            } else {
                initial.chars().count()
            },
        });
        self.events.push(UiEvent {
            kind: EventKind::KeyboardRequested,
            setting_id: Some(setting_id.to_owned()),
            value: None,
            message: None,
        });
        Ok(KeyboardSession {
            setting_id: setting_id.to_owned(),
            keyboard,
        })
    }

    pub fn accept_keyboard(
        &mut self,
        setting_id: &str,
        result: InputResult,
    ) -> Result<(), UiError> {
        let setting = self
            .registry
            .settings
            .iter()
            .find(|setting| setting.id == setting_id)
            .ok_or_else(|| UiError::UnknownSetting(setting_id.to_owned()))?;
        let value = match result {
            InputResult::Confirmed(TypedValue::Text(value)) if setting.kind == FieldKind::Text => {
                SettingValue::Text(value)
            }
            InputResult::Confirmed(TypedValue::Numeric(value))
                if setting.kind == FieldKind::Integer =>
            {
                SettingValue::Integer(
                    i64::try_from(value)
                        .map_err(|_| UiError::InvalidValue(setting_id.to_owned()))?,
                )
            }
            InputResult::Confirmed(TypedValue::Numeric(value))
                if setting.kind == FieldKind::Decimal =>
            {
                SettingValue::Decimal(value as f64)
            }
            InputResult::Confirmed(TypedValue::Secret(value))
                if setting.kind == FieldKind::Secret =>
            {
                self.secret_lengths
                    .insert(setting_id.to_owned(), value.chars().count());
                self.keyboard = None;
                return Ok(());
            }
            InputResult::Rejected(_) => {
                return self.reject(setting_id, UiError::InvalidValue(setting_id.to_owned()))
            }
            InputResult::Cancelled => {
                self.keyboard = None;
                return Ok(());
            }
            _ => return self.reject(setting_id, UiError::Keyboard(setting_id.to_owned())),
        };
        self.keyboard = None;
        self.set_value(setting_id, value)
    }

    pub fn confirm(&mut self) -> Result<(), UiError> {
        self.apply_internal(EventKind::Confirmed)
    }

    pub fn apply(&mut self) -> Result<(), UiError> {
        self.apply_internal(EventKind::Applied)
    }

    pub fn cancel(&mut self) {
        self.pending.clear();
        self.errors.clear();
        self.events.push(UiEvent {
            kind: EventKind::Cancelled,
            setting_id: None,
            value: None,
            message: None,
        });
    }

    pub fn back(&mut self) {
        if self.surface == Surface::Form {
            self.surface = Surface::SectionList;
            self.row_index = 0;
            self.events.push(UiEvent {
                kind: EventKind::Back,
                setting_id: None,
                value: None,
                message: None,
            });
        }
    }

    pub fn drain_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.events)
    }

    fn apply_internal(&mut self, event_kind: EventKind) -> Result<(), UiError> {
        if let Some((setting_id, _)) = self.errors.iter().next() {
            return Err(UiError::InvalidValue(setting_id.clone()));
        }
        let changes: Vec<_> = self
            .pending
            .iter()
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        for (setting_id, value) in changes {
            self.committed.insert(setting_id, value);
        }
        self.pending.clear();
        self.events.push(UiEvent {
            kind: event_kind,
            setting_id: None,
            value: None,
            message: None,
        });
        Ok(())
    }

    fn activate_selected(&mut self) -> Result<(), UiError> {
        let Some(control) = self.selected_control()? else {
            if self.surface == Surface::SectionList {
                self.surface = Surface::Form;
                self.row_index = 0;
                self.events.push(UiEvent {
                    kind: EventKind::Navigation,
                    setting_id: None,
                    value: None,
                    message: None,
                });
            }
            return Ok(());
        };
        if !control.enabled {
            return self.reject(
                &control.setting_id,
                UiError::Disabled(control.setting_id.clone()),
            );
        }
        match control.kind {
            FieldKind::Boolean => {
                let value = match self.value_for(&control.setting_id) {
                    Some(SettingValue::Boolean(value)) => !value,
                    _ => true,
                };
                self.set_value(&control.setting_id, SettingValue::Boolean(value))
            }
            FieldKind::EnumSingle | FieldKind::EnumMulti => self.change_selected(1),
            FieldKind::Text | FieldKind::Secret | FieldKind::Integer | FieldKind::Decimal => {
                self.open_keyboard(&control.setting_id).map(|_| ())
            }
            FieldKind::Action => {
                self.external_operations.push(ExternalOperationRequest {
                    setting_id: control.setting_id.clone(),
                    operation: control.apply,
                });
                self.events.push(UiEvent {
                    kind: EventKind::ExternalOperationRequested,
                    setting_id: Some(control.setting_id),
                    value: None,
                    message: None,
                });
                Ok(())
            }
            FieldKind::ReadOnly | FieldKind::Status => Ok(()),
        }
    }

    fn change_selected(&mut self, direction: i32) -> Result<(), UiError> {
        if self.surface == Surface::SectionList {
            self.move_section(direction);
            return Ok(());
        }
        let Some(control) = self.selected_control()? else {
            return Ok(());
        };
        if !control.enabled {
            return self.reject(
                &control.setting_id,
                UiError::Disabled(control.setting_id.clone()),
            );
        }
        match control.kind {
            FieldKind::Boolean => {
                let value = match self.value_for(&control.setting_id) {
                    Some(SettingValue::Boolean(value)) => !value,
                    _ => true,
                };
                self.set_value(&control.setting_id, SettingValue::Boolean(value))
            }
            FieldKind::EnumSingle => self.select_enum(&control, direction),
            FieldKind::EnumMulti => self.toggle_enum(&control, direction),
            FieldKind::Integer | FieldKind::Decimal => self.step_numeric(&control, direction),
            _ => Ok(()),
        }
    }

    fn select_enum(&mut self, control: &FormControl, direction: i32) -> Result<(), UiError> {
        let options = control
            .constraints
            .as_ref()
            .map(|constraints| &constraints.options)
            .ok_or_else(|| UiError::InvalidValue(control.setting_id.clone()))?;
        let current = match self.value_for(&control.setting_id) {
            Some(SettingValue::EnumSingle(value)) => value,
            _ => return Err(UiError::InvalidValue(control.setting_id.clone())),
        };
        let index = options
            .iter()
            .position(|option| option.value == current)
            .ok_or_else(|| UiError::InvalidValue(control.setting_id.clone()))?;
        let next = (index as i32 + direction).rem_euclid(options.len() as i32) as usize;
        self.set_value(
            &control.setting_id,
            SettingValue::EnumSingle(options[next].value.clone()),
        )
    }

    fn toggle_enum(&mut self, control: &FormControl, direction: i32) -> Result<(), UiError> {
        let options = control
            .constraints
            .as_ref()
            .map(|constraints| &constraints.options)
            .ok_or_else(|| UiError::InvalidValue(control.setting_id.clone()))?;
        let mut values = match self.value_for(&control.setting_id) {
            Some(SettingValue::EnumMulti(values)) => values,
            _ => Vec::new(),
        };
        let index = if direction < 0 { options.len() - 1 } else { 0 };
        let value = options[index].value.clone();
        if let Some(position) = values.iter().position(|item| item == &value) {
            values.remove(position);
        } else {
            values.push(value);
        }
        self.set_value(&control.setting_id, SettingValue::EnumMulti(values))
    }

    fn step_numeric(&mut self, control: &FormControl, direction: i32) -> Result<(), UiError> {
        let step = control
            .constraints
            .as_ref()
            .and_then(|constraints| constraints.range.as_ref())
            .map_or(1.0, |range| range.step);
        let value = match self.value_for(&control.setting_id) {
            Some(SettingValue::Integer(value)) => SettingValue::Integer(
                value.saturating_add((step as i64).saturating_mul(direction as i64)),
            ),
            Some(SettingValue::Decimal(value)) => {
                SettingValue::Decimal(value + step * direction as f64)
            }
            _ => return Err(UiError::InvalidValue(control.setting_id.clone())),
        };
        self.set_value(&control.setting_id, value)
    }

    fn move_row(&mut self, direction: i32) {
        if self.surface == Surface::SectionList {
            self.move_section(direction);
            return;
        }
        if let Ok(Some(control_count)) = self.selected_control_count() {
            self.row_index =
                (self.row_index as i32 + direction).rem_euclid(control_count as i32) as usize;
        }
        self.events.push(UiEvent {
            kind: EventKind::Navigation,
            setting_id: None,
            value: None,
            message: None,
        });
    }

    fn move_section(&mut self, direction: i32) {
        let section_count = self.scene().map_or(0, |scene| scene.sections.len());
        if section_count > 0 {
            self.section_index =
                (self.section_index as i32 + direction).rem_euclid(section_count as i32) as usize;
        }
        self.events.push(UiEvent {
            kind: EventKind::Navigation,
            setting_id: None,
            value: None,
            message: None,
        });
    }

    fn selected_control_count(&self) -> Result<Option<usize>, UiError> {
        let scene = self.scene()?;
        Ok(scene
            .sections
            .get(self.section_index)
            .map(|section| flattened_controls(section).len()))
    }

    fn selected_control(&self) -> Result<Option<FormControl>, UiError> {
        if self.surface != Surface::Form {
            return Ok(None);
        }
        let projection = self
            .registry
            .project(&self.projection_context())
            .map_err(|error| UiError::Registry(error.to_string()))?;
        Ok(projection
            .sections
            .get(self.section_index)
            .and_then(|section| {
                flattened_schema_controls(section)
                    .get(self.row_index)
                    .cloned()
            })
            .cloned())
    }

    fn is_enabled(&self, setting_id: &str) -> Result<bool, UiError> {
        Ok(self
            .registry
            .project(&self.projection_context())
            .map_err(|error| UiError::Registry(error.to_string()))?
            .sections
            .iter()
            .flat_map(|section| section.groups.iter())
            .flat_map(|group| group.controls.iter())
            .find(|control| control.setting_id == setting_id)
            .is_some_and(|control| control.enabled))
    }

    fn projection_context(&self) -> ProjectionContext {
        let mut values = self
            .committed
            .iter()
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        values.extend(
            self.pending
                .iter()
                .map(|(id, value)| (id.clone(), value.clone())),
        );
        ProjectionContext {
            values,
            capabilities: self.capabilities.clone(),
        }
    }

    fn value_for(&self, setting_id: &str) -> Option<SettingValue> {
        self.pending
            .get(setting_id)
            .cloned()
            .or_else(|| self.committed.get(setting_id).cloned())
    }

    fn control(&self, control: &FormControl) -> SettingControl {
        let value = self.value_for(&control.setting_id);
        SettingControl {
            setting_id: control.setting_id.clone(),
            kind: control.kind,
            label_key: control.label_key.clone(),
            description_key: control.description_key.clone(),
            value: match value.as_ref() {
                Some(value) => semantic_value(value, control.redacted),
                None if control.redacted => masked_value(&self.secret_lengths, &control.setting_id),
                None => SemanticValue::Empty,
            },
            pending: self
                .pending
                .get(&control.setting_id)
                .map(|value| semantic_value(value, control.redacted)),
            constraints: control.constraints.clone(),
            units: control.units.clone(),
            display: control.display.clone(),
            scope: control.scope,
            enabled: control.enabled,
            disabled_reason: if control.enabled {
                None
            } else {
                control
                    .unsupported_reason
                    .clone()
                    .or_else(|| Some("disabled by setting predicate".to_owned()))
            },
            apply: control.apply,
            badges: badges(control.apply),
            redacted: control.redacted,
        }
    }

    fn reject(&mut self, setting_id: &str, error: UiError) -> Result<(), UiError> {
        let message = match &error {
            UiError::InvalidValue(_) => "value failed descriptor validation",
            UiError::Disabled(_) => "control is unavailable",
            UiError::NotEditable(_) => "control is read-only",
            UiError::Keyboard(_) => "use the keyboard boundary for this control",
            _ => "setting action rejected",
        };
        self.errors
            .insert(setting_id.to_owned(), message.to_owned());
        self.events.push(UiEvent {
            kind: EventKind::ValidationFailed,
            setting_id: Some(setting_id.to_owned()),
            value: None,
            message: Some(message.to_owned()),
        });
        Err(error)
    }
}

fn controller_help() -> ControllerHelpStrip {
    ControllerHelpStrip {
        bindings: vec![
            HelpBinding {
                button: UiButton::Primary,
                label: "Select / edit".into(),
                action: Some(ui_model::SemanticAction::Primary),
            },
            HelpBinding {
                button: UiButton::Secondary,
                label: "Back".into(),
                action: Some(ui_model::SemanticAction::Secondary),
            },
            HelpBinding {
                button: UiButton::Start,
                label: "Apply".into(),
                action: Some(ui_model::SemanticAction::Start),
            },
            HelpBinding {
                button: UiButton::Menu,
                label: "Cancel".into(),
                action: Some(ui_model::SemanticAction::Select),
            },
        ],
    }
}

fn flattened_controls(section: &SectionScene) -> Vec<&SettingControl> {
    section
        .groups
        .iter()
        .flat_map(|group| group.controls.iter())
        .collect()
}

fn flattened_schema_controls(section: &settings_schema::MenuSection) -> Vec<&FormControl> {
    section
        .groups
        .iter()
        .flat_map(|group| group.controls.iter())
        .collect()
}

fn keyboard_field(kind: virtual_keyboard::FieldKind) -> KeyboardField {
    match kind {
        virtual_keyboard::FieldKind::Text => KeyboardField::Text,
        virtual_keyboard::FieldKind::Secret => KeyboardField::Secret,
        virtual_keyboard::FieldKind::Numeric => KeyboardField::Numeric,
    }
}

fn badges(apply: ApplyMode) -> Vec<ApplyBadge> {
    match apply {
        ApplyMode::RestartLauncher => vec![ApplyBadge::RestartLauncher],
        ApplyMode::RebootCandidate => vec![ApplyBadge::RebootCandidate],
        _ => Vec::new(),
    }
}

fn masked_value(lengths: &BTreeMap<String, usize>, setting_id: &str) -> SemanticValue {
    SemanticValue::Masked {
        length_scalars: lengths.get(setting_id).copied().unwrap_or(0),
    }
}

fn semantic_value(value: &SettingValue, redacted: bool) -> SemanticValue {
    if redacted {
        return SemanticValue::Masked { length_scalars: 0 };
    }
    match value {
        SettingValue::Boolean(value) => SemanticValue::Boolean(*value),
        SettingValue::Integer(value) => SemanticValue::Integer(*value),
        SettingValue::Decimal(value) => SemanticValue::Decimal(*value),
        SettingValue::Text(value) => SemanticValue::Text(value.clone()),
        SettingValue::Secret(_) => SemanticValue::Masked { length_scalars: 0 },
        SettingValue::EnumSingle(value) => SemanticValue::EnumSingle(value.clone()),
        SettingValue::EnumMulti(value) => SemanticValue::EnumMulti(value.clone()),
    }
}

fn matches_kind(value: &SettingValue, kind: FieldKind) -> bool {
    matches!(
        (value, kind),
        (SettingValue::Boolean(_), FieldKind::Boolean)
            | (SettingValue::Integer(_), FieldKind::Integer)
            | (SettingValue::Decimal(_), FieldKind::Decimal)
            | (SettingValue::Text(_), FieldKind::Text)
            | (SettingValue::EnumSingle(_), FieldKind::EnumSingle)
            | (SettingValue::EnumMulti(_), FieldKind::EnumMulti)
    )
}

fn valid_value(setting: &settings_schema::SettingDescriptor, value: &SettingValue) -> bool {
    let Some(constraints) = &setting.constraints else {
        return true;
    };
    if let Some(range) = &constraints.range {
        let number = match value {
            SettingValue::Integer(value) => *value as f64,
            SettingValue::Decimal(value) => *value,
            _ => return false,
        };
        let quotient = (number - range.min) / range.step;
        if !number.is_finite()
            || number < range.min
            || number > range.max
            || (quotient - quotient.round()).abs() > 1e-9
        {
            return false;
        }
    }
    if let Some(text) = &constraints.text {
        let SettingValue::Text(value) = value else {
            return false;
        };
        if value.chars().count() < text.min_length
            || value.chars().count() > text.max_length
            || (value.is_empty()
                && (setting.validation.required || !setting.validation.allow_empty))
            || (setting.validation.trim && value.trim() != value)
            || text
                .pattern
                .as_ref()
                .is_some_and(|pattern| match Regex::new(pattern) {
                    Ok(regex) => !regex.is_match(value),
                    Err(_) => true,
                })
        {
            return false;
        }
    }
    if !constraints.options.is_empty() {
        match value {
            SettingValue::EnumSingle(value) => {
                if !constraints
                    .options
                    .iter()
                    .any(|option| option.value == *value)
                {
                    return false;
                }
            }
            SettingValue::EnumMulti(values) => {
                if values.iter().any(|value| {
                    !constraints
                        .options
                        .iter()
                        .any(|option| option.value == *value)
                }) || values
                    .iter()
                    .enumerate()
                    .any(|(index, value)| values[..index].contains(value))
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn text_limits(setting: &settings_schema::SettingDescriptor) -> (usize, usize) {
    setting
        .constraints
        .as_ref()
        .and_then(|constraints| constraints.text.as_ref())
        .map_or((4096, 4096), |text| {
            (text.max_length.saturating_mul(4), text.max_length)
        })
}

fn text_value(value: Option<SettingValue>) -> String {
    match value {
        Some(SettingValue::Text(value)) => value,
        _ => String::new(),
    }
}

fn numeric_text(value: Option<SettingValue>, kind: FieldKind) -> String {
    match value {
        Some(SettingValue::Integer(value)) => value.to_string(),
        Some(SettingValue::Decimal(value))
            if kind == FieldKind::Decimal && value.fract() == 0.0 =>
        {
            value.to_string()
        }
        _ => String::new(),
    }
}
