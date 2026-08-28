mod control;

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use launch_contract::{
    validate as validate_launch_request, Catalog as LaunchCatalog, DisplaySettings, InputLayout,
    InputSettings, LaunchKind, LaunchRequest, LogicalPath, PathRoot, PowerSettings, ResumeMode,
    Scaling, SuspendMode, VersionedId,
};
use launcher_presentation::Screen as PresentationScreen;
use launcher_theme::ValidatedTheme;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use settings_schema::{ProjectionContext, Registry};
use settings_ui::SettingsUi;
use sim_domain::{Catalog as UiCatalog, Route, SessionState};
use sim_platform_contract::{
    Button, ButtonAction, ButtonEvent, HardwareChanges, Platform, PlatformResult, StorageMode,
    SuspendResult, SuspendState,
};
use ui_model::{Action as UiAction, PlatformCapabilities as UiCapabilities};
use wifi_manager::{GeneratedWifiBackend, WifiManager};
use wifi_settings_controller::{Metadata as WifiMetadata, WifiSettingsController};

const LANE: &str = "host-native userspace simulator";
const SESSION_ID: &str = "run-local";
const LAUNCH_CATALOG_BYTES: &[u8] =
    include_bytes!("../../../fixtures/launch-contract/generated-v1/catalog.synthetic.json");
const GENERATED_CONTENT_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const MAX_GENERATED_ENTRIES: usize = 32;
const SETTINGS_REGISTRY_BYTES: &[u8] =
    include_bytes!("../../../fixtures/settings-schema/registry-v1.json");
const WIFI_METADATA_BYTES: &[u8] =
    include_bytes!("../../../fixtures/wifi-settings-controller/generated-v1/workflow.json");
const WIFI_FIXTURE_BYTES: &[u8] = include_bytes!("../../../fixtures/wifi-manager/journeys.json");
const FAULTS: &[&str] = &[
    "adapter-fail",
    "adapter-crash",
    "input-drop",
    "suspend-fail",
];

struct Evidence {
    root: PathBuf,
    screenshots: PathBuf,
    checkpoints: PathBuf,
}

struct EventLog {
    file: File,
    sequence: u64,
    run_id: String,
}

#[derive(Serialize)]
struct Readiness<'a> {
    schema: &'a str,
    lane: &'a str,
    #[serde(rename = "targetSku")]
    target_sku: &'a str,
    #[serde(rename = "runId")]
    run_id: &'a str,
    ready: bool,
    #[serde(rename = "elapsedMs")]
    elapsed_ms: u64,
    reason: &'a str,
}

#[derive(Serialize)]
struct ExitStatus<'a> {
    lane: &'a str,
    #[serde(rename = "sessionId")]
    session_id: &'a str,
    #[serde(rename = "runId")]
    run_id: &'a str,
    #[serde(rename = "exitCode")]
    exit_code: i32,
    #[serde(rename = "cleanShutdown")]
    clean_shutdown: bool,
}

#[derive(Clone, Serialize)]
struct FakeSession {
    id: String,
    #[serde(rename = "contentId")]
    content_id: String,
    status: String,
    #[serde(rename = "exitStatus")]
    exit_status: Option<u8>,
    value: Option<i32>,
}

struct AppState {
    route: Route,
    selected_index: usize,
    active_fake_session: Option<FakeSession>,
    modal: Option<String>,
    faults: Vec<String>,
    readiness_generation: u64,
    presentation: PresentationState,
}

struct PresentationState {
    ui: ui_model::UiState,
    theme: ValidatedTheme,
    theme_fallback: Option<launcher_theme::Reason>,
    settings: SettingsUi,
    wifi: WifiSettingsController,
}

impl PresentationState {
    fn new() -> Result<Self> {
        let mut ui = ui_model::UiState::generated();
        ui = ui_model::reduce(
            &ui,
            UiAction::SetCapabilities(UiCapabilities {
                catalog: true,
                favorites: true,
                settings_persistence: true,
                session: true,
                scraper: true,
                wifi: true,
            }),
        );
        let registry = Registry::from_json(SETTINGS_REGISTRY_BYTES)
            .map_err(|error| anyhow!(error.to_string()))?;
        let mut context = ProjectionContext::default();
        context.capabilities.extend([
            "audio".into(),
            "network".into(),
            "theme-engine".into(),
            "wifi".into(),
            "scraper".into(),
        ]);
        let settings =
            SettingsUi::new(registry, context).map_err(|error| anyhow!(error.to_string()))?;
        let projection = settings
            .scene()
            .map_err(|error| anyhow!(error.to_string()))?;
        let entries = projection
            .sections
            .iter()
            .flat_map(|section| section.groups.iter())
            .flat_map(|group| group.controls.iter())
            .map(|control| ui_model::MenuEntry {
                id: ui_model::MenuId::new(control.setting_id.clone()),
                label: control.label_key.clone(),
                command: ui_model::MenuCommand::Navigate(ui_model::Route::Settings),
                enabled: control.enabled,
                disabled_reason: None,
                selected: false,
            })
            .collect();
        ui = ui_model::reduce(&ui, UiAction::SetSettingsMenuProjection { entries });
        let metadata = WifiMetadata::from_json(WIFI_METADATA_BYTES)
            .map_err(|error| anyhow!(error.to_string()))?;
        let backend = GeneratedWifiBackend::from_json(WIFI_FIXTURE_BYTES)
            .map_err(|error| anyhow!(error.to_string()))?;
        let wifi = WifiSettingsController::new(metadata, WifiManager::new(backend), true)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(Self {
            ui,
            theme: launcher_theme::safe_artbook().map_err(|error| anyhow!(error.to_string()))?,
            theme_fallback: None,
            settings,
            wifi,
        })
    }

    fn screen(&self) -> Result<PresentationScreen> {
        let settings = self
            .settings
            .scene()
            .map_err(|error| anyhow!(error.to_string()))?;
        let wifi = self.wifi.snapshot();
        Ok(launcher_presentation::build(
            &self.ui,
            &self.theme,
            self.theme_fallback,
            Some(&settings),
            Some(&wifi),
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ButtonArgs {
    button: Button,
    action: ButtonAction,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    #[serde(rename = "timeoutMs")]
    timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatteryChanges {
    percent: Option<u8>,
    charging: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageChanges {
    mode: Option<StorageMode>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RadioChanges {
    enabled: Option<bool>,
    connected: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuspendChanges {
    state: Option<SuspendState>,
    result: Option<SuspendResult>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HardwareArgs {
    battery: Option<BatteryChanges>,
    storage: Option<StorageChanges>,
    radio: Option<RadioChanges>,
    suspend: Option<SuspendChanges>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultArgs {
    name: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterArgs {
    action: String,
    status: u8,
    value: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactArgs {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresentationArgs {
    action: String,
}

const MAX_ADAPTER_VALUE: i32 = 1_000_000;

pub fn run<P, F>(
    catalog_path: &Path,
    evidence_path: &Path,
    keep_alive: bool,
    stop: &AtomicBool,
    make_platform: F,
) -> Result<()>
where
    P: Platform,
    F: FnOnce() -> PlatformResult<P>,
{
    let evidence = Evidence::new(evidence_path)?;
    let run_id = format!(
        "run-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let mut log = EventLog::new(&evidence.root, &run_id)?;
    let mut server = control::ControlServer::bind(&evidence.root)?;
    let result = make_platform()
        .map_err(|error| anyhow!("{error}"))
        .and_then(|platform| {
            run_session(
                platform,
                catalog_path,
                &evidence,
                &mut log,
                &mut server,
                keep_alive,
                stop,
            )
        });
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = log.emit("fatal_startup", 0, Map::new());
            let _ = write_json(
                evidence.root.join("readiness.json"),
                &Readiness {
                    schema: "sim-readiness/v1",
                    lane: LANE,
                    target_sku: "unknown",
                    run_id: &run_id,
                    ready: false,
                    elapsed_ms: 0,
                    reason: "startup-failed",
                },
            );
            let _ = write_json(
                evidence.root.join("exit-status.json"),
                &ExitStatus {
                    lane: LANE,
                    session_id: SESSION_ID,
                    run_id: &run_id,
                    exit_code: 1,
                    clean_shutdown: false,
                },
            );
            Err(error)
        }
    }
}

fn run_session<P: Platform>(
    mut platform: P,
    catalog_path: &Path,
    evidence: &Evidence,
    log: &mut EventLog,
    server: &mut control::ControlServer,
    keep_alive: bool,
    stop: &AtomicBool,
) -> Result<()> {
    let startup_started = Instant::now();
    let identity = platform.identity();
    let catalog_started = Instant::now();
    let launch_catalog: LaunchCatalog = launch_contract::parse_catalog_json(LAUNCH_CATALOG_BYTES)
        .map_err(|error| anyhow!(error.to_string()))?;
    launch_contract::validate_catalog_projection(&launch_catalog)
        .map_err(|error| anyhow!(error.to_string()))?;
    let catalog: UiCatalog =
        serde_json::from_slice(&fs::read(catalog_path)?).context("read generated catalog")?;
    if catalog.catalog_version != "1"
        || catalog.entries.is_empty()
        || catalog.entries.len() > MAX_GENERATED_ENTRIES
        || catalog.entries.iter().any(|entry| {
            entry.id.is_empty()
                || entry.id.len() > 64
                || entry.title.is_empty()
                || entry.title.len() > 128
                || entry.system != "synthetic"
                || !entry.id.starts_with("generated-")
        })
    {
        return Err(anyhow!("invalid generated catalog"));
    }
    log.emit(
        "catalog_list",
        platform.logical_time_ms(),
        json_map([
            ("entryCount", json!(catalog.entries.len())),
            ("latencyUs", json!(catalog_started.elapsed().as_micros())),
        ]),
    )?;
    let mut state = AppState {
        route: Route::Library,
        selected_index: 0,
        active_fake_session: None,
        modal: None,
        faults: Vec::new(),
        readiness_generation: 1,
        presentation: PresentationState::new()?,
    };
    refresh_presentation_affordances(&mut state.presentation, &platform)
        .map_err(|error| anyhow!(error))?;
    let screen = state.presentation.screen()?;

    present(&mut platform, &screen)?;
    let first_frame_us = startup_started.elapsed().as_micros();
    log.emit("ready", platform.logical_time_ms(), Map::new())?;
    write_json(
        evidence.root.join("readiness.json"),
        &Readiness {
            schema: "sim-readiness/v1",
            lane: LANE,
            target_sku: &identity.target_sku,
            run_id: &log.run_id,
            ready: true,
            elapsed_ms: 0,
            reason: "ready",
        },
    )?;
    let snapshot = platform.snapshot().map_err(|error| anyhow!(error))?;
    let first_frame_sequence = log.emit(
        "first_frame",
        platform.logical_time_ms(),
        json_map([
            ("logicalWidth", json!(1024)),
            ("logicalHeight", json!(768)),
            ("hostElapsedUs", json!(first_frame_us)),
            ("batteryLevelPercent", json!(snapshot.battery_level_percent)),
            ("charging", json!(snapshot.charging)),
            ("ledOn", json!(snapshot.led_on)),
            ("audioEnabled", json!(snapshot.audio_enabled)),
            ("radioEnabled", json!(snapshot.radio_enabled)),
            ("suspended", json!(snapshot.suspended)),
        ]),
    )?;
    capture_artifact(
        &mut platform,
        evidence,
        log,
        &state,
        &catalog,
        "screenshots",
        &format!("screen-{first_frame_sequence}"),
        "screenshot",
    )
    .map_err(|error| anyhow!(error))?;
    state.presentation.ui = ui_model::reduce(&state.presentation.ui, UiAction::FinishSplash);
    let initial_selection = route_selection(&state.route, &catalog, state.selected_index);
    write_route(&evidence.root, &state.route, initial_selection)?;
    emit_route_selection(
        log,
        platform.logical_time_ms(),
        &state.route,
        initial_selection,
    )?;

    let mut fixture_done = false;
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let mut did_work = false;
        if !fixture_done {
            if let Some(event) = platform
                .next_button_event()
                .map_err(|error| anyhow!(error))?
            {
                handle_button(
                    &mut platform,
                    evidence,
                    log,
                    &catalog,
                    &launch_catalog,
                    &mut state,
                    event,
                    &identity.target_sku,
                )?;
                did_work = true;
            } else {
                fixture_done = true;
            }
        }
        if let Some(incoming) = server.poll() {
            handle_request(
                &mut platform,
                evidence,
                log,
                &catalog,
                &launch_catalog,
                &mut state,
                incoming,
                &identity.target_sku,
            )?;
            did_work = true;
        }
        if !keep_alive && fixture_done && !did_work {
            break;
        }
        if !did_work {
            thread::sleep(Duration::from_millis(10));
        }
    }

    if state.active_fake_session.is_some() {
        write_session(&evidence.root, session_state(&state))?;
    }
    log.emit("clean_shutdown", platform.logical_time_ms(), Map::new())?;
    write_json(
        evidence.root.join("exit-status.json"),
        &ExitStatus {
            lane: LANE,
            session_id: SESSION_ID,
            run_id: &log.run_id,
            exit_code: 0,
            clean_shutdown: true,
        },
    )?;
    server.remove();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_request<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    state: &mut AppState,
    incoming: control::Incoming,
    target_sku: &str,
) -> Result<()> {
    let mut stream = incoming.stream;
    let request = match incoming.request {
        Ok(request) => request,
        Err(message) => {
            control::send_error(&mut stream, "", "protocol_rejected", &message)?;
            return Ok(());
        }
    };
    let id = request.id.clone();
    if request.version != control::PROTOCOL_VERSION
        || id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        control::send_error(
            &mut stream,
            &id,
            "protocol_rejected",
            "invalid version or request id",
        )?;
        return Ok(());
    }
    let result = match request.command.as_str() {
        "wait-ready" => wait_ready(request.args),
        "state" => parse_empty(request.args)
            .and_then(|_| state_json(platform, evidence, log, catalog, state)),
        "button" => parse::<ButtonArgs>(request.args).and_then(|args| {
            let event = ButtonEvent {
                at_ms: platform.logical_time_ms().saturating_add(1),
                button: args.button,
                action: args.action,
            };
            handle_button(
                platform,
                evidence,
                log,
                catalog,
                launch_catalog,
                state,
                event,
                target_sku,
            )
            .map_err(|error| error.to_string())
            .and_then(|_| state_json(platform, evidence, log, catalog, state))
        }),
        "hardware.set" => parse::<HardwareArgs>(request.args).and_then(|args| {
            apply_hardware(platform, log, args)?;
            refresh_presentation_affordances(&mut state.presentation, platform)?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "fault.set" => parse::<FaultArgs>(request.args).and_then(|args| {
            set_fault(log, state, args)?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "adapter" => parse::<AdapterArgs>(request.args).and_then(|args| {
            adapter_result(log, state, args)?;
            write_session(&evidence.root, session_state(state))
                .map_err(|error| error.to_string())?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "presentation" => parse::<PresentationArgs>(request.args).and_then(|args| {
            presentation_action(state, args)?;
            let screen = state
                .presentation
                .screen()
                .map_err(|error| error.to_string())?;
            present(platform, &screen).map_err(|error| error.to_string())?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "screenshot" => parse::<ArtifactArgs>(request.args).and_then(|args| {
            capture_artifact(
                platform,
                evidence,
                log,
                state,
                catalog,
                "screenshots",
                &args.name,
                "screenshot",
            )
        }),
        "checkpoint" => parse::<ArtifactArgs>(request.args).and_then(|args| {
            capture_artifact(
                platform,
                evidence,
                log,
                state,
                catalog,
                "checkpoints",
                &args.name,
                "checkpoint",
            )
        }),
        _ => Err("unknown command".to_string()),
    };
    match result {
        Ok(value) => control::send_ok(&mut stream, &id, &value)?,
        Err(message) => control::send_error(&mut stream, &id, "protocol_rejected", &message)?,
    }
    Ok(())
}

fn refresh_presentation_affordances<P: Platform>(
    state: &mut PresentationState,
    platform: &P,
) -> Result<(), String> {
    let snapshot = platform.snapshot().map_err(|error| error.to_string())?;
    let mut affordances = state.ui.affordances.clone();
    affordances.battery.percent = snapshot.battery_level_percent;
    affordances.battery.charging = snapshot.charging;
    state.ui = ui_model::reduce(&state.ui, UiAction::SetAffordances(affordances));
    Ok(())
}

fn presentation_action(state: &mut AppState, args: PresentationArgs) -> Result<(), String> {
    use ui_model::{Action, AmbiguousChoice, GameId, ScraperAction, ScraperProgress, WifiAction};

    let action = args.action.as_str();
    match action {
        "home" => reduce_route(state, ui_model::Route::Home),
        "systems" => reduce_route(state, ui_model::Route::Systems),
        "games" => reduce_route(state, ui_model::Route::Games),
        "favorites" => reduce_route(state, ui_model::Route::Favorites),
        "search" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::SetSearchQuery {
                    query: "Generated".into(),
                },
            );
        }
        "settings" => reduce_route(state, ui_model::Route::Settings),
        "settings-form" => {
            reduce_route(state, ui_model::Route::Settings);
            state
                .presentation
                .settings
                .press(virtual_keyboard::Button::Primary)
                .map_err(|e| e.to_string())?;
        }
        "recovery" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::ShowFallback {
                    reason: ui_model::FallbackReason::MissingContent,
                },
            );
        }
        "modal" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::ShowModal(ui_model::ModalState::Info {
                    title: "Generated notice".into(),
                    message: "Project-authored simulator modal".into(),
                }),
            );
        }
        "scraper-settings" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenSettings),
            )
        }
        "scraper-game" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenGame {
                    game_id: GameId::new("generated-game-01"),
                }),
            )
        }
        "scraper-queue" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::QueueGame {
                    game_id: GameId::new("generated-game-01"),
                }),
            )
        }
        "scraper-progress" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenBulkQueue),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::SetProgress(ScraperProgress {
                    completed: 1,
                    total: 3,
                    paused: false,
                })),
            )
        }
        "scraper-paused" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::Pause),
            )
        }
        "scraper-resumed" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::Resume),
            )
        }
        "scraper-ambiguity" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenAmbiguousChoice(AmbiguousChoice {
                    game_id: GameId::new("generated-game-01"),
                    candidates: vec!["Generated Match A".into(), "Generated Match B".into()],
                })),
            )
        }
        "scraper-complete" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::Complete),
            )
        }
        "scraper-cancel" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::Cancel),
            )
        }
        "wifi-scan" => {
            state.presentation.wifi.scan().map_err(|e| e.to_string())?;
            state.presentation.ui =
                ui_model::reduce(&state.presentation.ui, Action::Wifi(WifiAction::OpenScan));
        }
        "wifi-access-points" => {
            state.presentation.wifi.scan().map_err(|e| e.to_string())?;
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Wifi(WifiAction::SetAccessPoints {
                    access_points: wifi_access_points(&state.presentation.wifi),
                }),
            );
        }
        "wifi-password" => {
            state.presentation.wifi.scan().map_err(|e| e.to_string())?;
            state
                .presentation
                .wifi
                .press(virtual_keyboard::Button::Primary)
                .map_err(|e| e.to_string())?;
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Wifi(WifiAction::EnterSsid {
                    mode: ui_model::SsidEntryMode::Manual,
                    ssid: "Home Synthetic".into(),
                }),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Wifi(WifiAction::RequestMaskedPasswordKeyboard {
                    ssid: "Home Synthetic".into(),
                }),
            );
        }
        "wifi-hidden" => {
            state
                .presentation
                .wifi
                .open_manual()
                .map_err(|e| e.to_string())?;
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Wifi(WifiAction::OpenHiddenNetwork),
            );
        }
        "wifi-manual" => {
            state
                .presentation
                .wifi
                .open_manual()
                .map_err(|e| e.to_string())?;
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Wifi(WifiAction::OpenManualSsid),
            );
        }
        "wifi-progress" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Wifi(WifiAction::Connect {
                    ssid: "Home Synthetic".into(),
                }),
            )
        }
        "wifi-error" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Wifi(WifiAction::Error {
                    message: "generated-radio-unavailable".into(),
                }),
            )
        }
        "fallback" => {
            state.presentation.theme_fallback = Some(launcher_theme::Reason::MissingTheme);
            state.presentation.ui = ui_model::reduce(&state.presentation.ui, Action::DismissModal);
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::ShowFallback {
                    reason: ui_model::FallbackReason::InvalidState,
                },
            );
        }
        _ => return Err("presentation action is not allowlisted".into()),
    }
    Ok(())
}

fn reduce_route(state: &mut AppState, route: ui_model::Route) {
    state.presentation.ui =
        ui_model::reduce(&state.presentation.ui, ui_model::Action::Navigate(route));
}

fn wifi_access_points(state: &WifiSettingsController) -> Vec<ui_model::AccessPoint> {
    state
        .snapshot()
        .networks
        .into_iter()
        .map(|network| ui_model::AccessPoint {
            ssid: network.display_ssid,
            signal_percent: network.signal_quality,
            secured: network.security != wifi_manager::Security::Open,
        })
        .collect()
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, String> {
    if !args.is_object() {
        return Err("args must be an object".to_string());
    }
    serde_json::from_value(args).map_err(|error| format!("invalid command arguments: {error}"))
}

fn parse_empty(args: Value) -> Result<EmptyArgs, String> {
    parse(args)
}

fn wait_ready(args: Value) -> Result<Value, String> {
    let args: WaitArgs = parse(args)?;
    if args.timeout_ms == 0 || args.timeout_ms > control::MAX_TIMEOUT_MS {
        return Err("timeoutMs must be between 1 and 30000".to_string());
    }
    Ok(json!({"ready": true, "generation": 1}))
}

fn apply_hardware<P: Platform>(
    platform: &mut P,
    log: &mut EventLog,
    args: HardwareArgs,
) -> Result<Value, String> {
    let mut changes = HardwareChanges::default();
    let mut changed = false;
    if let Some(battery) = args.battery {
        if battery.percent.is_some_and(|value| value > 100) {
            return Err("battery percent must be between 0 and 100".to_string());
        }
        if let Some(value) = battery.percent {
            changes.battery_percent = Some(value);
            changed = true;
        }
        if let Some(value) = battery.charging {
            changes.charging = Some(value);
            changed = true;
        }
    }
    if let Some(value) = args.storage.and_then(|storage| storage.mode) {
        changes.storage_mode = Some(value);
        changed = true;
    }
    if let Some(radio) = args.radio {
        if let Some(value) = radio.enabled {
            changes.radio_enabled = Some(value);
            changed = true;
        }
        if let Some(value) = radio.connected {
            changes.radio_connected = Some(value);
            changed = true;
        }
    }
    if let Some(suspend) = args.suspend {
        if let Some(value) = suspend.state {
            changes.suspend_state = Some(value);
            changed = true;
        }
        if let Some(value) = suspend.result {
            changes.suspend_result = Some(value);
            changed = true;
        }
    }
    if !changed {
        return Err("hardware.set requires at least one typed field".to_string());
    }
    platform
        .mutate_hardware(changes)
        .map_err(|error| error.to_string())?;
    let hardware = platform
        .hardware_state()
        .map_err(|error| error.to_string())?;
    log.emit(
        "hardware",
        platform.logical_time_ms(),
        json_map([("hardware", hardware_json(&hardware))]),
    )
    .map_err(|error| error.to_string())?;
    Ok(hardware_json(&hardware))
}

fn set_fault(log: &mut EventLog, state: &mut AppState, args: FaultArgs) -> Result<(), String> {
    if !FAULTS.contains(&args.name.as_str()) {
        return Err("fault name is not allowlisted".to_string());
    }
    if args.enabled {
        if !state.faults.contains(&args.name) {
            state.faults.push(args.name.clone());
        }
    } else {
        state.faults.retain(|fault| fault != &args.name);
    }
    state.faults.sort();
    log.emit(
        "fault",
        0,
        json_map([("name", json!(args.name)), ("enabled", json!(args.enabled))]),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn adapter_result(
    log: &mut EventLog,
    state: &mut AppState,
    args: AdapterArgs,
) -> Result<(), String> {
    if !["complete", "fail", "exit", "crash"].contains(&args.action.as_str()) {
        return Err("adapter action must be complete, fail, exit, or crash".to_string());
    }
    if args.value.abs_diff(0) > MAX_ADAPTER_VALUE as u32 {
        return Err("adapter value must be between -1000000 and 1000000".to_string());
    }
    if state
        .active_fake_session
        .as_ref()
        .is_some_and(|session| session.status != "started")
    {
        return Err("fake session is already finished".to_string());
    }
    let Some(session) = state.active_fake_session.as_mut() else {
        return Err("no active fake session".to_string());
    };
    let failed = matches!(args.action.as_str(), "fail" | "crash")
        || state
            .faults
            .iter()
            .any(|fault| fault == "adapter-fail" || fault == "adapter-crash");
    session.status = if args.action == "crash" {
        "crashed"
    } else if failed {
        "failed"
    } else {
        "completed"
    }
    .to_string();
    session.exit_status = Some(args.status);
    session.value = Some(args.value);
    state.modal = Some(
        if failed {
            "fake-adapter-failed"
        } else {
            "fake-adapter-completed"
        }
        .to_string(),
    );
    log.emit(
        "adapter",
        0,
        json_map([
            ("action", json!(args.action)),
            ("status", json!(args.status)),
            ("value", json!(args.value)),
            ("result", json!(session.status)),
        ]),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_button<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    state: &mut AppState,
    event: ButtonEvent,
    target_sku: &str,
) -> Result<()> {
    log.emit(
        "control",
        event.at_ms,
        json_map([
            ("control", json!(button_name(event.button))),
            ("action", json!(action_name(event.action))),
        ]),
    )?;
    if event.action != ButtonAction::Press || state.faults.iter().any(|fault| fault == "input-drop")
    {
        return Ok(());
    }

    handle_presentation_button(&mut state.presentation, event.button)?;
    let input_started = Instant::now();
    let mut route_changed = false;
    let mut selection_changed = false;
    match (state.route.clone(), event.button) {
        (Route::Library, Button::Start) => {
            state.route = Route::Systems;
            route_changed = true;
        }
        (Route::Systems, Button::Down) => {
            state.route = Route::Games;
            route_changed = true;
        }
        (Route::Games, Button::Up) => {
            state.selected_index = state
                .selected_index
                .checked_sub(1)
                .unwrap_or(catalog.entries.len() - 1);
            selection_changed = true;
        }
        (Route::Games, Button::Down) => {
            state.selected_index = (state.selected_index + 1) % catalog.entries.len();
            selection_changed = true;
        }
        (Route::Games, Button::Start) => {
            state.route = Route::Library;
            route_changed = true;
        }
        (Route::Games, Button::Primary) => {
            let request = launch_request(&catalog.entries[state.selected_index]);
            let bytes = launch_contract::request_json(&request)
                .map_err(|error| anyhow!(error.to_string()))?
                .into_bytes();
            let parsed = launch_contract::parse_request_json(&bytes)
                .map_err(|error| anyhow!(error.to_string()))?;
            validate_launch_request(&parsed, launch_catalog)
                .map_err(|error| anyhow!(error.to_string()))?;
            write_bytes(evidence.root.join("launch-request.json"), &bytes)?;
            state.route = Route::Session;
            state.modal = Some("fake-session-started".to_string());
            state.active_fake_session = Some(FakeSession {
                id: SESSION_ID.to_string(),
                content_id: catalog.entries[state.selected_index].id.clone(),
                status: "started".to_string(),
                exit_status: None,
                value: None,
            });
            write_json(
                evidence.root.join("launch.json"),
                &json!({
                    "kind": "launch", "lane": LANE, "targetSku": target_sku, "sessionId": SESSION_ID,
                }),
            )?;
            write_session(&evidence.root, SessionState::Started)?;
            route_changed = true;
        }
        _ => {}
    }
    if route_changed {
        let presentation_route = match state.route {
            Route::Library | Route::Catalog => ui_model::Route::Home,
            Route::Systems => ui_model::Route::Systems,
            Route::Games | Route::Session => ui_model::Route::Games,
        };
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::Navigate(presentation_route),
        );
    }
    if route_changed || selection_changed {
        let selection = route_selection(&state.route, catalog, state.selected_index);
        emit_route_selection(log, event.at_ms, &state.route, selection)?;
    }
    let screen = state.presentation.screen()?;
    present(platform, &screen)?;
    log.emit(
        "input_to_frame",
        event.at_ms,
        json_map([("latencyUs", json!(input_started.elapsed().as_micros()))]),
    )?;
    write_route(
        &evidence.root,
        &state.route,
        route_selection(&state.route, catalog, state.selected_index),
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn capture_artifact<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    state: &AppState,
    catalog: &UiCatalog,
    directory: &str,
    name: &str,
    event_name: &str,
) -> Result<Value, String> {
    validate_name(name)?;
    let root = if directory == "screenshots" {
        &evidence.screenshots
    } else {
        &evidence.checkpoints
    };
    let png = root.join(format!("{name}.png"));
    let state_path = root.join(format!("{name}.json"));
    platform
        .capture_png(&png)
        .map_err(|error| error.to_string())?;
    if fs::metadata(&png).map_err(|error| error.to_string())?.len() == 0 {
        return Err("PNG artifact is empty".to_string());
    }
    write_json(
        state_path.clone(),
        &state_json(platform, evidence, log, catalog, state)?,
    )
    .map_err(|error| error.to_string())?;
    let sequence = log
        .emit(
            event_name,
            platform.logical_time_ms(),
            json_map([
                ("name", json!(name)),
                ("png", json!(format!("{directory}/{name}.png"))),
                ("state", json!(format!("{directory}/{name}.json"))),
            ]),
        )
        .map_err(|error| error.to_string())?;
    Ok(
        json!({"eventSequence": sequence, "png": format!("{directory}/{name}.png"), "state": format!("{directory}/{name}.json")}),
    )
}

fn state_json<P: Platform>(
    platform: &P,
    evidence: &Evidence,
    log: &EventLog,
    catalog: &UiCatalog,
    state: &AppState,
) -> Result<Value, String> {
    let _ = evidence;
    let presentation = state
        .presentation
        .screen()
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "schema": "sim-state/v1",
        "runId": log.run_id,
        "route": state.route.as_str(),
        "selectedContentId": catalog.entries[state.selected_index].id,
        "activeFakeSession": state.active_fake_session,
        "modal": state.modal,
        "readinessGeneration": state.readiness_generation,
        "hardware": hardware_json(&platform.hardware_state().map_err(|error| error.to_string())?),
        "faults": state.faults,
        "presentation": presentation,
    }))
}

fn hardware_json(hardware: &sim_platform_contract::HardwareState) -> Value {
    json!({
        "battery": {"percent": hardware.battery_percent, "charging": hardware.charging},
        "storage": {"mode": hardware.storage_mode},
        "radio": {"enabled": hardware.radio_enabled, "connected": hardware.radio_connected},
        "suspend": {"state": hardware.suspend_state, "result": hardware.suspend_result},
    })
}

fn session_state(state: &AppState) -> SessionState {
    match state
        .active_fake_session
        .as_ref()
        .map(|session| session.status.as_str())
    {
        Some("completed") => SessionState::Completed,
        Some("failed") | Some("crashed") => SessionState::Failed,
        _ => SessionState::Started,
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > control::MAX_NAME_BYTES
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        || !name.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(
            "name must be a basename of 1-48 ASCII letters, digits, '_' or '-'".to_string(),
        );
    }
    Ok(())
}

fn emit_route_selection(
    log: &mut EventLog,
    at_ms: u64,
    route: &Route,
    selection: &str,
) -> Result<()> {
    log.emit(
        "route_selection",
        at_ms,
        json_map([
            ("route", json!(route.as_str())),
            ("selection", json!(selection)),
        ]),
    )?;
    Ok(())
}

fn launch_request(entry: &sim_domain::CatalogEntry) -> LaunchRequest {
    LaunchRequest {
        schema: launch_contract::REQUEST_SCHEMA.to_string(),
        format: "brickpro-launch-request".to_string(),
        schema_version: 1,
        request_id: format!(
            "generated-request-{}",
            entry.id.trim_start_matches("generated-")
        ),
        kind: LaunchKind::Libretro,
        content_id: entry.id.clone(),
        content_sha256: GENERATED_CONTENT_SHA256.to_string(),
        content_path: LogicalPath {
            root: PathRoot::Roms,
            relative: "generated/content.bin".to_string(),
        },
        save_path: LogicalPath {
            root: PathRoot::DataSaves,
            relative: "generated/content.sav".to_string(),
        },
        state_path: LogicalPath {
            root: PathRoot::DataStates,
            relative: "generated/content.state".to_string(),
        },
        runner: VersionedId {
            id: "generated-libretro".to_string(),
            version: "1.0.0".to_string(),
        },
        package: None,
        core: Some(VersionedId {
            id: "generated-core".to_string(),
            version: "1.0.0".to_string(),
        }),
        profile_id: "generated-default".to_string(),
        resume_mode: ResumeMode::Fresh,
        display: DisplaySettings {
            width: 1024,
            height: 768,
            refresh_hz: 60,
            scaling: Scaling::Fit,
        },
        input: InputSettings {
            layout: InputLayout::Standard,
            rumble: false,
        },
        power: PowerSettings {
            suspend: SuspendMode::Allowed,
            battery_saver: false,
        },
    }
}

fn handle_presentation_button(state: &mut PresentationState, button: Button) -> Result<()> {
    if matches!(state.ui.route, ui_model::Route::Settings) {
        state
            .settings
            .press(to_keyboard_button(button))
            .map_err(|error| anyhow!(error.to_string()))?;
        return Ok(());
    }
    if matches!(state.ui.route, ui_model::Route::Wifi(_)) {
        state
            .wifi
            .press(to_keyboard_button(button))
            .map_err(|error| anyhow!(error.to_string()))?;
        return Ok(());
    }
    let action = match button {
        Button::Up => UiAction::MoveSelection(ui_model::Direction::Up),
        Button::Down => UiAction::MoveSelection(ui_model::Direction::Down),
        Button::Left => UiAction::MoveSelection(ui_model::Direction::Left),
        Button::Right => UiAction::MoveSelection(ui_model::Direction::Right),
        Button::Primary => UiAction::ActivateSelected,
        Button::Secondary => UiAction::Back,
        Button::Start => UiAction::Navigate(ui_model::Route::Home),
        Button::Select | Button::Menu => UiAction::SetFocus(ui_model::FocusTarget::Menu),
    };
    state.ui = ui_model::reduce(&state.ui, action);
    Ok(())
}

fn to_keyboard_button(button: Button) -> virtual_keyboard::Button {
    match button {
        Button::Up => virtual_keyboard::Button::Up,
        Button::Down => virtual_keyboard::Button::Down,
        Button::Left => virtual_keyboard::Button::Left,
        Button::Right => virtual_keyboard::Button::Right,
        Button::Primary => virtual_keyboard::Button::Primary,
        Button::Secondary => virtual_keyboard::Button::Secondary,
        Button::Start => virtual_keyboard::Button::Start,
        Button::Select => virtual_keyboard::Button::Select,
        Button::Menu => virtual_keyboard::Button::Menu,
    }
}

fn present<P: Platform>(platform: &mut P, screen: &PresentationScreen) -> Result<()> {
    platform.present(screen).map_err(|error| anyhow!("{error}"))
}

fn route_selection<'a>(route: &Route, catalog: &'a UiCatalog, selected_index: usize) -> &'a str {
    match route {
        Route::Library => "library",
        Route::Systems => "synthetic",
        Route::Games | Route::Session => catalog.entries[selected_index].id.as_str(),
        Route::Catalog => "library",
    }
}

fn write_route(root: &Path, route: &Route, selection: &str) -> Result<()> {
    write_json(
        root.join("route-selection.json"),
        &json!({
            "kind": "route-selection", "lane": LANE, "route": route.as_str(), "selection": selection,
        }),
    )
}

fn write_session(root: &Path, state: SessionState) -> Result<()> {
    write_json(
        root.join("session.json"),
        &json!({
            "kind": "session", "lane": LANE, "sessionId": SESSION_ID, "state": state,
        }),
    )
}

fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<()> {
    write_bytes(path, &serde_json::to_vec(value)?)
}

fn write_bytes(path: PathBuf, data: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("evidence path has no file name"))?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(".{file_name}.tmp"));
    let _ = fs::remove_file(&temporary);
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

impl Evidence {
    fn new(root: &Path) -> Result<Self> {
        let logs = root.join("logs");
        let screenshots = root.join("screenshots");
        let checkpoints = root.join("checkpoints");
        for directory in [&logs, &screenshots, &checkpoints] {
            fs::create_dir_all(directory)?;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o777))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            screenshots,
            checkpoints,
        })
    }
}

impl EventLog {
    fn new(root: &Path, run_id: &str) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(root.join("logs/launcher.jsonl"))?;
        Ok(Self {
            file,
            sequence: 0,
            run_id: run_id.to_string(),
        })
    }

    fn emit(&mut self, event: &str, at_ms: u64, details: Map<String, Value>) -> Result<u64> {
        if self.sequence >= 512 {
            return Err(anyhow!("event limit exceeded"));
        }
        let sequence = self.sequence;
        let mut object = json_map([
            ("runId", json!(self.run_id)),
            ("sequence", json!(sequence)),
            ("atMs", json!(at_ms)),
            ("lane", json!(LANE)),
            ("event", json!(event)),
        ]);
        object.extend(details);
        serde_json::to_writer(&mut self.file, &Value::Object(object))?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.sequence += 1;
        Ok(sequence)
    }
}

fn json_map<const N: usize>(entries: [(&str, Value); N]) -> Map<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn button_name(button: Button) -> &'static str {
    match button {
        Button::Up => "up",
        Button::Down => "down",
        Button::Left => "left",
        Button::Right => "right",
        Button::Primary => "primary",
        Button::Secondary => "secondary",
        Button::Start => "start",
        Button::Select => "select",
        Button::Menu => "menu",
    }
}

fn action_name(action: ButtonAction) -> &'static str {
    match action {
        ButtonAction::Press => "press",
        ButtonAction::Release => "release",
    }
}
