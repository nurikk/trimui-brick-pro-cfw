use std::fmt::Display;

use virtual_keyboard::{AllowedChars, Button, FieldPolicy, Keyboard};
use wifi_manager::{
    AutoReconnectDecision, GeneratedWifiBackend, GeneratedWifiFixture, NetworkId, ReasonCode,
    ReconnectConditions, Security, WifiManager, WifiPhase,
};
use wifi_settings_controller::{
    ControllerAction, ControllerError, ControllerEvent, Metadata, View, WifiSettingsController,
};

const METADATA: &[u8] =
    include_bytes!("../../../../fixtures/wifi-settings-controller/generated-v1/workflow.json");
const JOURNEYS: &[u8] =
    include_bytes!("../../../../fixtures/wifi-settings-controller/generated-v1/journeys.json");
const WIFI_FIXTURE: &[u8] = include_bytes!("../../../../fixtures/wifi-manager/journeys.json");
const SCHEMA: &[u8] = include_bytes!("../../../../schemas/wifi-settings-controller-v1.schema.json");
const JOURNEY_NAMES: [&str; 15] = [
    "metadata-validation-negatives",
    "byte-identical-snapshots",
    "controller-navigation",
    "scan-progress-and-deduplication",
    "open-journey",
    "wpa2-journey",
    "wpa3-journey",
    "secret-keyboard-masking",
    "hidden-ssid-redaction",
    "saved-reference-only-restart-reconnect",
    "disconnect-forget",
    "bad-credential-timeout-radio-unsupported",
    "dhcp-dns-offline-and-utf8",
    "retry-cancel",
    "privacy-assertions",
];

fn main() -> Result<(), String> {
    let metadata = Metadata::from_json(METADATA).map_err(error)?;
    check_journey_manifest()?;
    check_schema()?;
    metadata_negatives(&metadata)?;
    deterministic_snapshot(&metadata)?;
    navigation(&metadata)?;
    scan_and_results(&metadata)?;
    open_journey(&metadata)?;
    wpa_journey(&metadata, Security::Wpa2Psk, "net-home-strong")?;
    wpa_journey(&metadata, Security::Wpa3Sae, "net-known")?;
    secret_masking(&metadata)?;
    hidden_redaction(&metadata)?;
    restart_reconnect(&metadata)?;
    disconnect_forget(&metadata)?;
    failure_journeys(&metadata)?;
    dhcp_dns_offline_and_utf8(&metadata)?;
    retry_cancel(&metadata)?;
    privacy(&metadata)?;
    println!(
        "wifi-settings-controller-fixtures: {} deterministic journeys passed",
        JOURNEY_NAMES.len()
    );
    Ok(())
}

fn controller(metadata: &Metadata) -> Result<WifiSettingsController, String> {
    let fixture: GeneratedWifiFixture = serde_json::from_slice(WIFI_FIXTURE).map_err(error)?;
    let backend = GeneratedWifiBackend::from_fixture(fixture).map_err(error)?;
    WifiSettingsController::new(metadata.clone(), WifiManager::new(backend), true).map_err(error)
}

fn scanned(metadata: &Metadata) -> Result<WifiSettingsController, String> {
    let mut controller = controller(metadata)?;
    activate(&mut controller, "scan")?;
    Ok(controller)
}

fn activate(controller: &mut WifiSettingsController, id: &str) -> Result<(), String> {
    let snapshot = controller.snapshot();
    let current = snapshot
        .menu
        .iter()
        .position(|item| item.focused)
        .ok_or("no focused menu item")?;
    let target = snapshot
        .menu
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| format!("missing metadata menu item {id}"))?;
    for _ in 0..(target + snapshot.menu.len() - current) % snapshot.menu.len() {
        controller.press(Button::Down).map_err(error)?;
    }
    controller.press(Button::Primary).map_err(error)
}

fn select(controller: &mut WifiSettingsController, id: &str) -> Result<(), String> {
    controller
        .dispatch(ControllerAction::SelectNetwork {
            network_id: NetworkId::new(id).map_err(error)?,
        })
        .map_err(error)
}

fn type_xx_chars(controller: &mut WifiSettingsController) -> Result<(), String> {
    controller.press(Button::Down).map_err(error)?;
    controller.press(Button::Down).map_err(error)?;
    controller.press(Button::Right).map_err(error)?;
    controller.press(Button::Primary).map_err(error)?;
    controller.press(Button::Primary).map_err(error)
}

fn type_xx(controller: &mut WifiSettingsController) -> Result<(), String> {
    type_xx_chars(controller)?;
    controller.press(Button::Start).map_err(error)
}

fn check_journey_manifest() -> Result<(), String> {
    let manifest: serde_json::Value = serde_json::from_slice(JOURNEYS).map_err(error)?;
    let names = manifest["journeys"]
        .as_array()
        .ok_or("journey list missing")?;
    if names.len() != JOURNEY_NAMES.len()
        || JOURNEY_NAMES
            .iter()
            .any(|name| !names.iter().any(|item| item == name))
    {
        return Err("journey manifest is incomplete".into());
    }
    if serde_json::to_vec(&manifest).map_err(error)?
        != serde_json::to_vec(&manifest).map_err(error)?
    {
        return Err("journey manifest is not deterministic".into());
    }
    Ok(())
}

fn check_schema() -> Result<(), String> {
    let schema: serde_json::Value = serde_json::from_slice(SCHEMA).map_err(error)?;
    if schema["additionalProperties"] != false
        || schema["properties"]["controls"]["items"]["$ref"] != "#/$defs/menu"
    {
        return Err("controller schema is not closed".into());
    }
    Ok(())
}

fn metadata_negatives(metadata: &Metadata) -> Result<(), String> {
    let mut duplicate_id = metadata.clone();
    duplicate_id.actions[0].id = duplicate_id.controls[0].id.clone();
    expect_metadata_error(duplicate_id, "duplicate menu id")?;

    let mut duplicate_order = metadata.clone();
    duplicate_order.actions[0].order = duplicate_order.controls[0].order;
    expect_metadata_error(duplicate_order, "duplicate menu order")?;

    let mut unsupported = metadata.clone();
    unsupported.security_choices[0] = Security::Unsupported;
    expect_metadata_error(unsupported, "unsupported security")?;

    let mut incomplete_projection = metadata.clone();
    incomplete_projection.snapshot.fields.pop();
    expect_metadata_error(incomplete_projection, "incomplete snapshot projection")?;

    let mut bad_policy = metadata.clone();
    bad_policy.manual_ssid.max_bytes = wifi_manager::MAX_SSID_BYTES + 1;
    expect_metadata_error(bad_policy, "SSID byte bound")?;

    let source = std::str::from_utf8(METADATA).map_err(error)?;
    let duplicate_json = source.replacen(
        "\"format\": \"trimui-wifi-settings-controller\"",
        "\"format\": \"trimui-wifi-settings-controller\", \"format\": \"trimui-wifi-settings-controller\"",
        1,
    );
    if Metadata::from_json(duplicate_json.as_bytes()).is_ok() {
        return Err("duplicate JSON key was accepted".into());
    }
    Ok(())
}

fn expect_metadata_error(metadata: Metadata, label: &str) -> Result<(), String> {
    metadata
        .validate()
        .is_err()
        .then_some(())
        .ok_or_else(|| format!("{label} was accepted"))
}

fn deterministic_snapshot(metadata: &Metadata) -> Result<(), String> {
    let controller = controller(metadata)?;
    let first = serde_json::to_vec(&controller.snapshot()).map_err(error)?;
    let second = serde_json::to_vec(&controller.snapshot()).map_err(error)?;
    if first != second {
        return Err("initial snapshots differ".into());
    }
    Ok(())
}

fn navigation(metadata: &Metadata) -> Result<(), String> {
    let mut controller = controller(metadata)?;
    controller.press(Button::Down).map_err(error)?;
    let snapshot = controller.snapshot();
    if snapshot
        .menu
        .iter()
        .find(|item| item.focused)
        .map(|item| item.id.as_str())
        != Some("automatic-reconnect")
    {
        return Err("metadata-driven navigation skipped the second control".into());
    }
    controller
        .dispatch(ControllerAction::SetSecurity {
            security: Security::Wpa3Sae,
        })
        .map_err(error)?;
    if controller.snapshot().security_choices != metadata.security_choices {
        return Err("security choices were not projected from metadata".into());
    }
    Ok(())
}

fn scan_and_results(metadata: &Metadata) -> Result<(), String> {
    let mut controller = scanned(metadata)?;
    let snapshot = controller.snapshot();
    if snapshot.view != View::Networks || snapshot.networks.len() != 11 {
        return Err("scan did not project deduplicated results".into());
    }
    let home = snapshot
        .networks
        .iter()
        .find(|network| network.display_ssid == "Home Synthetic")
        .ok_or("collapsed home network missing")?;
    if home.signal_quality != 91 || home.security != Security::Wpa2Psk || !home.known {
        return Err("manager result collapse/status was not preserved".into());
    }
    if !controller
        .drain_events()
        .iter()
        .any(|event| matches!(event, ControllerEvent::ScanProgress))
    {
        return Err("scan progress event missing".into());
    }
    Ok(())
}

fn open_journey(metadata: &Metadata) -> Result<(), String> {
    let mut controller = scanned(metadata)?;
    select(&mut controller, "net-guest")?;
    expect_reason(controller.connect(), ReasonCode::ConfirmationRequired)?;
    if !controller.snapshot().open_confirmation {
        return Err("open confirmation was not requested".into());
    }
    controller.confirm_open().map_err(error)?;
    if controller.snapshot().phase != WifiPhase::Internet {
        return Err("open network did not connect".into());
    }
    Ok(())
}

fn wpa_journey(metadata: &Metadata, security: Security, id: &str) -> Result<(), String> {
    let mut controller = scanned(metadata)?;
    controller
        .dispatch(ControllerAction::SetSecurity { security })
        .map_err(error)?;
    select(&mut controller, id)?;
    if !controller
        .snapshot()
        .keyboard
        .as_ref()
        .is_some_and(|request| request.masked)
    {
        return Err("WPA selection did not request masked key input".into());
    }
    type_xx(&mut controller)?;
    if controller.snapshot().phase != WifiPhase::Internet {
        return Err("WPA network did not connect".into());
    }
    if !controller.drain_events().iter().any(|event| {
        matches!(
            event,
            ControllerEvent::PhaseChanged {
                phase: WifiPhase::Associating,
                ..
            }
        )
    }) {
        return Err("connecting progress event missing".into());
    }
    Ok(())
}

fn secret_masking(metadata: &Metadata) -> Result<(), String> {
    let mut controller = scanned(metadata)?;
    select(&mut controller, "net-known")?;
    let before = controller
        .snapshot()
        .keyboard
        .ok_or("secret keyboard missing")?;
    if !before.masked || before.length_scalars != 0 {
        return Err("secret keyboard was not masked".into());
    }
    type_xx_chars(&mut controller)?;
    let after = controller
        .snapshot()
        .keyboard
        .ok_or("secret keyboard ended early")?;
    if !after.masked || after.length_scalars != 2 {
        return Err("secret keyboard exposed an unmasked value".into());
    }
    controller.press(Button::Start).map_err(error)?;
    Ok(())
}

fn hidden_redaction(metadata: &Metadata) -> Result<(), String> {
    let mut controller = controller(metadata)?;
    controller
        .dispatch(ControllerAction::SetSecurity {
            security: Security::Wpa2Psk,
        })
        .map_err(error)?;
    controller
        .dispatch(ControllerAction::OpenManual)
        .map_err(error)?;
    type_xx(&mut controller)?;
    let snapshot = serde_json::to_string(&controller.snapshot()).map_err(error)?;
    let events = serde_json::to_string(&controller.drain_events()).map_err(error)?;
    if snapshot.contains("Network name") || snapshot.contains("xx") || events.contains("xx") {
        return Err("hidden/manual SSID leaked through public output".into());
    }
    if controller.snapshot().networks[0].display_ssid != "Hidden network" {
        return Err("manual SSID did not become Hidden network".into());
    }
    Ok(())
}

fn restart_reconnect(metadata: &Metadata) -> Result<(), String> {
    let mut first = scanned(metadata)?;
    select(&mut first, "net-known")?;
    type_xx(&mut first)?;
    let saved = first.saved_state();
    let persisted = serde_json::to_string(&saved).map_err(error)?;
    if !persisted.contains("cred-fixture-reference") || persisted.contains("xx") {
        return Err("saved persistence was not reference-only".into());
    }
    let fixture: GeneratedWifiFixture = serde_json::from_slice(WIFI_FIXTURE).map_err(error)?;
    let backend = GeneratedWifiBackend::from_fixture(fixture).map_err(error)?;
    let mut restarted = WifiSettingsController::new(
        Metadata::from_json(METADATA).map_err(error)?,
        WifiManager::from_saved_state(backend, saved).map_err(error)?,
        true,
    )
    .map_err(error)?;
    activate(&mut restarted, "scan")?;
    let decision = restarted.auto_reconnect(ReconnectConditions {
        battery_percent: 80,
        suspended: false,
        gameplay_active: false,
        capability_available: true,
    });
    if decision != AutoReconnectDecision::Attempted
        || restarted.snapshot().phase != WifiPhase::Internet
    {
        return Err("saved reference did not reconnect through manager policy".into());
    }
    Ok(())
}

fn disconnect_forget(metadata: &Metadata) -> Result<(), String> {
    let mut controller = scanned(metadata)?;
    select(&mut controller, "net-guest")?;
    controller.confirm_open().err();
    expect_reason(controller.connect(), ReasonCode::ConfirmationRequired)?;
    controller.confirm_open().map_err(error)?;
    controller.disconnect().map_err(error)?;
    if !controller.drain_events().iter().any(|event| {
        matches!(
            event,
            ControllerEvent::ConnectionChanged { connected: false }
        )
    }) {
        return Err("disconnect event missing".into());
    }
    controller.forget().map_err(error)?;
    if controller.snapshot().saved_network_count != 0 || controller.snapshot().selected_saved {
        return Err("disconnect/forget did not clear saved state".into());
    }
    Ok(())
}

fn failure_journeys(metadata: &Metadata) -> Result<(), String> {
    let mut bad = scanned(metadata)?;
    select(&mut bad, "net-bad")?;
    expect_reason_text(type_xx(&mut bad), ReasonCode::BadCredentials)?;

    let mut timeout = scanned(metadata)?;
    select(&mut timeout, "net-timeout")?;
    expect_reason_text(type_xx(&mut timeout), ReasonCode::Timeout)?;

    let mut unsupported = scanned(metadata)?;
    select(&mut unsupported, "net-unsupported")?;
    expect_reason(unsupported.connect(), ReasonCode::UnsupportedSecurity)?;

    let mut radio = scanned(metadata)?;
    select(&mut radio, "net-radio")?;
    expect_reason_text(type_xx(&mut radio), ReasonCode::RadioUnavailable)?;

    let fixture: GeneratedWifiFixture = serde_json::from_slice(WIFI_FIXTURE).map_err(error)?;
    let backend = GeneratedWifiBackend::from_fixture(GeneratedWifiFixture {
        radio_available: false,
        ..fixture
    })
    .map_err(error)?;
    let mut unavailable =
        WifiSettingsController::new(metadata.clone(), WifiManager::new(backend), true)
            .map_err(error)?;
    expect_reason(unavailable.scan(), ReasonCode::RadioUnavailable)
}

fn dhcp_dns_offline_and_utf8(metadata: &Metadata) -> Result<(), String> {
    for (network_id, reason, lan) in [
        ("net-dhcp", ReasonCode::DhcpFailed, false),
        ("net-dns", ReasonCode::DnsFailed, true),
        ("net-offline", ReasonCode::NoInternet, true),
    ] {
        let mut controller = scanned(metadata)?;
        select(&mut controller, network_id)?;
        expect_reason_text(type_xx(&mut controller), reason)?;
        let snapshot = controller.snapshot();
        if snapshot.reason != Some(reason)
            || (lan
                && (snapshot.phase != WifiPhase::Lan
                    || !snapshot
                        .selected_network
                        .is_some_and(|network| network.connected)))
        {
            return Err("connectivity failure lost its actionable state".into());
        }
    }
    for value in ["Café # Wi-Fi", "space name", "quote \" #", "日本語"] {
        Keyboard::new(FieldPolicy::secret(
            value,
            "Network key",
            63,
            63,
            AllowedChars::any(),
        ))
        .map_err(error)?;
    }
    Ok(())
}

fn retry_cancel(metadata: &Metadata) -> Result<(), String> {
    let mut retry = scanned(metadata)?;
    select(&mut retry, "net-timeout")?;
    expect_reason_text(type_xx(&mut retry), ReasonCode::Timeout)?;
    expect_reason(retry.retry(), ReasonCode::Timeout)?;

    let mut cancelled = scanned(metadata)?;
    select(&mut cancelled, "net-known")?;
    cancelled.cancel().map_err(error)?;
    if cancelled.snapshot().phase != WifiPhase::Cancelled {
        return Err("cancel did not expose cancelled phase".into());
    }
    if !cancelled.saved_state().networks.is_empty() {
        return Err("cancel persisted incomplete connection".into());
    }
    Ok(())
}

fn privacy(metadata: &Metadata) -> Result<(), String> {
    let mut controller = scanned(metadata)?;
    select(&mut controller, "net-known")?;
    type_xx(&mut controller)?;
    let debug = format!("{controller:?}");
    let snapshot = serde_json::to_string(&controller.snapshot()).map_err(error)?;
    let events = serde_json::to_string(&controller.drain_events()).map_err(error)?;
    if debug.contains("cred-fixture-reference")
        || snapshot.contains("cred-")
        || events.contains("cred-")
        || debug.contains("xx")
    {
        return Err("public controller output leaked secret/reference material".into());
    }
    let saved_debug = format!("{:?}", controller.saved_state());
    if saved_debug.contains("cred-fixture-reference") {
        return Err("saved-state Debug leaked credential reference".into());
    }
    Ok(())
}

fn expect_reason_text<T>(result: Result<T, String>, reason: ReasonCode) -> Result<(), String> {
    match result {
        Err(error) if error.contains(&format!("{reason:?}")) => Ok(()),
        Err(error) => Err(format!("expected {reason:?}, got {error}")),
        Ok(_) => Err(format!("expected {reason:?}, got success")),
    }
}

fn expect_reason<T>(result: Result<T, ControllerError>, reason: ReasonCode) -> Result<(), String> {
    match result {
        Err(ControllerError::Manager(actual) | ControllerError::InvalidInput(actual))
            if actual == reason =>
        {
            Ok(())
        }
        Err(error) => Err(format!("expected {reason:?}, got {error}")),
        Ok(_) => Err(format!("expected {reason:?}, got success")),
    }
}

fn error(error: impl Display) -> String {
    error.to_string()
}
