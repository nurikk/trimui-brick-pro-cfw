use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

pub const MAX_SSID_BYTES: usize = 32;
pub const MAX_CREDENTIAL_REFERENCE_BYTES: usize = 128;
pub const MAX_NETWORK_ID_BYTES: usize = 64;
pub const LOW_BATTERY_PERCENT: u8 = 20;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NetworkId(String);

impl NetworkId {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_opaque(&value, "net-", MAX_NETWORK_ID_BYTES, true)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for NetworkId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for NetworkId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialReference(String);

impl CredentialReference {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_opaque(&value, "cred-", MAX_CREDENTIAL_REFERENCE_BYTES, true)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for CredentialReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CredentialReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ssid(String);

impl Ssid {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_ssid(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Ssid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Ssid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Security {
    Open,
    Wpa2Psk,
    Wpa3Sae,
    Unsupported,
}

impl Security {
    fn needs_credential(self) -> bool {
        matches!(self, Self::Wpa2Psk | Self::Wpa3Sae)
    }
    fn supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiPhase {
    Idle,
    Scanning,
    AwaitingCredentials,
    Connecting,
    Connected,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasonCode {
    InvalidRequest,
    ConfirmationRequired,
    MissingCredential,
    BadCredentials,
    Timeout,
    RadioUnavailable,
    UnsupportedSecurity,
    NotFound,
    Cancelled,
    Busy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError(&'static str);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ValidationError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub rescan: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScanResultEntry {
    pub network_id: NetworkId,
    pub display_ssid: String,
    pub signal_quality: u8,
    pub security: Security,
    pub known: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedNetworkRecord {
    pub network_id: NetworkId,
    pub display_ssid: String,
    pub security: Security,
    pub credential_reference: Option<CredentialReference>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedState {
    pub enabled: bool,
    pub automatic_reconnect: bool,
    pub networks: Vec<SavedNetworkRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WifiState {
    pub enabled: bool,
    pub automatic_reconnect: bool,
    pub phase: WifiPhase,
    pub reason: Option<ReasonCode>,
    pub selected_network_id: Option<NetworkId>,
    pub connected_network_id: Option<NetworkId>,
    pub scan_results: Vec<ScanResultEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManualNetworkRequest {
    pub network_id: NetworkId,
    pub ssid: String,
    pub security: Security,
    pub hidden: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectRequest {
    pub network_id: NetworkId,
    pub open_confirmation: bool,
    pub credential_reference: Option<CredentialReference>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case", tag = "event")]
pub enum WifiEvent {
    PhaseChanged {
        phase: WifiPhase,
        reason: Option<ReasonCode>,
    },
    ScanCompleted {
        count: usize,
    },
    SelectionChanged {
        network_id: Option<NetworkId>,
    },
    ConnectionChanged {
        network_id: Option<NetworkId>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettingKind {
    Toggle,
    Action,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
}

pub fn settings_descriptors() -> [SettingDescriptor; 3] {
    [
        SettingDescriptor {
            id: "wifi-enabled",
            label: "Wi-Fi enabled",
            kind: SettingKind::Toggle,
        },
        SettingDescriptor {
            id: "automatic-reconnect",
            label: "Automatic reconnect",
            kind: SettingKind::Toggle,
        },
        SettingDescriptor {
            id: "scan",
            label: "Scan for networks",
            kind: SettingKind::Action,
        },
    ]
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "scene")]
pub enum ScenePayload {
    AccessPoints {
        entries: Vec<ScanResultEntry>,
    },
    ManualNetwork {
        security_choices: Vec<Security>,
        max_ssid_bytes: usize,
    },
    Keyboard {
        request: KeyboardInputRequest,
    },
    Progress {
        phase: WifiPhase,
        network_id: Option<NetworkId>,
    },
    Error {
        reason: ReasonCode,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyboardInputKind {
    Text,
    Secret,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyboardField {
    Ssid,
    Password,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardInputRequest {
    pub field: KeyboardField,
    pub input_kind: KeyboardInputKind,
    pub max_bytes: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReconnectBlock {
    LowBattery,
    Suspended,
    GameplayActive,
    CapabilityUnavailable,
    Disabled,
    NoSavedNetwork,
    OpenNetworkNeedsConfirmation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutoReconnectDecision {
    Attempted,
    Blocked(ReconnectBlock),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReconnectConditions {
    pub battery_percent: u8,
    pub suspended: bool,
    pub gameplay_active: bool,
    pub capability_available: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeneratedConnectOutcome {
    Success,
    BadPassword,
    Timeout,
    RadioUnavailable,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedAccessPoint {
    pub network_id: NetworkId,
    pub ssid: String,
    pub signal_quality: u8,
    pub security: Security,
    pub known: bool,
    pub connected: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedWifiFixture {
    pub format: String,
    pub fixture_version: u8,
    pub radio_available: bool,
    pub access_points: Vec<GeneratedAccessPoint>,
    pub connect_outcomes: BTreeMap<NetworkId, GeneratedConnectOutcome>,
    pub journeys: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendAccessPoint {
    pub network_id: NetworkId,
    pub ssid: String,
    pub signal_quality: u8,
    pub security: Security,
    pub known: bool,
    pub connected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    RadioUnavailable,
    BadPassword,
    Timeout,
    Unsupported,
    Cancelled,
    NotFound,
}

pub trait WifiBackend {
    fn capability_available(&self) -> bool;
    fn scan(&mut self) -> Result<Vec<BackendAccessPoint>, BackendError>;
    fn connect(
        &mut self,
        network_id: &NetworkId,
        credential_reference: Option<&CredentialReference>,
    ) -> Result<(), BackendError>;
    fn disconnect(&mut self, network_id: Option<&NetworkId>) -> Result<(), BackendError>;
    fn cancel(&mut self);
}

#[derive(Clone, Debug)]
pub struct GeneratedWifiBackend {
    fixture: GeneratedWifiFixture,
    cancelled: bool,
}

impl GeneratedWifiBackend {
    pub fn from_fixture(fixture: GeneratedWifiFixture) -> Result<Self, ValidationError> {
        if fixture.format != "generated-wifi-fixture" || fixture.fixture_version != 1 {
            return Err(ValidationError("unsupported generated Wi-Fi fixture"));
        }
        let mut ids = BTreeSet::new();
        for access_point in &fixture.access_points {
            if !ids.insert(access_point.network_id.clone()) || access_point.signal_quality > 100 {
                return Err(ValidationError(
                    "invalid or duplicate generated access point",
                ));
            }
            validate_ssid(&access_point.ssid)?;
            if access_point.hidden && access_point.ssid != "Hidden network" {
                return Err(ValidationError(
                    "hidden fixture access point must be redacted",
                ));
            }
        }
        for id in fixture.connect_outcomes.keys() {
            if !ids.contains(id) {
                return Err(ValidationError("outcome references unknown network"));
            }
        }
        Ok(Self {
            fixture,
            cancelled: false,
        })
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let fixture: GeneratedWifiFixture = serde_json::from_slice(bytes)?;
        Ok(Self::from_fixture(fixture)?)
    }
}

impl WifiBackend for GeneratedWifiBackend {
    fn capability_available(&self) -> bool {
        self.fixture.radio_available
    }

    fn scan(&mut self) -> Result<Vec<BackendAccessPoint>, BackendError> {
        if self.cancelled {
            self.cancelled = false;
            return Err(BackendError::Cancelled);
        }
        if !self.fixture.radio_available {
            return Err(BackendError::RadioUnavailable);
        }
        Ok(self
            .fixture
            .access_points
            .iter()
            .map(|point| BackendAccessPoint {
                network_id: point.network_id.clone(),
                ssid: if point.hidden {
                    "Hidden network".to_string()
                } else {
                    point.ssid.clone()
                },
                signal_quality: point.signal_quality,
                security: point.security,
                known: point.known,
                connected: point.connected,
            })
            .collect())
    }

    fn connect(
        &mut self,
        network_id: &NetworkId,
        _credential_reference: Option<&CredentialReference>,
    ) -> Result<(), BackendError> {
        if !self.fixture.radio_available {
            return Err(BackendError::RadioUnavailable);
        }
        match self
            .fixture
            .connect_outcomes
            .get(network_id)
            .copied()
            .unwrap_or(GeneratedConnectOutcome::Success)
        {
            GeneratedConnectOutcome::Success => Ok(()),
            GeneratedConnectOutcome::BadPassword => Err(BackendError::BadPassword),
            GeneratedConnectOutcome::Timeout => Err(BackendError::Timeout),
            GeneratedConnectOutcome::RadioUnavailable => Err(BackendError::RadioUnavailable),
            GeneratedConnectOutcome::Unsupported => Err(BackendError::Unsupported),
        }
    }

    fn disconnect(&mut self, _network_id: Option<&NetworkId>) -> Result<(), BackendError> {
        Ok(())
    }
    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

pub trait Tg4040WifiPort {
    fn capability_available(&self) -> bool;
    fn scan(&mut self) -> Result<Vec<BackendAccessPoint>, BackendError>;
    fn connect(
        &mut self,
        network_id: &NetworkId,
        credential_reference: Option<&CredentialReference>,
    ) -> Result<(), BackendError>;
    fn disconnect(&mut self, network_id: Option<&NetworkId>) -> Result<(), BackendError>;
    fn cancel(&mut self);
}

pub struct WifiManager<B: WifiBackend> {
    backend: B,
    saved_networks: BTreeMap<NetworkId, SavedNetworkRecord>,
    state: WifiState,
    events: Vec<WifiEvent>,
    last_credential_reference: Option<CredentialReference>,
    last_open_confirmation: bool,
}

impl<B: WifiBackend> WifiManager<B> {
    pub fn new(backend: B) -> Self {
        Self::from_saved_state(
            backend,
            SavedState {
                enabled: true,
                automatic_reconnect: true,
                networks: Vec::new(),
            },
        )
        .expect("empty saved state is valid")
    }

    pub fn from_saved_state(backend: B, saved: SavedState) -> Result<Self, ValidationError> {
        validate_saved_state(&saved)?;
        let saved_networks = saved
            .networks
            .into_iter()
            .map(|record| (record.network_id.clone(), record))
            .collect();
        Ok(Self {
            backend,
            saved_networks,
            state: WifiState {
                enabled: saved.enabled,
                automatic_reconnect: saved.automatic_reconnect,
                phase: WifiPhase::Idle,
                reason: None,
                selected_network_id: None,
                connected_network_id: None,
                scan_results: Vec::new(),
            },
            events: Vec::new(),
            last_credential_reference: None,
            last_open_confirmation: false,
        })
    }

    pub fn state(&self) -> &WifiState {
        &self.state
    }
    pub fn saved_state(&self) -> SavedState {
        SavedState {
            enabled: self.state.enabled,
            automatic_reconnect: self.state.automatic_reconnect,
            networks: self.saved_networks.values().cloned().collect(),
        }
    }
    pub fn take_events(&mut self) -> Vec<WifiEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), WifiError> {
        if !enabled {
            self.backend
                .disconnect(self.state.connected_network_id.as_ref())
                .map_err(map_backend)?;
            self.state.connected_network_id = None;
        }
        self.state.enabled = enabled;
        self.state.scan_results.clear();
        self.state.selected_network_id = None;
        self.set_phase(WifiPhase::Idle, None);
        Ok(())
    }

    pub fn set_automatic_reconnect(&mut self, enabled: bool) {
        self.state.automatic_reconnect = enabled;
    }

    pub fn scan(&mut self, _request: ScanRequest) -> Result<(), WifiError> {
        if !self.state.enabled || !self.backend.capability_available() {
            return self.fail_result(ReasonCode::RadioUnavailable);
        }
        self.set_phase(WifiPhase::Scanning, None);
        let access_points = match self.backend.scan() {
            Ok(access_points) => access_points,
            Err(BackendError::Cancelled) => {
                self.set_phase(WifiPhase::Cancelled, Some(ReasonCode::Cancelled));
                return Err(WifiError(ReasonCode::Cancelled));
            }
            Err(error) => {
                let reason = map_backend(error);
                return self.fail_result(reason.0);
            }
        };
        if access_points
            .iter()
            .any(|point| point.signal_quality > 100 || validate_ssid(&point.ssid).is_err())
        {
            return self.fail_result(ReasonCode::InvalidRequest);
        }
        self.state.scan_results = collapse_results(access_points, &self.saved_networks);
        self.state.connected_network_id = self
            .state
            .scan_results
            .iter()
            .find(|entry| entry.connected)
            .map(|entry| entry.network_id.clone());
        if self.state.connected_network_id.is_some() {
            self.set_phase(WifiPhase::Connected, None);
        } else {
            self.set_phase(WifiPhase::Idle, None);
        }
        self.events.push(WifiEvent::ScanCompleted {
            count: self.state.scan_results.len(),
        });
        Ok(())
    }

    pub fn rescan(&mut self) -> Result<(), WifiError> {
        self.scan(ScanRequest { rescan: true })
    }

    pub fn add_manual_network(&mut self, request: ManualNetworkRequest) -> Result<(), WifiError> {
        let ssid = Ssid::new(request.ssid).map_err(|_| WifiError(ReasonCode::InvalidRequest))?;
        if !request.security.supported() {
            return Err(WifiError(ReasonCode::UnsupportedSecurity));
        }
        let display_ssid = if request.hidden {
            "Hidden network".to_string()
        } else {
            ssid.as_str().to_string()
        };
        self.state
            .scan_results
            .retain(|entry| entry.network_id != request.network_id);
        self.state.scan_results.push(ScanResultEntry {
            network_id: request.network_id.clone(),
            display_ssid,
            signal_quality: 0,
            security: request.security,
            known: self.saved_networks.contains_key(&request.network_id),
            connected: false,
        });
        self.state.scan_results.sort_by(scan_order);
        self.select(&request.network_id)
    }

    pub fn select(&mut self, network_id: &NetworkId) -> Result<(), WifiError> {
        let entry = self
            .state
            .scan_results
            .iter()
            .find(|entry| &entry.network_id == network_id)
            .cloned()
            .ok_or(WifiError(ReasonCode::NotFound))?;
        self.state.selected_network_id = Some(network_id.clone());
        self.events.push(WifiEvent::SelectionChanged {
            network_id: Some(network_id.clone()),
        });
        if entry.security.needs_credential() {
            self.set_phase(WifiPhase::AwaitingCredentials, None);
        } else {
            self.set_phase(WifiPhase::Idle, None);
        }
        Ok(())
    }

    pub fn connect(&mut self, request: ConnectRequest) -> Result<(), WifiError> {
        if !self.state.enabled || !self.backend.capability_available() {
            return self.fail_result(ReasonCode::RadioUnavailable);
        }
        let entry = self
            .state
            .scan_results
            .iter()
            .find(|entry| entry.network_id == request.network_id)
            .cloned()
            .ok_or(WifiError(ReasonCode::NotFound))?;
        if entry.security == Security::Unsupported {
            return self.fail_result(ReasonCode::UnsupportedSecurity);
        }
        if entry.security == Security::Open {
            if !request.open_confirmation {
                return self.fail_result(ReasonCode::ConfirmationRequired);
            }
            if request.credential_reference.is_some() {
                return self.fail_result(ReasonCode::InvalidRequest);
            }
        } else if request.credential_reference.is_none() {
            return self.fail_result(ReasonCode::MissingCredential);
        }
        self.state.selected_network_id = Some(request.network_id.clone());
        self.last_credential_reference = request.credential_reference.clone();
        self.last_open_confirmation = request.open_confirmation;
        self.set_phase(WifiPhase::Connecting, None);
        match self
            .backend
            .connect(&request.network_id, request.credential_reference.as_ref())
        {
            Ok(()) => {
                self.state.connected_network_id = Some(request.network_id.clone());
                self.save_successful_connection(&entry, request.credential_reference);
                for result in &mut self.state.scan_results {
                    result.connected = result.network_id == request.network_id;
                    result.known = self.saved_networks.contains_key(&result.network_id);
                }
                self.set_phase(WifiPhase::Connected, None);
                self.events.push(WifiEvent::ConnectionChanged {
                    network_id: Some(request.network_id),
                });
                Ok(())
            }
            Err(BackendError::Cancelled) => {
                self.set_phase(WifiPhase::Cancelled, Some(ReasonCode::Cancelled));
                Err(WifiError(ReasonCode::Cancelled))
            }
            Err(error) => {
                let reason = map_backend(error);
                self.fail_result(reason.0)
            }
        }
    }

    pub fn disconnect(&mut self) -> Result<(), WifiError> {
        self.backend
            .disconnect(self.state.connected_network_id.as_ref())
            .map_err(map_backend)?;
        self.state.connected_network_id = None;
        for result in &mut self.state.scan_results {
            result.connected = false;
        }
        self.set_phase(WifiPhase::Idle, None);
        self.events
            .push(WifiEvent::ConnectionChanged { network_id: None });
        Ok(())
    }

    pub fn forget(&mut self, network_id: &NetworkId) -> Result<(), WifiError> {
        if !self.saved_networks.contains_key(network_id) {
            return Err(WifiError(ReasonCode::NotFound));
        }
        if self.state.connected_network_id.as_ref() == Some(network_id) {
            self.disconnect()?;
        }
        self.saved_networks.remove(network_id);
        for result in &mut self.state.scan_results {
            if &result.network_id == network_id {
                result.known = false;
            }
        }
        Ok(())
    }

    pub fn retry(&mut self) -> Result<(), WifiError> {
        let network_id = self
            .state
            .selected_network_id
            .clone()
            .ok_or(WifiError(ReasonCode::NotFound))?;
        let entry = self
            .state
            .scan_results
            .iter()
            .find(|entry| entry.network_id == network_id)
            .cloned()
            .ok_or(WifiError(ReasonCode::NotFound))?;
        let credential_reference = self.last_credential_reference.clone().or_else(|| {
            self.saved_networks
                .get(&network_id)
                .and_then(|record| record.credential_reference.clone())
        });
        self.connect(ConnectRequest {
            network_id,
            open_confirmation: self.last_open_confirmation,
            credential_reference: if entry.security.needs_credential() {
                credential_reference
            } else {
                None
            },
        })
    }

    pub fn cancel(&mut self) -> Result<(), WifiError> {
        self.backend.cancel();
        self.set_phase(WifiPhase::Cancelled, Some(ReasonCode::Cancelled));
        Ok(())
    }

    pub fn auto_reconnect(&mut self, conditions: ReconnectConditions) -> AutoReconnectDecision {
        let block = if !self.state.automatic_reconnect {
            Some(ReconnectBlock::Disabled)
        } else if conditions.battery_percent < LOW_BATTERY_PERCENT {
            Some(ReconnectBlock::LowBattery)
        } else if conditions.suspended {
            Some(ReconnectBlock::Suspended)
        } else if conditions.gameplay_active {
            Some(ReconnectBlock::GameplayActive)
        } else if !conditions.capability_available || !self.backend.capability_available() {
            Some(ReconnectBlock::CapabilityUnavailable)
        } else {
            None
        };
        if let Some(block) = block {
            return AutoReconnectDecision::Blocked(block);
        }
        let candidate = self
            .state
            .scan_results
            .iter()
            .filter_map(|entry| {
                self.saved_networks
                    .values()
                    .find(|record| {
                        record.security == entry.security
                            && (record.network_id == entry.network_id
                                || record.display_ssid == entry.display_ssid)
                    })
                    .map(|record| (entry.clone(), record.clone()))
            })
            .next();
        let (entry, record) = match candidate {
            Some(candidate) => candidate,
            None => return AutoReconnectDecision::Blocked(ReconnectBlock::NoSavedNetwork),
        };
        if entry.security == Security::Open {
            return AutoReconnectDecision::Blocked(ReconnectBlock::OpenNetworkNeedsConfirmation);
        }
        self.last_credential_reference = record.credential_reference.clone();
        self.last_open_confirmation = false;
        let _ = self.connect(ConnectRequest {
            network_id: entry.network_id.clone(),
            open_confirmation: false,
            credential_reference: record.credential_reference.clone(),
        });
        AutoReconnectDecision::Attempted
    }

    pub fn access_points_scene(&self) -> ScenePayload {
        ScenePayload::AccessPoints {
            entries: self.state.scan_results.clone(),
        }
    }
    pub fn manual_network_scene(&self) -> ScenePayload {
        ScenePayload::ManualNetwork {
            security_choices: vec![Security::Open, Security::Wpa2Psk, Security::Wpa3Sae],
            max_ssid_bytes: MAX_SSID_BYTES,
        }
    }
    pub fn ssid_keyboard_request(&self) -> ScenePayload {
        ScenePayload::Keyboard {
            request: KeyboardInputRequest {
                field: KeyboardField::Ssid,
                input_kind: KeyboardInputKind::Text,
                max_bytes: MAX_SSID_BYTES,
            },
        }
    }
    pub fn password_keyboard_request(&self) -> Option<ScenePayload> {
        let entry = self.state.selected_network_id.as_ref().and_then(|id| {
            self.state
                .scan_results
                .iter()
                .find(|entry| &entry.network_id == id)
        })?;
        if !entry.security.needs_credential() {
            return None;
        }
        Some(ScenePayload::Keyboard {
            request: KeyboardInputRequest {
                field: KeyboardField::Password,
                input_kind: KeyboardInputKind::Secret,
                max_bytes: MAX_CREDENTIAL_REFERENCE_BYTES,
            },
        })
    }
    pub fn scene_payload(&self) -> ScenePayload {
        match self.state.phase {
            WifiPhase::AwaitingCredentials => self
                .password_keyboard_request()
                .unwrap_or_else(|| self.access_points_scene()),
            WifiPhase::Connecting | WifiPhase::Scanning => ScenePayload::Progress {
                phase: self.state.phase,
                network_id: self.state.selected_network_id.clone(),
            },
            WifiPhase::Failed | WifiPhase::Cancelled => ScenePayload::Error {
                reason: self.state.reason.unwrap_or(ReasonCode::InvalidRequest),
            },
            WifiPhase::Idle | WifiPhase::Connected => self.access_points_scene(),
        }
    }

    fn save_successful_connection(
        &mut self,
        entry: &ScanResultEntry,
        credential_reference: Option<CredentialReference>,
    ) {
        self.saved_networks.insert(
            entry.network_id.clone(),
            SavedNetworkRecord {
                network_id: entry.network_id.clone(),
                display_ssid: entry.display_ssid.clone(),
                security: entry.security,
                credential_reference,
            },
        );
    }
    fn set_phase(&mut self, phase: WifiPhase, reason: Option<ReasonCode>) {
        self.state.phase = phase;
        self.state.reason = reason;
        self.events.push(WifiEvent::PhaseChanged { phase, reason });
    }
    fn fail(&mut self, reason: ReasonCode) {
        self.set_phase(WifiPhase::Failed, Some(reason));
    }
    fn fail_result(&mut self, reason: ReasonCode) -> Result<(), WifiError> {
        self.fail(reason);
        Err(WifiError(reason))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WifiError(pub ReasonCode);

impl fmt::Display for WifiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Wi-Fi operation failed: {:?}", self.0)
    }
}
impl std::error::Error for WifiError {}

fn map_backend(error: BackendError) -> WifiError {
    WifiError(match error {
        BackendError::RadioUnavailable => ReasonCode::RadioUnavailable,
        BackendError::BadPassword => ReasonCode::BadCredentials,
        BackendError::Timeout => ReasonCode::Timeout,
        BackendError::Unsupported => ReasonCode::UnsupportedSecurity,
        BackendError::Cancelled => ReasonCode::Cancelled,
        BackendError::NotFound => ReasonCode::NotFound,
    })
}

fn validate_ssid(value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > MAX_SSID_BYTES
        || value.chars().any(char::is_control)
        || contains_private_marker(value)
    {
        return Err(ValidationError(
            "SSID is empty, oversized, controlled, or private-bearing",
        ));
    }
    Ok(())
}

fn validate_opaque(
    value: &str,
    prefix: &str,
    max_bytes: usize,
    private_check: bool,
) -> Result<(), ValidationError> {
    if value.len() > max_bytes
        || !value.starts_with(prefix)
        || value[prefix.len()..].is_empty()
        || !value[prefix.len()..].bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        || (private_check && contains_private_marker(value))
    {
        return Err(ValidationError("malformed opaque identifier"));
    }
    Ok(())
}

fn contains_private_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains('/')
        || lower.contains('\\')
        || lower.contains("..")
        || [
            "password",
            "passwd",
            "secret",
            "token",
            "credential",
            "bssid",
            "/data",
            "/proc",
            "/sys",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        || looks_like_mac(value)
}

fn looks_like_mac(value: &str) -> bool {
    let parts: Vec<&str> = value.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn collapse_results(
    access_points: Vec<BackendAccessPoint>,
    saved: &BTreeMap<NetworkId, SavedNetworkRecord>,
) -> Vec<ScanResultEntry> {
    let mut selected: BTreeMap<String, BackendAccessPoint> = BTreeMap::new();
    for point in access_points {
        let key = point.ssid.clone();
        match selected.get_mut(&key) {
            Some(current) if candidate_is_better(&point, current) => {
                let known = current.known || point.known;
                let connected = current.connected || point.connected;
                let mut replacement = point;
                replacement.known = known;
                replacement.connected = connected;
                *current = replacement;
            }
            Some(current) => {
                current.known |= point.known;
                current.connected |= point.connected;
            }
            None => {
                selected.insert(key, point);
            }
        }
    }
    let mut results: Vec<_> = selected
        .into_values()
        .map(|point| ScanResultEntry {
            known: point.known
                || saved.contains_key(&point.network_id)
                || saved.values().any(|record| {
                    record.display_ssid == point.ssid && record.security == point.security
                }),
            network_id: point.network_id,
            display_ssid: point.ssid,
            signal_quality: point.signal_quality,
            security: point.security,
            connected: point.connected,
        })
        .collect();
    results.sort_by(scan_order);
    results
}

fn candidate_is_better(left: &BackendAccessPoint, right: &BackendAccessPoint) -> bool {
    match left.security.supported().cmp(&right.security.supported()) {
        std::cmp::Ordering::Equal => match left.signal_quality.cmp(&right.signal_quality) {
            std::cmp::Ordering::Equal => match security_rank(left.security)
                .cmp(&security_rank(right.security))
            {
                std::cmp::Ordering::Equal => left.network_id.as_str() < right.network_id.as_str(),
                ordering => ordering.is_gt(),
            },
            ordering => ordering.is_gt(),
        },
        ordering => ordering.is_gt(),
    }
}

fn security_rank(security: Security) -> u8 {
    match security {
        Security::Wpa3Sae => 3,
        Security::Wpa2Psk => 2,
        Security::Open => 1,
        Security::Unsupported => 0,
    }
}

fn scan_order(left: &ScanResultEntry, right: &ScanResultEntry) -> std::cmp::Ordering {
    right
        .connected
        .cmp(&left.connected)
        .then_with(|| right.known.cmp(&left.known))
        .then_with(|| right.signal_quality.cmp(&left.signal_quality))
        .then_with(|| left.display_ssid.cmp(&right.display_ssid))
        .then_with(|| left.network_id.as_str().cmp(right.network_id.as_str()))
}

fn validate_saved_state(saved: &SavedState) -> Result<(), ValidationError> {
    if saved.networks.len() > 64 {
        return Err(ValidationError("too many saved networks"));
    }
    let mut ids = BTreeSet::new();
    for record in &saved.networks {
        if !ids.insert(record.network_id.clone())
            || Ssid::new(record.display_ssid.clone()).is_err()
            || !record.security.supported()
        {
            return Err(ValidationError("invalid saved network"));
        }
        if record.security.needs_credential() != record.credential_reference.is_some() {
            return Err(ValidationError("saved credential does not match security"));
        }
    }
    Ok(())
}
