use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sim_domain::{Catalog, LaunchRequest, Route, SessionState};
use sim_platform_contract::{Button, ButtonAction, Platform, PlatformResult, Screen};

const LANE: &str = "host-native userspace simulator";
const SESSION_ID: &str = "run-local";

struct Evidence {
    root: PathBuf,
    screenshots: PathBuf,
}

struct EventLog {
    file: File,
    sequence: u64,
}

#[derive(Serialize)]
struct Readiness<'a> {
    schema: &'a str,
    lane: &'a str,
    #[serde(rename = "targetSku")]
    target_sku: &'a str,
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
    #[serde(rename = "exitCode")]
    exit_code: i32,
    #[serde(rename = "cleanShutdown")]
    clean_shutdown: bool,
}

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
    let mut log = EventLog::new(&evidence.root)?;
    let result = make_platform()
        .map_err(|error| anyhow!("{error}"))
        .and_then(|platform| {
            run_session(
                platform,
                catalog_path,
                &evidence,
                &mut log,
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
    let mut selected_index = 0;
    let mut route = Route::Catalog;
    let mut launched = false;
    let mut screen = make_screen(&route, &catalog, selected_index);

    present(&mut platform, &screen)?;
    log.emit("ready", platform.logical_time_ms(), Map::new())?;
    write_json(
        evidence.root.join("readiness.json"),
        &Readiness {
            schema: "sim-readiness/v1",
            lane: LANE,
            target_sku: "TG4040",
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
    platform
        .capture_png(
            &evidence
                .screenshots
                .join(format!("screen-{first_frame_sequence}.png")),
        )
        .map_err(|error| anyhow!("{error}"))?;
    write_route(&evidence.root, &route, &catalog.entries[selected_index])?;
    emit_route_selection(
        log,
        platform.logical_time_ms(),
        &route,
        &catalog.entries[selected_index],
    )?;

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Some(event) = platform
            .next_button_event()
            .map_err(|error| anyhow!("{error}"))?
        else {
            break;
        };
        log.emit(
            "control",
            event.at_ms,
            json_map([
                ("control", json!(button_name(event.button))),
                ("action", json!(action_name(event.action))),
            ]),
        )?;
        if event.action == ButtonAction::Press {
            let mut selection_changed = false;
            match event.button {
                Button::Up if route_matches_catalog(&route) => {
                    selected_index = selected_index
                        .checked_sub(1)
                        .unwrap_or(catalog.entries.len() - 1);
                    selection_changed = true;
                }
                Button::Down if route_matches_catalog(&route) => {
                    selected_index = (selected_index + 1) % catalog.entries.len();
                    selection_changed = true;
                }
                Button::Start => route = Route::Catalog,
                Button::Primary if route_matches_catalog(&route) => {
                    let _request = LaunchRequest {
                        selection: catalog.entries[selected_index].clone(),
                    };
                    route = Route::Session;
                    launched = true;
                    write_json(
                        evidence.root.join("launch.json"),
                        &json!({
                            "kind": "launch",
                            "lane": LANE,
                            "targetSku": "TG4040",
                            "sessionId": SESSION_ID,
                        }),
                    )?;
                    write_session(&evidence.root, SessionState::Started)?;
                }
                _ => {}
            }
            if selection_changed {
                emit_route_selection(log, event.at_ms, &route, &catalog.entries[selected_index])?;
            }
            screen = make_screen(&route, &catalog, selected_index);
            present(&mut platform, &screen)?;
            write_route(&evidence.root, &route, &catalog.entries[selected_index])?;
        }
    }

    if keep_alive {
        while !stop.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(50));
        }
    }
    if launched {
        write_session(&evidence.root, SessionState::Completed)?;
    }
    log.emit("clean_shutdown", platform.logical_time_ms(), Map::new())?;
    write_json(
        evidence.root.join("exit-status.json"),
        &ExitStatus {
            lane: LANE,
            session_id: SESSION_ID,
            exit_code: 0,
            clean_shutdown: true,
        },
    )?;
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
    platform
        .present(screen)
        .map_err(|error| anyhow!("{error}"))?;
    Ok(())
}

fn route_matches_catalog(route: &Route) -> bool {
    matches!(route, Route::Catalog)
}

fn write_route(root: &Path, route: &Route, selection: &sim_domain::CatalogEntry) -> Result<()> {
    write_json(
        root.join("route-selection.json"),
        &json!({
            "kind": "route-selection",
            "lane": LANE,
            "route": route.as_str(),
            "selection": selection.id,
        }),
    )
}

fn write_session(root: &Path, state: SessionState) -> Result<()> {
    write_json(
        root.join("session.json"),
        &json!({
            "kind": "session",
            "lane": LANE,
            "sessionId": SESSION_ID,
            "state": state,
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
        fs::create_dir_all(root.join("logs"))?;
        let screenshots = root.join("screenshots");
        fs::create_dir_all(&screenshots)?;
        Ok(Self {
            root: root.to_path_buf(),
            screenshots,
        })
    }
}

impl EventLog {
    fn new(root: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(root.join("logs/launcher.jsonl"))?;
        Ok(Self { file, sequence: 0 })
    }

    fn emit(&mut self, event: &str, at_ms: u64, details: Map<String, Value>) -> Result<u64> {
        if self.sequence >= 512 {
            return Err(anyhow!("event limit exceeded"));
        }
        let sequence = self.sequence;
        let mut object = json_map([
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
