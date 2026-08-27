use std::fmt::Display;

use wifi_manager::{
    AutoReconnectDecision, ConnectRequest, CredentialReference, GeneratedWifiBackend,
    GeneratedWifiFixture, KeyboardField, KeyboardInputKind, ManualNetworkRequest, NetworkId,
    ReasonCode, ReconnectBlock, ReconnectConditions, ScanRequest, ScenePayload, Security, Ssid,
    WifiError, WifiManager, WifiPhase,
};

const JOURNEYS: [&str; 14] = [
    "successful-scan-select-connect",
    "refresh",
    "strongest-radio-collapse",
    "hidden-ssid",
    "bad-password",
    "timeout",
    "radio-unavailable",
    "cancellation",
    "reconnect",
    "disconnect",
    "forget",
    "retry",
    "restart",
    "negative-validation",
];

fn main() -> Result<(), String> {
    let fixture = fixture()?;
    require(
        fixture.journeys.len() == JOURNEYS.len(),
        "fixture journey count",
    )?;
    for journey in JOURNEYS {
        require(fixture.journeys.iter().any(|name| name == journey), journey)?;
    }

    successful_scan_select_connect()?;
    refresh()?;
    strongest_radio_collapse()?;
    hidden_ssid()?;
    bad_password()?;
    timeout()?;
    radio_unavailable()?;
    cancellation()?;
    reconnect()?;
    disconnect()?;
    forget()?;
    retry()?;
    restart()?;
    negative_validation()?;

    println!("wifi-manager-fixtures: {} journeys passed", JOURNEYS.len());
    Ok(())
}

fn fixture() -> Result<GeneratedWifiFixture, String> {
    serde_json::from_slice(include_bytes!(
        "../../../../fixtures/wifi-manager/journeys.json"
    ))
    .map_err(|error| format!("fixture decode failed: {error}"))
}

fn manager() -> Result<WifiManager<GeneratedWifiBackend>, String> {
    let backend = GeneratedWifiBackend::from_fixture(fixture()?).map_err(error)?;
    Ok(WifiManager::new(backend))
}

fn manager_from(
    fixture: GeneratedWifiFixture,
) -> Result<WifiManager<GeneratedWifiBackend>, String> {
    let backend = GeneratedWifiBackend::from_fixture(fixture).map_err(error)?;
    Ok(WifiManager::new(backend))
}

fn id(value: &str) -> Result<NetworkId, String> {
    NetworkId::new(value).map_err(error)
}

fn credential(value: &str) -> Result<CredentialReference, String> {
    CredentialReference::new(value).map_err(error)
}

fn error(error: impl Display) -> String {
    error.to_string()
}

fn require(condition: bool, label: &str) -> Result<(), String> {
    condition
        .then_some(())
        .ok_or_else(|| format!("journey failed: {label}"))
}

fn expect_ok<T, E: Display>(result: Result<T, E>, label: &str) -> Result<T, String> {
    result.map_err(|error| format!("{label}: {error}"))
}

fn expect_error<T>(
    result: Result<T, WifiError>,
    reason: ReasonCode,
    label: &str,
) -> Result<(), String> {
    match result {
        Err(WifiError(actual)) if actual == reason => Ok(()),
        Err(WifiError(actual)) => Err(format!("{label}: expected {reason:?}, got {actual:?}")),
        Ok(_) => Err(format!("{label}: expected {reason:?}, got success")),
    }
}

fn scan(manager: &mut WifiManager<GeneratedWifiBackend>, rescan: bool) -> Result<(), String> {
    expect_ok(manager.scan(ScanRequest { rescan }), "scan")
}

fn connect(
    manager: &mut WifiManager<GeneratedWifiBackend>,
    network_id: &str,
    confirmation: bool,
    credential_reference: Option<&str>,
) -> Result<(), String> {
    expect_ok(
        manager.connect(ConnectRequest {
            network_id: id(network_id)?,
            open_confirmation: confirmation,
            credential_reference: credential_reference.map(credential).transpose()?,
        }),
        "connect",
    )
}

fn successful_scan_select_connect() -> Result<(), String> {
    let mut manager = manager()?;
    expect_ok(manager.set_enabled(false), "disable radio")?;
    require(!manager.state().enabled, "disabled radio state")?;
    expect_error(
        manager.scan(ScanRequest { rescan: false }),
        ReasonCode::RadioUnavailable,
        "disabled radio scan",
    )?;
    expect_ok(manager.set_enabled(true), "enable radio")?;
    require(manager.state().enabled, "enabled radio state")?;
    scan(&mut manager, false)?;
    expect_ok(manager.select(&id("net-home-strong")?), "select WPA2")?;
    connect(&mut manager, "net-home-strong", false, Some("cred-home"))?;
    require(
        manager.state().phase == WifiPhase::Connected,
        "successful WPA2 connection",
    )?;
    require(
        manager.saved_state().networks.len() == 1,
        "saved WPA2 record",
    )
}

fn refresh() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    let before = manager.state().scan_results.clone();
    scan(&mut manager, true)?;
    require(
        manager.state().scan_results == before,
        "deterministic refresh",
    )
}

fn strongest_radio_collapse() -> Result<(), String> {
    let mut fixture = fixture()?;
    fixture
        .access_points
        .push(wifi_manager::GeneratedAccessPoint {
            network_id: id("net-home-unsupported")?,
            ssid: "Home Synthetic".into(),
            signal_quality: 100,
            security: Security::Unsupported,
            known: false,
            connected: true,
            hidden: false,
        });
    let mut manager = manager_from(fixture)?;
    scan(&mut manager, false)?;
    let results = &manager.state().scan_results;
    require(
        results
            .iter()
            .filter(|entry| entry.display_ssid == "Home Synthetic")
            .count()
            == 1,
        "duplicate collapse",
    )?;
    let home = results
        .iter()
        .find(|entry| entry.display_ssid == "Home Synthetic")
        .ok_or_else(|| "collapsed home entry missing".to_string())?;
    require(
        home.network_id == id("net-home-strong")?,
        "strongest compatible candidate",
    )?;
    require(
        home.signal_quality == 91 && home.security == Security::Wpa2Psk,
        "signal/security ordering",
    )?;
    require(home.known && home.connected, "merged known/connected flags")?;
    let names: Vec<_> = results
        .iter()
        .map(|entry| entry.display_ssid.as_str())
        .collect();
    require(
        names
            == vec![
                "Home Synthetic",
                "Known Synthetic",
                "Guest Synthetic",
                "Hidden network",
                "Bad Login Synthetic",
                "Timeout Synthetic",
                "Radio Failure Synthetic",
                "Unsupported Synthetic",
            ],
        "stable AP ordering",
    )
}

fn hidden_ssid() -> Result<(), String> {
    let mut manager = manager()?;
    let raw_ssid = "Hidden Journey SSID";
    expect_ok(
        manager.add_manual_network(ManualNetworkRequest {
            network_id: id("net-manual")?,
            ssid: raw_ssid.into(),
            security: Security::Wpa2Psk,
            hidden: true,
        }),
        "hidden manual network",
    )?;
    require(
        manager.state().scan_results[0].display_ssid == "Hidden network",
        "hidden display redaction",
    )?;
    require(
        manager.ssid_keyboard_request()
            == ScenePayload::Keyboard {
                request: wifi_manager::KeyboardInputRequest {
                    field: KeyboardField::Ssid,
                    input_kind: KeyboardInputKind::Text,
                    max_bytes: wifi_manager::MAX_SSID_BYTES,
                },
            },
        "text keyboard request",
    )?;
    require(
        manager.password_keyboard_request()
            == Some(ScenePayload::Keyboard {
                request: wifi_manager::KeyboardInputRequest {
                    field: KeyboardField::Password,
                    input_kind: KeyboardInputKind::Secret,
                    max_bytes: wifi_manager::MAX_CREDENTIAL_REFERENCE_BYTES,
                },
            }),
        "secret keyboard request",
    )?;
    let state = serde_json::to_string(manager.state()).map_err(error)?;
    let events = serde_json::to_string(&manager.take_events()).map_err(error)?;
    require(
        !state.contains(raw_ssid) && !events.contains(raw_ssid),
        "hidden SSID public redaction",
    )
}

fn bad_password() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    expect_ok(
        manager.select(&id("net-bad")?),
        "select bad-password journey",
    )?;
    expect_error(
        manager.connect(ConnectRequest {
            network_id: id("net-bad")?,
            open_confirmation: false,
            credential_reference: Some(credential("cred-bad")?),
        }),
        ReasonCode::BadCredentials,
        "bad-password outcome",
    )?;
    require(
        manager.state().phase == WifiPhase::Failed,
        "bad-password failure phase",
    )
}

fn timeout() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    expect_error(
        manager.connect(ConnectRequest {
            network_id: id("net-timeout")?,
            open_confirmation: false,
            credential_reference: Some(credential("cred-timeout")?),
        }),
        ReasonCode::Timeout,
        "timeout outcome",
    )
}

fn radio_unavailable() -> Result<(), String> {
    let mut unavailable = fixture()?;
    unavailable.radio_available = false;
    let mut unavailable_manager = manager_from(unavailable)?;
    expect_error(
        unavailable_manager.scan(ScanRequest { rescan: false }),
        ReasonCode::RadioUnavailable,
        "unavailable TG4040 adapter capability",
    )?;
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    expect_error(
        manager.connect(ConnectRequest {
            network_id: id("net-radio")?,
            open_confirmation: false,
            credential_reference: Some(credential("cred-radio")?),
        }),
        ReasonCode::RadioUnavailable,
        "radio-unavailable fixture outcome",
    )
}

fn cancellation() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    expect_ok(manager.cancel(), "cancel")?;
    require(
        manager.state().phase == WifiPhase::Cancelled,
        "cancelled phase",
    )?;
    expect_error(
        manager.scan(ScanRequest { rescan: true }),
        ReasonCode::Cancelled,
        "cancelled backend operation",
    )
}

fn reconnect() -> Result<(), String> {
    let mut empty = manager()?;
    scan(&mut empty, false)?;
    let conditions = ReconnectConditions {
        battery_percent: 80,
        suspended: false,
        gameplay_active: false,
        capability_available: true,
    };
    require(
        empty.auto_reconnect(conditions)
            == AutoReconnectDecision::Blocked(ReconnectBlock::NoSavedNetwork),
        "no saved candidate gate",
    )?;

    let mut manager = manager()?;
    scan(&mut manager, false)?;
    connect(&mut manager, "net-home-strong", false, Some("cred-home"))?;
    let saved = manager.saved_state();
    let mut restarted = WifiManager::from_saved_state(
        GeneratedWifiBackend::from_fixture(fixture()?).map_err(error)?,
        saved,
    )
    .map_err(error)?;
    scan(&mut restarted, true)?;
    restarted.set_automatic_reconnect(false);
    require(
        restarted.auto_reconnect(conditions)
            == AutoReconnectDecision::Blocked(ReconnectBlock::Disabled),
        "disabled reconnect policy gate",
    )?;
    restarted.set_automatic_reconnect(true);
    for condition in [
        ReconnectConditions {
            battery_percent: 10,
            ..conditions
        },
        ReconnectConditions {
            suspended: true,
            ..conditions
        },
        ReconnectConditions {
            gameplay_active: true,
            ..conditions
        },
        ReconnectConditions {
            capability_available: false,
            ..conditions
        },
    ] {
        require(
            matches!(
                restarted.auto_reconnect(condition),
                AutoReconnectDecision::Blocked(_)
            ),
            "reconnect safety gate",
        )?;
    }
    require(
        restarted.auto_reconnect(conditions) == AutoReconnectDecision::Attempted
            && restarted.state().phase == WifiPhase::Connected,
        "saved-network reconnect",
    )
}

fn disconnect() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    connect(&mut manager, "net-guest", true, None)?;
    expect_ok(manager.disconnect(), "disconnect")?;
    require(
        manager.state().connected_network_id.is_none(),
        "disconnect state",
    )
}

fn forget() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    connect(&mut manager, "net-guest", true, None)?;
    let guest = id("net-guest")?;
    expect_ok(manager.forget(&guest), "forget")?;
    require(
        manager.saved_state().networks.is_empty(),
        "forgotten persistence",
    )?;
    require(
        manager
            .state()
            .scan_results
            .iter()
            .all(|entry| entry.network_id != guest || !entry.known),
        "forgotten known flag",
    )
}

fn retry() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    expect_ok(manager.select(&id("net-timeout")?), "select retry journey")?;
    expect_error(
        manager.connect(ConnectRequest {
            network_id: id("net-timeout")?,
            open_confirmation: false,
            credential_reference: Some(credential("cred-timeout")?),
        }),
        ReasonCode::Timeout,
        "initial retry failure",
    )?;
    expect_error(manager.retry(), ReasonCode::Timeout, "retry outcome")
}

fn restart() -> Result<(), String> {
    let mut manager = manager()?;
    scan(&mut manager, false)?;
    connect(&mut manager, "net-known", false, Some("cred-known"))?;
    let saved = manager.saved_state();
    let serialized = serde_json::to_string(&saved).map_err(error)?;
    require(
        serialized.contains("cred-known"),
        "opaque reference persistence",
    )?;
    require(
        !serialized.contains("password") && !serialized.contains("bssid"),
        "persistence redaction",
    )?;
    let restarted = WifiManager::from_saved_state(
        GeneratedWifiBackend::from_fixture(fixture()?).map_err(error)?,
        saved,
    )
    .map_err(error)?;
    require(
        restarted.state().phase == WifiPhase::Idle,
        "restart resets transient state",
    )
}

fn negative_validation() -> Result<(), String> {
    for ssid in [
        "",
        "123456789012345678901234567890123",
        "bad\nssid",
        "/data/private",
        "aa:bb:cc:dd:ee:ff",
    ] {
        require(Ssid::new(ssid).is_err(), "malformed SSID rejection")?;
    }
    for reference in ["password", "cred-password", "cred-/private", "cred-../x"] {
        require(
            CredentialReference::new(reference).is_err(),
            "malformed reference rejection",
        )?;
    }
    require(
        NetworkId::new("not-opaque").is_err(),
        "malformed network ID rejection",
    )?;

    let mut manager = manager()?;
    scan(&mut manager, false)?;
    expect_error(
        manager.connect(ConnectRequest {
            network_id: id("net-unsupported")?,
            open_confirmation: true,
            credential_reference: None,
        }),
        ReasonCode::UnsupportedSecurity,
        "unsupported security",
    )?;
    connect(&mut manager, "net-known", false, Some("cred-known"))?;
    connect(&mut manager, "net-guest", true, None)
}
