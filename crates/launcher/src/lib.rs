mod control;
mod launcher_state;
mod rom_index;
#[cfg(feature = "simulator")]
mod simulator_session;

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
use compatibility_recipes::{
    ApplyReceipt, LauncherAction, LauncherResponse, LocalOverrides, MatchedRecipe, Preview,
    ValidationContext,
};
use launch_contract::{
    validate as validate_launch_request, Catalog as LaunchCatalog, DisplaySettings, InputLayout,
    InputSettings, LaunchKind, LaunchRequest, LogicalPath, PathRoot, PowerSettings, ResumeMode,
    Scaling, SuspendMode, VersionedId,
};
use launcher_presentation::Screen as PresentationScreen;
use launcher_theme::ValidatedTheme;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use session_broker::{
    resume::{CheckpointReason, CommitFault, ResumeDecision},
    LifecycleCheckpointPolicy, SessionBrokerClient, SessionHandle, SessionResult,
};
use settings_schema::{ProjectionContext, Registry};
use settings_ui::SettingsUi;
use sim_domain::{Catalog as UiCatalog, Route, SessionState};
use sim_platform_contract::{
    lifecycle::{
        CheckpointHook, LifecycleClock, LifecycleController, LifecycleFault, LifecycleMarker,
        LifecyclePhase, ResumeRequest, SuspendRequest, WakeSource, DEFAULT_SLEEP_DURATION_MINUTES,
    },
    Button, ButtonAction, ButtonEvent, HardwareChanges, Platform, PlatformResult, StorageMode,
};
use ui_model::{Action as UiAction, PlatformCapabilities as UiCapabilities};
use wifi_manager::{GeneratedWifiBackend, WifiManager};
use wifi_settings_controller::{Metadata as WifiMetadata, WifiSettingsController};

const LANE: &str = "host-native userspace simulator";
const SESSION_ID: &str = "run-local";
const LAUNCH_CATALOG_BYTES: &[u8] =
    include_bytes!("../../../fixtures/launch-contract/generated-v1/catalog.synthetic.json");
const NEBULA_CONTENT_SHA256: &str =
    "eb0b39e700629b526932cf8555468761d65d0e7897807df5df5f7acb2ba28a51";
const MIRROR_CONTENT_SHA256: &str =
    "1f4ea13bac997d4cb3f2245aa16bc76a02fe49a2fbdb89c6a54b9ed9242158e9";
const ORBIT_CONTENT_SHA256: &str =
    "667fc8a4f6a35cf1a3f31feb7df60281e619d3f5480910d6334db830651d6461";
const SIGNAL_CONTENT_SHA256: &str =
    "15462e6b849f258655b640e8fd8434a4cbc50c763e2a5b1a1d1135926edc047c";
const MAX_GENERATED_ENTRIES: usize = rom_index::MAX_ENTRIES;
const INPUT_PROFILE_BYTES: &[u8] = include_bytes!("../../../config/input/profiles.json");
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
    "checkpoint-fail",
    "quiesce-audio-fail",
    "quiesce-input-fail",
    "quiesce-radios-fail",
    "resume-radios-fail",
    "resume-input-fail",
    "resume-audio-fail",
    "hal-loss",
    "arm-fail",
    "verify-fail",
    "clear-fail",
    "crash-before-suspend",
    "crash-armed-journal",
    "shutdown-fail",
    "deadline",
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

struct AppState {
    route: Route,
    selected_content_id: String,
    groups: rom_index::GroupIndex,
    input_profile: input_profile::Catalog,
    broker: Box<dyn SessionBrokerClient>,
    save_vault: SaveVaultUi,
    save_sync: Option<launcher_presentation::SaveSyncView>,
    active_session: Option<SessionHandle>,
    last_session: Option<SessionResult>,
    modal: Option<String>,
    faults: Vec<String>,
    readiness_generation: u64,
    persisted: launcher_state::State,
    presentation: PresentationState,
    session_step: u32,
    lifecycle: LifecycleController,
}

struct SaveVaultUi {
    screen: String,
    history_count: usize,
    protected_count: usize,
    preview: Option<session_broker::SaveVaultPreview>,
    confirmed: bool,
}

impl Default for SaveVaultUi {
    fn default() -> Self {
        Self {
            screen: "hidden".into(),
            history_count: 0,
            protected_count: 0,
            preview: None,
            confirmed: false,
        }
    }
}

struct PresentationState {
    ui: ui_model::UiState,
    theme: ValidatedTheme,
    theme_fallback: Option<launcher_theme::Reason>,
    settings: SettingsUi,
    wifi: WifiSettingsController,
    index: launcher_presentation::IndexView,
    recent: Vec<String>,
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
        let providers = metadata_scraper::registered_providers()
            .into_iter()
            .map(|provider| settings_schema::ProviderMetadata {
                id: provider.id,
                enabled: provider.enabled,
                requires_credentials: provider.requires_credentials,
                credential_configured: provider.credential_configured,
                priority: provider.priority,
                max_concurrency: provider.max_concurrency,
            })
            .collect::<Vec<_>>();
        let registry = Registry::from_json(SETTINGS_REGISTRY_BYTES)
            .and_then(|registry| registry.with_provider_metadata(&providers))
            .map_err(|error| anyhow!(error.to_string()))?;
        let mut context = ProjectionContext::default();
        context.capabilities.extend([
            "audio".into(),
            "network".into(),
            "theme-engine".into(),
            "wifi".into(),
            "scraper".into(),
            "cap.power.bounded-sleep".into(),
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
            index: launcher_presentation::IndexView::default(),
            recent: Vec::new(),
        })
    }

    fn screen(&self) -> Result<PresentationScreen> {
        let settings = self
            .settings
            .scene()
            .map_err(|error| anyhow!(error.to_string()))?;
        let wifi = self.wifi.snapshot();
        Ok(launcher_presentation::build_with_recent(
            &self.ui,
            &self.theme,
            self.theme_fallback,
            Some(&settings),
            Some(&wifi),
            &self.index,
            &self.recent,
        ))
    }
}

fn screen_for_state(state: &AppState, catalog: &UiCatalog) -> Result<PresentationScreen> {
    let mut screen = state.presentation.screen()?;
    if matches!(state.route, Route::Games | Route::Session) {
        let selected_index = selected_catalog_index(state, catalog);
        screen.game_rows = catalog
            .entries
            .iter()
            .skip(selected_index)
            .take(rom_index::MAX_VISIBLE_ROWS)
            .map(|entry| launcher_presentation::ScreenItem {
                id: entry.id.clone(),
                label: entry.title.clone(),
                selected: entry.id == state.selected_content_id,
                enabled: true,
            })
            .collect();
        screen.selected_label = catalog.entries[selected_catalog_index(state, catalog)]
            .title
            .clone();
    }
    if matches!(state.route, Route::Session)
        && matches!(state.presentation.ui.route, ui_model::Route::Games)
    {
        let entry = &catalog.entries[selected_catalog_index(state, catalog)];
        screen.title = entry.title.clone();
        screen.modal = Some(format!("{} FRAME {}", entry.title, state.session_step));
    }
    screen.save_sync = state.save_sync.clone();
    Ok(screen)
}

fn sync_view(status: save_sync::SyncStatus) -> launcher_presentation::SaveSyncView {
    let view = |candidate: save_sync::CandidateView| launcher_presentation::SaveSyncCandidateView {
        logical_id: candidate.logical_id,
        content_id: candidate.content_id,
        device_id: candidate.device_id,
        device_name: candidate.device_name,
        generation: candidate.generation,
        hash_prefix: candidate.hash_prefix,
        parent_hash_prefix: candidate.parent_hash_prefix,
        ancestry: candidate.ancestry,
        save_kind: format!("{:?}", candidate.save_kind).to_ascii_lowercase(),
        timestamp_ms: candidate.timestamp_ms,
        size: candidate.size,
        status: format!("{:?}", candidate.status).to_ascii_lowercase(),
        deleted: candidate.deleted,
    };
    launcher_presentation::SaveSyncView {
        local: view(status.local),
        remote: view(status.remote),
        state: status.state,
        transport_outcome: status.transport_outcome,
        actions: status
            .actions
            .into_iter()
            .map(|action| match action {
                save_sync::ResolutionAction::KeepLocal => "keep-local".into(),
                save_sync::ResolutionAction::KeepRemote => "keep-remote".into(),
                save_sync::ResolutionAction::KeepBoth => "keep-both".into(),
            })
            .collect(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveVaultRestoreArgs {
    confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveSyncResolveArgs {
    action: save_sync::ResolutionAction,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ButtonArgs {
    button: Button,
    action: ButtonAction,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticActionArgs {
    action: input_profile::Action,
    #[serde(default)]
    phase: Option<ButtonAction>,
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
    #[serde(rename = "externalPower")]
    external_power: Option<bool>,
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
struct HardwareArgs {
    battery: Option<BatteryChanges>,
    storage: Option<StorageChanges>,
    radio: Option<RadioChanges>,
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
struct AutosaveArgs {
    reason: CheckpointReason,
    #[serde(default)]
    fault: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeArgs {
    #[serde(rename = "contentId")]
    content_id: String,
    decision: String,
    #[serde(default, rename = "runnerId")]
    runner_id: Option<String>,
    #[serde(default, rename = "runnerVersion")]
    runner_version: Option<String>,
    #[serde(default, rename = "coreId")]
    core_id: Option<String>,
    #[serde(default, rename = "coreVersion")]
    core_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresentationArgs {
    action: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleArgs {
    operation: String,
    #[serde(rename = "timeoutMs")]
    timeout_ms: u64,
    #[serde(default, rename = "durationMinutes")]
    duration_minutes: Option<u16>,
    #[serde(default, rename = "wakeSource")]
    wake_source: Option<WakeSource>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockArgs {
    operation: String,
    #[serde(rename = "monotonicMs")]
    monotonic_ms: u64,
    #[serde(rename = "wallClockMs")]
    wall_clock_ms: u64,
}

const MAX_ADAPTER_VALUE: i32 = 1_000_000;

pub struct CompatibilityRecipeController {
    root: PathBuf,
    target_id: String,
    authenticated: MatchedRecipe,
    context: ValidationContext,
}

pub enum CompatibilityRecipeAction {
    Preview {
        local_overrides: LocalOverrides,
    },
    Apply {
        local_overrides: LocalOverrides,
        replace_collisions: std::collections::BTreeSet<String>,
    },
    Rollback,
}

#[allow(clippy::large_enum_variant)]
pub enum CompatibilityRecipeResult {
    Preview(Preview),
    Applied(ApplyReceipt),
    RolledBack,
}

impl CompatibilityRecipeController {
    pub fn new(
        root: impl Into<PathBuf>,
        repository: &Path,
        target_id: &str,
        rom_sha256: &str,
        context: ValidationContext,
    ) -> Result<Self> {
        let authenticated =
            compatibility_recipes::match_recipe(repository, target_id, rom_sha256, &context)
                .map_err(|error| anyhow!(error.to_string()))?;
        Ok(Self {
            root: root.into(),
            target_id: target_id.to_string(),
            authenticated,
            context,
        })
    }

    pub fn dispatch(&self, action: CompatibilityRecipeAction) -> Result<CompatibilityRecipeResult> {
        let action = match action {
            CompatibilityRecipeAction::Preview { local_overrides } => {
                LauncherAction::Preview { local_overrides }
            }
            CompatibilityRecipeAction::Apply {
                local_overrides,
                replace_collisions,
            } => LauncherAction::Apply {
                local_overrides,
                replace_collisions,
            },
            CompatibilityRecipeAction::Rollback => LauncherAction::Rollback,
        };
        match compatibility_recipes::launcher_dispatch(
            &self.root,
            &self.authenticated,
            &self.context,
            &self.target_id,
            action,
        )
        .map_err(|error| anyhow!(error.to_string()))?
        {
            LauncherResponse::Preview(value) => Ok(CompatibilityRecipeResult::Preview(value)),
            LauncherResponse::Applied(value) => Ok(CompatibilityRecipeResult::Applied(value)),
            LauncherResponse::RolledBack => Ok(CompatibilityRecipeResult::RolledBack),
        }
    }
}

#[cfg(feature = "simulator")]
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
    run_with_broker(
        catalog_path,
        evidence_path,
        keep_alive,
        stop,
        Box::new(simulator_session::SimulatorSessionAdapter::with_root(
            evidence_path.join("data"),
        )),
        make_platform,
    )
}

pub fn run_with_broker<P, F>(
    catalog_path: &Path,
    evidence_path: &Path,
    keep_alive: bool,
    stop: &AtomicBool,
    broker: Box<dyn SessionBrokerClient>,
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
                broker,
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

#[allow(clippy::too_many_arguments)]
fn run_session<P: Platform>(
    mut platform: P,
    catalog_path: &Path,
    evidence: &Evidence,
    log: &mut EventLog,
    server: &mut control::ControlServer,
    broker: Box<dyn SessionBrokerClient>,
    keep_alive: bool,
    stop: &AtomicBool,
) -> Result<()> {
    let startup_started = Instant::now();
    let identity = platform.identity();
    let display = platform.display_state().map_err(|error| anyhow!(error))?;
    let catalog_started = Instant::now();
    let launch_catalog: LaunchCatalog = launch_contract::parse_catalog_json(LAUNCH_CATALOG_BYTES)
        .map_err(|error| anyhow!(error.to_string()))?;
    launch_contract::validate_catalog_projection(&launch_catalog)
        .map_err(|error| anyhow!(error.to_string()))?;
    let mut catalog: UiCatalog =
        serde_json::from_slice(&fs::read(catalog_path)?).context("read generated catalog")?;
    if catalog.catalog_version != "1"
        || catalog.entries.is_empty()
        || catalog.entries.len() > MAX_GENERATED_ENTRIES
        || catalog.entries.iter().any(|entry| {
            entry.id.is_empty()
                || entry.id.len() > 64
                || entry.title.is_empty()
                || entry.title.len() > 128
                || entry.system.len() > 64
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
    rom_index::sort_catalog(&mut catalog);
    let input_profile = input_profile::Catalog::from_json(INPUT_PROFILE_BYTES)
        .map_err(|error| anyhow!(error.to_string()))?;
    let state_root = evidence.root.join("data");
    fs::create_dir_all(&state_root)?;
    // Simulator evidence is mounted from the caller; keep its state directory caller-cleanable.
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o777))?;
    let index = rom_index::spawn(catalog_path.to_path_buf(), state_root.clone())
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| anyhow!("ROM index worker did not report"))?;
    log.emit(
        "index",
        platform.logical_time_ms(),
        json_map([
            ("status", json!(index.report.status)),
            ("entryCount", json!(index.report.entry_count)),
            ("visibleRows", json!(index.report.visible_rows)),
            ("searchResults", json!(index.report.search_results)),
            ("queueDepth", json!(index.report.queue_depth)),
        ]),
    )?;
    let persisted = launcher_state::load(&state_root);
    let mut presentation = PresentationState::new()?;
    if let Some(progress) = persisted.scraper_progress.clone() {
        presentation.ui = ui_model::reduce(
            &presentation.ui,
            UiAction::Scraper(ui_model::ScraperAction::OpenBulkQueue),
        );
        presentation.ui = ui_model::reduce(
            &presentation.ui,
            UiAction::Scraper(ui_model::ScraperAction::SetProgress(progress)),
        );
    }
    presentation.index = launcher_presentation::IndexView {
        status: index.report.status,
        entry_count: index.report.entry_count,
        visible_rows: index.report.visible_rows,
        search_results: index.report.search_results,
        queue_depth: index.report.queue_depth,
    };
    presentation.recent = persisted
        .recent
        .iter()
        .map(|item| item.content_id.clone())
        .collect();
    let preferences = persisted.preferences.clone();
    for change in [
        ui_model::PreferenceChange::ArtworkMode(preferences.artwork_mode),
        ui_model::PreferenceChange::MetadataVisibility(preferences.metadata_visibility),
        ui_model::PreferenceChange::FontScale(preferences.font_scale),
        ui_model::PreferenceChange::ColorScheme(preferences.color_scheme),
    ] {
        presentation.ui = ui_model::reduce(&presentation.ui, UiAction::SetPreference(change));
    }
    for favorite in &persisted.favorites {
        let game_id = presentation
            .ui
            .games
            .iter()
            .find(|game| game.id.0 == *favorite && !game.favorite)
            .map(|game| game.id.clone());
        if let Some(game_id) = game_id {
            presentation.ui =
                ui_model::reduce(&presentation.ui, UiAction::ToggleFavorite { game_id });
        }
    }
    let catalog_groups = rom_index::GroupIndex::from_catalog(&catalog);
    let mut state = AppState {
        route: Route::Library,
        selected_content_id: catalog.entries[0].id.clone(),
        groups: catalog_groups,
        input_profile,
        broker,
        save_vault: SaveVaultUi::default(),
        save_sync: None,
        active_session: None,
        last_session: None,
        modal: None,
        faults: Vec::new(),
        readiness_generation: 1,
        persisted,
        presentation,
        session_step: 0,
        lifecycle: load_lifecycle(&evidence.root),
    };
    launcher_state::save(&state_root, &state.persisted)
        .map_err(|error| anyhow!(error.to_string()))?;
    refresh_resume_projection(&mut state, &catalog, &launch_catalog)
        .map_err(|error| anyhow!(error))?;
    refresh_presentation_affordances(&mut state.presentation, &platform)
        .map_err(|error| anyhow!(error))?;
    if let Ok(status) = state.broker.save_sync_status() {
        state.save_sync = Some(sync_view(status));
    }
    let screen = screen_for_state(&state, &catalog)?;

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
            ("logicalWidth", json!(display.logical_width)),
            ("logicalHeight", json!(display.logical_height)),
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
    let initial_selection = route_selection(
        &state.route,
        &catalog,
        selected_catalog_index(&state, &catalog),
    );
    write_route(&evidence.root, &state.route, initial_selection)?;
    emit_route_selection(
        log,
        platform.logical_time_ms(),
        &state.route,
        initial_selection,
    )?;

    let mut fixture_done = false;
    let lifecycle_policy = LifecycleCheckpointPolicy::default();
    let mut periodic_deadline = Instant::now() + lifecycle_policy.periodic_interval();
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let mut did_work = false;
        if state.lifecycle.is_awake()
            && state.active_session.is_some()
            && Instant::now() >= periodic_deadline
        {
            let _ = state
                .broker
                .checkpoint(CheckpointReason::Periodic, CommitFault::None);
            let _ = refresh_resume_projection(&mut state, &catalog, &launch_catalog);
            periodic_deadline = Instant::now() + lifecycle_policy.periodic_interval();
            did_work = true;
        }
        if !fixture_done {
            if let Some(event) = platform
                .next_button_event()
                .map_err(|error| anyhow!(error))?
            {
                if state.lifecycle.is_awake() {
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
                } else {
                    log.emit(
                        "input_blocked",
                        event.at_ms,
                        json_map([("phase", json!(state.lifecycle.phase()))]),
                    )?;
                }
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

    if state.active_session.is_some() || state.last_session.is_some() {
        write_session(&evidence.root, session_state(&state))?;
    }
    if state.lifecycle.terminal_shutdown() {
        platform
            .clear_wake_deadline()
            .map_err(|error| anyhow!(error.to_string()))?;
        sync_lifecycle_marker(&evidence.root, &LifecycleController::new())
            .map_err(|error| anyhow!(error))?;
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
        "action" => parse::<SemanticActionArgs>(request.args).and_then(|args| {
            let at_ms = platform.logical_time_ms().saturating_add(1);
            handle_semantic_action(
                platform,
                evidence,
                log,
                catalog,
                launch_catalog,
                state,
                args.action,
                args.phase.unwrap_or(ButtonAction::Press),
                None,
                at_ms,
                target_sku,
            )
            .map_err(|error| error.to_string())
            .and_then(|_| state_json(platform, evidence, log, catalog, state))
        }),
        "hardware.set" => parse::<HardwareArgs>(request.args).and_then(|args| {
            state.lifecycle.gate("hardware mutation")?;
            let checkpoint_reason = args
                .battery
                .as_ref()
                .and_then(|change| change.percent)
                .filter(|percent| *percent <= 10)
                .map(|_| CheckpointReason::LowBattery);
            apply_hardware(platform, log, args)?;
            if let Some(reason) = checkpoint_reason {
                let _ = state.broker.checkpoint(reason, CommitFault::None);
                refresh_resume_projection(state, catalog, launch_catalog)?;
                let _ = state.lifecycle.low_battery(platform);
                sync_lifecycle_marker(&evidence.root, &state.lifecycle)?;
                if !state.lifecycle.is_awake() {
                    state.active_session = None;
                    state.last_session = None;
                    state.route = Route::Library;
                    write_session(&evidence.root, SessionState::Aborted)
                        .map_err(|error| error.to_string())?;
                }
            }
            refresh_presentation_affordances(&mut state.presentation, platform)?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "clock" => parse::<ClockArgs>(request.args).and_then(|args| {
            if !["advance", "jump"].contains(&args.operation.as_str()) {
                return Err("clock operation must be advance or jump".into());
            }
            let (monotonic_ms, wall_clock_ms) = if args.operation == "jump" {
                (platform.logical_time_ms(), args.wall_clock_ms)
            } else {
                (
                    platform.logical_time_ms().saturating_add(args.monotonic_ms),
                    platform.wall_clock_ms().saturating_add(args.wall_clock_ms),
                )
            };
            platform
                .semantic_clock(monotonic_ms, wall_clock_ms)
                .map_err(|error| error.to_string())?;
            if state.lifecycle.deadline_due(LifecycleClock {
                monotonic_ms: platform.logical_time_ms(),
                boot_time_ms: platform.wall_clock_ms(),
            }) {
                let _ = lifecycle_control(
                    platform,
                    evidence,
                    log,
                    catalog,
                    launch_catalog,
                    state,
                    LifecycleArgs {
                        operation: "resume".into(),
                        timeout_ms: 5_000,
                        duration_minutes: None,
                        wake_source: Some(WakeSource::Deadline),
                    },
                );
            }
            state_json(platform, evidence, log, catalog, state)
        }),
        "lifecycle" => parse::<LifecycleArgs>(request.args).and_then(|args| {
            lifecycle_control(
                platform,
                evidence,
                log,
                catalog,
                launch_catalog,
                state,
                args,
            )
        }),
        "fault.set" => parse::<FaultArgs>(request.args).and_then(|args| {
            set_fault(log, state, args)?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "adapter" => parse::<AdapterArgs>(request.args).and_then(|args| {
            adapter_result(log, state, catalog, launch_catalog, args)?;
            write_session(&evidence.root, session_state(state))
                .map_err(|error| error.to_string())?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "presentation" => parse::<PresentationArgs>(request.args).and_then(|args| {
            presentation_action(state, args)?;
            let screen = screen_for_state(state, catalog).map_err(|error| error.to_string())?;
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
        "save-vault.history" => parse_empty(request.args).and_then(|_| {
            let history = state
                .broker
                .save_vault_history()
                .map_err(|error| error.to_string())?;
            state.save_vault.history_count = history.len();
            state.save_vault.protected_count =
                history.iter().filter(|entry| entry.protected).count();
            state.save_vault.screen = "history".into();
            serde_json::to_value(history).map_err(|error| error.to_string())
        }),
        "save-vault.preview" => parse_empty(request.args).and_then(|_| {
            let preview = state
                .broker
                .save_vault_preview()
                .map_err(|error| error.to_string())?;
            state.save_vault.preview = Some(preview.clone());
            state.save_vault.screen = "preview".into();
            serde_json::to_value(preview).map_err(|error| error.to_string())
        }),
        "save-sync.status" => parse_empty(request.args).and_then(|_| {
            let status = state
                .broker
                .save_sync_status()
                .map_err(|error| error.to_string())?;
            state.save_sync = Some(sync_view(status.clone()));
            serde_json::to_value(status).map_err(|error| error.to_string())
        }),
        "save-sync.resolve" => parse::<SaveSyncResolveArgs>(request.args).and_then(|args| {
            let receipt = state
                .broker
                .save_sync_resolve(args.action)
                .map_err(|error| error.to_string())?;
            if let Ok(status) = state.broker.save_sync_status() {
                state.save_sync = Some(sync_view(status));
            }
            serde_json::to_value(receipt).map_err(|error| error.to_string())
        }),
        "save-vault.restore" => parse::<SaveVaultRestoreArgs>(request.args).and_then(|args| {
            if !args.confirmed {
                return Err("save-vault restore requires explicit confirmation".into());
            }
            state
                .broker
                .save_vault_restore(true)
                .map_err(|error| error.to_string())?;
            state.save_vault.confirmed = true;
            state.save_vault.screen = "restored".into();
            Ok(json!({"restored": true}))
        }),
        "autosave" => parse::<AutosaveArgs>(request.args).and_then(|args| {
            let fault = match args.fault.as_deref() {
                None | Some("none") => CommitFault::None,
                Some("artifact") => CommitFault::Artifact,
                Some("metadata") => CommitFault::Metadata,
                Some("promotion") => CommitFault::Promotion,
                Some("pointer") => CommitFault::Pointer,
                Some(_) => return Err("unknown autosave fault".into()),
            };
            state
                .broker
                .checkpoint(args.reason, fault)
                .map(|record| json!({"generation": record.generation, "reason": record.reason}))
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    refresh_resume_projection(state, catalog, launch_catalog)?;
                    Ok(value)
                })
        }),
        "resume" => parse::<ResumeArgs>(request.args)
            .and_then(|args| resume_control(state, catalog, launch_catalog, args)),
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
    if result.is_ok() {
        sync_scraper_persistence(state);
        launcher_state::save(&evidence.root.join("data"), &state.persisted)
            .map_err(|error| anyhow!(error.to_string()))?;
    }
    match result {
        Ok(value) => control::send_ok(&mut stream, &id, &value)?,
        Err(message) => control::send_error(&mut stream, &id, "protocol_rejected", &message)?,
    }
    Ok(())
}

fn refresh_resume_projection(
    state: &mut AppState,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
) -> Result<(), String> {
    let requests = catalog
        .entries
        .iter()
        .filter_map(|entry| launch_request(entry, launch_catalog).ok())
        .collect::<Vec<_>>();
    let summaries = state
        .broker
        .resume_entries(&requests)
        .map_err(|error| error.to_string())?;
    let entries = summaries
        .into_iter()
        .map(|summary| ui_model::ResumeProjection {
            label: resume_label(&summary.content_id),
            content_id: summary.content_id,
            status: summary.status,
            screenshot: format!("generated-resume-{}", summary.generation),
            choices: summary
                .choices
                .into_iter()
                .map(resume_decision_name)
                .collect(),
        })
        .collect();
    state.presentation.ui = ui_model::reduce(
        &state.presentation.ui,
        UiAction::SetResumeEntries { entries },
    );
    Ok(())
}

fn resume_decision_name(decision: ResumeDecision) -> String {
    match decision {
        ResumeDecision::Resume => "resume",
        ResumeDecision::RetainedMatchingCore => "retained-matching-core",
        ResumeDecision::ColdStartSram => "cold-start-sram",
        ResumeDecision::Cancel => "cancel",
    }
    .into()
}

fn resume_label(content_id: &str) -> String {
    match content_id {
        "nebula-nes" => "Nebula Notes".into(),
        "mirror-ps1" => "Mirror Museum".into(),
        "orbit-garden" => "Orbit Garden".into(),
        "signal-workshop" => "Signal Workshop".into(),
        _ => "Unavailable checkpoint".into(),
    }
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
    use ui_model::{Action, AmbiguousChoice, GameId, ScraperAction, WifiAction};

    let action = args.action.as_str();
    match action {
        "home" => {
            reduce_route(state, ui_model::Route::Home);
            state.route = Route::Library;
        }
        "systems" => reduce_route(state, ui_model::Route::Systems),
        "games" => reduce_route(state, ui_model::Route::Games),
        "favorites" => reduce_route(state, ui_model::Route::Favorites),
        "recent" => reduce_route(state, ui_model::Route::Recent),
        "resume" => {
            reduce_route(state, ui_model::Route::GameSwitcher);
            state.route = Route::GameSwitcher;
        }
        "favorite" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                UiAction::ToggleFavorite {
                    game_id: ui_model::GameId::new("generated-game-02"),
                },
            );
            state.persisted.favorites = state
                .presentation
                .ui
                .games
                .iter()
                .filter(|game| game.favorite)
                .map(|game| game.id.0.clone())
                .collect();
        }
        "media-details" => {
            reduce_route(state, ui_model::Route::Games);
            state.modal = Some("media-details-projected".into());
        }
        "theme-garden" => {
            state.modal = Some("theme-garden-unavailable".into());
        }
        "update" => {
            state.modal = Some("update-unavailable".into());
        }
        "unavailable" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                UiAction::ShowModal(ui_model::ModalState::Unavailable(
                    ui_model::CapabilityError {
                        capability: ui_model::Capability::Session,
                        code: "capability-unavailable".into(),
                        message: "Capability is unavailable in the simulator.".into(),
                    },
                )),
            );
        }
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
            metadata_scraper::ScrapeRequest::new("generated-game-01", "generated-system-alpha")
                .with_filename("generated-game-01.bin")
                .validate()
                .map_err(|error| error.to_string())?;
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenGame {
                    game_id: GameId::new("generated-game-01"),
                }),
            )
        }
        "scraper-queue" => {
            metadata_scraper::ScrapeRequest::new("generated-game-01", "generated-system-alpha")
                .with_filename("generated-game-01.bin")
                .validate()
                .map_err(|error| error.to_string())?;
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::QueueGame {
                    game_id: GameId::new("generated-game-01"),
                }),
            )
        }
        "scraper-progress"
        | "scraper-progress-zero"
        | "scraper-progress-2"
        | "scraper-progress-4" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenBulkQueue),
            );
            let slots = if action.ends_with("-4") { 4 } else { 2 };
            let completed = if action.ends_with("-zero") { 0 } else { 1 };
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::SetProgress(synthetic_progress(
                    slots, completed,
                ))),
            )
        }
        "scraper-fallback" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenBulkQueue),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::SetProgress(synthetic_progress(2, 1))),
            );
        }
        "scraper-background" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenBulkQueue),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::SetProgress(synthetic_progress(2, 1))),
            );
            state.presentation.ui =
                ui_model::reduce(&state.presentation.ui, Action::Scraper(ScraperAction::Hide));
        }
        "scraper-paused" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::OpenBulkQueue),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::SetProgress(synthetic_progress(2, 1))),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::PauseForGate {
                    reason: "network".into(),
                }),
            );
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
                Action::Scraper(ScraperAction::OpenBulkQueue),
            );
            let mut progress = synthetic_progress(2, 4);
            progress.counts = ui_model::ScraperCounts {
                succeeded: 2,
                fallback: 1,
                not_found: 1,
                ..Default::default()
            };
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::SetProgress(progress)),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::Complete),
            );
        }
        "scraper-cancel" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::Cancel),
            )
        }
        "scraper-confirm-cancel" => {
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::Cancel),
            );
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                Action::Scraper(ScraperAction::ConfirmCancel),
            );
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
        "save-sync-status" => refresh_sync_view(state)?,
        "save-sync-keep-local" => resolve_sync_view(state, save_sync::ResolutionAction::KeepLocal)?,
        "save-sync-keep-remote" => {
            resolve_sync_view(state, save_sync::ResolutionAction::KeepRemote)?
        }
        "save-sync-keep-both" => resolve_sync_view(state, save_sync::ResolutionAction::KeepBoth)?,
        "save-vault-history" => {
            let entries = state
                .broker
                .save_vault_history()
                .map_err(|error| error.to_string())?;
            state.save_vault.history_count = entries.len();
            state.save_vault.protected_count =
                entries.iter().filter(|entry| entry.protected).count();
            state.save_vault.preview = None;
            state.save_vault.confirmed = false;
            state.save_vault.screen = "history".into();
        }
        "save-vault-preview" => {
            state.save_vault.preview = Some(
                state
                    .broker
                    .save_vault_preview()
                    .map_err(|error| error.to_string())?,
            );
            state.save_vault.screen = "preview".into();
        }
        "save-vault-confirm" => {
            if state.save_vault.preview.is_none() {
                return Err("save-vault preview is required".into());
            }
            state.save_vault.confirmed = true;
            state.save_vault.screen = "confirm".into();
        }
        "save-vault-restore" => {
            if !state.save_vault.confirmed {
                return Err("save-vault confirmation is required".into());
            }
            state
                .broker
                .save_vault_restore(true)
                .map_err(|error| error.to_string())?;
            state.save_vault.screen = "restored".into();
        }
        "save-vault-cancel" => {
            state.save_vault.confirmed = false;
            state.save_vault.screen = "cancelled".into();
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

fn refresh_sync_view(state: &mut AppState) -> Result<(), String> {
    let status = state
        .broker
        .save_sync_status()
        .map_err(|error| error.to_string())?;
    state.save_sync = Some(sync_view(status));
    Ok(())
}

fn resolve_sync_view(
    state: &mut AppState,
    action: save_sync::ResolutionAction,
) -> Result<(), String> {
    state
        .broker
        .save_sync_resolve(action)
        .map_err(|error| error.to_string())?;
    refresh_sync_view(state)
}

fn resume_control(
    state: &mut AppState,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    args: ResumeArgs,
) -> Result<Value, String> {
    state.lifecycle.gate("launch")?;
    let decision = match args.decision.as_str() {
        "resume" => ResumeDecision::Resume,
        "retained-matching-core" => ResumeDecision::RetainedMatchingCore,
        "cold-start-sram" => ResumeDecision::ColdStartSram,
        "cancel" => ResumeDecision::Cancel,
        _ => return Err("resume decision is not allowlisted".into()),
    };
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == args.content_id)
        .ok_or_else(|| "resume content is not allowlisted".to_string())?;
    let mut request = launch_request(entry, launch_catalog).map_err(|error| error.to_string())?;
    if let Some(runner_id) = args.runner_id {
        request.runner.id = runner_id;
    }
    if let Some(runner_version) = args.runner_version {
        request.runner.version = runner_version;
    }
    if let Some(core_id) = args.core_id {
        request.core = Some(VersionedId {
            id: core_id,
            version: args.core_version.unwrap_or_else(|| "1.0.0".to_string()),
        });
    } else if let Some(core_version) = args.core_version {
        if let Some(core) = request.core.as_mut() {
            core.version = core_version;
        }
    }
    request.resume_mode = if matches!(
        decision,
        ResumeDecision::Cancel | ResumeDecision::ColdStartSram
    ) {
        ResumeMode::Fresh
    } else {
        ResumeMode::Resume
    };
    let choices = state
        .broker
        .resume_choices(&request)
        .map_err(|error| error.to_string())?;
    if !choices.contains(&decision) {
        state.modal = Some("resume-unavailable".into());
        return Ok(json!({
            "accepted": false,
            "reason": "resume choice is unavailable",
            "availableChoices": choices,
        }));
    }
    let result = match state.broker.resume_decision(request.clone(), decision) {
        Ok(result) => result,
        Err(error) => {
            state.modal = Some("resume-unavailable".into());
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                UiAction::ShowModal(ui_model::ModalState::Info {
                    title: "Resume unavailable".into(),
                    message: "This checkpoint is unavailable or incompatible.".into(),
                }),
            );
            return Ok(json!({"accepted": false, "reason": error.to_string()}));
        }
    };
    if decision == ResumeDecision::Cancel {
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::Navigate(ui_model::Route::Home),
        );
        state.route = Route::Library;
        return Ok(json!({"accepted": true, "decision": "cancel"}));
    }
    let effective_core = result.effective_core.clone();
    if let Some(core) = effective_core.clone() {
        request.core = Some(core);
    }
    let accepted = state
        .broker
        .submit(request, launch_catalog)
        .map_err(|error| error.to_string())?;
    state.active_session = Some(accepted);
    state.last_session = None;
    state.route = Route::Session;
    state.presentation.ui = ui_model::reduce(
        &state.presentation.ui,
        UiAction::Navigate(ui_model::Route::Games),
    );
    state.modal = Some("resume-accepted".into());
    Ok(json!({
        "accepted": true,
        "decision": args.decision,
        "availableChoices": choices,
        "effectiveCore": effective_core,
        "generation": result.generation,
        "usedSram": result.used_sram,
    }))
}

fn synthetic_progress(slots: u8, completed: u16) -> ui_model::ScraperProgress {
    let titles = [
        "Nebula Notes",
        "Mirror Museum",
        "Orbit Garden",
        "Signal Workshop",
    ];
    let rows = titles
        .iter()
        .take(slots as usize)
        .enumerate()
        .map(|(index, title)| ui_model::ScraperRow {
            game_id: ui_model::GameId::new(format!("generated-game-0{}", index + 1)),
            title: (*title).into(),
            provider: Some(
                if index == 0 {
                    "fixture-secondary"
                } else {
                    "fixture-tertiary"
                }
                .into(),
            ),
            phase: if index == 0 {
                ui_model::ScraperPhase::FallingBack
            } else {
                ui_model::ScraperPhase::Searching
            },
            fallback_transition: (index == 0)
                .then_some("fixture-primary: not found → fixture-secondary".into()),
        })
        .collect();
    ui_model::ScraperProgress {
        completed,
        total: 4,
        percent: ((u32::from(completed) * 100) / 4) as u8,
        configured_slots: slots,
        paused: false,
        paused_reason: None,
        background: false,
        counts: ui_model::ScraperCounts {
            succeeded: completed,
            ..Default::default()
        },
        rows,
    }
}

fn sync_scraper_persistence(state: &mut AppState) {
    state.persisted.scraper_progress = state.presentation.ui.scraper.progress.clone();
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

struct BrokerCheckpoint<'a> {
    broker: &'a mut dyn SessionBrokerClient,
    fault: CommitFault,
}

impl CheckpointHook for BrokerCheckpoint<'_> {
    fn checkpoint(&mut self) -> Result<u64, String> {
        self.broker
            .checkpoint(CheckpointReason::PreSuspend, self.fault)
            .map(|record| record.generation)
            .map_err(|error| error.to_string())
    }
}

fn lifecycle_control<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    state: &mut AppState,
    args: LifecycleArgs,
) -> Result<Value, String> {
    if args.timeout_ms == 0 || args.timeout_ms > control::MAX_TIMEOUT_MS {
        return Err("lifecycle timeoutMs must be between 1 and 30000".into());
    }
    let timeout = Duration::from_millis(args.timeout_ms);
    let fault = state
        .faults
        .iter()
        .find_map(|name| LifecycleFault::from_name(name));
    let result = match args.operation.as_str() {
        "suspend" => {
            let checkpoint_fault = if state.faults.iter().any(|fault| fault == "checkpoint-fail") {
                CommitFault::Artifact
            } else {
                CommitFault::None
            };
            let mut checkpoint = BrokerCheckpoint {
                broker: state.broker.as_mut(),
                fault: checkpoint_fault,
            };
            let now_ms = platform.logical_time_ms();
            let wall_clock_ms = platform.wall_clock_ms();
            state.lifecycle.suspend(
                platform,
                &mut checkpoint,
                SuspendRequest {
                    timeout,
                    clock: LifecycleClock {
                        monotonic_ms: now_ms,
                        boot_time_ms: wall_clock_ms,
                    },
                    duration_minutes: args
                        .duration_minutes
                        .unwrap_or(DEFAULT_SLEEP_DURATION_MINUTES),
                    fault,
                },
            )
        }
        "resume" => state.lifecycle.resume(
            platform,
            ResumeRequest {
                timeout,
                clock: LifecycleClock {
                    monotonic_ms: platform.logical_time_ms(),
                    boot_time_ms: platform.wall_clock_ms(),
                },
                source: args.wake_source,
                fault,
            },
        ),
        "shutdown" => state.lifecycle.retry_shutdown(platform, fault),
        _ => return Err("lifecycle operation must be suspend, resume, or shutdown".into()),
    };
    sync_lifecycle_marker(&evidence.root, &state.lifecycle)?;
    let phase = state.lifecycle.phase();
    if matches!(
        phase,
        LifecyclePhase::ResumedForDeadline | LifecyclePhase::OrderlyShutdown
    ) {
        state.active_session = None;
        state.last_session = None;
        state.route = Route::Library;
        write_session(&evidence.root, SessionState::Aborted).map_err(|error| error.to_string())?;
    }
    let details = json_map([
        ("operation", json!(args.operation)),
        ("phase", json!(phase)),
        (
            "lifecycle",
            serde_json::to_value(state.lifecycle.evidence()).map_err(|error| error.to_string())?,
        ),
    ]);
    log.emit("lifecycle", platform.logical_time_ms(), details)
        .map_err(|error| error.to_string())?;
    match result {
        Ok(()) => {
            if phase == LifecyclePhase::Suspended {
                refresh_resume_projection(state, catalog, launch_catalog)?;
            }
            Ok(json!({"accepted": true, "operation": args.operation, "phase": phase}))
        }
        Err(error) => Err(error.to_string()),
    }
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
        if let Some(value) = battery.external_power {
            changes.external_power = Some(value);
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
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    args: AdapterArgs,
) -> Result<(), String> {
    if !["complete", "fail", "exit", "crash"].contains(&args.action.as_str()) {
        return Err("session result action must be complete, fail, exit, or crash".to_string());
    }
    if args.value.abs_diff(0) > MAX_ADAPTER_VALUE as u32 {
        return Err("adapter value must be between -1000000 and 1000000".to_string());
    }
    state.lifecycle.gate("background session completion")?;
    if state.active_session.is_none() {
        return Err("no active session".to_string());
    }
    let failed = matches!(args.action.as_str(), "fail" | "crash")
        || state
            .faults
            .iter()
            .any(|fault| fault == "adapter-fail" || fault == "adapter-crash");
    let content_id = state
        .active_session
        .as_ref()
        .map(|session| session.content_id.clone())
        .ok_or_else(|| "no active session".to_string())?;
    let result = state
        .broker
        .complete(
            if failed {
                args.status.max(1) as i32
            } else {
                args.status as i32
            },
            0,
        )
        .map_err(|error| error.to_string())?;
    state.active_session = None;
    state.last_session = Some(result.clone());
    refresh_resume_projection(state, catalog, launch_catalog)?;
    state
        .persisted
        .recent
        .retain(|item| item.content_id != content_id);
    state.persisted.recent.insert(
        0,
        launcher_state::RecentItem {
            content_id,
            playtime_ms: result.duration_ms,
        },
    );
    state.persisted.recent.truncate(64);
    state.presentation.recent = state
        .persisted
        .recent
        .iter()
        .map(|item| item.content_id.clone())
        .collect();
    state.route = Route::Library;
    state.presentation.ui = ui_model::reduce(
        &state.presentation.ui,
        UiAction::Navigate(ui_model::Route::Home),
    );
    state.modal = Some(
        if failed {
            "session-failed"
        } else {
            "session-returned"
        }
        .to_string(),
    );
    log.emit(
        "session_result",
        0,
        json_map([
            ("action", json!(args.action)),
            ("status", json!(args.status)),
            ("value", json!(args.value)),
            ("reason", json!(result.reason)),
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
    let action = raw_control(event.button)
        .and_then(|control| state.input_profile.action_for_control(control).ok())
        .ok_or_else(|| anyhow!("input profile has no action for {:?}", event.button))?;
    handle_semantic_action(
        platform,
        evidence,
        log,
        catalog,
        launch_catalog,
        state,
        action,
        event.action,
        Some(event.button),
        event.at_ms,
        target_sku,
    )
}

#[allow(clippy::too_many_arguments)]
fn handle_semantic_action<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    log: &mut EventLog,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    state: &mut AppState,
    action: input_profile::Action,
    phase: ButtonAction,
    raw_button: Option<Button>,
    at_ms: u64,
    target_sku: &str,
) -> Result<()> {
    state
        .lifecycle
        .gate("input")
        .map_err(|error| anyhow!(error))?;
    log.emit(
        "control",
        at_ms,
        json_map([
            (
                "control",
                raw_button.map_or_else(|| "semantic".into(), |button| button_name(button).into()),
            ),
            ("action", json!(action_name(phase))),
            ("semanticAction", json!(semantic_action_name(action))),
        ]),
    )?;
    if phase != ButtonAction::Press || state.faults.iter().any(|fault| fault == "input-drop") {
        return Ok(());
    }

    let selection_before = state.selected_content_id.clone();
    let route_before_presentation = state.route.clone();
    let vault_button = match raw_button {
        Some(button) => handle_save_vault_button(state, button)?,
        None => false,
    };
    if !vault_button {
        handle_presentation_action(&mut state.presentation, action)?;
    }
    if matches!(state.presentation.ui.route, ui_model::Route::Games)
        && state.route != Route::Session
    {
        state.route = Route::Games;
    } else if matches!(state.presentation.ui.route, ui_model::Route::Systems) {
        state.route = Route::Systems;
    }
    if matches!(state.presentation.ui.route, ui_model::Route::GameSwitcher) {
        state.route = Route::GameSwitcher;
    } else if matches!(state.presentation.ui.route, ui_model::Route::Home)
        && state.route == Route::GameSwitcher
    {
        state.route = Route::Library;
    }
    if matches!(
        action,
        input_profile::Action::JumpNextGroup | input_profile::Action::JumpPreviousGroup
    ) {
        jump_group(
            state,
            catalog,
            matches!(action, input_profile::Action::JumpNextGroup),
        );
    }
    if matches!(state.presentation.ui.route, ui_model::Route::GameSwitcher)
        && action == input_profile::Action::Primary
    {
        if let ui_model::SessionState::Requested(game_id) = state.presentation.ui.session.clone() {
            let _ = resume_control(
                state,
                catalog,
                launch_catalog,
                ResumeArgs {
                    content_id: game_id.0,
                    decision: "resume".into(),
                    runner_id: None,
                    runner_version: None,
                    core_id: None,
                    core_version: None,
                },
            );
        }
    }
    let input_started = Instant::now();
    let mut route_changed = state.route != route_before_presentation;
    let mut selection_changed = state.selected_content_id != selection_before;
    match (state.route.clone(), action) {
        (Route::Library, input_profile::Action::Start) => {
            state.route = Route::Systems;
            route_changed = true;
        }
        (Route::Systems, input_profile::Action::MoveDown) => {
            state.route = Route::Games;
            route_changed = true;
        }
        (Route::Games, input_profile::Action::MoveUp) => {
            let index = selected_catalog_index(state, catalog)
                .checked_sub(1)
                .unwrap_or(catalog.entries.len() - 1);
            state.selected_content_id = catalog.entries[index].id.clone();
            selection_changed = true;
        }
        (Route::Games, input_profile::Action::MoveDown) => {
            let index = (selected_catalog_index(state, catalog) + 1) % catalog.entries.len();
            state.selected_content_id = catalog.entries[index].id.clone();
            selection_changed = true;
        }
        (Route::Games, input_profile::Action::Start)
        | (Route::Session, input_profile::Action::Start) => {
            state.route = Route::Library;
            route_changed = true;
        }
        (Route::Games, input_profile::Action::Primary)
            if route_before_presentation == Route::Games
                && state.presentation.ui.route == ui_model::Route::Games =>
        {
            let request = launch_request(
                &catalog.entries[selected_catalog_index(state, catalog)],
                launch_catalog,
            )
            .map_err(|error| anyhow!(error))?;
            let bytes = launch_contract::request_json(&request)
                .map_err(|error| anyhow!(error.to_string()))?
                .into_bytes();
            let parsed = launch_contract::parse_request_json(&bytes)
                .map_err(|error| anyhow!(error.to_string()))?;
            validate_launch_request(&parsed, launch_catalog)
                .map_err(|error| anyhow!(error.to_string()))?;
            let accepted = state
                .broker
                .submit(parsed, launch_catalog)
                .map_err(|error| anyhow!(error.to_string()))?;
            write_bytes(evidence.root.join("launch-request.json"), &bytes)?;
            state.route = Route::Session;
            state.session_step = 0;
            state.modal = Some("session-accepted".to_string());
            state.active_session = Some(accepted);
            write_json(
                evidence.root.join("launch.json"),
                &json!({
                    "kind": "launch", "lane": LANE, "targetSku": target_sku,
                    "sessionId": state.active_session.as_ref().map(|session| session.session_id.as_str()),
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
            Route::GameSwitcher => ui_model::Route::GameSwitcher,
            Route::Systems => ui_model::Route::Systems,
            Route::Games | Route::Session => ui_model::Route::Games,
        };
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::Navigate(presentation_route),
        );
    }
    if route_changed || selection_changed {
        let selection = route_selection(
            &state.route,
            catalog,
            selected_catalog_index(state, catalog),
        );
        emit_route_selection(log, at_ms, &state.route, selection)?;
    }
    if matches!(state.route, Route::Session)
        && matches!(
            action,
            input_profile::Action::MoveUp
                | input_profile::Action::MoveDown
                | input_profile::Action::MoveLeft
                | input_profile::Action::MoveRight
        )
    {
        state.session_step = state.session_step.saturating_add(1);
    }
    let screen = screen_for_state(state, catalog)?;
    present(platform, &screen)?;
    log.emit(
        "input_to_frame",
        at_ms,
        json_map([
            ("latencyUs", json!(input_started.elapsed().as_micros())),
            ("sessionStep", json!(state.session_step)),
        ]),
    )?;
    write_route(
        &evidence.root,
        &state.route,
        route_selection(
            &state.route,
            catalog,
            selected_catalog_index(state, catalog),
        ),
    )?;
    Ok(())
}

fn jump_group(state: &mut AppState, catalog: &UiCatalog, next: bool) {
    if state.route != Route::Games || state.groups.boundaries.len() < 2 {
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::SetGroupJump(ui_model::GroupJumpState {
                current: Some(current_group(state, catalog)),
                target: None,
                visible: true,
            }),
        );
        state.presentation.ui =
            ui_model::reduce(&state.presentation.ui, UiAction::SetGroupBoundaryFeedback);
        return;
    }
    let current_index = selected_catalog_index(state, catalog);
    let current = rom_index::title_group(&catalog.entries[current_index].title);
    let Some(target) = state.groups.jump_index(current_index, next) else {
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::SetGroupJump(ui_model::GroupJumpState {
                current: Some(current),
                target: None,
                visible: true,
            }),
        );
        state.presentation.ui =
            ui_model::reduce(&state.presentation.ui, UiAction::SetGroupBoundaryFeedback);
        return;
    };
    state.selected_content_id = catalog.entries[target.first_index].id.clone();
    state.presentation.ui = ui_model::reduce(
        &state.presentation.ui,
        UiAction::SetGroupJump(ui_model::GroupJumpState {
            current: Some(current),
            target: Some(target.group.clone()),
            visible: true,
        }),
    );
}

fn current_group(state: &AppState, catalog: &UiCatalog) -> String {
    rom_index::title_group(&catalog.entries[selected_catalog_index(state, catalog)].title)
}

fn raw_control(button: Button) -> Option<input_profile::RawControl> {
    Some(match button {
        Button::Up => input_profile::RawControl::Up,
        Button::Down => input_profile::RawControl::Down,
        Button::Left => input_profile::RawControl::Left,
        Button::Right => input_profile::RawControl::Right,
        Button::Primary => input_profile::RawControl::A,
        Button::Secondary => input_profile::RawControl::B,
        Button::Start => input_profile::RawControl::Start,
        Button::Select => input_profile::RawControl::Select,
        Button::L1 => input_profile::RawControl::L1,
        Button::R1 => input_profile::RawControl::R1,
        Button::Menu => input_profile::RawControl::Home,
    })
}

fn semantic_action_name(action: input_profile::Action) -> &'static str {
    match action {
        input_profile::Action::MoveUp => "move-up",
        input_profile::Action::MoveDown => "move-down",
        input_profile::Action::MoveLeft => "move-left",
        input_profile::Action::MoveRight => "move-right",
        input_profile::Action::Primary => "primary",
        input_profile::Action::Secondary => "secondary",
        input_profile::Action::Start => "start",
        input_profile::Action::Select => "select",
        input_profile::Action::LeftStickClick => "left-stick-click",
        input_profile::Action::RightStickClick => "right-stick-click",
        input_profile::Action::JumpNextGroup => "jump-next-group",
        input_profile::Action::JumpPreviousGroup => "jump-previous-group",
        input_profile::Action::F1 => "f1",
        input_profile::Action::F2 => "f2",
        input_profile::Action::Fn => "fn",
        input_profile::Action::Home => "home",
    }
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
    let presentation = screen_for_state(state, catalog).map_err(|error| error.to_string())?;
    Ok(json!({
        "schema": "sim-state/v1",
        "runId": log.run_id,
        "route": state.route.as_str(),
        "selectedContentId": catalog.entries[selected_catalog_index(state, catalog)].id,
        "activeSession": state.active_session,
        "lastSessionResult": state.last_session,
        "modal": state.modal,
        "readinessGeneration": state.readiness_generation,
        "sessionStep": state.session_step,
        "hardware": hardware_json(&platform.hardware_state().map_err(|error| error.to_string())?),
        "platformState": platform.platform_state().map_err(|error| error.to_string())?,
        "faults": state.faults,
        "lifecycle": state.lifecycle.evidence(),
        "clock": {"monotonicMs": platform.logical_time_ms(), "wallClockMs": platform.wall_clock_ms()},
        "saveVault": save_vault_json(state),
        "presentation": presentation,
    }))
}

fn load_lifecycle(root: &Path) -> LifecycleController {
    let data = root.join("data");
    let marker_path = data.join("lifecycle-marker.json");
    let journal_path = data.join("lifecycle-journal.json");
    let marker_bytes = match fs::read(&marker_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let orphaned = [
                data.join("lifecycle-marker.checksum"),
                journal_path,
                data.join("lifecycle-journal.checksum"),
            ]
            .iter()
            .any(|path| path.exists());
            return if orphaned {
                LifecycleController::from_pending_marker(recovery_marker(
                    "orphaned-lifecycle-journal",
                ))
            } else {
                LifecycleController::new()
            };
        }
        Err(_) => {
            return LifecycleController::from_pending_marker(recovery_marker(
                "unreadable-lifecycle-marker",
            ));
        }
    };
    let marker: LifecycleMarker = match serde_json::from_slice(&marker_bytes) {
        Ok(marker) => marker,
        Err(_) => {
            return LifecycleController::from_pending_marker(recovery_marker(
                "invalid-lifecycle-marker",
            ));
        }
    };
    let marker_checksum = match fs::read_to_string(data.join("lifecycle-marker.checksum")) {
        Ok(checksum) => checksum,
        Err(_) => {
            return LifecycleController::from_pending_marker(recovery_marker(
                "missing-lifecycle-checksum",
            ));
        }
    };
    if marker_checksum.trim() != sim_platform_contract::lifecycle::marker_checksum(&marker) {
        return LifecycleController::from_pending_marker(recovery_marker(
            "invalid-lifecycle-checksum",
        ));
    }
    let journal: Vec<sim_platform_contract::lifecycle::LifecycleJournalEntry> =
        match fs::read(&journal_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(journal) => journal,
            None => {
                return LifecycleController::from_pending_marker(recovery_marker(
                    "invalid-lifecycle-journal",
                ));
            }
        };
    let journal_checksum = match fs::read_to_string(data.join("lifecycle-journal.checksum")) {
        Ok(checksum) => checksum,
        Err(_) => {
            return LifecycleController::from_pending_marker(recovery_marker(
                "missing-lifecycle-journal-checksum",
            ));
        }
    };
    if journal_checksum.trim() != sim_platform_contract::lifecycle::journal_checksum(&journal) {
        return LifecycleController::from_pending_marker(recovery_marker(
            "invalid-lifecycle-journal-checksum",
        ));
    }
    LifecycleController::from_pending_marker(marker)
}

fn recovery_marker(reason: &str) -> LifecycleMarker {
    LifecycleMarker {
        phase: LifecyclePhase::Recovery,
        reason: reason.into(),
        checkpoint_generation: None,
        deadline_ms: 0,
        armed_deadline: None,
        wake_source: None,
    }
}

fn sync_lifecycle_marker(root: &Path, lifecycle: &LifecycleController) -> Result<(), String> {
    let data = root.join("data");
    fs::create_dir_all(&data).map_err(|error| error.to_string())?;
    let marker_path = data.join("lifecycle-marker.json");
    let marker_checksum_path = data.join("lifecycle-marker.checksum");
    let journal_path = data.join("lifecycle-journal.json");
    let journal_checksum_path = data.join("lifecycle-journal.checksum");
    let evidence = lifecycle.evidence();
    match evidence.marker {
        Some(marker) => {
            let marker_checksum = sim_platform_contract::lifecycle::marker_checksum(&marker);
            write_json(marker_path, &marker).map_err(|error| error.to_string())?;
            write_bytes(marker_checksum_path, marker_checksum.as_bytes())
                .map_err(|error| error.to_string())?;
            let journal_checksum =
                sim_platform_contract::lifecycle::journal_checksum(&evidence.journal);
            write_json(journal_path, &evidence.journal).map_err(|error| error.to_string())?;
            write_bytes(journal_checksum_path, journal_checksum.as_bytes())
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        None => {
            for path in [
                marker_path,
                marker_checksum_path,
                journal_path,
                journal_checksum_path,
            ] {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            Ok(())
        }
    }
}

fn hardware_json(hardware: &sim_platform_contract::HardwareState) -> Value {
    json!({
        "battery": {"percent": hardware.battery_percent, "charging": hardware.charging},
        "externalPower": hardware.external_power,
        "storage": {"mode": hardware.storage_mode},
        "radio": {"enabled": hardware.radio_enabled, "connected": hardware.radio_connected},
        "suspend": {"state": hardware.suspend_state, "result": hardware.suspend_result},
    })
}

fn session_state(state: &AppState) -> SessionState {
    if state.active_session.is_some() {
        SessionState::Started
    } else if state
        .last_session
        .as_ref()
        .is_some_and(|result| result.reason == "success")
    {
        SessionState::Completed
    } else {
        SessionState::Aborted
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

fn launch_request(
    entry: &sim_domain::CatalogEntry,
    catalog: &LaunchCatalog,
) -> Result<LaunchRequest, String> {
    let (request_id, content_sha256, kind, package_id, core_id, profile_id) =
        match (entry.id.as_str(), entry.system.as_str()) {
            ("nebula-nes", "nes") => (
                "nebula-nes-request",
                NEBULA_CONTENT_SHA256,
                LaunchKind::Libretro,
                None,
                Some("generated-core"),
                "default",
            ),
            ("mirror-ps1", "ps1") => (
                "mirror-ps1-request",
                MIRROR_CONTENT_SHA256,
                LaunchKind::Libretro,
                None,
                Some("generated-core"),
                "default",
            ),
            ("orbit-garden", "portmaster") => (
                "orbit-garden-request",
                ORBIT_CONTENT_SHA256,
                LaunchKind::Portmaster,
                Some("orbit-garden"),
                None,
                "generated-default",
            ),
            ("signal-workshop", "portmaster") => (
                "signal-workshop-request",
                SIGNAL_CONTENT_SHA256,
                LaunchKind::Portmaster,
                Some("signal-workshop"),
                None,
                "generated-default",
            ),
            _ => return Err("selected demo is not allowlisted".to_string()),
        };
    let runner = catalog
        .runners
        .iter()
        .find(|runner| runner.kinds.contains(&kind))
        .ok_or_else(|| "catalog has no approved runner for selected demo".to_string())?;
    let core = core_id.and_then(|id| {
        catalog
            .cores
            .iter()
            .find(|core| core.id == id && core.kind == kind && core.runner_id == runner.id)
    });
    if core_id.is_some() && core.is_none() {
        return Err("catalog has no compatible core for selected demo".to_string());
    }
    let profile = catalog
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "catalog has no selected profile".to_string())?;
    Ok(LaunchRequest {
        schema: launch_contract::REQUEST_SCHEMA.to_string(),
        format: "brickpro-launch-request".to_string(),
        schema_version: 1,
        request_id: request_id.to_string(),
        kind,
        content_id: entry.id.clone(),
        content_sha256: content_sha256.to_string(),
        content_path: LogicalPath {
            root: PathRoot::Roms,
            relative: format!("{}.synthetic", entry.id),
        },
        save_path: LogicalPath {
            root: PathRoot::DataSaves,
            relative: format!("{}.save", entry.id),
        },
        state_path: LogicalPath {
            root: PathRoot::DataStates,
            relative: format!("{}.state", entry.id),
        },
        runner: VersionedId {
            id: runner.id.clone(),
            version: runner.version.clone(),
        },
        package: package_id.map(|id| VersionedId {
            id: id.to_string(),
            version: "1.0.0".to_string(),
        }),
        core: core.map(|core| VersionedId {
            id: core.id.clone(),
            version: core.version.clone(),
        }),
        profile_id: profile.id.clone(),
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
    })
}

fn handle_save_vault_button(state: &mut AppState, button: Button) -> Result<bool> {
    if state.save_vault.screen == "hidden" {
        return Ok(false);
    }
    match (state.save_vault.screen.as_str(), button) {
        ("history", Button::Primary) => {
            state.save_vault.preview = Some(
                state
                    .broker
                    .save_vault_preview()
                    .map_err(|error| anyhow!(error.to_string()))?,
            );
            state.save_vault.screen = "preview".into();
        }
        ("preview", Button::Primary) => {
            state.save_vault.confirmed = true;
            state.save_vault.screen = "confirm".into();
        }
        ("confirm", Button::Primary) => {
            state
                .broker
                .save_vault_restore(true)
                .map_err(|error| anyhow!(error.to_string()))?;
            state.save_vault.screen = "restored".into();
        }
        (_, Button::Secondary) => {
            state.save_vault.confirmed = false;
            state.save_vault.screen = "cancelled".into();
        }
        _ => {}
    }
    Ok(true)
}

fn save_vault_json(state: &AppState) -> Value {
    let preview = state.save_vault.preview.as_ref().map(|value| {
        json!({
            "generation": value.generation,
            "runnerVersion": value.runner_version,
            "coreVersion": value.core_version,
            "oldSize": value.old_size,
            "newSize": value.new_size,
            "oldHashStatus": value.old_hash_status,
            "newHashStatus": value.new_hash_status,
            "oldHashPrefix": value.old_hash_prefix,
            "newHashPrefix": value.new_hash_prefix,
            "affectedKinds": value.affected_kinds,
            "reason": value.reason,
            "timestampMs": value.timestamp_ms,
        })
    });
    json!({
        "screen": state.save_vault.screen,
        "historyCount": state.save_vault.history_count,
        "protectedCount": state.save_vault.protected_count,
        "preview": preview,
        "confirmed": state.save_vault.confirmed,
    })
}

fn handle_presentation_action(
    state: &mut PresentationState,
    action: input_profile::Action,
) -> Result<()> {
    if let Some(button) = to_keyboard_button(action) {
        if matches!(state.ui.route, ui_model::Route::Settings) {
            state
                .settings
                .press(button)
                .map_err(|error| anyhow!(error.to_string()))?;
            return Ok(());
        }
        if matches!(state.ui.route, ui_model::Route::Wifi(_)) {
            state
                .wifi
                .press(button)
                .map_err(|error| anyhow!(error.to_string()))?;
            return Ok(());
        }
    }
    let action = match action {
        input_profile::Action::MoveUp => UiAction::MoveSelection(ui_model::Direction::Up),
        input_profile::Action::MoveDown => UiAction::MoveSelection(ui_model::Direction::Down),
        input_profile::Action::MoveLeft => UiAction::MoveSelection(ui_model::Direction::Left),
        input_profile::Action::MoveRight => UiAction::MoveSelection(ui_model::Direction::Right),
        input_profile::Action::Primary => UiAction::ActivateSelected,
        input_profile::Action::Secondary => UiAction::Back,
        input_profile::Action::Start => UiAction::Navigate(ui_model::Route::Home),
        input_profile::Action::Select | input_profile::Action::Home => {
            UiAction::SetFocus(ui_model::FocusTarget::Menu)
        }
        input_profile::Action::JumpNextGroup
        | input_profile::Action::JumpPreviousGroup
        | input_profile::Action::LeftStickClick
        | input_profile::Action::RightStickClick
        | input_profile::Action::F1
        | input_profile::Action::F2
        | input_profile::Action::Fn => return Ok(()),
    };
    state.ui = ui_model::reduce(&state.ui, action);
    Ok(())
}

fn to_keyboard_button(action: input_profile::Action) -> Option<virtual_keyboard::Button> {
    Some(match action {
        input_profile::Action::MoveUp => virtual_keyboard::Button::Up,
        input_profile::Action::MoveDown => virtual_keyboard::Button::Down,
        input_profile::Action::MoveLeft => virtual_keyboard::Button::Left,
        input_profile::Action::MoveRight => virtual_keyboard::Button::Right,
        input_profile::Action::Primary => virtual_keyboard::Button::Primary,
        input_profile::Action::Secondary => virtual_keyboard::Button::Secondary,
        input_profile::Action::Start => virtual_keyboard::Button::Start,
        input_profile::Action::Select => virtual_keyboard::Button::Select,
        input_profile::Action::Home => virtual_keyboard::Button::Menu,
        _ => return None,
    })
}

fn present<P: Platform>(platform: &mut P, screen: &PresentationScreen) -> Result<()> {
    platform.present(screen).map_err(|error| anyhow!("{error}"))
}

fn selected_catalog_index(state: &AppState, catalog: &UiCatalog) -> usize {
    catalog
        .entries
        .iter()
        .position(|entry| entry.id == state.selected_content_id)
        .unwrap_or(0)
}

fn route_selection<'a>(route: &Route, catalog: &'a UiCatalog, selected_index: usize) -> &'a str {
    match route {
        Route::Library => "library",
        Route::Systems => catalog.entries[selected_index].system.as_str(),
        Route::Games | Route::Session => catalog.entries[selected_index].id.as_str(),
        Route::Catalog => "library",
        Route::GameSwitcher => "game-switcher",
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
        Button::L1 => "l1",
        Button::R1 => "r1",
        Button::Menu => "menu",
    }
}

fn action_name(action: ButtonAction) -> &'static str {
    match action {
        ButtonAction::Press => "press",
        ButtonAction::Release => "release",
    }
}
