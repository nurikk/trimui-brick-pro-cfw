use std::{collections::HashSet, fmt};

use serde::{de, Deserialize, Deserializer, Serialize};
use virtual_keyboard::{AllowedChars, FieldPolicy, InputResult, Keyboard, TypedValue};
use wifi_manager::{
    ConnectRequest, GeneratedWifiBackend, KeyboardField as WifiKeyboardField, ManualNetworkRequest,
    NetworkId, ReasonCode, ReconnectConditions, SavedState, ScanRequest, Security, WifiEvent,
    WifiManager, WifiPhase,
};

pub const SCHEMA: &str = "https://json-schema.org/draft/2020-12/schema";
pub const FORMAT: &str = "trimui-wifi-settings-controller";
pub const SCENE_WIDTH: u16 = 1024;
pub const SCENE_HEIGHT: u16 = 768;
pub const MAX_METADATA_BYTES: usize = 128 * 1024;
pub const MAX_CONTROLS: usize = 32;
pub const MAX_ACTIONS: usize = 32;
pub const MAX_MENU_ITEMS: usize = MAX_CONTROLS + MAX_ACTIONS;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub capability: String,
    pub controls: Vec<Control>,
    pub actions: Vec<Action>,
    #[serde(rename = "securityChoices")]
    pub security_choices: Vec<Security>,
    #[serde(rename = "manualSsid")]
    pub manual_ssid: InputPolicy,
    #[serde(rename = "networkKey")]
    pub network_key: InputPolicy,
    pub snapshot: SnapshotProjection,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub id: String,
    pub order: u16,
    pub label: String,
    pub help: String,
    pub operation: ControlOperation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlOperation {
    ToggleEnabled,
    ToggleAutomaticReconnect,
    Scan,
    OpenManual,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub order: u16,
    pub label: String,
    pub help: String,
    pub operation: ActionOperation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionOperation {
    Connect,
    Disconnect,
    Forget,
    Retry,
    Cancel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputPolicy {
    #[serde(rename = "maxBytes")]
    pub max_bytes: usize,
    #[serde(rename = "maxScalars")]
    pub max_scalars: usize,
    pub allowed: AllowedInput,
    pub placeholder: String,
    pub hidden: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AllowedInput {
    Any,
    Ascii,
    UrlSafe,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotProjection {
    pub fields: Vec<SnapshotField>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotField {
    Canvas,
    Menu,
    Networks,
    Phase,
    Selection,
    Saved,
    Keyboard,
    Confirmation,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataError(String);

impl fmt::Display for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for MetadataError {}

impl Metadata {
    pub fn from_json(bytes: &[u8]) -> Result<Self, MetadataError> {
        if bytes.len() > MAX_METADATA_BYTES {
            return Err(invalid("metadata exceeds byte budget"));
        }
        reject_duplicate_keys(bytes)?;
        let metadata: Self = serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("invalid metadata JSON: {error}")))?;
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn validate(&self) -> Result<(), MetadataError> {
        if self.schema != SCHEMA {
            return Err(invalid("unsupported metadata schema"));
        }
        if self.format != FORMAT || self.schema_version != 1 {
            return Err(invalid("unsupported metadata format or version"));
        }
        if self.capability != "wifi" {
            return Err(invalid("metadata capability must be wifi"));
        }
        if self.controls.is_empty() || self.controls.len() > MAX_CONTROLS {
            return Err(invalid("invalid control count"));
        }
        if self.actions.is_empty() || self.actions.len() > MAX_ACTIONS {
            return Err(invalid("invalid action count"));
        }
        let mut ids = HashSet::new();
        let mut orders = HashSet::new();
        for control in &self.controls {
            validate_id(&control.id, "control id")?;
            validate_text(&control.label, "control label", 128)?;
            validate_text(&control.help, "control help", 256)?;
            if !ids.insert(control.id.as_str()) || !orders.insert(control.order) {
                return Err(invalid("duplicate menu id or order"));
            }
        }
        for action in &self.actions {
            validate_id(&action.id, "action id")?;
            validate_text(&action.label, "action label", 128)?;
            validate_text(&action.help, "action help", 256)?;
            if !ids.insert(action.id.as_str()) || !orders.insert(action.order) {
                return Err(invalid("duplicate menu id or order"));
            }
        }
        require_control(&self.controls, ControlOperation::ToggleEnabled)?;
        require_control(&self.controls, ControlOperation::ToggleAutomaticReconnect)?;
        require_control(&self.controls, ControlOperation::Scan)?;
        require_control(&self.controls, ControlOperation::OpenManual)?;
        for operation in [
            ActionOperation::Connect,
            ActionOperation::Disconnect,
            ActionOperation::Forget,
            ActionOperation::Retry,
            ActionOperation::Cancel,
        ] {
            if self
                .actions
                .iter()
                .filter(|action| action.operation == operation)
                .count()
                != 1
            {
                return Err(invalid("metadata must declare each workflow action once"));
            }
        }
        if self.security_choices.is_empty()
            || self.security_choices.len() > 3
            || self.security_choices.iter().any(|security| {
                !matches!(
                    security,
                    Security::Open | Security::Wpa2Psk | Security::Wpa3Sae
                )
            })
        {
            return Err(invalid("unsupported or invalid security choice"));
        }
        for (index, security) in self.security_choices.iter().enumerate() {
            if self.security_choices[..index].contains(security) {
                return Err(invalid("duplicate security choice"));
            }
        }
        validate_policy(&self.manual_ssid, true)?;
        validate_policy(&self.network_key, false)?;
        if !self.manual_ssid.hidden || self.manual_ssid.max_bytes > wifi_manager::MAX_SSID_BYTES {
            return Err(invalid("manual SSID policy must be bounded and hidden"));
        }
        if self.network_key.hidden
            || self.network_key.max_bytes == 0
            || self.network_key.max_bytes > 63
        {
            return Err(invalid("network key policy is outside bounds"));
        }
        let mut fields = HashSet::new();
        for field in &self.snapshot.fields {
            if !fields.insert(*field) {
                return Err(invalid("duplicate snapshot projection field"));
            }
        }
        if fields.len() != 9 {
            return Err(invalid("snapshot projection is incomplete"));
        }
        Ok(())
    }

    pub fn menu(&self) -> Vec<MenuItem> {
        let mut menu = self
            .controls
            .iter()
            .map(|control| MenuItem {
                id: control.id.clone(),
                order: control.order,
                label: control.label.clone(),
                help: control.help.clone(),
                control: Some(control.operation),
                action: None,
            })
            .chain(self.actions.iter().map(|action| MenuItem {
                id: action.id.clone(),
                order: action.order,
                label: action.label.clone(),
                help: action.help.clone(),
                control: None,
                action: Some(action.operation),
            }))
            .collect::<Vec<_>>();
        menu.sort_by_key(|item| (item.order, item.id.clone()));
        menu
    }
}

fn require_control(controls: &[Control], operation: ControlOperation) -> Result<(), MetadataError> {
    if controls
        .iter()
        .filter(|control| control.operation == operation)
        .count()
        == 1
    {
        Ok(())
    } else {
        Err(invalid("metadata must declare each workflow control once"))
    }
}

fn validate_policy(policy: &InputPolicy, text: bool) -> Result<(), MetadataError> {
    if policy.max_bytes == 0
        || policy.max_scalars == 0
        || policy.max_bytes > 4096
        || policy.max_scalars > 4096
    {
        return Err(invalid("input policy exceeds bounds"));
    }
    validate_text(&policy.placeholder, "input placeholder", 128)?;
    if text && policy.allowed == AllowedInput::UrlSafe {
        return Err(invalid("text input cannot be URL-only"));
    }
    Ok(())
}

fn validate_id(value: &str, label: &str) -> Result<(), MetadataError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || value.starts_with('-')
        || value.ends_with('-')
    {
        return Err(invalid(format!("invalid {label}")));
    }
    Ok(())
}

fn validate_text(value: &str, label: &str, max_bytes: usize) -> Result<(), MetadataError> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(invalid(format!("invalid {label}")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> MetadataError {
    MetadataError(message.into())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    pub id: String,
    pub order: u16,
    pub label: String,
    pub help: String,
    pub control: Option<ControlOperation>,
    pub action: Option<ActionOperation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    Menu,
    Networks,
    Keyboard,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSnapshot {
    pub display_ssid: String,
    pub security: Security,
    pub signal_quality: u8,
    pub known: bool,
    pub connected: bool,
    pub selected: bool,
    pub priority: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardRequest {
    pub field: WifiKeyboardField,
    pub masked: bool,
    pub length_scalars: usize,
    pub max_bytes: usize,
    pub max_scalars: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub width: u16,
    pub height: u16,
    pub view: View,
    pub menu: Vec<MenuItemSnapshot>,
    pub networks: Vec<NetworkSnapshot>,
    pub phase: WifiPhase,
    pub reason: Option<ReasonCode>,
    pub selected_network: Option<NetworkSnapshot>,
    pub saved_network_count: usize,
    pub selected_saved: bool,
    pub security_choices: Vec<Security>,
    pub keyboard: Option<KeyboardRequest>,
    pub open_confirmation: bool,
    pub retry_after_ms: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MenuItemSnapshot {
    pub id: String,
    pub label: String,
    pub help: String,
    pub focused: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "event")]
pub enum ControllerEvent {
    Navigation {
        focused_item: String,
    },
    PhaseChanged {
        phase: WifiPhase,
        reason: Option<ReasonCode>,
    },
    ScanProgress,
    ScanCompleted {
        count: usize,
    },
    SelectionChanged,
    KeyboardRequested(KeyboardRequest),
    ConnectionChanged {
        connected: bool,
    },
    Error {
        reason: ReasonCode,
    },
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControllerError {
    Metadata(String),
    CapabilityUnavailable,
    Manager(ReasonCode),
    Keyboard,
    InvalidInput(ReasonCode),
}

impl fmt::Display for ControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Metadata(message) => {
                write!(formatter, "invalid Wi-Fi controller metadata: {message}")
            }
            Self::CapabilityUnavailable => formatter.write_str("Wi-Fi capability is unavailable"),
            Self::Manager(reason) | Self::InvalidInput(reason) => {
                write!(formatter, "Wi-Fi operation failed: {reason:?}")
            }
            Self::Keyboard => formatter.write_str("keyboard session unavailable"),
        }
    }
}
impl std::error::Error for ControllerError {}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case", tag = "action")]
pub enum ControllerAction {
    Press(virtual_keyboard::Button),
    SetSecurity { security: Security },
    SetPriority { priority: u8 },
    SelectNetwork { network_id: NetworkId },
    OpenManual,
    Connect,
    ConfirmOpen,
    Disconnect,
    Forget,
    Retry,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyboardPurpose {
    ManualSsid,
    NetworkKey,
}

pub struct WifiSettingsController {
    metadata: Metadata,
    manager: WifiManager<GeneratedWifiBackend>,
    capability_available: bool,
    menu: Vec<MenuItem>,
    focus: usize,
    view: View,
    network_index: usize,
    selected_network_id: Option<NetworkId>,
    selected_security: Security,
    keyboard: Option<Keyboard>,
    keyboard_purpose: Option<KeyboardPurpose>,
    open_confirmation: bool,
    events: Vec<ControllerEvent>,
}

impl WifiSettingsController {
    pub fn new(
        metadata: Metadata,
        manager: WifiManager<GeneratedWifiBackend>,
        capability_available: bool,
    ) -> Result<Self, ControllerError> {
        metadata
            .validate()
            .map_err(|error| ControllerError::Metadata(error.to_string()))?;
        let menu = metadata.menu();
        let selected_security = metadata.security_choices[0];
        Ok(Self {
            metadata,
            manager,
            capability_available,
            menu,
            focus: 0,
            view: View::Menu,
            network_index: 0,
            selected_network_id: None,
            selected_security,
            keyboard: None,
            keyboard_purpose: None,
            open_confirmation: false,
            events: Vec::new(),
        })
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn saved_state(&self) -> SavedState {
        self.manager.saved_state()
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = self.manager.state();
        let saved = self.manager.saved_state();
        let networks = state
            .scan_results
            .iter()
            .map(|entry| NetworkSnapshot {
                display_ssid: entry.display_ssid.clone(),
                security: entry.security,
                signal_quality: entry.signal_quality,
                known: entry.known,
                connected: entry.connected,
                selected: self.selected_network_id.as_ref() == Some(&entry.network_id),
                priority: saved
                    .networks
                    .iter()
                    .find(|record| record.network_id == entry.network_id)
                    .map(|record| record.priority),
            })
            .collect::<Vec<_>>();
        let selected_network = state
            .scan_results
            .iter()
            .find(|entry| self.selected_network_id.as_ref() == Some(&entry.network_id))
            .map(|entry| NetworkSnapshot {
                display_ssid: entry.display_ssid.clone(),
                security: entry.security,
                signal_quality: entry.signal_quality,
                known: entry.known,
                connected: entry.connected,
                selected: true,
                priority: saved
                    .networks
                    .iter()
                    .find(|record| record.network_id == entry.network_id)
                    .map(|record| record.priority),
            });
        Snapshot {
            width: SCENE_WIDTH,
            height: SCENE_HEIGHT,
            view: self.view,
            menu: self
                .menu
                .iter()
                .enumerate()
                .map(|(index, item)| MenuItemSnapshot {
                    id: item.id.clone(),
                    label: item.label.clone(),
                    help: item.help.clone(),
                    focused: self.view == View::Menu && index == self.focus,
                })
                .collect(),
            networks,
            phase: state.phase,
            reason: state.reason,
            selected_network,
            saved_network_count: saved.networks.len(),
            selected_saved: self
                .selected_network_id
                .as_ref()
                .is_some_and(|id| saved.networks.iter().any(|record| &record.network_id == id)),
            security_choices: self.metadata.security_choices.clone(),
            keyboard: self.keyboard_request(),
            open_confirmation: self.open_confirmation,
            retry_after_ms: state.retry_after_ms,
        }
    }

    pub fn keyboard_request(&self) -> Option<KeyboardRequest> {
        let keyboard = self.keyboard.as_ref()?;
        let semantic = keyboard.semantic_snapshot();
        let (field, policy) = match self.keyboard_purpose? {
            KeyboardPurpose::ManualSsid => (WifiKeyboardField::Ssid, &self.metadata.manual_ssid),
            KeyboardPurpose::NetworkKey => {
                (WifiKeyboardField::Password, &self.metadata.network_key)
            }
        };
        Some(KeyboardRequest {
            field,
            masked: semantic.field == virtual_keyboard::FieldKind::Secret,
            length_scalars: semantic.length_scalars,
            max_bytes: policy.max_bytes,
            max_scalars: policy.max_scalars,
        })
    }

    pub fn press(&mut self, button: virtual_keyboard::Button) -> Result<(), ControllerError> {
        if self.keyboard.is_some() {
            return self.press_keyboard(button);
        }
        if self.view == View::Networks {
            return self.press_networks(button);
        }
        match button {
            virtual_keyboard::Button::Up => self.move_focus(-1),
            virtual_keyboard::Button::Down => self.move_focus(1),
            virtual_keyboard::Button::Secondary => self.view = View::Menu,
            virtual_keyboard::Button::Menu => self.cancel_operation()?,
            virtual_keyboard::Button::Primary => self.activate_menu()?,
            _ => {}
        }
        Ok(())
    }

    pub fn set_security(&mut self, security: Security) -> Result<(), ControllerError> {
        if !self.metadata.security_choices.contains(&security) {
            return Err(ControllerError::InvalidInput(
                ReasonCode::UnsupportedSecurity,
            ));
        }
        self.selected_security = security;
        Ok(())
    }

    pub fn set_priority(&mut self, priority: u8) -> Result<(), ControllerError> {
        let network_id = self.selected_id()?;
        self.manager
            .set_profile_priority(&network_id, priority)
            .map_err(|error| self.manager_error(error.0))
    }

    pub fn scan(&mut self) -> Result<(), ControllerError> {
        self.require_capability()?;
        self.manager
            .scan(ScanRequest { rescan: false })
            .map_err(|error| self.manager_error(error.0))?;
        self.sync_manager_events();
        self.network_index = 0;
        self.view = View::Networks;
        Ok(())
    }

    pub fn open_manual(&mut self) -> Result<(), ControllerError> {
        self.require_capability()?;
        self.manual_keyboard()?;
        self.view = View::Keyboard;
        Ok(())
    }

    pub fn select_network(&mut self, network_id: NetworkId) -> Result<(), ControllerError> {
        self.manager
            .select(&network_id)
            .map_err(|error| ControllerError::Manager(error.0))?;
        self.selected_network_id = Some(network_id);
        self.open_confirmation = false;
        self.sync_manager_events();
        if self.manager.password_keyboard_request().is_some() {
            self.network_key_keyboard()?;
            self.view = View::Keyboard;
        } else {
            self.view = View::Menu;
        }
        Ok(())
    }

    pub fn connect(&mut self) -> Result<(), ControllerError> {
        self.connect_action()
    }

    pub fn confirm_open(&mut self) -> Result<(), ControllerError> {
        if !self.open_confirmation {
            return Err(ControllerError::InvalidInput(
                ReasonCode::ConfirmationRequired,
            ));
        }
        self.connect_action()
    }

    pub fn dispatch(&mut self, action: ControllerAction) -> Result<(), ControllerError> {
        match action {
            ControllerAction::Press(button) => self.press(button),
            ControllerAction::SetSecurity { security } => self.set_security(security),
            ControllerAction::SetPriority { priority } => self.set_priority(priority),
            ControllerAction::SelectNetwork { network_id } => self.select_network(network_id),
            ControllerAction::OpenManual => self.open_manual(),
            ControllerAction::Connect => self.connect(),
            ControllerAction::ConfirmOpen => self.confirm_open(),
            ControllerAction::Disconnect => self.disconnect(),
            ControllerAction::Forget => self.forget(),
            ControllerAction::Retry => self.retry(),
            ControllerAction::Cancel => self.cancel(),
        }
    }

    pub fn disconnect(&mut self) -> Result<(), ControllerError> {
        self.require_capability()?;
        self.manager
            .disconnect()
            .map_err(|error| self.manager_error(error.0))?;
        self.sync_manager_events();
        Ok(())
    }

    pub fn forget(&mut self) -> Result<(), ControllerError> {
        self.require_capability()?;
        let network_id = self.selected_id()?;
        self.manager
            .forget(&network_id)
            .map_err(|error| self.manager_error(error.0))?;
        self.sync_manager_events();
        Ok(())
    }

    pub fn retry(&mut self) -> Result<(), ControllerError> {
        self.require_capability()?;
        self.manager
            .retry()
            .map_err(|error| self.manager_error(error.0))?;
        self.sync_manager_events();
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), ControllerError> {
        self.cancel_operation()
    }

    pub fn manager_state(&self) -> &wifi_manager::WifiState {
        self.manager.state()
    }

    pub fn auto_reconnect(
        &mut self,
        conditions: ReconnectConditions,
    ) -> wifi_manager::AutoReconnectDecision {
        let decision = self.manager.auto_reconnect(conditions);
        self.sync_manager_events();
        decision
    }

    pub fn drain_events(&mut self) -> Vec<ControllerEvent> {
        self.sync_manager_events();
        std::mem::take(&mut self.events)
    }

    fn require_capability(&self) -> Result<(), ControllerError> {
        self.capability_available
            .then_some(())
            .ok_or(ControllerError::CapabilityUnavailable)
    }

    fn move_focus(&mut self, direction: i32) {
        self.focus = (self.focus as i32 + direction).rem_euclid(self.menu.len() as i32) as usize;
        self.events.push(ControllerEvent::Navigation {
            focused_item: self.menu[self.focus].id.clone(),
        });
    }

    fn press_networks(&mut self, button: virtual_keyboard::Button) -> Result<(), ControllerError> {
        let count = self.manager.state().scan_results.len();
        if count == 0 {
            self.view = View::Menu;
            return Ok(());
        }
        match button {
            virtual_keyboard::Button::Up => {
                self.network_index = (self.network_index + count - 1) % count;
            }
            virtual_keyboard::Button::Down => self.network_index = (self.network_index + 1) % count,
            virtual_keyboard::Button::Primary => {
                let network_id = self.manager.state().scan_results[self.network_index]
                    .network_id
                    .clone();
                self.select_network(network_id)?;
            }
            virtual_keyboard::Button::Secondary => self.view = View::Menu,
            _ => {}
        }
        Ok(())
    }

    fn activate_menu(&mut self) -> Result<(), ControllerError> {
        let item = self.menu[self.focus].clone();
        if let Some(operation) = item.control {
            match operation {
                ControlOperation::ToggleEnabled => {
                    let enabled = !self.manager.state().enabled;
                    self.require_capability()?;
                    self.manager
                        .set_enabled(enabled)
                        .map_err(|error| self.manager_error(error.0))?;
                    self.sync_manager_events();
                }
                ControlOperation::ToggleAutomaticReconnect => {
                    self.manager
                        .set_automatic_reconnect(!self.manager.state().automatic_reconnect);
                }
                ControlOperation::Scan => self.scan()?,
                ControlOperation::OpenManual => self.open_manual()?,
            }
        }
        if let Some(operation) = item.action {
            match operation {
                ActionOperation::Connect => self.connect_action()?,
                ActionOperation::Disconnect => self.disconnect()?,
                ActionOperation::Forget => self.forget()?,
                ActionOperation::Retry => self.retry()?,
                ActionOperation::Cancel => self.cancel_operation()?,
            }
        }
        Ok(())
    }

    fn connect_action(&mut self) -> Result<(), ControllerError> {
        let network_id = self.selected_id()?;
        let security = self.selected_security_for(&network_id)?;
        if security == Security::Open {
            let request = ConnectRequest {
                network_id,
                open_confirmation: self.open_confirmation,
                credential_reference: None,
            };
            match self.manager.connect(request) {
                Ok(()) => {
                    self.open_confirmation = false;
                    self.sync_manager_events();
                    Ok(())
                }
                Err(error) if error.0 == ReasonCode::ConfirmationRequired => {
                    self.open_confirmation = true;
                    self.sync_manager_events();
                    Err(ControllerError::Manager(error.0))
                }
                Err(error) => Err(self.manager_error(error.0)),
            }
        } else {
            let result = self.manager.connect(ConnectRequest {
                network_id,
                open_confirmation: false,
                credential_reference: None,
            });
            match result {
                Ok(()) => {
                    self.sync_manager_events();
                    Ok(())
                }
                Err(error) => Err(self.manager_error(error.0)),
            }
        }
    }

    fn selected_id(&self) -> Result<NetworkId, ControllerError> {
        self.selected_network_id
            .clone()
            .ok_or(ControllerError::Manager(ReasonCode::NotFound))
    }

    fn selected_security_for(&self, network_id: &NetworkId) -> Result<Security, ControllerError> {
        self.manager
            .state()
            .scan_results
            .iter()
            .find(|entry| &entry.network_id == network_id)
            .map(|entry| entry.security)
            .ok_or(ControllerError::Manager(ReasonCode::NotFound))
    }

    fn manager_error(&mut self, reason: ReasonCode) -> ControllerError {
        self.sync_manager_events();
        ControllerError::Manager(reason)
    }

    fn cancel_operation(&mut self) -> Result<(), ControllerError> {
        self.keyboard = None;
        self.keyboard_purpose = None;
        self.open_confirmation = false;
        self.manager
            .cancel()
            .map_err(|error| self.manager_error(error.0))?;
        self.sync_manager_events();
        Ok(())
    }

    fn manual_keyboard(&mut self) -> Result<(), ControllerError> {
        let policy = &self.metadata.manual_ssid;
        self.keyboard = Some(
            Keyboard::new(FieldPolicy::text(
                "",
                &policy.placeholder,
                policy.max_bytes,
                policy.max_scalars,
                allowed_chars(policy.allowed),
            ))
            .map_err(|_| ControllerError::Keyboard)?,
        );
        self.keyboard_purpose = Some(KeyboardPurpose::ManualSsid);
        let request = self.keyboard_request().ok_or(ControllerError::Keyboard)?;
        self.events
            .push(ControllerEvent::KeyboardRequested(request));
        Ok(())
    }

    fn network_key_keyboard(&mut self) -> Result<(), ControllerError> {
        let policy = &self.metadata.network_key;
        self.keyboard = Some(
            Keyboard::new(FieldPolicy::secret(
                "",
                &policy.placeholder,
                policy.max_bytes,
                policy.max_scalars,
                allowed_chars(policy.allowed),
            ))
            .map_err(|_| ControllerError::Keyboard)?,
        );
        self.keyboard_purpose = Some(KeyboardPurpose::NetworkKey);
        let request = self.keyboard_request().ok_or(ControllerError::Keyboard)?;
        self.events
            .push(ControllerEvent::KeyboardRequested(request));
        Ok(())
    }

    fn press_keyboard(&mut self, button: virtual_keyboard::Button) -> Result<(), ControllerError> {
        let result = self
            .keyboard
            .as_mut()
            .ok_or(ControllerError::Keyboard)?
            .press(button);
        match result {
            InputResult::Confirmed(TypedValue::Text(value)) => self.finish_manual(value),
            InputResult::Confirmed(TypedValue::Secret(value)) => self.finish_network_key(value),
            InputResult::Cancelled => {
                self.keyboard = None;
                self.keyboard_purpose = None;
                self.view = View::Menu;
                self.events.push(ControllerEvent::Cancelled);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn finish_manual(&mut self, ssid: String) -> Result<(), ControllerError> {
        self.keyboard = None;
        self.keyboard_purpose = None;
        let network_id = NetworkId::new("net-manual").map_err(|_| ControllerError::Keyboard)?;
        self.manager
            .add_manual_network(ManualNetworkRequest {
                network_id: network_id.clone(),
                ssid,
                security: self.selected_security,
                hidden: true,
            })
            .map_err(|error| self.manager_error(error.0))?;
        self.selected_network_id = Some(network_id.clone());
        self.sync_manager_events();
        if self.selected_security != Security::Open {
            self.network_key_keyboard()?;
        } else {
            self.view = View::Menu;
        }
        Ok(())
    }

    fn finish_network_key(&mut self, secret: String) -> Result<(), ControllerError> {
        self.keyboard = None;
        self.keyboard_purpose = None;
        let network_id = self.selected_id()?;
        let credential_reference = issue_fixture_reference(&secret)?;
        let result = self.manager.connect(ConnectRequest {
            network_id,
            open_confirmation: false,
            credential_reference: Some(credential_reference),
        });
        if let Err(error) = result {
            return Err(self.manager_error(error.0));
        }
        self.sync_manager_events();
        self.view = View::Menu;
        Ok(())
    }

    fn sync_manager_events(&mut self) {
        for event in self.manager.take_events() {
            match event {
                WifiEvent::PhaseChanged { phase, reason } => {
                    if phase == WifiPhase::Scanning {
                        self.events.push(ControllerEvent::ScanProgress);
                    }
                    if let Some(reason) = reason {
                        self.events.push(ControllerEvent::Error { reason });
                    }
                    self.events
                        .push(ControllerEvent::PhaseChanged { phase, reason });
                    if phase == WifiPhase::Cancelled {
                        self.events.push(ControllerEvent::Cancelled);
                    }
                }
                WifiEvent::ScanCompleted { count } => {
                    self.events.push(ControllerEvent::ScanCompleted { count });
                }
                WifiEvent::SelectionChanged { .. } => {
                    self.events.push(ControllerEvent::SelectionChanged);
                }
                WifiEvent::ConnectionChanged { network_id } => {
                    self.events.push(ControllerEvent::ConnectionChanged {
                        connected: network_id.is_some(),
                    });
                }
            }
        }
    }
}

impl fmt::Debug for WifiSettingsController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WifiSettingsController")
            .field("metadata", &self.metadata)
            .field("capability_available", &self.capability_available)
            .field("focus", &self.focus)
            .field("view", &self.view)
            .field("network_index", &self.network_index)
            .field("selected_network_id", &self.selected_network_id)
            .field("selected_security", &self.selected_security)
            .field("keyboard", &self.keyboard)
            .field("keyboard_purpose", &self.keyboard_purpose)
            .field("open_confirmation", &self.open_confirmation)
            .field("events", &self.events)
            .finish()
    }
}

fn allowed_chars(allowed: AllowedInput) -> AllowedChars {
    match allowed {
        AllowedInput::Any => AllowedChars::any(),
        AllowedInput::Ascii => AllowedChars::ascii(),
        AllowedInput::UrlSafe => AllowedChars::url_safe(),
    }
}

fn issue_fixture_reference(
    _secret: &str,
) -> Result<wifi_manager::CredentialReference, ControllerError> {
    wifi_manager::CredentialReference::new("cred-fixture-reference")
        .map_err(|_| ControllerError::InvalidInput(ReasonCode::InvalidRequest))
}

fn reject_duplicate_keys(bytes: &[u8]) -> Result<(), MetadataError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer
        .deserialize_any(RejectVisitor)
        .map_err(|error| invalid(format!("malformed JSON or duplicate key: {error}")))?;
    deserializer
        .end()
        .map_err(|error| invalid(format!("trailing JSON: {error}")))
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
