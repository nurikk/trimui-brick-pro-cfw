use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use save_sync::{
    syncthing::SyncthingAdapter,
    webdav::{PutCondition, PutResponse, WebDavAdapter, WebDavOutcome},
    Candidate, Exchange, StagedCandidate, SyncError, SyncGate,
};
use serde::{Deserialize, Serialize};
use wifi_manager::{WifiPhase, WifiState};

pub const CONFIG_FILE: &str = "services.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceKind {
    SshSftp,
    Samba,
    WebFileTransfer,
    Syncthing,
    Mdns,
}

impl ServiceKind {
    pub const fn port(self) -> Option<u16> {
        match self {
            Self::SshSftp => Some(8022),
            Self::Samba => Some(1445),
            Self::WebFileTransfer => Some(8080),
            Self::Syncthing => Some(8384),
            Self::Mdns => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServiceConfig {
    pub kind: ServiceKind,
    pub enabled: bool,
    pub on_boot: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServiceSettings {
    pub hostname: String,
    pub services: Vec<ServiceConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub kind: ServiceKind,
    pub enabled: bool,
    pub on_boot: bool,
    pub running: bool,
    pub port: Option<u16>,
    pub addresses: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceError(&'static str);

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for ServiceError {}

struct ManagedService {
    config: ServiceConfig,
    listener: Option<TcpListener>,
    running: bool,
    last_error: Option<String>,
}

pub struct ServiceController {
    hostname: String,
    ip_address: Option<String>,
    wifi_connected: bool,
    services: BTreeMap<ServiceKind, ManagedService>,
    inhibitor: SleepInhibitor,
}

impl ServiceController {
    pub fn new(settings: ServiceSettings) -> Result<Self, ServiceError> {
        if !valid_hostname(&settings.hostname) {
            return Err(ServiceError("service hostname is invalid"));
        }
        let mut services = BTreeMap::new();
        for config in settings.services {
            if services.contains_key(&config.kind) {
                return Err(ServiceError("service configuration is duplicated"));
            }
            services.insert(
                config.kind,
                ManagedService {
                    config,
                    listener: None,
                    running: false,
                    last_error: None,
                },
            );
        }
        Ok(Self {
            hostname: settings.hostname,
            ip_address: None,
            wifi_connected: false,
            services,
            inhibitor: SleepInhibitor::default(),
        })
    }

    pub fn settings(&self) -> ServiceSettings {
        ServiceSettings {
            hostname: self.hostname.clone(),
            services: self
                .services
                .values()
                .map(|service| service.config.clone())
                .collect(),
        }
    }

    pub fn set_network(&mut self, wifi: &WifiState, ip_address: Option<&str>) {
        self.wifi_connected = matches!(wifi.phase, WifiPhase::Lan | WifiPhase::Internet)
            && wifi.connected_network_id.is_some();
        self.ip_address = ip_address
            .filter(|address| valid_ip(address))
            .map(str::to_owned);
        if !self.wifi_connected {
            self.stop_all();
        }
    }

    pub fn start_on_boot(&mut self) {
        for kind in self
            .services
            .iter()
            .filter_map(|(kind, service)| {
                (service.config.enabled && service.config.on_boot).then_some(*kind)
            })
            .collect::<Vec<_>>()
        {
            self.start(kind);
        }
    }

    pub fn set_enabled(&mut self, kind: ServiceKind, enabled: bool) -> Result<(), ServiceError> {
        let service = self
            .services
            .get_mut(&kind)
            .ok_or(ServiceError("service is not packaged"))?;
        service.config.enabled = enabled;
        if !enabled {
            service.listener = None;
            service.running = false;
        } else {
            self.start(kind);
        }
        Ok(())
    }

    pub fn start(&mut self, kind: ServiceKind) {
        let Some(service) = self.services.get_mut(&kind) else {
            return;
        };
        if !service.config.enabled || !self.wifi_connected {
            service.listener = None;
            service.running = false;
            return;
        }
        if kind == ServiceKind::Mdns {
            service.running = true;
            return;
        }
        if service.listener.is_some() {
            return;
        }
        let Some(port) = kind.port() else { return };
        match TcpListener::bind(("0.0.0.0", port)) {
            Ok(listener) => {
                service.listener = Some(listener);
                service.running = true;
                service.last_error = None;
            }
            Err(_) => {
                service.running = false;
                service.last_error = Some("service could not bind its local port".into());
            }
        }
    }

    pub fn stop(&mut self, kind: ServiceKind) {
        if let Some(service) = self.services.get_mut(&kind) {
            service.listener = None;
            service.running = false;
        }
    }

    pub fn stop_all(&mut self) {
        for service in self.services.values_mut() {
            service.listener = None;
            service.running = false;
        }
    }

    pub fn status(&self) -> Vec<ServiceStatus> {
        self.services
            .iter()
            .map(|(kind, service)| {
                let running = service.config.enabled && self.wifi_connected && service.running;
                ServiceStatus {
                    kind: *kind,
                    enabled: service.config.enabled,
                    on_boot: service.config.on_boot,
                    running,
                    port: kind.port(),
                    addresses: running.then(|| self.addresses(*kind)).unwrap_or_default(),
                    last_error: service.last_error.clone(),
                }
            })
            .collect()
    }

    pub fn begin_transfer(&self) -> SleepLease {
        self.inhibitor.acquire()
    }

    pub fn begin_index(&self) -> SleepLease {
        self.inhibitor.acquire()
    }

    pub fn sleep_inhibited(&self) -> bool {
        self.inhibitor.active()
    }

    fn addresses(&self, kind: ServiceKind) -> Vec<String> {
        let suffix = kind
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        let mut addresses = vec![format!("{}.local{suffix}", self.hostname)];
        if let Some(address) = &self.ip_address {
            addresses.push(format!("{address}{suffix}"));
        }
        addresses
    }
}

#[derive(Clone, Default)]
pub struct SleepInhibitor(Arc<AtomicUsize>);

impl SleepInhibitor {
    pub fn active(&self) -> bool {
        self.0.load(Ordering::Acquire) != 0
    }

    fn acquire(&self) -> SleepLease {
        self.0.fetch_add(1, Ordering::AcqRel);
        SleepLease(self.0.clone())
    }
}

pub struct SleepLease(Arc<AtomicUsize>);

impl Drop for SleepLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveSyncLifecycle {
    gate: SyncGate,
}

impl Default for SaveSyncLifecycle {
    fn default() -> Self {
        Self {
            gate: SyncGate::Ready,
        }
    }
}

impl SaveSyncLifecycle {
    pub fn gate(self) -> SyncGate {
        self.gate
    }

    pub fn before_game_launch(&mut self) {
        self.gate = SyncGate::Gameplay;
    }

    pub fn before_suspend(&mut self) {
        self.gate = SyncGate::SaveFlush;
    }

    pub fn checkpoint_complete(&mut self) {
        self.gate = SyncGate::Ready;
    }

    pub fn after_game_exit(&mut self) {
        self.gate = SyncGate::Ready;
    }
}

pub fn ingest_syncthing(
    exchange: Exchange,
    file_name: &str,
    candidate: Candidate,
    payload: &[u8],
) -> Result<StagedCandidate, SyncError> {
    SyncthingAdapter::new(exchange).ingest(file_name, candidate, payload)
}

pub fn finish_webdav(condition: &PutCondition, response: PutResponse) -> WebDavOutcome {
    WebDavAdapter::finish(condition, response)
}

pub struct ServiceStore {
    root: PathBuf,
}

impl ServiceStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn load(&self) -> Result<ServiceSettings, ServiceError> {
        let bytes =
            fs::read(self.path()).map_err(|_| ServiceError("service settings are unavailable"))?;
        let settings: ServiceSettings = serde_json::from_slice(&bytes)
            .map_err(|_| ServiceError("service settings are invalid"))?;
        ServiceController::new(settings.clone())?;
        Ok(settings)
    }

    pub fn save(&self, settings: &ServiceSettings) -> Result<(), ServiceError> {
        ServiceController::new(settings.clone())?;
        fs::create_dir_all(&self.root)
            .map_err(|_| ServiceError("service settings cannot be saved"))?;
        let temporary = self.root.join(".services.json.tmp");
        let bytes = serde_json::to_vec_pretty(settings)
            .map_err(|_| ServiceError("service settings cannot be saved"))?;
        fs::write(&temporary, bytes)
            .map_err(|_| ServiceError("service settings cannot be saved"))?;
        fs::rename(temporary, self.path())
            .map_err(|_| ServiceError("service settings cannot be saved"))
    }

    fn path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }
}

fn valid_hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn valid_ip(value: &str) -> bool {
    value.parse::<std::net::IpAddr>().is_ok()
}
