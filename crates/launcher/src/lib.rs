mod control;

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sim_domain::{Catalog, LaunchRequest, Route, SessionState};
use sim_platform_contract::{
    Button, ButtonAction, ButtonEvent, HardwareChanges, Platform, PlatformResult, Screen,
    StorageMode, SuspendResult, SuspendState,
};

const LANE: &str = "host-native userspace simulator";
const SESSION_ID: &str = "run-local";
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
                    target_sku: "TG4040",
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
    let catalog: Catalog =
        serde_json::from_slice(&fs::read(catalog_path)?).context("read generated catalog")?;
    if catalog.catalog_version != "1"
        || catalog.entries.is_empty()
        || catalog
            .entries
            .iter()
            .any(|entry| entry.system != "synthetic" || !entry.id.starts_with("generated-"))
    {
        return Err(anyhow!("invalid generated catalog"));
    }
    let mut state = AppState {
        route: Route::Catalog,
        selected_index: 0,
        active_fake_session: None,
        modal: None,
        faults: Vec::new(),
        readiness_generation: 1,
    };
    let screen = make_screen(&state.route, &catalog, state.selected_index);

    present(&mut platform, &screen)?;
    log.emit("ready", platform.logical_time_ms(), Map::new())?;
    write_json(
        evidence.root.join("readiness.json"),
        &Readiness {
            schema: "sim-readiness/v1",
            lane: LANE,
            target_sku: "TG4040",
            run_id: &log.run_id,
            ready: true,
            elapsed_ms: 0,
            reason: "ready",
        },
    )?;
    let snapshot = platform.snapshot();
    let first_frame_sequence = log.emit(
        "first_frame",
        platform.logical_time_ms(),
        json_map([
            ("logicalWidth", json!(1024)),
            ("logicalHeight", json!(768)),
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
    write_route(
        &evidence.root,
        &state.route,
        &catalog.entries[state.selected_index],
    )?;
    emit_route_selection(
        log,
        platform.logical_time_ms(),
        &state.route,
        &catalog.entries[state.selected_index],
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
                handle_button(&mut platform, evidence, log, &catalog, &mut state, event)?;
                did_work = true;
            } else {
                fixture_done = true;
            }
        }
        if let Some(incoming) = server.poll() {
            handle_request(&mut platform, evidence, log, &catalog, &mut state, incoming)?;
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

fn handle_request<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    catalog: &Catalog,
    state: &mut AppState,
    incoming: control::Incoming,
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
        "state" => {
            parse_empty(request.args).map(|_| state_json(platform, evidence, log, catalog, state))
        }
        "button" => parse::<ButtonArgs>(request.args).and_then(|args| {
            let event = ButtonEvent {
                at_ms: platform.logical_time_ms().saturating_add(1),
                button: args.button,
                action: args.action,
            };
            handle_button(platform, evidence, log, catalog, state, event)
                .map_err(|error| error.to_string())
                .map(|_| state_json(platform, evidence, log, catalog, state))
        }),
        "hardware.set" => parse::<HardwareArgs>(request.args).and_then(|args| {
            apply_hardware(platform, log, args)?;
            Ok(state_json(platform, evidence, log, catalog, state))
        }),
        "fault.set" => parse::<FaultArgs>(request.args).and_then(|args| {
            set_fault(log, state, args)?;
            Ok(state_json(platform, evidence, log, catalog, state))
        }),
        "adapter" => parse::<AdapterArgs>(request.args).and_then(|args| {
            adapter_result(log, state, args)?;
            write_session(&evidence.root, session_state(state))
                .map_err(|error| error.to_string())?;
            Ok(state_json(platform, evidence, log, catalog, state))
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
    if let Some(storage) = args.storage {
        if let Some(value) = storage.mode {
            changes.storage_mode = Some(value);
            changed = true;
        }
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
    let hardware = platform.hardware_state();
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

fn handle_button<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    catalog: &Catalog,
    state: &mut AppState,
    event: ButtonEvent,
) -> Result<()> {
    log.emit(
        "control",
        event.at_ms,
        json_map([
            ("control", json!(button_name(event.button))),
            ("action", json!(action_name(event.action))),
        ]),
    )?;
    if event.action == ButtonAction::Press
        && !state.faults.iter().any(|fault| fault == "input-drop")
    {
        let mut selection_changed = false;
        match event.button {
            Button::Up if route_matches_catalog(&state.route) => {
                state.selected_index = state
                    .selected_index
                    .checked_sub(1)
                    .unwrap_or(catalog.entries.len() - 1);
                selection_changed = true;
            }
            Button::Down if route_matches_catalog(&state.route) => {
                state.selected_index = (state.selected_index + 1) % catalog.entries.len();
                selection_changed = true;
            }
            Button::Start => state.route = Route::Catalog,
            Button::Primary if route_matches_catalog(&state.route) => {
                let _request = LaunchRequest {
                    selection: catalog.entries[state.selected_index].clone(),
                };
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
                        "kind": "launch", "lane": LANE, "targetSku": "TG4040", "sessionId": SESSION_ID,
                    }),
                )?;
                write_session(&evidence.root, SessionState::Started)?;
            }
            _ => {}
        }
        if selection_changed {
            emit_route_selection(
                log,
                event.at_ms,
                &state.route,
                &catalog.entries[state.selected_index],
            )?;
        }
        let screen = make_screen(&state.route, catalog, state.selected_index);
        present(platform, &screen)?;
        write_route(
            &evidence.root,
            &state.route,
            &catalog.entries[state.selected_index],
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn capture_artifact<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    state: &AppState,
    catalog: &Catalog,
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
        &state_json(platform, evidence, log, catalog, state),
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
    catalog: &Catalog,
    state: &AppState,
) -> Value {
    let _ = evidence;
    let _ = catalog;
    json!({
        "schema": "sim-state/v1",
        "runId": log.run_id,
        "route": state.route.as_str(),
        "selectedContentId": catalog.entries[state.selected_index].id,
        "activeFakeSession": state.active_fake_session,
        "modal": state.modal,
        "readinessGeneration": state.readiness_generation,
        "hardware": hardware_json(&platform.hardware_state()),
        "faults": state.faults,
    })
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
    selection: &sim_domain::CatalogEntry,
) -> Result<()> {
    log.emit(
        "route_selection",
        at_ms,
        json_map([
            ("route", json!(route.as_str())),
            ("selection", json!(selection.id)),
        ]),
    )?;
    Ok(())
}

fn make_screen(route: &Route, catalog: &Catalog, selected_index: usize) -> Screen {
    Screen {
        route: route.clone(),
        selection: catalog.entries[selected_index].clone(),
        selected_index,
        entry_count: catalog.entries.len(),
    }
}

fn present<P: Platform>(platform: &mut P, screen: &Screen) -> Result<()> {
    platform.present(screen).map_err(|error| anyhow!("{error}"))
}

fn route_matches_catalog(route: &Route) -> bool {
    matches!(route, Route::Catalog)
}

fn write_route(root: &Path, route: &Route, selection: &sim_domain::CatalogEntry) -> Result<()> {
    write_json(
        root.join("route-selection.json"),
        &json!({
            "kind": "route-selection", "lane": LANE, "route": route.as_str(), "selection": selection.id,
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
    let data = serde_json::to_vec(value)?;
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
        file.write_all(&data)?;
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
