use std::{
    fs, io,
    net::{SocketAddr, TcpStream},
    process,
    time::Duration,
};

use network_services::{
    SaveSyncLifecycle, ServiceConfig, ServiceController, ServiceKind, ServiceSettings, ServiceStore,
};
use wifi_manager::{NetworkId, WifiPhase, WifiState};

fn main() {
    if let Err(error) = run() {
        eprintln!("network-services journey: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let settings = ServiceSettings {
        hostname: format!("brickpro-{}", process::id()),
        services: [
            ServiceKind::SshSftp,
            ServiceKind::Samba,
            ServiceKind::WebFileTransfer,
            ServiceKind::Syncthing,
            ServiceKind::Mdns,
        ]
        .into_iter()
        .map(|kind| ServiceConfig {
            kind,
            enabled: true,
            on_boot: true,
        })
        .collect(),
    };
    let mut services = ServiceController::new(settings).map_err(|error| error.to_string())?;
    let connected = WifiState {
        enabled: true,
        automatic_reconnect: true,
        phase: WifiPhase::Lan,
        reason: None,
        selected_network_id: None,
        connected_network_id: Some(NetworkId::new("net-lan").map_err(|error| error.to_string())?),
        scan_results: Vec::new(),
        retry_after_ms: None,
    };
    services.set_network(&connected, Some("192.168.50.2"));
    services.start_on_boot();

    for status in services.status() {
        if !status.running || status.addresses.len() != 2 {
            return Err(format!("{:?} did not publish LAN status", status.kind));
        }
        if let Some(port) = status.port {
            reachable(port)?;
        }
    }
    let syncthing = services
        .status()
        .into_iter()
        .find(|status| status.kind == ServiceKind::Syncthing)
        .ok_or("Syncthing status is unavailable")?;
    services
        .set_enabled(ServiceKind::Syncthing, false)
        .map_err(|error| error.to_string())?;
    let config_root = std::env::temp_dir().join(format!("brickpro-services-{}", process::id()));
    let _ = fs::remove_dir_all(&config_root);
    let store = ServiceStore::new(&config_root);
    store
        .save(&services.settings())
        .map_err(|error| error.to_string())?;
    if store.load().map_err(|error| error.to_string())? != services.settings() {
        return Err("approved service configuration did not persist".into());
    }
    let _ = fs::remove_dir_all(config_root);
    if services
        .status()
        .into_iter()
        .any(|status| status.kind == ServiceKind::Syncthing && status.running)
    {
        return Err("disabled Syncthing still reports running".into());
    }
    unreachable(syncthing.port.ok_or("Syncthing has no port")?)?;

    if services.sleep_inhibited() {
        return Err("idle service unexpectedly inhibits sleep".into());
    }
    {
        let _transfer = services.begin_transfer();
        if !services.sleep_inhibited() {
            return Err("transfer did not inhibit sleep".into());
        }
    }
    if services.sleep_inhibited() {
        return Err("idle transfer inhibitor was retained".into());
    }

    let mut sync = SaveSyncLifecycle::default();
    sync.before_game_launch();
    if sync.gate() != save_sync::SyncGate::Gameplay {
        return Err("save sync did not pause for gameplay".into());
    }
    sync.before_suspend();
    if sync.gate() != save_sync::SyncGate::SaveFlush {
        return Err("save sync did not enter checkpoint flush".into());
    }
    sync.checkpoint_complete();
    sync.after_game_exit();
    if sync.gate() != save_sync::SyncGate::Ready {
        return Err("save sync did not resume after exit".into());
    }

    services.stop_all();
    if services.status().into_iter().any(|status| status.running) {
        return Err("service survived clean shutdown".into());
    }
    println!("network-services journey: PASS (LAN status, port closure, bounded sleep, save-sync lifecycle)");
    Ok(())
}

fn reachable(port: u16) -> Result<(), String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&address, Duration::from_secs(1))
        .map(drop)
        .map_err(|error| format!("port {port} is not reachable: {error}"))
}

fn unreachable(port: u16) -> Result<(), String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    match TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
        Ok(_) => Err(format!("port {port} remained reachable after disable")),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(()),
        Err(error) => Err(format!(
            "port {port} closure could not be confirmed: {error}"
        )),
    }
}
