mod control;
mod launcher_state;
pub mod rom_index;
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
    LifecycleCheckpointPolicy, SessionHandle, SessionResult,
};
use settings_schema::{ProjectionContext, Registry};
use settings_ui::SettingsUi;
use sim_domain::{Catalog as UiCatalog, Route, SessionState};
use sim_platform_contract::{
    battery::{
        BatteryDecision, BatteryHealth, BatteryObservation, BatteryPolicy, BatteryPolicyController,
        ChargingStatus, LowBatteryAction, PolicyAction,
    },
    lifecycle::{
        CheckpointHook, LifecycleClock, LifecycleController, LifecycleFault, LifecycleMarker,
        LifecyclePhase, ResumeRequest, SuspendRequest, WakeSource, DEFAULT_SLEEP_DURATION_MINUTES,
    },
    power::PowerPolicyController,
    tg4040::{BluetoothPhase, BluetoothRole, InputSignal, LedSettings, Tg4040State},
    Button, ButtonAction, ButtonEvent, HardwareChanges, LedState, Platform, PlatformError,
    PlatformResult, RumbleState, StorageMode,
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
const CONTROLLER_ROUTE_COUNT: usize = 66;
const PARENT_GESTURE: [Button; 4] = [Button::Start, Button::Select, Button::Start, Button::Select];
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
    input_mappings: input_profile::InputMappings,
    broker: simulator_session::SimulatorSessionAdapter,
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
    power: PowerPolicyController,
    battery: BatteryPolicyController,
    journey: ProductJourneyState,
    controller_routes: bool,
    resume_content: Option<DemoContent>,
    resume_marker: u32,
    package_root: PathBuf,
    shutdown_pressed: bool,
    exit_requested: bool,
    parent_gesture: usize,
}

#[derive(Clone, Debug)]
enum ProductJourneyState {
    Home {
        selected: HomeItem,
    },
    Systems {
        selected: SystemItem,
    },
    Games {
        surface: GameSurface,
    },
    Settings {
        section: SettingsSection,
        pending: Option<SettingChange>,
        validation: Option<&'static str>,
    },
    Wifi {
        view: WifiJourneyView,
    },
    Theme {
        stage: ThemeJourneyStage,
    },
    Scraper {
        stage: ScraperJourneyStage,
    },
    Diagnostics {
        page: DiagnosticPage,
    },
    Session {
        content: DemoContent,
        marker: u32,
        restored: bool,
    },
    QuickMenu {
        selected: QuickMenuItem,
        content: DemoContent,
        marker: u32,
        preview: &'static str,
    },
    GameSwitcher {
        page: SwitcherPage,
        content: DemoContent,
        marker: u32,
    },
    Portmaster {
        page: PortmasterPage,
    },
    ShutdownConfirm,
    FocusAdmin {
        selected: FocusAdminItem,
    },
    FocusHome {
        selected: usize,
    },
    FocusRecovery,
    KidQuickMenu {
        selected: KidQuickItem,
        content: DemoContent,
        marker: u32,
        saved: bool,
    },
}

impl Default for ProductJourneyState {
    fn default() -> Self {
        Self::Home {
            selected: HomeItem::Systems,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum FocusAdminItem {
    Add,
    Remove,
    MoveUp,
    MoveDown,
    DefaultHome,
    KidSafe,
}

#[derive(Clone, Copy, Debug)]
enum KidQuickItem {
    Continue,
    Save,
    Exit,
}

#[derive(Clone, Copy, Debug)]
enum HomeItem {
    Systems,
    Favorites,
    Recent,
    Settings,
}
#[derive(Clone, Copy, Debug)]
enum SystemItem {
    Library,
    Nebula,
    Mirror,
    Portmaster,
}
#[derive(Clone, Copy, Debug)]
enum GameSurface {
    List,
    Details,
    Favorite,
    Favorites,
    Recent,
    SearchKeyboard,
    SearchResults,
}
#[derive(Clone, Copy, Debug)]
enum SettingsSection {
    Root,
    Display,
    Input,
    Audio,
    Power,
    Library,
    Scraper,
    Theme,
    System,
}
#[derive(Clone, Debug)]
struct SettingChange {
    name: &'static str,
    old: &'static str,
    new: &'static str,
}
#[derive(Clone, Copy, Debug)]
enum WifiJourneyView {
    Scan,
    OpenConfirmation,
    Password,
    Hidden,
    ManualSsid,
    Saved,
    ForgetConfirmation,
    Forgotten,
    Progress,
    RetryError,
}
#[derive(Clone, Copy, Debug)]
enum ThemeJourneyStage {
    Catalog,
    Preview,
    Install,
    Update,
    Remove,
    Fallback,
}
#[derive(Clone, Copy, Debug)]
enum ScraperJourneyStage {
    Settings,
    Game,
    Queue,
    Progress,
    Paused,
    Ambiguity,
    Success,
    Failure,
}
#[derive(Clone, Copy, Debug)]
enum DiagnosticPage {
    Root,
    SafeMode,
    Updater,
    Rollback,
    StorageFull,
    LowBattery,
}
#[derive(Clone, Copy, Debug)]
enum DemoContent {
    Orbit,
    Signal,
    Nebula,
    Mirror,
}
#[derive(Clone, Copy, Debug)]
enum QuickMenuItem {
    Continue,
    SaveSlot,
    LoadSlot,
    Restart,
    RetroArch,
    Exit,
}

#[derive(Clone, Copy, Debug)]
enum SwitcherPage {
    Autosave,
    Exit,
    List,
    Resume,
    Restoration,
}
#[derive(Clone, Copy, Debug)]
enum PortmasterPage {
    Catalog,
    Install,
    UninstallProtected,
}

fn canonical_route_id(state: &ProductJourneyState) -> Option<&'static str> {
    use ProductJourneyState::*;
    match state {
        Home { .. } => None,
        Systems { .. } => Some("home-systems"),
        Games {
            surface: GameSurface::List,
        } => Some("home-game-list"),
        Games {
            surface: GameSurface::Details,
        } => Some("games-details"),
        Games {
            surface: GameSurface::Favorite,
        } => Some("games-favorite-toggle"),
        Games {
            surface: GameSurface::Favorites,
        } => Some("games-favorites"),
        Games {
            surface: GameSurface::Recent,
        } => Some("games-recent"),
        Games {
            surface: GameSurface::SearchKeyboard,
        } => Some("games-search-keyboard"),
        Games {
            surface: GameSurface::SearchResults,
        } => Some("games-search-results"),
        Settings {
            section: SettingsSection::Root,
            ..
        } => Some("settings-root"),
        Settings {
            section: SettingsSection::Display,
            pending: None,
            ..
        } => Some("settings-display"),
        Settings {
            section: SettingsSection::Display,
            pending: Some(_),
            ..
        } => Some("settings-confirm-apply-cancel"),
        Settings {
            section: SettingsSection::Input,
            validation: Some(_),
            ..
        } => Some("settings-validation"),
        Settings {
            section: SettingsSection::Input,
            ..
        } => Some("settings-input"),
        Settings {
            section: SettingsSection::Audio,
            ..
        } => Some("settings-audio"),
        Settings {
            section: SettingsSection::Power,
            ..
        } => Some("settings-power"),
        Settings {
            section: SettingsSection::Library,
            ..
        } => Some("settings-library"),
        Settings {
            section: SettingsSection::Scraper,
            ..
        } => Some("settings-scraper"),
        Settings {
            section: SettingsSection::Theme,
            ..
        } => Some("settings-theme"),
        Settings {
            section: SettingsSection::System,
            ..
        } => Some("settings-system"),
        Wifi { view } => Some(match view {
            WifiJourneyView::Scan => "wifi-scan",
            WifiJourneyView::OpenConfirmation => "wifi-open-confirmation",
            WifiJourneyView::Password => "wifi-secure-password",
            WifiJourneyView::Hidden => "wifi-hidden",
            WifiJourneyView::ManualSsid => "wifi-manual-ssid",
            WifiJourneyView::Saved => "wifi-saved-network",
            WifiJourneyView::ForgetConfirmation => "wifi-forget-confirmation",
            WifiJourneyView::Forgotten => "wifi-forgotten",
            WifiJourneyView::Progress => "wifi-connect-progress",
            WifiJourneyView::RetryError => "wifi-retry-error",
        }),
        Theme { stage } => Some(match stage {
            ThemeJourneyStage::Catalog => "theme-garden-catalog",
            ThemeJourneyStage::Preview => "theme-garden-preview",
            ThemeJourneyStage::Install => "theme-garden-install",
            ThemeJourneyStage::Update => "theme-garden-update",
            ThemeJourneyStage::Remove => "theme-garden-remove",
            ThemeJourneyStage::Fallback => "theme-garden-fallback",
        }),
        Scraper { stage } => Some(match stage {
            ScraperJourneyStage::Settings => "scraper-settings",
            ScraperJourneyStage::Game => "scraper-game",
            ScraperJourneyStage::Queue => "scraper-queue",
            ScraperJourneyStage::Progress => "scraper-progress",
            ScraperJourneyStage::Paused => "scraper-paused",
            ScraperJourneyStage::Ambiguity => "scraper-ambiguity",
            ScraperJourneyStage::Success => "scraper-success",
            ScraperJourneyStage::Failure => "scraper-failure",
        }),
        Diagnostics { page } => Some(match page {
            DiagnosticPage::Root => "diagnostics",
            DiagnosticPage::SafeMode => "diagnostics-safe-mode",
            DiagnosticPage::Updater => "updater-available",
            DiagnosticPage::Rollback => "updater-rollback",
            DiagnosticPage::StorageFull => "faults-storage-full",
            DiagnosticPage::LowBattery => "faults-low-battery",
        }),
        Session {
            content, restored, ..
        } => Some(match (content, restored) {
            (DemoContent::Orbit, _) => "portmaster-launch-orbit",
            (DemoContent::Signal, _) => "portmaster-launch-signal",
            (DemoContent::Nebula, false) => "platform-nebula-launch",
            (DemoContent::Nebula, true) => "platform-nebula-restored",
            (DemoContent::Mirror, false) => "platform-mirror-launch",
            (DemoContent::Mirror, true) => "platform-mirror-restored",
        }),
        QuickMenu { .. } => Some("game-quick-menu"),
        GameSwitcher { page, .. } => Some(match page {
            SwitcherPage::Autosave => "game-switcher-autosave",
            SwitcherPage::Exit => "game-switcher-exit",
            SwitcherPage::List => "game-switcher-list",
            SwitcherPage::Resume => "game-switcher-resume",
            SwitcherPage::Restoration => "game-switcher-restoration",
        }),
        Portmaster { page } => Some(match page {
            PortmasterPage::Catalog => "portmaster-catalog",
            PortmasterPage::Install => "portmaster-install",
            PortmasterPage::UninstallProtected => "portmaster-uninstall-protected-data",
        }),
        ShutdownConfirm => Some("shutdown-confirm"),
        FocusAdmin { .. } => Some("focus-admin"),
        FocusHome { .. } => Some("focus-home"),
        FocusRecovery => Some("focus-recovery"),
        KidQuickMenu { .. } => Some("kid-quick-menu"),
    }
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

#[derive(Debug, Deserialize)]
struct DeviceCompatibility {
    display: DisplayCompatibility,
}

#[derive(Debug, Deserialize)]
struct DisplayCompatibility {
    #[serde(rename = "defaultTheme")]
    default_theme: String,
    #[serde(rename = "themeAspect")]
    theme_aspect: String,
}

fn device_config_path(target_sku: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/platform")
        .join(target_sku.to_ascii_lowercase())
        .join("compatibility.json")
}

fn bundled_theme_for_device(target_sku: &str) -> Result<ValidatedTheme> {
    let config_path = device_config_path(target_sku);
    let config: DeviceCompatibility = serde_json::from_slice(&fs::read(config_path)?)?;
    let aspect = config.display.theme_aspect.replace(':', "-");
    let theme_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../themes/upstream")
        .join(&config.display.default_theme);
    launcher_theme::load_bundled_theme(&theme_path, &aspect)
        .map_err(|error| anyhow!(error.to_string()))
}

fn selected_theme_for_device(
    target_sku: &str,
) -> Result<(ValidatedTheme, Option<launcher_theme::Reason>)> {
    let Some(path) = std::env::var_os("TRIMUI_THEME_DIR") else {
        return Ok((bundled_theme_for_device(target_sku)?, None));
    };
    let device = device_profile::DeviceProfile::from_path(&device_config_path(target_sku))
        .map_err(|error| anyhow!(error.to_string()))?;
    match launcher_theme::load_theme_dir(Path::new(&path))
        .and_then(|theme| launcher_theme::validate_for_device(theme, &device))
    {
        Ok(theme) => Ok((theme, None)),
        Err(error) => Ok((bundled_theme_for_device(target_sku)?, Some(error.reason))),
    }
}

struct PresentationState {
    ui: ui_model::UiState,
    theme: ValidatedTheme,
    theme_fallback: Option<launcher_theme::Reason>,
    theme_garden: bool,
    metadata_off: bool,
    settings: SettingsUi,
    wifi: WifiSettingsController,
    index: launcher_presentation::IndexView,
    recent: Vec<String>,
}

impl PresentationState {
    fn new(target_sku: &str) -> Result<Self> {
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
            "cap.power.charging-led".into(),
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
        let (theme, theme_fallback) = selected_theme_for_device(target_sku)?;
        Ok(Self {
            ui,
            theme,
            theme_fallback,
            theme_garden: false,
            metadata_off: false,
            settings,
            wifi,
            index: launcher_presentation::IndexView::default(),
            recent: Vec::new(),
        })
    }

    fn restore_visual_preferences(&mut self, preferences: &ui_model::UiPreferences) -> Result<()> {
        self.settings
            .set_value(
                "core.display.visual-preset",
                settings_schema::SettingValue::EnumSingle(
                    visual_preset_name(preferences.visual_preset).into(),
                ),
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        self.settings
            .set_value(
                "core.display.night-schedule",
                settings_schema::SettingValue::EnumSingle(
                    night_schedule_name(preferences.night_schedule).into(),
                ),
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        for change in [
            ui_model::PreferenceChange::VisualPreset(preferences.visual_preset),
            ui_model::PreferenceChange::NightSchedule(preferences.night_schedule),
        ] {
            self.ui = ui_model::reduce(&self.ui, UiAction::SetPreference(change));
        }
        Ok(())
    }

    fn sync_visual_preferences(&mut self) -> Result<()> {
        let scene = self
            .settings
            .scene()
            .map_err(|error| anyhow!(error.to_string()))?;
        let value = |id: &str| {
            scene
                .sections
                .iter()
                .flat_map(|section| &section.groups)
                .flat_map(|group| &group.controls)
                .find(|control| control.setting_id == id)
                .and_then(|control| match &control.value {
                    settings_ui::SemanticValue::EnumSingle(value) => Some(value.as_str()),
                    _ => None,
                })
        };
        if let Some(preset) = value("core.display.visual-preset").and_then(visual_preset) {
            self.ui = ui_model::reduce(
                &self.ui,
                UiAction::SetPreference(ui_model::PreferenceChange::VisualPreset(preset)),
            );
        }
        if let Some(schedule) = value("core.display.night-schedule").and_then(night_schedule) {
            self.ui = ui_model::reduce(
                &self.ui,
                UiAction::SetPreference(ui_model::PreferenceChange::NightSchedule(schedule)),
            );
        }
        Ok(())
    }

    fn reset_visual_preferences(&mut self) -> Result<()> {
        self.restore_visual_preferences(&ui_model::UiPreferences::default())
    }

    fn screen(&self) -> Result<PresentationScreen> {
        let settings = self
            .settings
            .scene()
            .map_err(|error| anyhow!(error.to_string()))?;
        let wifi = self.wifi.snapshot();
        let mut screen = launcher_presentation::build_with_recent(
            &self.ui,
            &self.theme,
            self.theme_fallback,
            Some(&settings),
            Some(&wifi),
            &self.index,
            &self.recent,
        );
        if self.theme_fallback.is_some() {
            screen.route = "recovery".into();
            screen.title = "Safe theme restored".into();
            screen.selected_label =
                "Invalid theme rejected — open Theme Garden to choose another".into();
        }
        Ok(screen)
    }
}

fn screen_for_state(state: &AppState, catalog: &UiCatalog) -> Result<PresentationScreen> {
    let mut screen = state.presentation.screen()?;
    if state.presentation.metadata_off && state.presentation.ui.route == ui_model::Route::Games {
        screen.route = "games-no-metadata".into();
    }
    if state.presentation.theme_garden {
        screen.route = "theme-garden".into();
        screen.focus = "preview".into();
        screen.title = "THEME GARDEN".into();
        screen.selected_label = format!("ACTIVE THEME: {}", state.presentation.theme.name());
        screen.system_media = launcher_presentation::media_for_system("portmaster");
    }
    if matches!(state.route, Route::Games | Route::Session)
        && state.presentation.ui.route != ui_model::Route::Systems
    {
        let entries = if matches!(state.journey, ProductJourneyState::FocusHome { .. }) {
            focus_catalog_entries(state, catalog)
        } else {
            catalog.entries.iter().collect()
        };
        let selected_index = entries
            .iter()
            .position(|entry| entry.id == state.selected_content_id)
            .unwrap_or(0);
        screen.game_rows = entries
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
        let entry = entries[selected_index];
        screen.selected_label = entry.title.clone();
        screen.selected_game =
            launcher_presentation::catalog_game_details(&entry.id, &entry.title, &entry.system);
        screen.game_media = launcher_presentation::media_for_content(&entry.id);
        if !matches!(state.presentation.theme.name(), "SimpleLife" | "Techdweeb") {
            screen.system_media = launcher_presentation::media_for_system(&entry.system);
        }
    } else if state.route == Route::Systems
        || state.presentation.ui.route == ui_model::Route::Systems
    {
        let system_id = if screen.selected_label == "LUMA STATION" {
            "portmaster"
        } else {
            "nes"
        };
        if !matches!(state.presentation.theme.name(), "SimpleLife" | "Techdweeb") {
            screen.system_media = launcher_presentation::media_for_system(system_id);
        }
    }
    if matches!(state.route, Route::Session)
        && matches!(state.presentation.ui.route, ui_model::Route::Games)
        && state.active_session.is_some()
        && !state.presentation.theme_garden
    {
        let entry = &catalog.entries[selected_catalog_index(state, catalog)];
        screen.route = "session".into();
        screen.title = entry.title.clone();
        screen.modal = Some(format!("{} FRAME {}", entry.title, state.session_step));
    }
    if state.presentation.theme_garden {
        screen.selected_game = None;
        screen.game_media.clear();
        if !matches!(state.presentation.theme.name(), "SimpleLife" | "Techdweeb") {
            screen.system_media = launcher_presentation::media_for_system("portmaster");
        } else {
            screen.system_media = None;
        }
    }
    screen.save_sync = state.save_sync.clone();
    if let Some(id) = canonical_route_id(&state.journey) {
        screen.route = id.into();
        screen.menu = product_surface_rows(&state.journey);
        screen.title = screen
            .menu
            .first()
            .map_or_else(|| "PRODUCT".into(), |row| row.label.clone());
        screen.selected_label = screen
            .menu
            .iter()
            .find(|row| row.selected)
            .map_or_else(|| "Ready".into(), |row| row.label.clone());
        match &state.journey {
            ProductJourneyState::Games {
                surface: GameSurface::Details | GameSurface::Favorite,
            } => {
                screen.selected_game = launcher_presentation::catalog_game_details(
                    "nebula-nes",
                    "Nebula Notes",
                    "nes",
                );
                screen.game_media = launcher_presentation::media_for_content("nebula-nes");
                screen.focus = "Launch action selected".into();
            }
            ProductJourneyState::Games {
                surface: GameSurface::SearchKeyboard,
            } => {
                screen.focus = "Keyboard focus: Q · text cursor after Nebula".into();
            }
            ProductJourneyState::Wifi {
                view: WifiJourneyView::Password,
            } => {
                screen.focus = "Keyboard focus: Q · masked cursor at 12".into();
            }
            ProductJourneyState::Wifi {
                view: WifiJourneyView::ManualSsid,
            } => {
                screen.focus = "Keyboard focus: Q · text cursor after Home-Lab".into();
            }
            ProductJourneyState::Theme {
                stage: ThemeJourneyStage::Preview,
            } => {
                screen.system_media = launcher_presentation::media_for_system("portmaster");
                screen.focus = "Preview canvas · Apply selected".into();
            }
            ProductJourneyState::FocusAdmin { .. }
            | ProductJourneyState::FocusHome { .. }
            | ProductJourneyState::FocusRecovery
            | ProductJourneyState::KidQuickMenu { .. } => {
                screen.menu = focus_rows(state, catalog);
                screen.title = match state.journey {
                    ProductJourneyState::FocusAdmin { .. } => "Focus library".into(),
                    ProductJourneyState::FocusHome { .. } => "Focus library".into(),
                    ProductJourneyState::FocusRecovery => "Focus needs attention".into(),
                    ProductJourneyState::KidQuickMenu { .. } => "Game menu".into(),
                    _ => unreachable!(),
                };
                screen.selected_label = screen
                    .menu
                    .iter()
                    .find(|row| row.selected)
                    .map_or_else(|| "Ready".into(), |row| row.label.clone());
            }
            _ => {}
        }
    }
    Ok(screen)
}

fn product_surface_rows(state: &ProductJourneyState) -> Vec<launcher_presentation::ScreenItem> {
    use ProductJourneyState::*;
    let rows: Vec<String> = match state {
        Home { selected } => ["Systems", "Favorites", "Recent", "System menu"]
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                format!(
                    "{}{}",
                    if index == *selected as usize {
                        "> "
                    } else {
                        "  "
                    },
                    row
                )
            })
            .collect(),
        Systems { selected } => [
            "Game library",
            "Nebula Notes (NES)",
            "Mirror Museum (PS1)",
            "PortMaster",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            format!(
                "{}{}",
                if index == *selected as usize {
                    "> "
                } else {
                    "  "
                },
                row
            )
        })
        .collect(),
        Games {
            surface: GameSurface::List,
        } => vec![
            "Nebula Notes".into(),
            "Mirror Museum".into(),
            "Artwork · rating · release date".into(),
        ],
        Games {
            surface: GameSurface::Details,
        } => vec![
            "Nebula Notes".into(),
            "Chart a quiet starship through forgotten constellations.".into(),
            "Rating: 92% · Release date: 1994-04-12".into(),
            "[A] Launch game · Select: favourite".into(),
        ],
        Games {
            surface: GameSurface::Favorite,
        } => vec![
            "Nebula Notes".into(),
            "Favourite: ON".into(),
            "Saved to favorites".into(),
        ],
        Games {
            surface: GameSurface::Favorites,
        } => vec![
            "Favorites".into(),
            "Nebula Notes".into(),
            "Select: remove favourite".into(),
        ],
        Games {
            surface: GameSurface::Recent,
        } => vec![
            "Recent games".into(),
            "Mirror Museum".into(),
            "A: launch · B: back".into(),
        ],
        Games {
            surface: GameSurface::SearchKeyboard,
        } => vec![
            "Search games".into(),
            "Editable query: Nebula|".into(),
            "[Q] W E R T Y U I O P".into(),
            " A  S D F G H J K L".into(),
            " Z  X C V B N M  Space  Backspace".into(),
            "Start: search · B: Cancel".into(),
        ],
        Games {
            surface: GameSurface::SearchResults,
        } => vec![
            "Results for Nebula".into(),
            "Nebula Notes".into(),
            "1 match".into(),
        ],
        Settings {
            section: SettingsSection::Root,
            ..
        } => [
            "Library & gameplay",
            "Display",
            "Input",
            "Audio",
            "Power",
            "Advanced features",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Settings {
            section: _,
            pending: Some(change),
            ..
        } => vec![
            format!("{}: {} → {}", change.name, change.old, change.new),
            "A: Apply".into(),
            "B: Cancel".into(),
        ],
        Settings {
            section,
            validation: Some(reason),
            ..
        } => vec![
            format!("{:?}", section),
            format!("Validation: {reason}"),
            "Committed value retained".into(),
        ],
        Settings { section, .. } => settings_form_rows(*section),
        Wifi { view } => match view {
            WifiJourneyView::Scan => vec![
                "Wi-Fi".into(),
                "3 SSIDs · deduplicated scan results".into(),
                "Home Synthetic · WPA2 · SAVED · signal 91%".into(),
                "Known Synthetic · WPA3 · SAVED · signal 38%".into(),
                "Guest Synthetic · OPEN · NOT SAVED · signal 70%".into(),
                "Actions: Select network · Rescan · Add hidden".into(),
            ],
            WifiJourneyView::OpenConfirmation => vec![
                "Guest Synthetic · OPEN".into(),
                "Warning: open network has no password".into(),
                "Action: Confirm connect".into(),
                "Action: Back".into(),
            ],
            WifiJourneyView::Password => vec![
                "Home Synthetic · WPA2".into(),
                "Network key: ••••••••••••| (12 characters)".into(),
                "[Q] W E R T Y U I O P".into(),
                " A  S D F G H J K L".into(),
                " Z  X C V B N M  Space  Backspace".into(),
                "Action: Connect".into(),
                "Action: Cancel".into(),
            ],
            WifiJourneyView::Hidden => vec![
                "Hidden network · SSID redacted".into(),
                "Security: WPA2 or WPA3".into(),
                "Action: Enter network name".into(),
            ],
            WifiJourneyView::ManualSsid => vec![
                "Manual SSID · bounded text input (1–32 bytes)".into(),
                "Network name: Home-Lab|".into(),
                "[Q] W E R T Y U I O P".into(),
                " A  S D F G H J K L".into(),
                " Z  X C V B N M  Space  Backspace".into(),
                "Action: Continue · Cancel".into(),
            ],
            WifiJourneyView::Saved => vec![
                "Known Synthetic · WPA3 · SAVED".into(),
                "Reconnect automatically · enabled".into(),
                "Actions: Reconnect · Forget".into(),
            ],
            WifiJourneyView::ForgetConfirmation => vec![
                "Forget Known Synthetic?".into(),
                "Saved credentials will be removed".into(),
                "Action: Confirm Forget · Cancel".into(),
            ],
            WifiJourneyView::Forgotten => vec![
                "Known Synthetic · NOT SAVED".into(),
                "Network remains in scan results".into(),
                "Action: Rescan · Select network".into(),
            ],
            WifiJourneyView::Progress => vec![
                "Connecting to Home Synthetic · WPA2".into(),
                "Phase: authentication → address".into(),
                "Action: Cancel".into(),
            ],
            WifiJourneyView::RetryError => vec![
                "Home Synthetic · connection failed".into(),
                "Reason: authentication timeout".into(),
                "Action: Retry · Back to scan".into(),
            ],
        },
        Theme { stage } => match stage {
            ThemeJourneyStage::Catalog => vec![
                "Theme Garden · curated local catalog".into(),
                "Artbook · v1.0.0 · ACTIVE".into(),
                "High Contrast · v1.1.0 · UPDATE AVAILABLE".into(),
                "Minimal Grid · v1.0.0 · AVAILABLE".into(),
                "3 packages · screenshots available".into(),
                "Actions: Preview · Install · Update".into(),
            ],
            ThemeJourneyStage::Preview => vec![
                "High Contrast · v1.1.0".into(),
                "Live 4:3 preview canvas · generated library artwork".into(),
                "Status: update available".into(),
                "Actions: Apply · Back".into(),
            ],
            ThemeJourneyStage::Install => vec![
                "Installing Minimal Grid · v1.0.0".into(),
                "Package verified · screenshot cached".into(),
                "Result: installed · Apply available".into(),
            ],
            ThemeJourneyStage::Update => vec![
                "Updating High Contrast · v1.0.0 → v1.1.0".into(),
                "Package verified · preserving active theme".into(),
                "Result: update ready to apply".into(),
            ],
            ThemeJourneyStage::Remove => vec![
                "Remove High Contrast · v1.1.0?".into(),
                "Active Artbook remains usable".into(),
                "Actions: Remove · Keep".into(),
            ],
            ThemeJourneyStage::Fallback => vec![
                "Invalid theme rejected".into(),
                "Fallback active: Safe Art Book".into(),
                "Reason: theme validation failed".into(),
                "Route back: Theme Garden · B".into(),
            ],
        },
        Scraper { stage } => match stage {
            ScraperJourneyStage::Settings => vec![
                "Scraper providers · fixture-primary / fixture-secondary".into(),
                "Parallel workers: 2".into(),
                "Action: Start scrape".into(),
            ],
            ScraperJourneyStage::Game => vec![
                "Nebula Notes · NES".into(),
                "Providers: fixture-primary → fixture-secondary".into(),
                "Actions: Scrape game · Queue".into(),
            ],
            ScraperJourneyStage::Queue => vec![
                "Bulk queue · 4 titles · QUEUED".into(),
                "Nebula Notes · queued".into(),
                "Mirror Museum · queued".into(),
                "Orbit Garden · queued".into(),
                "Signal Workshop · queued".into(),
                "Action: Start queue".into(),
            ],
            ScraperJourneyStage::Progress => vec![
                "Bulk scrape · RUNNING · 1/4 · 2 slots".into(),
                "Nebula Notes · fixture-secondary · falling back".into(),
                "Mirror Museum · fixture-tertiary · searching".into(),
                "Fallback: fixture-primary not found → fixture-secondary".into(),
                "Actions: Pause · Results".into(),
            ],
            ScraperJourneyStage::Paused => vec![
                "Bulk scrape · PAUSED · 1/4".into(),
                "Reason: network gate".into(),
                "Nebula Notes · fallback pending".into(),
                "Action: Resume".into(),
            ],
            ScraperJourneyStage::Ambiguity => vec![
                "Nebula Notes · ambiguous result".into(),
                "Candidate: Nebula Notes (US)".into(),
                "Candidate: Nebula Notes (EU)".into(),
                "Action: Choose match".into(),
            ],
            ScraperJourneyStage::Success => vec![
                "Bulk scrape · COMPLETED 4/4".into(),
                "Found 2 · Fallback 1 · Not found 1".into(),
                "Nebula Notes · SUCCEEDED".into(),
                "Mirror Museum · SUCCEEDED".into(),
                "Orbit Garden · FALLBACK".into(),
                "Signal Workshop · NOT FOUND".into(),
                "Actions: Inspect results · Start again".into(),
            ],
            ScraperJourneyStage::Failure => vec![
                "Scrape failed · fixture-primary".into(),
                "Reason: provider unavailable".into(),
                "Nebula Notes · retry available".into(),
                "Action: Retry".into(),
            ],
        },
        Diagnostics { page } => match page {
            DiagnosticPage::Root => vec![
                "Diagnostics · health checks · action follows each result".into(),
                "Build/SKU · PASS · TG4040 verified".into(),
                "Storage · WARN · SD1 mounted · SD2 unavailable · choose internal storage".into(),
                "Battery/power · PASS · 78% · not charging".into(),
                "Input/display/audio · PASS · required controls and speaker route".into(),
                "Wi-Fi · UNAVAILABLE · network stays disabled".into(),
                "Last failed stage · UNAVAILABLE · retry only reported work".into(),
                "Last crash · crash-001 · watchdog-timeout".into(),
                "Support bundle · Preview included fields before export".into(),
            ],
            DiagnosticPage::SafeMode => vec![
                "Safe Mode · confirmation and policy".into(),
                "Network auto-start · DISABLED · Wi-Fi stays unavailable".into(),
                "Third-party themes and modules · DISABLED".into(),
                "Background indexing and auto-resume · DISABLED".into(),
                "Saves and diagnostics · READ ONLY".into(),
                "Recovery actions · reset UI · disable last module/theme · internal storage".into(),
                "Action: Confirm Safe Mode · Back cancels".into(),
            ],
            DiagnosticPage::Updater => vec![
                "Updater · available".into(),
                "Synthetic firmware · v1.1.0".into(),
                "Action: Install update".into(),
            ],
            DiagnosticPage::Rollback => vec![
                "Rollback · slot B available".into(),
                "Previous core · mainline v0.1.0".into(),
                "Action: Restore previous".into(),
            ],
            DiagnosticPage::StorageFull => vec![
                "Storage full · saves protected".into(),
                "Recovery: remove cache only".into(),
                "Action: Review storage".into(),
            ],
            DiagnosticPage::LowBattery => vec![
                "Low battery · autosave complete".into(),
                "Sleep now or cancel".into(),
                "Action: Sleep · Cancel".into(),
            ],
        },
        Session {
            content,
            marker,
            restored,
        } => vec![
            format!("{:?}", content),
            if *restored {
                format!("Restored interaction marker: {marker}")
            } else {
                format!("Interaction marker: {marker}")
            },
            "D-pad: interact · Menu: quick menu · B: autosave and exit".into(),
        ],
        QuickMenu {
            selected,
            content,
            marker,
            preview,
        } => [
            format!("Quick menu · {:?}", content),
            "Continue".into(),
            "Save slot 1".into(),
            "Load slot 1".into(),
            "Restart game".into(),
            "RetroArch menu".into(),
            "Exit game".into(),
            format!("Slot preview: {preview} · checkpoint {marker}"),
            "D-pad: choose · A: select · B: continue".into(),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            if index == *selected as usize + 1 {
                format!("> {row}")
            } else {
                format!("  {row}")
            }
        })
        .collect(),
        GameSwitcher {
            page,
            content,
            marker,
        } => vec![
            "Game Switcher".into(),
            format!("{:?} · checkpoint {marker}", content),
            format!("{:?}", page),
        ],
        Portmaster {
            page: PortmasterPage::Catalog,
        } => vec![
            "PortMaster catalog".into(),
            "Orbit Garden · 32-bit SDL · not installed".into(),
            "Signal Workshop · 64-bit OpenGL / GL4ES / Weston · not installed".into(),
            "Ready checks: runtime, libraries, audio, input, writable space, network".into(),
            "Not ready: missing game data · missing library · incompatible architecture · launch crash".into(),
            "USB/network imports: PortMaster/Imports · per-launch logs: PortMaster/Logs".into(),
        ],
        Portmaster {
            page: PortmasterPage::Install,
        } => vec![
            "Package install".into(),
            "Orbit Garden · signature verified · 32-bit SDL ready".into(),
            "Signal Workshop · signature verified · 64-bit GL4ES / Weston ready".into(),
            "Pinned runtime versions stay with each installed port; rollback is available".into(),
        ],
        Portmaster {
            page: PortmasterPage::UninstallProtected,
        } => vec![
            "Orbit Garden removed".into(),
            "Protected save retained".into(),
            "Signal Workshop remains installed".into(),
        ],
        FocusAdmin { .. } => vec!["Focus library".into()],
        FocusHome { .. } => vec!["Focus library".into()],
        FocusRecovery => vec!["Focus needs attention".into()],
        KidQuickMenu { .. } => vec!["Game menu".into()],
        ShutdownConfirm => vec![
            "Power off".into(),
            "A: orderly shutdown".into(),
            "B: cancel".into(),
        ],
    };
    rows.into_iter()
        .enumerate()
        .map(|(index, label)| launcher_presentation::ScreenItem {
            id: format!("product-{index}"),
            label,
            selected: index == 0,
            enabled: true,
        })
        .collect()
}

fn focus_rows(state: &AppState, catalog: &UiCatalog) -> Vec<launcher_presentation::ScreenItem> {
    match state.journey {
        ProductJourneyState::FocusAdmin { selected } => {
            let actions = [
                (FocusAdminItem::Add, "Add selected game"),
                (FocusAdminItem::Remove, "Remove selected game"),
                (FocusAdminItem::MoveUp, "Move selected up"),
                (FocusAdminItem::MoveDown, "Move selected down"),
                (FocusAdminItem::DefaultHome, "Use Focus as home"),
                (FocusAdminItem::KidSafe, "Kid-safe mode"),
            ];
            let mut rows = actions
                .into_iter()
                .map(|(item, label)| launcher_presentation::ScreenItem {
                    id: format!("focus-{label}"),
                    label: match item {
                        FocusAdminItem::DefaultHome => {
                            format!(
                                "{label}: {}",
                                if state.persisted.focus_home {
                                    "ON"
                                } else {
                                    "OFF"
                                }
                            )
                        }
                        FocusAdminItem::KidSafe => {
                            format!(
                                "{label}: {}",
                                if state.persisted.kid_safe {
                                    "ON"
                                } else {
                                    "OFF"
                                }
                            )
                        }
                        _ => label.into(),
                    },
                    selected: std::mem::discriminant(&item) == std::mem::discriminant(&selected),
                    enabled: !matches!(item, FocusAdminItem::KidSafe)
                        || !state.persisted.focus.is_empty(),
                })
                .collect::<Vec<_>>();
            rows.extend(state.persisted.focus.iter().map(|id| {
                launcher_presentation::ScreenItem {
                    id: id.clone(),
                    label: catalog
                        .entries
                        .iter()
                        .find(|entry| entry.id == *id)
                        .map_or_else(|| format!("Missing: {id}"), |entry| entry.title.clone()),
                    selected: false,
                    enabled: true,
                }
            }));
            rows
        }
        ProductJourneyState::FocusHome { selected } => focus_catalog_entries(state, catalog)
            .into_iter()
            .enumerate()
            .map(|(index, entry)| launcher_presentation::ScreenItem {
                id: entry.id.clone(),
                label: entry.title.clone(),
                selected: index == selected,
                enabled: true,
            })
            .collect(),
        ProductJourneyState::FocusRecovery => vec![launcher_presentation::ScreenItem {
            id: "focus-recovery".into(),
            label: "Focus entries are missing. Parent gesture opens the full library.".into(),
            selected: true,
            enabled: true,
        }],
        ProductJourneyState::KidQuickMenu {
            selected, saved, ..
        } => [
            (KidQuickItem::Continue, "Continue"),
            (KidQuickItem::Save, if saved { "Saved" } else { "Save" }),
            (KidQuickItem::Exit, "Exit game"),
        ]
        .into_iter()
        .map(|(item, label)| launcher_presentation::ScreenItem {
            id: format!("kid-{label}"),
            label: label.into(),
            selected: std::mem::discriminant(&item) == std::mem::discriminant(&selected),
            enabled: true,
        })
        .collect(),
        _ => Vec::new(),
    }
}

fn focus_catalog_entries<'a>(
    state: &AppState,
    catalog: &'a UiCatalog,
) -> Vec<&'a sim_domain::CatalogEntry> {
    state
        .persisted
        .focus
        .iter()
        .filter_map(|id| catalog.entries.iter().find(|entry| entry.id == *id))
        .collect()
}

fn demo_content(id: &str) -> Option<DemoContent> {
    match id {
        "nebula-nes" => Some(DemoContent::Nebula),
        "mirror-ps1" => Some(DemoContent::Mirror),
        "orbit-garden" => Some(DemoContent::Orbit),
        "signal-workshop" => Some(DemoContent::Signal),
        _ => None,
    }
}

fn enter_focus_home(state: &mut AppState, catalog: &UiCatalog) {
    if !state.persisted.focus_home && !state.persisted.kid_safe {
        return;
    }
    let entries = focus_catalog_entries(state, catalog);
    if entries.is_empty() {
        state.journey = ProductJourneyState::FocusRecovery;
        state.route = Route::Library;
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::Navigate(ui_model::Route::Home),
        );
        state.modal = Some("Focus needs attention".into());
    } else {
        state.selected_content_id = entries[0].id.clone();
        state.journey = ProductJourneyState::FocusHome { selected: 0 };
        state.route = Route::Games;
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::Navigate(ui_model::Route::Games),
        );
    }
}

fn settings_form_rows(section: SettingsSection) -> Vec<String> {
    let (title, control, value, help, apply, badge) = match section {
        SettingsSection::Display => (
            "Display settings",
            "Scaling mode",
            "Aspect",
            "Preserve the source aspect ratio without cropping.",
            "Apply mode: immediate after validation",
            "Pending changes: 0",
        ),
        SettingsSection::Input => (
            "Input settings",
            "Controller profile",
            "TG4040 default",
            "Maps Menu, face buttons, shoulders, and directional controls.",
            "Apply mode: restart launcher",
            "Badge: RESTART LAUNCHER · Pending changes: 0",
        ),
        SettingsSection::Audio => (
            "Audio settings",
            "Master volume",
            "72%",
            "Sets launcher and game-session output volume.",
            "Apply mode: immediate",
            "Pending changes: 0",
        ),
        SettingsSection::Power => (
            "Power settings",
            "Low battery / game profile",
            "Warn 20% · Critical 10% · Balanced game profile",
            "Checkpointed save-and-exit is optional; critical shutdown is bounded. Hardware values come from the platform HAL.",
            "Apply mode: persisted device policy; launcher and suspend use Eco",
            "Charging LED off · charging display on · hardware calibration pending",
        ),
        SettingsSection::Library => (
            "Library settings",
            "Show hidden games",
            "Off",
            "Include entries marked hidden in generated catalog views.",
            "Apply mode: rescan library",
            "Badge: RESCAN REQUIRED · Pending changes: 0",
        ),
        SettingsSection::Scraper => (
            "Scraper settings",
            "Parallel workers",
            "2",
            "Limits concurrent metadata requests for the active provider.",
            "Apply mode: next scrape",
            "Pending changes: 0",
        ),
        SettingsSection::Theme => (
            "Theme settings",
            "Active theme",
            "Art Book Next",
            "Choose a validated 4:3 launcher theme from Theme Garden.",
            "Apply mode: restart launcher",
            "Badge: RESTART LAUNCHER · Pending changes: 0",
        ),
        SettingsSection::System => (
            "System settings",
            "Wi-Fi",
            "Enabled · disconnected",
            "Scan, connect, or forget generated simulator networks.",
            "Apply mode: external operation",
            "Pending changes: 0",
        ),
        SettingsSection::Root => unreachable!("settings root has a section list"),
    };
    vec![
        title.into(),
        format!("> {control}"),
        format!("Current value: {value}"),
        format!("Help: {help}"),
        apply.into(),
        badge.into(),
    ]
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
    available: Option<bool>,
    percent: Option<u8>,
    charging: Option<bool>,
    full: Option<bool>,
    health: Option<BatteryHealth>,
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
struct PowerArgs {
    operation: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default, rename = "temperatureC")]
    temperature_c: Option<i16>,
    #[serde(default, rename = "warningPercent")]
    warning_percent: Option<u8>,
    #[serde(default, rename = "criticalPercent")]
    critical_percent: Option<u8>,
    #[serde(default, rename = "lowBatteryAction")]
    low_battery_action: Option<LowBatteryAction>,
    #[serde(default, rename = "chargingLed")]
    charging_led: Option<bool>,
    #[serde(default, rename = "chargingDisplay")]
    charging_display: Option<bool>,
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
struct ResumeDeleteArgs {
    #[serde(rename = "contentId")]
    content_id: String,
    generation: u64,
    confirmed: bool,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tg4040Args {
    operation: String,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default, rename = "brightnessPercent")]
    brightness_percent: Option<u8>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    signal: Option<InputSignal>,
    #[serde(default)]
    role: Option<BluetoothRole>,
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
    let broker = simulator_session::SimulatorSessionAdapter::with_root(evidence_path.join("data"));
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
    broker: simulator_session::SimulatorSessionAdapter,
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
    let input_mappings_path = state_root.join("input-mappings.json");
    let input_mappings = match fs::symlink_metadata(&input_mappings_path) {
        Ok(_) => input_profile::load_mappings(&input_mappings_path)
            .map_err(|error| anyhow!(error.to_string()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            input_profile::InputMappings::default()
        }
        Err(error) => return Err(anyhow!("read input mappings: {error}")),
    };
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
    let mut presentation = PresentationState::new(&identity.target_sku)?;
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
    presentation.restore_visual_preferences(&preferences)?;
    for change in [
        ui_model::PreferenceChange::ArtworkMode(preferences.artwork_mode),
        ui_model::PreferenceChange::MetadataVisibility(preferences.metadata_visibility),
        ui_model::PreferenceChange::UiSize(preferences.ui_size),
        ui_model::PreferenceChange::ColorScheme(preferences.color_scheme),
    ] {
        presentation.ui = ui_model::reduce(&presentation.ui, UiAction::SetPreference(change));
    }
    presentation.ui = ui_model::reduce(
        &presentation.ui,
        UiAction::SetVisualClock {
            wall_clock_ms: platform.wall_clock_ms(),
        },
    );
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
    let mut battery = BatteryPolicyController::new(persisted.battery_policy.clone())
        .map_err(anyhow::Error::msg)?;
    let initial_battery = battery.observe(battery_observation(
        platform.snapshot().map_err(|error| anyhow!(error))?,
    ));
    let catalog_groups = rom_index::GroupIndex::from_catalog(&catalog);
    let mut state = AppState {
        route: Route::Library,
        selected_content_id: catalog.entries[0].id.clone(),
        groups: catalog_groups,
        input_profile,
        input_mappings,
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
        power: PowerPolicyController::new().map_err(anyhow::Error::msg)?,
        battery,
        journey: ProductJourneyState::default(),
        controller_routes: keep_alive,
        resume_content: None,
        resume_marker: 0,
        package_root: state_root.join("portmaster-packages"),
        shutdown_pressed: false,
        exit_requested: false,
        parent_gesture: 0,
    };
    enter_focus_home(&mut state, &catalog);
    launcher_state::save(&state_root, &state.persisted)
        .map_err(|error| anyhow!(error.to_string()))?;
    refresh_resume_projection(&mut state, &catalog, &launch_catalog)
        .map_err(|error| anyhow!(error))?;
    if !state.presentation.ui.resume_entries.is_empty() {
        state.presentation.ui = ui_model::reduce(
            &state.presentation.ui,
            UiAction::Navigate(ui_model::Route::GameSwitcher),
        );
        state.route = Route::GameSwitcher;
    }
    refresh_presentation_affordances(
        &mut state.presentation,
        &initial_battery,
        state.battery.policy(),
    );
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
    let periodic_interval_ms = lifecycle_policy.periodic_interval().as_millis() as u64;
    let mut periodic_deadline = platform
        .logical_time_ms()
        .saturating_add(periodic_interval_ms);
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if state.exit_requested {
            break;
        }
        let mut did_work = false;
        if state.lifecycle.is_awake()
            && state.active_session.is_some()
            && platform.logical_time_ms() >= periodic_deadline
        {
            let _ = state
                .broker
                .checkpoint(CheckpointReason::Periodic, CommitFault::None);
            let _ = refresh_resume_projection(&mut state, &catalog, &launch_catalog);
            periodic_deadline = platform
                .logical_time_ms()
                .saturating_add(periodic_interval_ms);
            did_work = true;
        }
        if !fixture_done || keep_alive {
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
        "power" => {
            parse::<PowerArgs>(request.args).and_then(|args| power_control(platform, state, args))
        }
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
                None,
                at_ms,
                target_sku,
            )
            .map_err(|error| error.to_string())
            .and_then(|_| state_json(platform, evidence, log, catalog, state))
        }),
        "hardware.set" => parse::<HardwareArgs>(request.args).and_then(|args| {
            let battery_low = args
                .battery
                .as_ref()
                .and_then(|change| change.percent)
                .map(|percent| percent <= 10);
            apply_hardware(platform, log, args)?;
            if let Some(low) = battery_low {
                update_tg4040(platform, |state| state.set_low_battery(low))?;
            }
            let decision = state.battery.observe(battery_observation(
                platform.snapshot().map_err(|error| error.to_string())?,
            ));
            handle_battery_decision(
                platform,
                evidence,
                catalog,
                launch_catalog,
                state,
                &decision,
            )?;
            refresh_presentation_affordances(
                &mut state.presentation,
                &decision,
                state.battery.policy(),
            );
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
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                UiAction::SetVisualClock { wall_clock_ms },
            );
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
        "tg4040" => {
            parse::<Tg4040Args>(request.args).and_then(|args| tg4040_control(platform, args))
        }
        "fault.set" => parse::<FaultArgs>(request.args).and_then(|args| {
            set_fault(log, state, args)?;
            state_json(platform, evidence, log, catalog, state)
        }),
        "adapter" => parse::<AdapterArgs>(request.args).and_then(|args| {
            adapter_result(platform, log, state, catalog, launch_catalog, args)?;
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
        "resume.delete" => parse::<ResumeDeleteArgs>(request.args)
            .and_then(|args| resume_delete_control(state, catalog, launch_catalog, args)),
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
        .filter_map(|entry| {
            launch_request(
                entry,
                launch_catalog,
                &state.input_profile,
                &state.input_mappings,
            )
            .ok()
        })
        .collect::<Vec<_>>();
    let summaries = state
        .broker
        .resume_entries(&requests)
        .map_err(|error| error.to_string())?;
    let entries = summaries
        .into_iter()
        .map(|summary| {
            let catalog_entry = catalog
                .entries
                .iter()
                .find(|entry| entry.id == summary.content_id);
            ui_model::ResumeProjection {
                label: catalog_entry.map_or_else(
                    || resume_label(&summary.content_id),
                    |entry| entry.title.clone(),
                ),
                system: catalog_entry
                    .map_or_else(|| "Missing content".into(), |entry| entry.system.clone()),
                content_id: summary.content_id,
                status: summary.status,
                timestamp_ms: summary.timestamp_ms,
                screenshot: format!(
                    "resume-preview-{}-{}",
                    summary.generation,
                    summary
                        .screenshot
                        .sha256
                        .chars()
                        .take(12)
                        .collect::<String>()
                ),
                choices: summary
                    .choices
                    .into_iter()
                    .map(resume_decision_name)
                    .collect(),
            }
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
        ResumeDecision::RestorePrevious => "restore-previous",
        ResumeDecision::FreshStart => "fresh-start",
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

fn refresh_presentation_affordances(
    state: &mut PresentationState,
    decision: &BatteryDecision,
    policy: &BatteryPolicy,
) {
    let mut affordances = state.ui.affordances.clone();
    affordances.battery.percent = decision.displayed_percent;
    affordances.battery.charging_status = match decision.charging_status {
        ChargingStatus::Charging => "charging",
        ChargingStatus::Full => "full",
        ChargingStatus::NotCharging => "not-charging",
        ChargingStatus::Unknown => "unknown",
    }
    .into();
    affordances.battery.external_power = decision.observation.external_power;
    affordances.battery.health = format!("{:?}", decision.observation.health).to_ascii_lowercase();
    affordances.battery.level = format!("{:?}", decision.level).to_ascii_lowercase();
    affordances.battery.show_charging_status = policy.charging_display;
    state.ui = ui_model::reduce(&state.ui, UiAction::SetAffordances(affordances));
}

fn next_theme_garden_name(current: &str) -> &'static str {
    match current {
        "Art Book Next (Batocera ES Edition)" => "Luma Station",
        "Luma Station" => "SimpleLife",
        "SimpleLife" => "Techdweeb",
        _ => "Art Book Next (Batocera ES Edition)",
    }
}

fn presentation_action(state: &mut AppState, args: PresentationArgs) -> Result<(), String> {
    use ui_model::{Action, AmbiguousChoice, GameId, ScraperAction, WifiAction};

    let action = args.action.as_str();
    match action {
        "home" => {
            reduce_route(state, ui_model::Route::Home);
            state.route = Route::Library;
            state.power.game_exit();
        }
        "systems" => reduce_route(state, ui_model::Route::Systems),
        "games" => {
            state.presentation.metadata_off = false;
            reduce_route(state, ui_model::Route::Games);
        }
        "games-no-metadata" => {
            state.presentation.metadata_off = true;
            reduce_route(state, ui_model::Route::Games);
        }
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
            state.presentation.theme_garden = true;
            state.presentation.metadata_off = false;
            state.presentation.theme = match next_theme_garden_name(state.presentation.theme.name())
            {
                "Luma Station" => {
                    launcher_theme::luma_station().map_err(|error| error.to_string())?
                }
                "SimpleLife" => launcher_theme::simplelife().map_err(|error| error.to_string())?,
                "Techdweeb" => launcher_theme::techdweeb().map_err(|error| error.to_string())?,
                _ => launcher_theme::safe_artbook().map_err(|error| error.to_string())?,
            };
            state.modal = None;
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
            state.presentation.metadata_off = false;
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

fn resume_delete_control(
    state: &mut AppState,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    args: ResumeDeleteArgs,
) -> Result<Value, String> {
    if !args.confirmed {
        return Err("resume deletion requires explicit confirmation".into());
    }
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == args.content_id)
        .ok_or_else(|| "resume content is not allowlisted".to_string())?;
    let request = launch_request(
        entry,
        launch_catalog,
        &state.input_profile,
        &state.input_mappings,
    )
    .map_err(|error| error.to_string())?;
    state
        .broker
        .resume_delete(request, args.generation, true)
        .map_err(|error| error.to_string())?;
    refresh_resume_projection(state, catalog, launch_catalog)?;
    Ok(json!({"deleted": true, "generation": args.generation}))
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
        "restore-previous" => ResumeDecision::RestorePrevious,
        "fresh-start" => ResumeDecision::FreshStart,
        "cancel" => ResumeDecision::Cancel,
        _ => return Err("resume decision is not allowlisted".into()),
    };
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == args.content_id)
        .ok_or_else(|| "resume content is not allowlisted".to_string())?;
    let mut request = launch_request(
        entry,
        launch_catalog,
        &state.input_profile,
        &state.input_mappings,
    )
    .map_err(|error| error.to_string())?;
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
        ResumeDecision::Cancel | ResumeDecision::ColdStartSram | ResumeDecision::FreshStart
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
    state.power.begin_game(&entry.system, &entry.id)?;
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

fn visual_preset_name(preset: ui_model::VisualPreset) -> &'static str {
    match preset {
        ui_model::VisualPreset::Default => "default",
        ui_model::VisualPreset::NightWarm => "night-warm",
        ui_model::VisualPreset::LowBrightness => "low-brightness",
        ui_model::VisualPreset::PixelAccurate => "pixel-accurate",
        ui_model::VisualPreset::DenseList => "dense-list",
    }
}

fn visual_preset(value: &str) -> Option<ui_model::VisualPreset> {
    Some(match value {
        "default" => ui_model::VisualPreset::Default,
        "night-warm" => ui_model::VisualPreset::NightWarm,
        "low-brightness" => ui_model::VisualPreset::LowBrightness,
        "pixel-accurate" => ui_model::VisualPreset::PixelAccurate,
        "dense-list" => ui_model::VisualPreset::DenseList,
        _ => return None,
    })
}

fn night_schedule_name(schedule: ui_model::NightSchedule) -> &'static str {
    match schedule {
        ui_model::NightSchedule::Manual => "manual",
        ui_model::NightSchedule::LocalTime => "local-time",
    }
}

fn night_schedule(value: &str) -> Option<ui_model::NightSchedule> {
    Some(match value {
        "manual" => ui_model::NightSchedule::Manual,
        "local-time" => ui_model::NightSchedule::LocalTime,
        _ => return None,
    })
}

fn sync_settings_ui_size(
    state: &mut PresentationState,
    action: input_profile::Action,
) -> Result<()> {
    let scene = state
        .settings
        .scene()
        .map_err(|error| anyhow!(error.to_string()))?;
    let pending = scene
        .pending
        .changes
        .iter()
        .find(|change| change.setting_id == "core.display.ui-size")
        .and_then(|change| match &change.value {
            settings_ui::SemanticValue::EnumSingle(value) => Some(value.as_str()),
            _ => None,
        });
    if let Some(value) = pending {
        let value = match value {
            "automatic" => ui_model::UiSize::Automatic,
            "compact" => ui_model::UiSize::Compact,
            "normal" => ui_model::UiSize::Normal,
            "comfortable" => ui_model::UiSize::Comfortable,
            "large" => ui_model::UiSize::Large,
            "extra-large" => ui_model::UiSize::ExtraLarge,
            _ => return Ok(()),
        };
        state.ui = ui_model::reduce(&state.ui, UiAction::PreviewUiSize(value));
    } else if action == input_profile::Action::Start {
        state.ui = ui_model::reduce(&state.ui, UiAction::ConfirmUiSizePreview);
    } else if matches!(
        action,
        input_profile::Action::Secondary | input_profile::Action::Home
    ) {
        state.ui = ui_model::reduce(&state.ui, UiAction::CancelUiSizePreview);
    }
    Ok(())
}

fn sync_scraper_persistence(state: &mut AppState) {
    state.persisted.scraper_progress = state.presentation.ui.scraper.progress.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlatform;

    impl Platform for TestPlatform {
        fn next_button_event(&mut self) -> PlatformResult<Option<ButtonEvent>> {
            Ok(None)
        }

        fn present(&mut self, _screen: &PresentationScreen) -> PlatformResult<()> {
            Ok(())
        }

        fn capture_png(&mut self, _path: &Path) -> PlatformResult<()> {
            Ok(())
        }

        fn logical_time_ms(&self) -> u64 {
            0
        }

        fn snapshot(&self) -> PlatformResult<sim_platform_contract::PlatformSnapshot> {
            Ok(sim_platform_contract::PlatformSnapshot {
                battery_level_percent: Some(100),
                charging: Some(false),
                full: Some(true),
                battery_health: BatteryHealth::Good,
                external_power: Some(true),
                led_on: false,
                audio_enabled: false,
                radio_enabled: false,
                suspended: false,
            })
        }

        fn platform_state(&self) -> PlatformResult<sim_platform_contract::PlatformState> {
            Err(sim_platform_contract::PlatformError::unsupported(
                sim_platform_contract::HardwareDomain::Display,
                "test platform state",
            ))
        }

        fn hardware_state(&self) -> PlatformResult<sim_platform_contract::HardwareState> {
            Err(sim_platform_contract::PlatformError::unsupported(
                sim_platform_contract::HardwareDomain::Display,
                "test hardware state",
            ))
        }

        fn mutate_hardware(
            &mut self,
            _changes: sim_platform_contract::HardwareChanges,
        ) -> PlatformResult<()> {
            Ok(())
        }
    }

    fn test_state(
        route: Route,
        presentation_route: ui_model::Route,
    ) -> (AppState, UiCatalog, std::path::PathBuf) {
        let catalog: UiCatalog =
            serde_json::from_slice(include_bytes!("../../../sim/fixtures/catalog.json"))
                .expect("generated catalog");
        let mut presentation = PresentationState::new("tg4040").expect("presentation state");
        presentation.ui =
            ui_model::reduce(&presentation.ui, UiAction::Navigate(presentation_route));
        let broker_root = std::env::temp_dir().join(format!(
            "trimui-route-surface-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock")
                .as_nanos()
        ));
        let state = AppState {
            route,
            selected_content_id: catalog.entries[0].id.clone(),
            groups: rom_index::GroupIndex::from_catalog(&catalog),
            input_profile: input_profile::Catalog::from_json(INPUT_PROFILE_BYTES)
                .expect("input profile"),
            input_mappings: input_profile::InputMappings::default(),
            broker: simulator_session::SimulatorSessionAdapter::with_root(broker_root.clone()),
            save_vault: SaveVaultUi::default(),
            save_sync: None,
            active_session: None,
            last_session: None,
            modal: None,
            faults: Vec::new(),
            readiness_generation: 1,
            persisted: launcher_state::State::default(),
            presentation,
            session_step: 0,
            lifecycle: LifecycleController::new(),
            power: PowerPolicyController::new().expect("validated power policy fixture"),
            battery: BatteryPolicyController::new(BatteryPolicy::default())
                .expect("validated battery policy"),
            journey: ProductJourneyState::default(),
            controller_routes: true,
            resume_content: None,
            resume_marker: 0,
            package_root: broker_root.join("packages"),
            shutdown_pressed: false,
            exit_requested: false,
            parent_gesture: 0,
        };
        (state, catalog, broker_root)
    }

    #[test]
    fn visual_preferences_survive_reload_and_reset_to_safe_default() {
        let root = std::env::temp_dir().join(format!(
            "trimui-visual-preferences-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock")
                .as_nanos()
        ));
        let preferences = ui_model::UiPreferences {
            visual_preset: ui_model::VisualPreset::DenseList,
            night_schedule: ui_model::NightSchedule::LocalTime,
            ..Default::default()
        };
        launcher_state::save(
            &root,
            &launcher_state::State {
                preferences: preferences.clone(),
                ..Default::default()
            },
        )
        .expect("save visual preferences");
        let restored = launcher_state::load(&root).preferences;
        assert_eq!(restored, preferences);

        let mut presentation = PresentationState::new("tg4040").expect("presentation state");
        presentation
            .restore_visual_preferences(&restored)
            .expect("restore visual settings");
        assert_eq!(
            presentation
                .screen()
                .expect("visual presentation")
                .visual_profile
                .preset,
            ui_model::VisualPreset::DenseList
        );

        presentation
            .reset_visual_preferences()
            .expect("safe visual reset");
        assert_eq!(
            presentation.ui.preferences.visual_preset,
            ui_model::VisualPreset::Default
        );
        assert_eq!(
            presentation.ui.preferences.night_schedule,
            ui_model::NightSchedule::Manual
        );
        let scene = presentation.settings.scene().expect("settings readback");
        let controls = scene
            .sections
            .iter()
            .flat_map(|section| &section.groups)
            .flat_map(|group| &group.controls)
            .collect::<Vec<_>>();
        assert_eq!(
            controls
                .iter()
                .find(|control| control.setting_id == "core.display.visual-preset")
                .expect("visual preset readback")
                .value,
            settings_ui::SemanticValue::EnumSingle("default".into())
        );
        assert_eq!(
            controls
                .iter()
                .find(|control| control.setting_id == "core.display.night-schedule")
                .expect("night schedule readback")
                .value,
            settings_ui::SemanticValue::EnumSingle("manual".into())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn launch_request_applies_game_input_override() {
        let catalog: UiCatalog =
            serde_json::from_slice(include_bytes!("../../../sim/fixtures/catalog.json"))
                .expect("generated catalog");
        let launch_catalog =
            launch_contract::parse_catalog_json(LAUNCH_CATALOG_BYTES).expect("launch catalog");
        let input_catalog =
            input_profile::Catalog::from_json(INPUT_PROFILE_BYTES).expect("input profile");
        let mut mappings = input_profile::InputMappings::default();
        mappings
            .set_binding(
                input_profile::MappingScope::Game {
                    system_id: "nes",
                    game_id: "nebula-nes",
                },
                input_profile::Binding {
                    control: input_profile::PhysicalControl::A,
                    action: input_profile::LogicalAction::Menu,
                    policy: input_profile::DangerousActionPolicy::Immediate,
                },
            )
            .expect("game override");
        let entry = catalog
            .entries
            .iter()
            .find(|entry| entry.id == "nebula-nes")
            .expect("nebula entry");
        let request = launch_request(entry, &launch_catalog, &input_catalog, &mappings)
            .expect("launch request");
        assert!(request.input.bindings.iter().any(|binding| {
            binding.control == input_profile::PhysicalControl::A
                && binding.action == input_profile::LogicalAction::Menu
        }));
    }

    fn press_controller_button(
        state: &mut AppState,
        catalog: &UiCatalog,
        launch_catalog: &LaunchCatalog,
        evidence: &Evidence,
        log: &mut EventLog,
        platform: &mut TestPlatform,
        button: Button,
    ) {
        handle_button(
            platform,
            evidence,
            log,
            catalog,
            launch_catalog,
            state,
            ButtonEvent {
                at_ms: 1,
                button,
                action: ButtonAction::Press,
            },
            "tg4040",
        )
        .expect("controller input");
    }

    #[test]
    fn focus_home_filters_rendered_rows_and_controller_selection() {
        let (mut state, catalog, broker_root) = test_state(Route::Library, ui_model::Route::Home);
        let unrestricted_rows = screen_for_state(&state, &catalog)
            .expect("unrestricted Home screen")
            .game_rows
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let approved = ["nebula-nes", "mirror-ps1"];
        state.persisted.focus = approved.iter().map(|id| (*id).into()).collect();
        state.persisted.focus_home = true;
        state.persisted.kid_safe = true;
        enter_focus_home(&mut state, &catalog);

        let focus_screen = screen_for_state(&state, &catalog).expect("Focus home screen");
        for rows in [&focus_screen.menu, &focus_screen.game_rows] {
            assert_eq!(
                rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
                approved,
                "Focus rows must use the approved stable IDs"
            );
            assert!(rows.iter().all(|row| {
                !row.label.contains("Orbit Garden") && !row.label.contains("Signal Workshop")
            }));
        }

        let evidence_root = broker_root.join("evidence");
        let evidence = Evidence::new(&evidence_root).expect("evidence");
        let mut log = EventLog::new(&evidence.root, "focus-filter").expect("event log");
        let launch_catalog =
            launch_contract::parse_catalog_json(LAUNCH_CATALOG_BYTES).expect("launch catalog");
        let mut platform = TestPlatform;
        for button in [Button::Down, Button::Up] {
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                button,
            );
            assert!(approved.contains(&state.selected_content_id.as_str()));
        }
        press_controller_button(
            &mut state,
            &catalog,
            &launch_catalog,
            &evidence,
            &mut log,
            &mut platform,
            Button::Primary,
        );
        assert!(matches!(
            state.journey,
            ProductJourneyState::Session {
                content: DemoContent::Nebula | DemoContent::Mirror,
                ..
            }
        ));

        state.active_session = None;
        state.persisted.focus = vec!["missing-entry".into()];
        state.persisted.focus_home = true;
        state.persisted.kid_safe = true;
        enter_focus_home(&mut state, &catalog);
        assert!(matches!(state.journey, ProductJourneyState::FocusRecovery));
        for button in PARENT_GESTURE {
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                button,
            );
        }
        assert!(!state.persisted.kid_safe);
        assert!(matches!(state.journey, ProductJourneyState::Home { .. }));
        let full_home = screen_for_state(&state, &catalog).expect("full Home screen");
        assert_eq!(
            full_home
                .game_rows
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>(),
            unrestricted_rows
        );

        state.persisted.focus = approved.iter().map(|id| (*id).into()).collect();
        state.selected_content_id = "orbit-garden".into();
        state.journey = ProductJourneyState::FocusAdmin {
            selected: FocusAdminItem::Add,
        };
        assert!(handle_focus_controller(
            &mut state,
            &catalog,
            &launch_catalog,
            Button::Primary
        ));
        assert_eq!(
            state.persisted.focus,
            ["nebula-nes", "mirror-ps1", "orbit-garden"]
        );

        drop(log);
        drop(evidence);
        drop(platform);
        drop(state);
        fs::remove_dir_all(broker_root).expect("test broker cleanup");
    }

    #[test]
    fn fresh_home_manifest_controls_reach_declared_routes() {
        let graph: Value =
            serde_json::from_slice(include_bytes!("../../../sim/routes/controller-routes.json"))
                .expect("controller route graph");
        let routes = graph["routes"].as_array().expect("routes array");
        let button = |name: &str| match name {
            "down" => Button::Down,
            "left" => Button::Left,
            "right" => Button::Right,
            "up" => Button::Up,
            "primary" => Button::Primary,
            "secondary" => Button::Secondary,
            "select" => Button::Select,
            "start" => Button::Start,
            "menu" => Button::Menu,
            "l1" => Button::L1,
            "r1" => Button::R1,
            other => panic!("unsupported controller button: {other}"),
        };

        for route in routes
            .iter()
            .filter(|route| route["from"].as_str().unwrap_or("fresh-home") == "fresh-home")
        {
            let mut journey = ProductJourneyState::default();
            let buttons = route["buttons"].as_array().expect("route buttons");
            for name in buttons
                .iter()
                .map(|button| button.as_str().expect("button name"))
            {
                reduce_product_state(&mut journey, button(name));
            }
            assert_eq!(
                canonical_route_id(&journey),
                route["id"].as_str(),
                "fresh-home controls did not reach {}",
                route["id"]
            );
        }

        let favorites = routes
            .iter()
            .find(|route| route["id"] == "games-favorites")
            .expect("favorites route");
        let diagnostics = routes
            .iter()
            .find(|route| route["id"] == "diagnostics")
            .expect("diagnostics route");
        assert_eq!(favorites["buttons"], serde_json::json!(["down", "primary"]));
        assert_eq!(
            diagnostics["buttons"],
            serde_json::json!(["down", "down", "down", "primary", "start"])
        );
    }

    #[test]
    fn product_routes_are_distinct_and_controller_reachable() {
        let mut journey = ProductJourneyState::default();
        assert!(!reduce_product_state(&mut journey, Button::Down));
        assert!(reduce_product_state(&mut journey, Button::Primary));
        assert_eq!(canonical_route_id(&journey), Some("games-favorites"));

        journey = ProductJourneyState::default();
        assert!(reduce_product_state(&mut journey, Button::Primary));
        assert_eq!(canonical_route_id(&journey), Some("home-systems"));
        assert!(reduce_product_state(&mut journey, Button::Primary));
        assert_eq!(canonical_route_id(&journey), Some("home-game-list"));
    }

    #[test]
    fn mandatory_product_surfaces_render_stage_specific_evidence() {
        let cases: Vec<(ProductJourneyState, &[&str])> = vec![
            (
                ProductJourneyState::Wifi {
                    view: WifiJourneyView::Scan,
                },
                &["Home Synthetic", "WPA2", "SAVED", "signal 91%"],
            ),
            (
                ProductJourneyState::Theme {
                    stage: ThemeJourneyStage::Catalog,
                },
                &[
                    "High Contrast",
                    "v1.1.0",
                    "UPDATE AVAILABLE",
                    "Minimal Grid",
                ],
            ),
            (
                ProductJourneyState::Scraper {
                    stage: ScraperJourneyStage::Queue,
                },
                &[
                    "Nebula Notes",
                    "Mirror Museum",
                    "Orbit Garden",
                    "Signal Workshop",
                ],
            ),
            (
                ProductJourneyState::Diagnostics {
                    page: DiagnosticPage::Root,
                },
                &[
                    "Build/SKU",
                    "Storage",
                    "Battery/power",
                    "Last crash",
                    "Support bundle",
                ],
            ),
            (
                ProductJourneyState::Diagnostics {
                    page: DiagnosticPage::SafeMode,
                },
                &[
                    "Network auto-start · DISABLED",
                    "Third-party themes and modules · DISABLED",
                    "Background indexing and auto-resume · DISABLED",
                    "Saves and diagnostics · READ ONLY",
                ],
            ),
        ];
        for (state, required) in cases {
            let labels = product_surface_rows(&state)
                .into_iter()
                .map(|row| row.label)
                .collect::<Vec<_>>();
            for text in required {
                assert!(
                    labels.iter().any(|label| label.contains(text)),
                    "missing {text} in {labels:?}"
                );
            }
        }

        let queue = product_surface_rows(&ProductJourneyState::Scraper {
            stage: ScraperJourneyStage::Queue,
        });
        let success = product_surface_rows(&ProductJourneyState::Scraper {
            stage: ScraperJourneyStage::Success,
        });
        let queue_labels = queue.iter().map(|row| &row.label).collect::<Vec<_>>();
        let success_labels = success.iter().map(|row| &row.label).collect::<Vec<_>>();
        assert_ne!(queue_labels, success_labels);
        assert!(success.iter().any(|row| row.label.contains("Found 2")));
        assert!(success.iter().any(|row| row.label.contains("NOT FOUND")));
    }

    #[test]
    fn controller_reached_forms_expose_concrete_visual_content() {
        let (mut state, catalog, _broker_root) = test_state(Route::Library, ui_model::Route::Home);

        for button in [Button::Primary, Button::Primary, Button::Right] {
            reduce_product_state(&mut state.journey, button);
        }
        let details = screen_for_state(&state, &catalog).expect("game details screen");
        assert_eq!(details.route, "games-details");
        assert_eq!(
            details.selected_game.as_ref().map(|game| game.id.as_str()),
            Some("nebula-nes")
        );
        assert!(details
            .game_media
            .iter()
            .any(|media| { media.content_id == "nebula-nes" && media.kind == "box-art" }));
        assert!(details
            .menu
            .iter()
            .any(|row| row.label.contains("forgotten constellations")));
        assert!(details
            .menu
            .iter()
            .any(|row| row.label.contains("Release date")));

        state.journey = ProductJourneyState::default();
        for button in [Button::Primary, Button::Primary, Button::Select] {
            reduce_product_state(&mut state.journey, button);
        }
        let search = screen_for_state(&state, &catalog).expect("search keyboard screen");
        assert_eq!(search.route, "games-search-keyboard");
        assert!(search.focus.contains("Keyboard focus: Q"));
        assert!(search.menu.iter().any(|row| row.label.contains("Nebula|")));
        assert!(search
            .menu
            .iter()
            .any(|row| row.label.contains("Backspace")));

        state.journey = ProductJourneyState::Settings {
            section: SettingsSection::Library,
            pending: None,
            validation: None,
        };
        let settings = screen_for_state(&state, &catalog).expect("settings form screen");
        for required in [
            "Current value: Off",
            "Help:",
            "Apply mode: rescan library",
            "RESCAN REQUIRED",
        ] {
            assert!(
                settings.menu.iter().any(|row| row.label.contains(required)),
                "missing {required}"
            );
        }

        state.journey = ProductJourneyState::Theme {
            stage: ThemeJourneyStage::Preview,
        };
        let preview = screen_for_state(&state, &catalog).expect("theme preview screen");
        assert_eq!(preview.route, "theme-garden-preview");
        assert!(preview.system_media.is_some());
        assert!(preview.focus.contains("Preview canvas"));
    }

    #[test]
    fn product_session_preserves_interaction_marker_for_restore() {
        let mut journey = ProductJourneyState::default();
        reduce_product_state(&mut journey, Button::Primary);
        reduce_product_state(&mut journey, Button::Down);
        assert!(reduce_product_state(&mut journey, Button::Primary));
        assert!(matches!(
            journey,
            ProductJourneyState::Session {
                content: DemoContent::Nebula,
                ..
            }
        ));
        assert!(!reduce_product_state(&mut journey, Button::Right));
        assert!(matches!(
            journey,
            ProductJourneyState::Session { marker: 1, .. }
        ));
        assert!(reduce_product_state(&mut journey, Button::Select));
        assert_eq!(canonical_route_id(&journey), Some("game-switcher-autosave"));
    }

    #[test]
    fn quick_menu_exposes_controller_actions_without_shortcuts() {
        let mut journey = ProductJourneyState::default();
        for button in [Button::Primary, Button::Down, Button::Primary, Button::Menu] {
            reduce_product_state(&mut journey, button);
        }
        assert_eq!(canonical_route_id(&journey), Some("game-quick-menu"));
        let labels = product_surface_rows(&journey)
            .into_iter()
            .map(|row| row.label)
            .collect::<Vec<_>>();
        for label in [
            "Continue",
            "Save slot 1",
            "Load slot 1",
            "Restart game",
            "RetroArch menu",
            "Exit game",
        ] {
            assert!(
                labels.iter().any(|row| row.contains(label)),
                "missing {label}"
            );
        }
        reduce_product_state(&mut journey, Button::Down);
        reduce_product_state(&mut journey, Button::Primary);
        assert!(product_surface_rows(&journey)
            .iter()
            .any(|row| row.label.contains("saved just now")));
        reduce_product_state(&mut journey, Button::Down);
        reduce_product_state(&mut journey, Button::Primary);
        assert!(matches!(
            journey,
            ProductJourneyState::Session { restored: true, .. }
        ));
    }

    #[test]
    fn portmaster_demos_interact_exit_and_restore_through_controller_input() {
        for content in [DemoContent::Orbit, DemoContent::Signal] {
            let (mut state, catalog, broker_root) =
                test_state(Route::Library, ui_model::Route::Home);
            state.journey = ProductJourneyState::Portmaster {
                page: match content {
                    DemoContent::Orbit => PortmasterPage::Install,
                    DemoContent::Signal => PortmasterPage::Catalog,
                    _ => unreachable!(),
                },
            };
            let evidence_root = broker_root.join("evidence");
            let evidence = Evidence::new(&evidence_root).expect("evidence");
            let mut log = EventLog::new(&evidence.root, "test-run").expect("event log");
            let launch_catalog =
                launch_contract::parse_catalog_json(LAUNCH_CATALOG_BYTES).expect("launch catalog");
            let mut platform = TestPlatform;
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                match content {
                    DemoContent::Orbit => Button::Primary,
                    DemoContent::Signal => Button::Down,
                    _ => unreachable!(),
                },
            );
            if matches!(content, DemoContent::Signal) {
                press_controller_button(
                    &mut state,
                    &catalog,
                    &launch_catalog,
                    &evidence,
                    &mut log,
                    &mut platform,
                    Button::Primary,
                );
            }
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                Button::Right,
            );
            assert!(matches!(
                state.journey,
                ProductJourneyState::Session { marker: 1, .. }
            ));
            let interaction_screen =
                screen_for_state(&state, &catalog).expect("interaction screen");
            assert!(interaction_screen
                .menu
                .iter()
                .any(|row| row.label == "Interaction marker: 1"));
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                Button::Menu,
            );
            assert!(matches!(
                state.journey,
                ProductJourneyState::QuickMenu { .. }
            ));
            assert!(state.active_session.is_some());
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                Button::Secondary,
            );
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                Button::Secondary,
            );
            assert!(state.active_session.is_none());
            assert!(state
                .last_session
                .as_ref()
                .is_some_and(|result| result.resume_published));
            press_controller_button(
                &mut state,
                &catalog,
                &launch_catalog,
                &evidence,
                &mut log,
                &mut platform,
                Button::Menu,
            );
            for _ in 0..3 {
                press_controller_button(
                    &mut state,
                    &catalog,
                    &launch_catalog,
                    &evidence,
                    &mut log,
                    &mut platform,
                    Button::Down,
                );
            }
            for button in [
                Button::Primary,
                Button::Select,
                Button::Primary,
                Button::Primary,
                Button::Primary,
            ] {
                press_controller_button(
                    &mut state,
                    &catalog,
                    &launch_catalog,
                    &evidence,
                    &mut log,
                    &mut platform,
                    button,
                );
            }
            assert!(
                matches!(
                    state.journey,
                    ProductJourneyState::Session {
                        marker: 1,
                        restored: true,
                        ..
                    }
                ),
                "{content:?} ended at {:?}",
                state.journey
            );
            let screen = screen_for_state(&state, &catalog).expect("restored screen");
            assert!(screen
                .menu
                .iter()
                .any(|row| row.label == "Restored interaction marker: 1"));
            drop(log);
            let events = fs::read_to_string(evidence.root.join("logs/launcher.jsonl"))
                .expect("controller event log");
            assert!(events.contains("\"event\":\"session_checkpoint\""));
            assert!(events.contains("\"marker\":1"));
            drop(evidence);
            drop(platform);
            drop(state);
            fs::remove_dir_all(broker_root).expect("test broker cleanup");
        }
    }

    #[test]
    fn portmaster_uninstall_preserves_orbit_data_and_signal_installation() {
        let (mut state, catalog, broker_root) = test_state(Route::Library, ui_model::Route::Home);
        state.journey = ProductJourneyState::Portmaster {
            page: PortmasterPage::UninstallProtected,
        };
        apply_portmaster_route(&mut state, "portmaster-uninstall-protected-data")
            .expect("uninstall route");
        let orbit_state = state
            .package_root
            .join(".brickpro/package-state/orbit-garden.json");
        let signal_state = state
            .package_root
            .join(".brickpro/package-state/signal-workshop.json");
        let protected_save = state
            .package_root
            .join("data/saves/orbit-garden/protected.sav");
        assert!(!orbit_state.exists());
        assert!(signal_state.is_file());
        assert_eq!(
            fs::read(protected_save).expect("protected save"),
            b"protected Orbit Garden save"
        );
        let screen = screen_for_state(&state, &catalog).expect("uninstall screen");
        assert!(screen
            .menu
            .iter()
            .any(|row| row.label == "Protected save retained"));
        assert!(screen
            .menu
            .iter()
            .any(|row| row.label == "Signal Workshop remains installed"));
        drop(state);
        fs::remove_dir_all(broker_root).expect("test broker cleanup");
    }

    #[test]
    fn direct_games_actions_leave_session_surface() {
        for action in ["games", "media-details"] {
            let (mut state, catalog, broker_root) =
                test_state(Route::Session, ui_model::Route::Games);
            presentation_action(
                &mut state,
                PresentationArgs {
                    action: action.into(),
                },
            )
            .expect("presentation action");

            assert_eq!(state.route, Route::Games);
            let screen = screen_for_state(&state, &catalog).expect("screen");
            assert_eq!(screen.route, "games");
            assert_eq!(screen.modal, None);
            drop(state);
            fs::remove_dir_all(broker_root).expect("test broker cleanup");
        }
    }

    #[test]
    fn stale_session_routes_do_not_render_session_frames() {
        let cases = [
            (Route::Games, ui_model::Route::Games, "games", None),
            (Route::Session, ui_model::Route::Games, "games", None),
            (Route::Session, ui_model::Route::Settings, "settings", None),
            (
                Route::Session,
                ui_model::Route::Wifi(ui_model::WifiRoute::Scan),
                "wifi-scan",
                None,
            ),
            (Route::Session, ui_model::Route::Systems, "systems", None),
        ];

        for (route, presentation_route, expected_route, expected_modal) in cases {
            let (state, catalog, broker_root) = test_state(route, presentation_route);
            let screen = screen_for_state(&state, &catalog).expect("screen");
            assert_eq!(screen.route, expected_route);
            assert_eq!(screen.modal.as_deref(), expected_modal);
            drop(state);
            fs::remove_dir_all(broker_root).expect("test broker cleanup");
        }
    }

    #[test]
    fn active_session_preserves_route_and_frame_on_games_action() {
        let (mut state, catalog, broker_root) = test_state(Route::Session, ui_model::Route::Games);
        state.active_session = Some(SessionHandle {
            schema: session_broker::HANDLE_SCHEMA.into(),
            session_id: "test-session".into(),
            content_id: catalog.entries[0].id.clone(),
            phase: "active",
        });

        presentation_action(
            &mut state,
            PresentationArgs {
                action: "games".into(),
            },
        )
        .expect("presentation action");

        assert_eq!(state.route, Route::Session);
        let screen = screen_for_state(&state, &catalog).expect("screen");
        assert_eq!(screen.route, "session");
        assert_eq!(screen.modal.as_deref(), Some("Nebula Notes FRAME 0"));
        drop(state);
        fs::remove_dir_all(broker_root).expect("test broker cleanup");
    }

    #[test]
    fn theme_garden_surface_takes_precedence_over_session_frame() {
        let (mut state, catalog, broker_root) = test_state(Route::Session, ui_model::Route::Games);
        state.presentation.theme_garden = true;
        let screen = screen_for_state(&state, &catalog).expect("screen");
        assert_eq!(screen.route, "theme-garden");
        assert_eq!(screen.modal, None);
        drop(state);
        fs::remove_dir_all(broker_root).expect("test broker cleanup");
    }

    #[test]
    fn controller_secondary_exits_settings_only_after_internal_form_back() {
        let mut presentation = PresentationState::new("tg4040").expect("presentation state");
        presentation.ui = ui_model::reduce(
            &presentation.ui,
            UiAction::Navigate(ui_model::Route::Settings),
        );
        presentation
            .settings
            .press(virtual_keyboard::Button::Down)
            .expect("select section");
        presentation
            .settings
            .press(virtual_keyboard::Button::Primary)
            .expect("open form");
        assert_eq!(
            presentation
                .settings
                .scene()
                .expect("settings scene")
                .surface,
            settings_ui::Surface::Form
        );

        handle_presentation_action(&mut presentation, input_profile::Action::Secondary)
            .expect("leave form");
        assert_eq!(presentation.ui.route, ui_model::Route::Settings);
        handle_presentation_action(&mut presentation, input_profile::Action::Secondary)
            .expect("leave settings");
        assert_eq!(presentation.ui.route, ui_model::Route::Home);
    }

    #[test]
    fn controller_secondary_exits_wifi_menu_after_internal_view_back() {
        let mut presentation = PresentationState::new("tg4040").expect("presentation state");
        presentation.ui = ui_model::reduce(
            &presentation.ui,
            UiAction::Navigate(ui_model::Route::Wifi(ui_model::WifiRoute::Scan)),
        );
        presentation.wifi.scan().expect("scan");
        assert_eq!(
            presentation.wifi.snapshot().view,
            wifi_settings_controller::View::Networks
        );

        handle_presentation_action(&mut presentation, input_profile::Action::Secondary)
            .expect("leave scan");
        assert_eq!(
            presentation.ui.route,
            ui_model::Route::Wifi(ui_model::WifiRoute::Scan)
        );
        handle_presentation_action(&mut presentation, input_profile::Action::Secondary)
            .expect("leave Wi-Fi");
        assert_eq!(presentation.ui.route, ui_model::Route::Home);
    }

    #[test]
    fn wifi_controller_states_reconcile_routes() {
        let mut presentation = PresentationState::new("tg4040").expect("presentation state");
        presentation.ui = ui_model::reduce(
            &presentation.ui,
            UiAction::Navigate(ui_model::Route::Wifi(ui_model::WifiRoute::Scan)),
        );
        presentation.wifi.scan().expect("scan");
        presentation
            .wifi
            .select_network(wifi_manager::NetworkId::new("net-home-strong").expect("network id"))
            .expect("select network");
        let mut snapshot = presentation.wifi.snapshot();
        assert_eq!(
            wifi_route_for_snapshot(&ui_model::WifiRoute::Scan, &snapshot),
            ui_model::WifiRoute::PasswordEntry
        );
        assert_eq!(snapshot.view, wifi_settings_controller::View::Keyboard);
        assert_eq!(snapshot.phase, wifi_manager::WifiPhase::AwaitingCredentials);

        presentation.ui = ui_model::reduce(
            &presentation.ui,
            UiAction::Navigate(ui_model::Route::Wifi(ui_model::WifiRoute::PasswordEntry)),
        );
        handle_presentation_action(&mut presentation, input_profile::Action::Secondary)
            .expect("cancel password entry");
        let cancelled_snapshot = presentation.wifi.snapshot();
        assert_eq!(
            cancelled_snapshot.view,
            wifi_settings_controller::View::Menu
        );
        assert_eq!(
            cancelled_snapshot.phase,
            wifi_manager::WifiPhase::AwaitingCredentials
        );
        assert_eq!(cancelled_snapshot.keyboard, None);
        assert_eq!(
            presentation.ui.route,
            ui_model::Route::Wifi(ui_model::WifiRoute::Scan)
        );

        let mut settled = PresentationState::new("tg4040").expect("presentation state");
        settled.wifi.scan().expect("scan");
        settled
            .wifi
            .select_network(wifi_manager::NetworkId::new("net-guest").expect("network id"))
            .expect("select open network");
        settled.ui = ui_model::reduce(
            &settled.ui,
            UiAction::Navigate(ui_model::Route::Wifi(ui_model::WifiRoute::PasswordEntry)),
        );
        handle_presentation_action(&mut settled, input_profile::Action::MoveUp)
            .expect("reconcile settled menu");
        let settled_snapshot = settled.wifi.snapshot();
        assert_eq!(settled_snapshot.view, wifi_settings_controller::View::Menu);
        assert_eq!(settled_snapshot.phase, wifi_manager::WifiPhase::Idle);
        assert_eq!(
            settled.ui.route,
            ui_model::Route::Wifi(ui_model::WifiRoute::Scan)
        );

        presentation.wifi.open_manual().expect("manual network");
        snapshot = presentation.wifi.snapshot();
        assert_eq!(
            wifi_route_for_snapshot(&ui_model::WifiRoute::Scan, &snapshot),
            ui_model::WifiRoute::ManualSsid
        );

        snapshot.keyboard = None;
        snapshot.phase = wifi_manager::WifiPhase::Associating;
        assert_eq!(
            wifi_route_for_snapshot(&ui_model::WifiRoute::Scan, &snapshot),
            ui_model::WifiRoute::Progress
        );
        snapshot.phase = wifi_manager::WifiPhase::Failed;
        snapshot.reason = Some(wifi_manager::ReasonCode::RadioUnavailable);
        assert_eq!(
            wifi_route_for_snapshot(&ui_model::WifiRoute::Scan, &snapshot),
            ui_model::WifiRoute::Error
        );
    }

    #[test]
    fn theme_garden_secondary_exits_to_library_through_controller_path() {
        let (mut state, catalog, broker_root) = test_state(Route::Library, ui_model::Route::Home);
        state.presentation.theme_garden = true;
        let evidence_root = broker_root.join("evidence");
        let evidence = Evidence::new(&evidence_root).expect("evidence");
        let mut log = EventLog::new(&evidence.root, "test-run").expect("event log");
        let launch_catalog =
            launch_contract::parse_catalog_json(LAUNCH_CATALOG_BYTES).expect("launch catalog");
        let mut platform = TestPlatform;

        handle_button(
            &mut platform,
            &evidence,
            &mut log,
            &catalog,
            &launch_catalog,
            &mut state,
            ButtonEvent {
                at_ms: 1,
                button: Button::Secondary,
                action: ButtonAction::Press,
            },
            "tg4040",
        )
        .expect("leave Theme Garden");

        assert!(!state.presentation.theme_garden);
        assert_eq!(state.route, Route::Library);
        drop(log);
        drop(evidence);
        drop(platform);
        drop(state);
        fs::remove_dir_all(broker_root).expect("test broker cleanup");
    }

    #[test]
    fn settings_projection_shows_only_the_selected_form_section() {
        let mut presentation = PresentationState::new("tg4040").expect("presentation state");
        presentation.ui = ui_model::reduce(
            &presentation.ui,
            UiAction::Navigate(ui_model::Route::Settings),
        );
        let section_list = presentation
            .screen()
            .expect("section list")
            .settings
            .expect("settings");
        assert!(section_list
            .sections
            .iter()
            .all(|section| section.controls.is_empty()));
        assert_eq!(section_list.selected_section_id.as_deref(), Some("display"));

        presentation
            .settings
            .press(virtual_keyboard::Button::Down)
            .expect("select section");
        presentation
            .settings
            .press(virtual_keyboard::Button::Primary)
            .expect("open form");
        let form = presentation
            .screen()
            .expect("form")
            .settings
            .expect("settings");
        assert_eq!(form.surface, settings_ui::Surface::Form);
        assert_eq!(form.sections.len(), 1);
        assert_eq!(form.sections[0].id, "audio");
        assert_eq!(form.selected_section_id.as_deref(), Some("audio"));
        assert!(form.sections[0].controls.iter().any(|control| {
            Some(control.setting_id.as_str()) == form.selected_setting_id.as_deref()
        }));
    }

    #[test]
    fn art_book_settings_and_wifi_surfaces_have_themed_panels() {
        let routes = [
            ui_model::Route::Settings,
            ui_model::Route::Wifi(ui_model::WifiRoute::Scan),
        ];
        for presentation_route in routes {
            let (mut state, catalog, broker_root) = test_state(Route::Library, presentation_route);
            state.presentation.ui =
                ui_model::reduce(&state.presentation.ui, UiAction::FinishSplash);
            let screen = screen_for_state(&state, &catalog).expect("screen");
            let mut platform = sim_host_platform::HostPlatform::new(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/device/tg4040-host.json"),
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../config/platform/tg4040/compatibility.json"),
                sim_host_platform::Backend::Dummy,
            )
            .expect("host platform");
            platform.present(&screen).expect("render route");
            let png = broker_root.join("surface.png");
            platform.capture_png(&png).expect("capture route");
            let decoder = png::Decoder::new(fs::File::open(&png).expect("captured PNG"));
            let mut reader = decoder.read_info().expect("PNG header");
            let mut pixels = vec![0; reader.output_buffer_size()];
            let frame = reader.next_frame(&mut pixels).expect("PNG frame");
            let offset = (300 * frame.width + 500) as usize * 4;
            let pixel: [u8; 4] = pixels[offset..offset + 4].try_into().expect("pixel");
            assert_ne!(
                pixel, screen.palette.background,
                "{} stayed bare",
                screen.route
            );
            drop(platform);
            drop(state);
            fs::remove_dir_all(broker_root).expect("test broker cleanup");
        }
    }

    #[test]
    fn theme_garden_cycles_real_imports_before_default() {
        assert_eq!(
            next_theme_garden_name("Art Book Next (Batocera ES Edition)"),
            "Luma Station"
        );
        assert_eq!(next_theme_garden_name("Luma Station"), "SimpleLife");
        assert_eq!(next_theme_garden_name("SimpleLife"), "Techdweeb");
        assert_eq!(
            next_theme_garden_name("Techdweeb"),
            "Art Book Next (Batocera ES Edition)"
        );
    }
}

fn wifi_route_for_snapshot(
    current: &ui_model::WifiRoute,
    snapshot: &wifi_settings_controller::Snapshot,
) -> ui_model::WifiRoute {
    if snapshot.reason.is_some() || snapshot.phase == wifi_manager::WifiPhase::Failed {
        return ui_model::WifiRoute::Error;
    }
    if matches!(
        snapshot.phase,
        wifi_manager::WifiPhase::Scanning
            | wifi_manager::WifiPhase::Associating
            | wifi_manager::WifiPhase::Authenticating
            | wifi_manager::WifiPhase::Dhcp
    ) {
        return ui_model::WifiRoute::Progress;
    }
    if snapshot.view == wifi_settings_controller::View::Menu {
        return ui_model::WifiRoute::Scan;
    }
    if let Some(keyboard) = &snapshot.keyboard {
        return match keyboard.field {
            wifi_manager::KeyboardField::Password => ui_model::WifiRoute::PasswordEntry,
            wifi_manager::KeyboardField::Ssid => match current {
                ui_model::WifiRoute::HiddenNetwork => ui_model::WifiRoute::HiddenNetwork,
                _ => ui_model::WifiRoute::ManualSsid,
            },
        };
    }
    if snapshot.phase == wifi_manager::WifiPhase::AwaitingCredentials {
        return ui_model::WifiRoute::PasswordEntry;
    }
    if snapshot.view == wifi_settings_controller::View::Networks {
        return match current {
            ui_model::WifiRoute::AccessPointSelection => ui_model::WifiRoute::AccessPointSelection,
            _ => ui_model::WifiRoute::Scan,
        };
    }
    current.clone()
}

fn exit_theme_garden(state: &mut AppState, action: input_profile::Action) {
    if state.presentation.theme_garden && action == input_profile::Action::Secondary {
        state.presentation.theme_garden = false;
        state.route = Route::Library;
    }
}

fn reduce_route(state: &mut AppState, route: ui_model::Route) {
    if state.route == Route::Session
        && route == ui_model::Route::Games
        && state.active_session.is_none()
    {
        state.route = Route::Games;
    }
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

fn power_control<P: Platform>(
    platform: &mut P,
    state: &mut AppState,
    args: PowerArgs,
) -> Result<Value, String> {
    match args.operation.as_str() {
        "override" => {
            state.power.set_game_override(
                args.profile
                    .as_deref()
                    .ok_or_else(|| "power override requires profile".to_string())?,
            )?;
            Ok(state.power.evidence())
        }
        "temperature" => {
            state.power.set_temperature(
                args.temperature_c
                    .ok_or_else(|| "power temperature requires temperatureC".to_string())?,
            )?;
            Ok(state.power.evidence())
        }
        "battery-policy" => {
            let policy = BatteryPolicy {
                warning_percent: args
                    .warning_percent
                    .ok_or_else(|| "battery policy requires warningPercent".to_string())?,
                critical_percent: args
                    .critical_percent
                    .ok_or_else(|| "battery policy requires criticalPercent".to_string())?,
                low_battery_action: args
                    .low_battery_action
                    .ok_or_else(|| "battery policy requires lowBatteryAction".to_string())?,
                charging_led: args
                    .charging_led
                    .ok_or_else(|| "battery policy requires chargingLed".to_string())?,
                charging_display: args
                    .charging_display
                    .ok_or_else(|| "battery policy requires chargingDisplay".to_string())?,
            };
            policy.validate()?;
            apply_charging_led(platform, &policy, state.battery.decision())?;
            state.battery.set_policy(policy.clone())?;
            refresh_presentation_affordances(
                &mut state.presentation,
                &state.battery.decision().clone(),
                &policy,
            );
            state.persisted.battery_policy = policy;
            serde_json::to_value(state.battery.evidence()).map_err(|error| error.to_string())
        }
        _ => Err("power operation must be override, temperature, or battery-policy".into()),
    }
}

struct BrokerCheckpoint<'a> {
    broker: &'a mut simulator_session::SimulatorSessionAdapter,
    fault: CommitFault,
    active: bool,
}

impl CheckpointHook for BrokerCheckpoint<'_> {
    fn checkpoint(&mut self) -> Result<u64, String> {
        if !self.active {
            return if self.fault == CommitFault::None {
                Ok(0)
            } else {
                Err("injected checkpoint fault".into())
            };
        }
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
            state.power.suspend();
            let checkpoint_fault = if state.faults.iter().any(|fault| fault == "checkpoint-fail") {
                CommitFault::Artifact
            } else {
                CommitFault::None
            };
            let mut checkpoint = BrokerCheckpoint {
                broker: &mut state.broker,
                fault: checkpoint_fault,
                active: state.active_session.is_some(),
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
        "resume" => {
            state.power.wake();
            state.lifecycle.resume(
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
            )
        }
        "shutdown" => {
            state.power.game_exit();
            state.lifecycle.orderly_shutdown(platform, fault)
        }
        _ => return Err("lifecycle operation must be suspend, resume, or shutdown".into()),
    };
    sync_lifecycle_marker(&evidence.root, &state.lifecycle)?;
    state.presentation.ui = ui_model::reduce(
        &state.presentation.ui,
        UiAction::SetVisualClock {
            wall_clock_ms: platform.wall_clock_ms(),
        },
    );
    let phase = state.lifecycle.phase();
    if result.is_ok() {
        match args.operation.as_str() {
            "suspend" => update_tg4040(platform, Tg4040State::suspend)?,
            "resume" => update_tg4040(platform, Tg4040State::resume)?,
            "shutdown" => update_tg4040(platform, Tg4040State::stop_session_effects)?,
            _ => unreachable!("lifecycle operation was validated"),
        }
    }
    if matches!(
        phase,
        LifecyclePhase::ResumedForDeadline | LifecyclePhase::OrderlyShutdown
    ) {
        state.power.wake();
        state.active_session = None;
        stop_session_effects(platform)?;
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

fn battery_observation(snapshot: sim_platform_contract::PlatformSnapshot) -> BatteryObservation {
    BatteryObservation {
        percent: snapshot.battery_level_percent,
        charging: snapshot.charging,
        full: snapshot.full,
        external_power: snapshot.external_power,
        health: snapshot.battery_health,
    }
}

fn handle_battery_decision<P: Platform>(
    platform: &mut P,
    evidence: &Evidence,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    state: &mut AppState,
    decision: &BatteryDecision,
) -> Result<(), String> {
    apply_charging_led(platform, state.battery.policy(), decision)?;
    let Some(action) = decision.action else {
        return Ok(());
    };
    if matches!(
        action,
        PolicyAction::CheckpointAndExit | PolicyAction::CheckpointAndShutdown
    ) && state.active_session.is_some()
    {
        state
            .broker
            .checkpoint(CheckpointReason::LowBattery, CommitFault::None)
            .map_err(|error| error.to_string())?;
        refresh_resume_projection(state, catalog, launch_catalog)?;
    }
    match action {
        PolicyAction::Warn => state.modal = Some("Low battery".into()),
        PolicyAction::CheckpointAndExit | PolicyAction::ExitWithoutSave => {
            let result = if state.active_session.is_some() {
                Some(
                    state
                        .broker
                        .complete(0, 0)
                        .map_err(|error| error.to_string())?,
                )
            } else {
                None
            };
            state.power.game_exit();
            state.active_session = None;
            state.last_session = result;
            state.route = Route::Library;
            write_session(&evidence.root, SessionState::Completed)
                .map_err(|error| error.to_string())?;
        }
        PolicyAction::CheckpointAndShutdown => {
            state
                .lifecycle
                .low_battery(platform)
                .map_err(|error| error.to_string())?;
            sync_lifecycle_marker(&evidence.root, &state.lifecycle)?;
            state.power.game_exit();
            state.active_session = None;
            state.last_session = None;
            state.route = Route::Library;
            write_session(&evidence.root, SessionState::Aborted)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn apply_charging_led<P: Platform>(
    platform: &mut P,
    policy: &BatteryPolicy,
    decision: &BatteryDecision,
) -> Result<(), String> {
    let on = policy.charging_led
        && decision.observation.external_power == Some(true)
        && (decision.observation.charging == Some(true) || decision.observation.full == Some(true));
    platform
        .set_leds(LedState {
            on,
            brightness_percent: if on { 20 } else { 0 },
        })
        .map_err(|error| error.to_string())
}

fn apply_hardware<P: Platform>(
    platform: &mut P,
    log: &mut EventLog,
    args: HardwareArgs,
) -> Result<Value, String> {
    let mut changes = HardwareChanges::default();
    let mut changed = false;
    if let Some(battery) = args.battery {
        if let Some(value) = battery.available {
            changes.battery_available = Some(value);
            changed = true;
        }
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
        if let Some(value) = battery.full {
            changes.full = Some(value);
            changed = true;
        }
        if let Some(value) = battery.health {
            changes.battery_health = Some(value);
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

fn adapter_result<P: Platform>(
    platform: &mut P,
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
    stop_session_effects(platform)?;
    state.active_session = None;
    state.power.game_exit();
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

fn update_tg4040<P: Platform>(
    platform: &mut P,
    update: impl FnOnce(&mut Tg4040State),
) -> Result<(), String> {
    match platform.tg4040_state() {
        Ok(mut state) => {
            update(&mut state);
            sync_tg4040(platform, state)
        }
        Err(PlatformError::Unsupported { .. }) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn sync_tg4040<P: Platform>(platform: &mut P, state: Tg4040State) -> Result<(), String> {
    platform
        .set_leds(LedState {
            on: state.effective_led_enabled,
            brightness_percent: if state.effective_led_enabled {
                state.persisted_led.brightness_percent
            } else {
                0
            },
        })
        .map_err(|error| error.to_string())?;
    platform
        .set_rumble(RumbleState {
            active: state.rumble_active,
        })
        .map_err(|error| error.to_string())?;
    let mut radios = platform.radios_state().map_err(|error| error.to_string())?;
    radios.bluetooth.enabled = state.bluetooth.role.is_some();
    radios.bluetooth.connected = state.bluetooth.phase == BluetoothPhase::Connected;
    platform
        .set_radios(radios)
        .map_err(|error| error.to_string())?;
    platform
        .set_tg4040_state(state)
        .map_err(|error| error.to_string())
}

fn stop_session_effects<P: Platform>(platform: &mut P) -> Result<(), String> {
    update_tg4040(platform, Tg4040State::stop_session_effects)
}

fn tg4040_control<P: Platform>(platform: &mut P, args: Tg4040Args) -> Result<Value, String> {
    if args.brightness_percent.is_some_and(|value| value > 100) {
        return Err("tg4040 LED brightness must be between 0 and 100".into());
    }

    let mut state = platform.tg4040_state().map_err(|error| error.to_string())?;
    match args.operation.as_str() {
        "led" => state.set_led(LedSettings {
            enabled: args.enabled.ok_or("tg4040 LED enabled is required")?,
            brightness_percent: args
                .brightness_percent
                .unwrap_or(state.persisted_led.brightness_percent),
        }),
        "rumble" => state.set_rumble_active(args.active.ok_or("tg4040 rumble active is required")?),
        "low-battery" => {
            state.set_low_battery(args.active.ok_or("tg4040 low-battery active is required")?)
        }
        "input" => state.observe_input(args.signal.ok_or("tg4040 input signal is required")?),
        "scan" => state.scan(args.role.ok_or("tg4040 Bluetooth role is required")?),
        "pair" => state.pair(),
        "paired" => state.paired(),
        "connected" => state.connected(),
        "reconnect" => state.reconnect(),
        "reboot" => state.reboot(),
        "reset" => state.reset_to_baseline(),
        _ => return Err("unsupported TG4040 operation".into()),
    }
    sync_tg4040(platform, state.clone())?;
    serde_json::to_value(state).map_err(|error| error.to_string())
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
    let selected = &catalog.entries[selected_catalog_index(state, catalog)];
    let mappings = state
        .input_profile
        .launch_mappings_with(
            &state.input_mappings,
            Some(&selected.system),
            Some(&selected.id),
        )
        .map_err(|error| anyhow!(error.to_string()))?;
    let action = mappings
        .bindings
        .iter()
        .find(|binding| binding.control == physical_control(event.button))
        .and_then(|binding| frontend_action(binding.action))
        .ok_or_else(|| {
            anyhow!(
                "input mapping has no frontend action for {:?}",
                event.button
            )
        })?;
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
        frontend_button(action),
        event.at_ms,
        target_sku,
    )
}

fn handle_focus_controller(
    state: &mut AppState,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    button: Button,
) -> bool {
    if state.persisted.kid_safe {
        state.parent_gesture = if button == PARENT_GESTURE[state.parent_gesture] {
            state.parent_gesture + 1
        } else {
            usize::from(button == PARENT_GESTURE[0])
        };
        if state.parent_gesture == PARENT_GESTURE.len() {
            state.persisted.kid_safe = false;
            state.parent_gesture = 0;
            state.journey = ProductJourneyState::default();
            state.route = Route::Library;
            state.presentation.ui = ui_model::reduce(
                &state.presentation.ui,
                UiAction::Navigate(ui_model::Route::Home),
            );
            state.modal = Some("Parent mode restored".into());
            return true;
        }
    }

    if let ProductJourneyState::FocusAdmin { selected } = &state.journey {
        match button {
            Button::Down => {
                state.journey = ProductJourneyState::FocusAdmin {
                    selected: next_focus_admin(*selected, true),
                };
            }
            Button::Up => {
                state.journey = ProductJourneyState::FocusAdmin {
                    selected: next_focus_admin(*selected, false),
                };
            }
            Button::Menu => {
                state.journey = ProductJourneyState::default();
                state.route = Route::Library;
                state.presentation.ui = ui_model::reduce(
                    &state.presentation.ui,
                    UiAction::Navigate(ui_model::Route::Home),
                );
            }
            Button::Secondary => {
                state.journey = ProductJourneyState::Settings {
                    section: SettingsSection::Library,
                    pending: None,
                    validation: None,
                };
            }
            Button::Primary => match selected {
                FocusAdminItem::Add => {
                    let candidate = catalog
                        .entries
                        .iter()
                        .find(|entry| {
                            entry.id == state.selected_content_id
                                && !state.persisted.focus.contains(&entry.id)
                        })
                        .or_else(|| {
                            catalog
                                .entries
                                .iter()
                                .find(|entry| !state.persisted.focus.contains(&entry.id))
                        });
                    if let Some(entry) = candidate {
                        state.persisted.focus.push(entry.id.clone());
                    }
                }
                FocusAdminItem::Remove => {
                    state
                        .persisted
                        .focus
                        .retain(|id| id != &state.selected_content_id);
                    if state.persisted.focus.is_empty() {
                        state.persisted.kid_safe = false;
                    }
                }
                FocusAdminItem::MoveUp | FocusAdminItem::MoveDown => {
                    if let Some(index) = state
                        .persisted
                        .focus
                        .iter()
                        .position(|id| id == &state.selected_content_id)
                    {
                        let other = if matches!(selected, FocusAdminItem::MoveUp) {
                            index.checked_sub(1)
                        } else {
                            (index + 1 < state.persisted.focus.len()).then_some(index + 1)
                        };
                        if let Some(other) = other {
                            state.persisted.focus.swap(index, other);
                        }
                    }
                }
                FocusAdminItem::DefaultHome => {
                    state.persisted.focus_home = !state.persisted.focus_home
                }
                FocusAdminItem::KidSafe => {
                    if !state.persisted.focus.is_empty() {
                        state.persisted.kid_safe = true;
                        state.persisted.focus_home = true;
                        enter_focus_home(state, catalog);
                    }
                }
            },
            _ => {}
        }
        return true;
    }

    if let ProductJourneyState::FocusHome { selected } = &state.journey {
        let entries = focus_catalog_entries(state, catalog);
        if entries.is_empty() {
            state.journey = ProductJourneyState::FocusRecovery;
            return true;
        }
        match button {
            Button::Down | Button::Up => {
                let next = if button == Button::Down {
                    (*selected + 1) % entries.len()
                } else {
                    selected.checked_sub(1).unwrap_or(entries.len() - 1)
                };
                state.selected_content_id = entries[next].id.clone();
                state.journey = ProductJourneyState::FocusHome { selected: next };
            }
            Button::Primary => {
                if let Some(content) = demo_content(&entries[*selected].id) {
                    state.selected_content_id = entries[*selected].id.clone();
                    state.journey = ProductJourneyState::Session {
                        content,
                        marker: 0,
                        restored: false,
                    };
                }
            }
            Button::Menu if !state.persisted.kid_safe => {
                state.journey = ProductJourneyState::default();
                state.route = Route::Library;
                state.presentation.ui = ui_model::reduce(
                    &state.presentation.ui,
                    UiAction::Navigate(ui_model::Route::Home),
                );
            }
            _ => {}
        }
        return true;
    }

    if matches!(state.journey, ProductJourneyState::FocusRecovery) {
        if !state.persisted.kid_safe && button == Button::Secondary {
            state.persisted.focus_home = false;
            state.journey = ProductJourneyState::Home {
                selected: HomeItem::Settings,
            };
        }
        return state.persisted.kid_safe || button == Button::Secondary;
    }

    if state.persisted.kid_safe {
        match &state.journey {
            ProductJourneyState::Session {
                content, marker, ..
            } if button == Button::Menu => {
                state.journey = ProductJourneyState::KidQuickMenu {
                    selected: KidQuickItem::Continue,
                    content: *content,
                    marker: *marker,
                    saved: false,
                };
                return true;
            }
            ProductJourneyState::KidQuickMenu {
                selected,
                content,
                marker,
                saved,
            } => {
                let (selected, content, marker, saved) = (*selected, *content, *marker, *saved);
                match button {
                    Button::Down | Button::Up => {
                        let selected = match (selected, button) {
                            (KidQuickItem::Continue, Button::Down) => KidQuickItem::Save,
                            (KidQuickItem::Save, Button::Down) => KidQuickItem::Exit,
                            (KidQuickItem::Exit, Button::Down) => KidQuickItem::Continue,
                            (KidQuickItem::Continue, Button::Up) => KidQuickItem::Exit,
                            (KidQuickItem::Save, Button::Up) => KidQuickItem::Continue,
                            (KidQuickItem::Exit, Button::Up) => KidQuickItem::Save,
                            _ => selected,
                        };
                        state.journey = ProductJourneyState::KidQuickMenu {
                            selected,
                            content,
                            marker,
                            saved,
                        };
                    }
                    Button::Primary if matches!(selected, KidQuickItem::Continue) => {
                        state.journey = ProductJourneyState::Session {
                            content,
                            marker,
                            restored: false,
                        };
                    }
                    Button::Primary if matches!(selected, KidQuickItem::Save) => {
                        let _ = state
                            .broker
                            .checkpoint(CheckpointReason::Periodic, CommitFault::None);
                        let _ = refresh_resume_projection(state, catalog, launch_catalog);
                        state.journey = ProductJourneyState::KidQuickMenu {
                            selected,
                            content,
                            marker,
                            saved: true,
                        };
                    }
                    Button::Primary if matches!(selected, KidQuickItem::Exit) => {
                        state.journey = ProductJourneyState::QuickMenu {
                            selected: QuickMenuItem::Exit,
                            content,
                            marker,
                            preview: "kid exit",
                        };
                        return false;
                    }
                    _ => {}
                }
                return true;
            }
            _ => return true,
        }
    }
    false
}

fn next_focus_admin(item: FocusAdminItem, forward: bool) -> FocusAdminItem {
    use FocusAdminItem::*;
    match (item, forward) {
        (Add, true) => Remove,
        (Remove, true) => MoveUp,
        (MoveUp, true) => MoveDown,
        (MoveDown, true) => DefaultHome,
        (DefaultHome, true) => KidSafe,
        (KidSafe, true) => Add,
        (Add, false) => KidSafe,
        (Remove, false) => Add,
        (MoveUp, false) => Remove,
        (MoveDown, false) => MoveUp,
        (DefaultHome, false) => MoveDown,
        (KidSafe, false) => DefaultHome,
    }
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
    frontend_button: Option<Button>,
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
    if state.presentation.theme_garden {
        exit_theme_garden(state, action);
    }
    if state.faults.iter().any(|fault| fault == "input-drop") {
        return Ok(());
    }
    if phase == ButtonAction::Release
        && frontend_button == Some(Button::Primary)
        && state.shutdown_pressed
    {
        lifecycle_control(
            platform,
            evidence,
            log,
            catalog,
            launch_catalog,
            state,
            LifecycleArgs {
                operation: "shutdown".into(),
                timeout_ms: 1_000,
                duration_minutes: None,
                wake_source: None,
            },
        )
        .map_err(anyhow::Error::msg)?;
        state.shutdown_pressed = false;
        state.exit_requested = state.lifecycle.terminal_shutdown();
        return Ok(());
    }
    if phase != ButtonAction::Press {
        return Ok(());
    }

    if frontend_button == Some(Button::Primary)
        && canonical_route_id(&state.journey) == Some("shutdown-confirm")
    {
        state.shutdown_pressed = true;
        return Ok(());
    }

    if state.controller_routes {
        if let Some(button) = frontend_button {
            let before = canonical_route_id(&state.journey);
            if handle_focus_controller(state, catalog, launch_catalog, button) {
                let after = canonical_route_id(&state.journey);
                if before != after {
                    if let Some(route_id) = after {
                        log.emit(
                            "controller_route_visit",
                            at_ms,
                            json_map([("routeId", json!(route_id))]),
                        )?;
                    }
                }
                if matches!(state.journey, ProductJourneyState::Session { .. })
                    && state.active_session.is_none()
                    && before != canonical_route_id(&state.journey)
                {
                    launch_product_session(state, catalog, launch_catalog, evidence, false)?;
                    write_session(&evidence.root, SessionState::Started)?;
                }

                launcher_state::save(&evidence.root.join("data"), &state.persisted)
                    .map_err(|error| anyhow!(error.to_string()))?;
                let screen = screen_for_state(state, catalog)?;
                present(platform, &screen)?;
                log.emit(
                    "input_to_frame",
                    at_ms,
                    json_map([
                        ("latencyUs", json!(0)),
                        ("sessionStep", json!(state.session_step)),
                    ]),
                )?;
                return Ok(());
            }
            if button == Button::Secondary && before == Some("shutdown-confirm") {
                state.shutdown_pressed = false;
            }
            let exiting_session = match (&state.journey, button) {
                (
                    ProductJourneyState::Session {
                        content, marker, ..
                    },
                    Button::Select | Button::Secondary,
                )
                | (
                    ProductJourneyState::GameSwitcher {
                        page: SwitcherPage::Autosave | SwitcherPage::Exit,
                        content,
                        marker,
                    },
                    Button::Menu,
                )
                | (
                    ProductJourneyState::QuickMenu {
                        selected: QuickMenuItem::Exit,
                        content,
                        marker,
                        ..
                    },
                    Button::Primary,
                )
                | (
                    ProductJourneyState::QuickMenu {
                        content, marker, ..
                    },
                    Button::Menu,
                ) if state.active_session.is_some() => Some((*content, *marker)),
                _ => None,
            };
            let changed = reduce_product_state(&mut state.journey, button);
            if state.persisted.kid_safe && matches!(state.journey, ProductJourneyState::Home { .. })
            {
                enter_focus_home(state, catalog);
            }

            if changed {
                if matches!(
                    state.journey,
                    ProductJourneyState::GameSwitcher {
                        page: SwitcherPage::List,
                        ..
                    }
                ) {
                    if let Some(content) = state.resume_content {
                        state.journey = ProductJourneyState::GameSwitcher {
                            page: SwitcherPage::List,
                            content,
                            marker: state.resume_marker,
                        };
                    }
                }
                match canonical_route_id(&state.journey) {
                    Some("portmaster-install") | Some("portmaster-uninstall-protected-data") => {
                        apply_portmaster_route(
                            state,
                            canonical_route_id(&state.journey).expect("route checked"),
                        )?
                    }
                    _ => {}
                }
                if canonical_route_id(&state.journey) == Some("diagnostics-safe-mode") {
                    state.power.safe_mode_reset();
                    state.presentation.reset_visual_preferences()?;
                    state.persisted.preferences = state.presentation.ui.preferences.clone();
                    launcher_state::save(&evidence.root.join("data"), &state.persisted)
                        .map_err(|error| anyhow!(error.to_string()))?;
                    state.route = Route::Library;
                }
                if matches!(state.journey, ProductJourneyState::Session { .. })
                    && state.active_session.is_none()
                    && before != canonical_route_id(&state.journey)
                {
                    launch_product_session(
                        state,
                        catalog,
                        launch_catalog,
                        evidence,
                        matches!(
                            state.journey,
                            ProductJourneyState::Session { restored: true, .. }
                        ),
                    )?;
                    write_session(&evidence.root, SessionState::Started)?;
                }
                if let Some((content, marker)) = exiting_session {
                    state.resume_content = Some(content);
                    state.resume_marker = marker;
                    let result = state
                        .broker
                        .complete(0, 0)
                        .map_err(|error| anyhow!(error.to_string()))?;
                    state.active_session = None;
                    state.power.game_exit();
                    state.last_session = Some(result);
                    log.emit(
                        "session_checkpoint",
                        at_ms,
                        json_map([
                            ("contentId", json!(format!("{:?}", content))),
                            ("marker", json!(marker)),
                        ]),
                    )?;
                    refresh_resume_projection(state, catalog, launch_catalog)
                        .map_err(anyhow::Error::msg)?;
                }
                let after = canonical_route_id(&state.journey);
                if before != after {
                    if let Some(route_id) = after {
                        log.emit(
                            "controller_route_visit",
                            at_ms,
                            json_map([("routeId", json!(route_id))]),
                        )?;
                    }
                }
            }
            let screen = screen_for_state(state, catalog)?;
            present(platform, &screen)?;
            log.emit(
                "input_to_frame",
                at_ms,
                json_map([
                    ("latencyUs", json!(0)),
                    ("sessionStep", json!(state.session_step)),
                ]),
            )?;
            return Ok(());
        }
    }

    let selection_before = state.selected_content_id.clone();
    let controller_route = matches!(
        state.presentation.ui.route,
        ui_model::Route::Settings | ui_model::Route::Wifi(_)
    );
    let route_before_presentation = state.route.clone();
    exit_theme_garden(state, action);
    let vault_button = match frontend_button {
        Some(button) => handle_save_vault_button(state, button)?,
        None => false,
    };
    if !vault_button {
        handle_presentation_action(&mut state.presentation, action)?;
        if controller_route && state.presentation.ui.route == ui_model::Route::Home {
            state.route = Route::Library;
        }
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
    state.persisted.preferences = state.presentation.ui.preferences.clone();
    launcher_state::save(&evidence.root.join("data"), &state.persisted)
        .map_err(|error| anyhow!(error.to_string()))?;
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
        (Route::Games, input_profile::Action::Start) => {
            state.route = Route::Library;
            state.power.game_exit();
            route_changed = true;
        }
        (Route::Session, input_profile::Action::Start) => {
            let result = state
                .broker
                .complete(0, 0)
                .map_err(|error| anyhow!(error.to_string()))?;
            state.active_session = None;
            state.last_session = Some(result);
            state.route = Route::Library;
            state.power.game_exit();
            refresh_resume_projection(state, catalog, launch_catalog)
                .map_err(anyhow::Error::msg)?;
            route_changed = true;
        }
        (Route::Games, input_profile::Action::Primary)
            if route_before_presentation == Route::Games
                && state.presentation.ui.route == ui_model::Route::Games =>
        {
            let entry = &catalog.entries[selected_catalog_index(state, catalog)];
            let request = launch_request(
                entry,
                launch_catalog,
                &state.input_profile,
                &state.input_mappings,
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
            state
                .power
                .begin_game(&entry.system, &entry.id)
                .map_err(anyhow::Error::msg)?;
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

fn apply_portmaster_route(state: &mut AppState, route_id: &str) -> Result<()> {
    fs::create_dir_all(state.package_root.join("data/saves"))?;
    fs::create_dir_all(state.package_root.join("data/states"))?;
    package_manager::portmaster_user_paths(&state.package_root)?;
    let fixture = |id: &str| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/demo-content")
            .join(id)
            .join("payload")
    };
    let install = |state: &AppState, id: &str| {
        if package_manager::resolve_portmaster(&state.package_root, id, "1.0.0").is_ok() {
            return Ok(());
        }
        let payload = fixture(id);
        package_manager::install(
            &state.package_root,
            &payload.join("manifest.json"),
            &payload,
            package_manager::TransactionOptions::default(),
        )
        .map(|_| ())
        .map_err(|error| anyhow!(error.to_string()))
    };
    match route_id {
        "portmaster-install" => {
            install(state, "orbit-garden")?;
            install(state, "signal-workshop")?;
        }
        "portmaster-uninstall-protected-data" => {
            install(state, "orbit-garden")?;
            install(state, "signal-workshop")?;
            let save = state
                .package_root
                .join("data/saves/orbit-garden/protected.sav");
            fs::create_dir_all(save.parent().expect("save parent"))?;
            fs::write(save, b"protected Orbit Garden save")?;
            package_manager::uninstall(
                &state.package_root,
                "orbit-garden",
                package_manager::TransactionOptions::default(),
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn reduce_product_state(state: &mut ProductJourneyState, button: Button) -> bool {
    use ProductJourneyState::*;
    if button == Button::Menu {
        if let Session {
            content, marker, ..
        } = state
        {
            *state = QuickMenu {
                selected: QuickMenuItem::Continue,
                content: *content,
                marker: *marker,
                preview: "slot 1 · no save yet",
            };
        } else {
            *state = Home {
                selected: HomeItem::Systems,
            };
        }
        return true;
    }
    let next = match state {
        Home { selected } => match button {
            Button::Down => {
                *selected = match selected {
                    HomeItem::Systems => HomeItem::Favorites,
                    HomeItem::Favorites => HomeItem::Recent,
                    HomeItem::Recent => HomeItem::Settings,
                    HomeItem::Settings => HomeItem::Systems,
                };
                return false;
            }
            Button::Up => {
                *selected = match selected {
                    HomeItem::Systems => HomeItem::Settings,
                    HomeItem::Favorites => HomeItem::Systems,
                    HomeItem::Recent => HomeItem::Favorites,
                    HomeItem::Settings => HomeItem::Recent,
                };
                return false;
            }
            Button::Primary => Some(match selected {
                HomeItem::Systems => Systems {
                    selected: SystemItem::Library,
                },
                HomeItem::Favorites => Games {
                    surface: GameSurface::Favorites,
                },
                HomeItem::Recent => Games {
                    surface: GameSurface::Recent,
                },
                HomeItem::Settings => Settings {
                    section: SettingsSection::Root,
                    pending: None,
                    validation: None,
                },
            }),
            _ => None,
        },
        Systems { selected } => match button {
            Button::Down => {
                *selected = match selected {
                    SystemItem::Library => SystemItem::Nebula,
                    SystemItem::Nebula => SystemItem::Mirror,
                    SystemItem::Mirror => SystemItem::Portmaster,
                    SystemItem::Portmaster => SystemItem::Library,
                };
                return false;
            }
            Button::Up => {
                *selected = match selected {
                    SystemItem::Library => SystemItem::Portmaster,
                    SystemItem::Nebula => SystemItem::Library,
                    SystemItem::Mirror => SystemItem::Nebula,
                    SystemItem::Portmaster => SystemItem::Mirror,
                };
                return false;
            }
            Button::Primary => Some(match selected {
                SystemItem::Library => Games {
                    surface: GameSurface::List,
                },
                SystemItem::Nebula => Session {
                    content: DemoContent::Nebula,
                    marker: 0,
                    restored: false,
                },
                SystemItem::Mirror => Session {
                    content: DemoContent::Mirror,
                    marker: 0,
                    restored: false,
                },
                SystemItem::Portmaster => Portmaster {
                    page: PortmasterPage::Catalog,
                },
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Systems,
            }),
            _ => None,
        },
        Games {
            surface: GameSurface::List,
        } => match button {
            Button::Right => Some(Games {
                surface: GameSurface::Details,
            }),
            Button::Select => Some(Games {
                surface: GameSurface::SearchKeyboard,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Systems,
            }),
            _ => None,
        },
        Games {
            surface: GameSurface::Details,
        } => match button {
            Button::Select => Some(Games {
                surface: GameSurface::Favorite,
            }),
            Button::Primary => Some(Session {
                content: DemoContent::Nebula,
                marker: 0,
                restored: false,
            }),
            Button::Secondary => Some(Games {
                surface: GameSurface::List,
            }),
            _ => None,
        },
        Games {
            surface: GameSurface::Favorite,
        } => match button {
            Button::Secondary => Some(Home {
                selected: HomeItem::Favorites,
            }),
            _ => None,
        },
        Games {
            surface: GameSurface::Favorites,
        } => match button {
            Button::Primary => Some(Games {
                surface: GameSurface::Details,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Favorites,
            }),
            _ => None,
        },
        Games {
            surface: GameSurface::Recent,
        } => match button {
            Button::Primary => Some(Games {
                surface: GameSurface::Details,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Recent,
            }),
            _ => None,
        },
        Games {
            surface: GameSurface::SearchKeyboard,
        } => match button {
            Button::Start => Some(Games {
                surface: GameSurface::SearchResults,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Systems,
            }),
            _ => None,
        },
        Games {
            surface: GameSurface::SearchResults,
        } => match button {
            Button::Secondary => Some(Home {
                selected: HomeItem::Systems,
            }),
            _ => None,
        },
        Settings {
            section: SettingsSection::Root,
            ..
        } => match button {
            Button::Down => Some(Settings {
                section: SettingsSection::Display,
                pending: None,
                validation: None,
            }),
            Button::Primary => Some(Settings {
                section: SettingsSection::Display,
                pending: None,
                validation: None,
            }),
            Button::Select => Some(GameSwitcher {
                page: SwitcherPage::List,
                content: DemoContent::Nebula,
                marker: 0,
            }),
            Button::Start => Some(Diagnostics {
                page: DiagnosticPage::Root,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Settings,
            }),
            _ => None,
        },
        Settings {
            section,
            pending: None,
            validation: None,
        } => match button {
            Button::Down => Some(Settings {
                section: next_setting(*section),
                pending: None,
                validation: None,
            }),
            Button::Right if matches!(section, SettingsSection::Display) => Some(Settings {
                section: *section,
                pending: Some(SettingChange {
                    name: "Scaling",
                    old: "Aspect",
                    new: "Integer",
                }),
                validation: None,
            }),
            Button::Primary if matches!(section, SettingsSection::Input) => Some(Settings {
                section: *section,
                pending: None,
                validation: Some("controller mapping conflicts with Menu"),
            }),
            Button::Primary if matches!(section, SettingsSection::Library) => Some(FocusAdmin {
                selected: FocusAdminItem::Add,
            }),
            Button::Primary if matches!(section, SettingsSection::System) => Some(Wifi {
                view: WifiJourneyView::Scan,
            }),
            Button::Primary if matches!(section, SettingsSection::Theme) => Some(Theme {
                stage: ThemeJourneyStage::Catalog,
            }),
            Button::Primary if matches!(section, SettingsSection::Scraper) => Some(Scraper {
                stage: ScraperJourneyStage::Settings,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Settings,
            }),
            _ => None,
        },
        Settings {
            pending: Some(_),
            section,
            ..
        } => match button {
            Button::Primary => Some(Settings {
                section: *section,
                pending: None,
                validation: None,
            }),
            Button::Secondary => Some(Settings {
                section: *section,
                pending: None,
                validation: None,
            }),
            _ => None,
        },
        Settings {
            section,
            validation: Some(_),
            ..
        } => match button {
            Button::Secondary => Some(Settings {
                section: *section,
                pending: None,
                validation: None,
            }),
            _ => None,
        },
        Wifi { view } => match button {
            Button::Primary => Some(match view {
                WifiJourneyView::Scan => Wifi {
                    view: WifiJourneyView::Password,
                },
                WifiJourneyView::Password => Wifi {
                    view: WifiJourneyView::Progress,
                },
                WifiJourneyView::OpenConfirmation => Wifi {
                    view: WifiJourneyView::Progress,
                },
                WifiJourneyView::Hidden => Wifi {
                    view: WifiJourneyView::ManualSsid,
                },
                WifiJourneyView::Saved => Wifi {
                    view: WifiJourneyView::ForgetConfirmation,
                },
                WifiJourneyView::ForgetConfirmation => Wifi {
                    view: WifiJourneyView::Forgotten,
                },
                WifiJourneyView::Progress => Wifi {
                    view: WifiJourneyView::RetryError,
                },
                WifiJourneyView::RetryError => Wifi {
                    view: WifiJourneyView::Scan,
                },
                _ => Wifi { view: *view },
            }),
            Button::Down => Some(match view {
                WifiJourneyView::Scan => Wifi {
                    view: WifiJourneyView::Saved,
                },
                WifiJourneyView::Saved => Wifi {
                    view: WifiJourneyView::Hidden,
                },
                WifiJourneyView::Hidden => Wifi {
                    view: WifiJourneyView::OpenConfirmation,
                },
                _ => Wifi { view: *view },
            }),
            Button::Secondary => Some(Settings {
                section: SettingsSection::System,
                pending: None,
                validation: None,
            }),
            _ => None,
        },
        Theme { stage } => match button {
            Button::Primary => Some(Theme {
                stage: match stage {
                    ThemeJourneyStage::Catalog => ThemeJourneyStage::Preview,
                    ThemeJourneyStage::Preview => ThemeJourneyStage::Install,
                    ThemeJourneyStage::Install => ThemeJourneyStage::Update,
                    ThemeJourneyStage::Update => ThemeJourneyStage::Remove,
                    ThemeJourneyStage::Remove => ThemeJourneyStage::Catalog,
                    ThemeJourneyStage::Fallback => ThemeJourneyStage::Catalog,
                },
            }),
            Button::Down if matches!(stage, ThemeJourneyStage::Preview) => Some(Theme {
                stage: ThemeJourneyStage::Fallback,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Settings,
            }),
            _ => None,
        },
        Scraper { stage } => match button {
            Button::Primary => Some(Scraper {
                stage: match stage {
                    ScraperJourneyStage::Settings => ScraperJourneyStage::Game,
                    ScraperJourneyStage::Game => ScraperJourneyStage::Queue,
                    ScraperJourneyStage::Queue => ScraperJourneyStage::Progress,
                    ScraperJourneyStage::Progress => ScraperJourneyStage::Failure,
                    ScraperJourneyStage::Paused => ScraperJourneyStage::Progress,
                    ScraperJourneyStage::Ambiguity => ScraperJourneyStage::Success,
                    ScraperJourneyStage::Success | ScraperJourneyStage::Failure => {
                        ScraperJourneyStage::Settings
                    }
                },
            }),
            Button::Select if matches!(stage, ScraperJourneyStage::Game) => Some(Scraper {
                stage: ScraperJourneyStage::Queue,
            }),
            Button::Select if matches!(stage, ScraperJourneyStage::Progress) => Some(Scraper {
                stage: ScraperJourneyStage::Paused,
            }),
            Button::Right if matches!(stage, ScraperJourneyStage::Progress) => Some(Scraper {
                stage: ScraperJourneyStage::Ambiguity,
            }),
            Button::Start if matches!(stage, ScraperJourneyStage::Progress) => Some(Scraper {
                stage: ScraperJourneyStage::Success,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Settings,
            }),
            _ => None,
        },
        Diagnostics { page } => match button {
            Button::Down => Some(Diagnostics {
                page: next_diagnostic(*page),
            }),
            Button::Primary => Some(match page {
                DiagnosticPage::Root => Diagnostics {
                    page: DiagnosticPage::SafeMode,
                },
                DiagnosticPage::LowBattery => ShutdownConfirm,
                _ => Diagnostics { page: *page },
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Settings,
            }),
            _ => None,
        },
        Session {
            content, marker, ..
        } => match button {
            Button::Up | Button::Down | Button::Left | Button::Right => {
                *marker = marker.saturating_add(1);
                return false;
            }
            Button::Select => Some(GameSwitcher {
                page: SwitcherPage::Autosave,
                content: *content,
                marker: *marker,
            }),
            Button::Secondary => Some(GameSwitcher {
                page: SwitcherPage::Exit,
                content: *content,
                marker: *marker,
            }),
            _ => None,
        },
        QuickMenu {
            selected,
            content,
            marker,
            preview: _,
        } => match button {
            Button::Down => {
                *selected = match selected {
                    QuickMenuItem::Continue => QuickMenuItem::SaveSlot,
                    QuickMenuItem::SaveSlot => QuickMenuItem::LoadSlot,
                    QuickMenuItem::LoadSlot => QuickMenuItem::Restart,
                    QuickMenuItem::Restart => QuickMenuItem::RetroArch,
                    QuickMenuItem::RetroArch => QuickMenuItem::Exit,
                    QuickMenuItem::Exit => QuickMenuItem::Continue,
                };
                return false;
            }
            Button::Up => {
                *selected = match selected {
                    QuickMenuItem::Continue => QuickMenuItem::Exit,
                    QuickMenuItem::SaveSlot => QuickMenuItem::Continue,
                    QuickMenuItem::LoadSlot => QuickMenuItem::SaveSlot,
                    QuickMenuItem::Restart => QuickMenuItem::LoadSlot,
                    QuickMenuItem::RetroArch => QuickMenuItem::Restart,
                    QuickMenuItem::Exit => QuickMenuItem::RetroArch,
                };
                return false;
            }
            Button::Primary => Some(match selected {
                QuickMenuItem::Continue => Session {
                    content: *content,
                    marker: *marker,
                    restored: false,
                },
                QuickMenuItem::SaveSlot => QuickMenu {
                    selected: *selected,
                    content: *content,
                    marker: *marker,
                    preview: "slot 1 · saved just now",
                },
                QuickMenuItem::LoadSlot => Session {
                    content: *content,
                    marker: *marker,
                    restored: true,
                },
                QuickMenuItem::Restart => Session {
                    content: *content,
                    marker: 0,
                    restored: false,
                },
                QuickMenuItem::RetroArch => QuickMenu {
                    selected: *selected,
                    content: *content,
                    marker: *marker,
                    preview: "RetroArch menu opened",
                },
                QuickMenuItem::Exit => Home {
                    selected: HomeItem::Systems,
                },
            }),
            Button::Secondary => Some(Session {
                content: *content,
                marker: *marker,
                restored: false,
            }),
            _ => None,
        },
        GameSwitcher {
            page,
            content,
            marker,
        } => match button {
            Button::Primary => Some(match page {
                SwitcherPage::Autosave => GameSwitcher {
                    page: SwitcherPage::Exit,
                    content: *content,
                    marker: *marker,
                },
                SwitcherPage::Exit => Home {
                    selected: HomeItem::Systems,
                },
                SwitcherPage::List => GameSwitcher {
                    page: SwitcherPage::Resume,
                    content: *content,
                    marker: *marker,
                },
                SwitcherPage::Resume => GameSwitcher {
                    page: SwitcherPage::Restoration,
                    content: *content,
                    marker: *marker,
                },
                SwitcherPage::Restoration => Session {
                    content: *content,
                    marker: *marker,
                    restored: true,
                },
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Systems,
            }),
            _ => None,
        },
        Portmaster { page } => match button {
            Button::Primary => Some(match page {
                PortmasterPage::Catalog => Portmaster {
                    page: PortmasterPage::Install,
                },
                PortmasterPage::Install => Session {
                    content: DemoContent::Orbit,
                    marker: 0,
                    restored: false,
                },
                PortmasterPage::UninstallProtected => Portmaster {
                    page: PortmasterPage::UninstallProtected,
                },
            }),
            Button::Down if matches!(page, PortmasterPage::Catalog) => Some(Session {
                content: DemoContent::Signal,
                marker: 0,
                restored: false,
            }),
            Button::Select if matches!(page, PortmasterPage::Catalog) => Some(Portmaster {
                page: PortmasterPage::UninstallProtected,
            }),
            Button::Secondary => Some(Home {
                selected: HomeItem::Systems,
            }),
            _ => None,
        },
        FocusAdmin { .. } | FocusHome { .. } | FocusRecovery | KidQuickMenu { .. } => None,
        ShutdownConfirm => match button {
            Button::Secondary => Some(Home {
                selected: HomeItem::Settings,
            }),
            _ => None,
        },
    };
    if let Some(next) = next {
        *state = next;
        true
    } else {
        false
    }
}

fn next_setting(section: SettingsSection) -> SettingsSection {
    match section {
        SettingsSection::Root => SettingsSection::Display,
        SettingsSection::Display => SettingsSection::Input,
        SettingsSection::Input => SettingsSection::Audio,
        SettingsSection::Audio => SettingsSection::Power,
        SettingsSection::Power => SettingsSection::Library,
        SettingsSection::Library => SettingsSection::Scraper,
        SettingsSection::Scraper => SettingsSection::Theme,
        SettingsSection::Theme => SettingsSection::System,
        SettingsSection::System => SettingsSection::Display,
    }
}
fn next_diagnostic(page: DiagnosticPage) -> DiagnosticPage {
    match page {
        DiagnosticPage::Root => DiagnosticPage::SafeMode,
        DiagnosticPage::SafeMode => DiagnosticPage::Updater,
        DiagnosticPage::Updater => DiagnosticPage::Rollback,
        DiagnosticPage::Rollback => DiagnosticPage::StorageFull,
        DiagnosticPage::StorageFull => DiagnosticPage::LowBattery,
        DiagnosticPage::LowBattery => DiagnosticPage::Root,
    }
}

fn launch_product_session(
    state: &mut AppState,
    catalog: &UiCatalog,
    launch_catalog: &LaunchCatalog,
    evidence: &Evidence,
    restored: bool,
) -> Result<()> {
    let ProductJourneyState::Session { content, .. } = state.journey else {
        return Ok(());
    };
    let content_id = match content {
        DemoContent::Orbit => "orbit-garden",
        DemoContent::Signal => "signal-workshop",
        DemoContent::Nebula => "nebula-nes",
        DemoContent::Mirror => "mirror-ps1",
    };
    if matches!(content, DemoContent::Orbit | DemoContent::Signal) {
        apply_portmaster_route(state, "portmaster-install")?;
    }
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == content_id)
        .ok_or_else(|| anyhow!("demo content is absent from catalog"))?;
    let request = launch_request(
        entry,
        launch_catalog,
        &state.input_profile,
        &state.input_mappings,
    )
    .map_err(|error| anyhow!(error))?;
    if restored {
        let choices = state
            .broker
            .resume_choices(&request)
            .map_err(|error| anyhow!(error.to_string()))?;
        if choices.contains(&ResumeDecision::Resume) {
            state
                .broker
                .resume_decision(request.clone(), ResumeDecision::Resume)
                .map_err(|error| anyhow!(error.to_string()))?;
        }
    }
    let bytes = launch_contract::request_json(&request)
        .map_err(|error| anyhow!(error.to_string()))?
        .into_bytes();
    let accepted = state
        .broker
        .submit(request, launch_catalog)
        .map_err(|error| anyhow!(error.to_string()))?;
    state
        .power
        .begin_game(&entry.system, &entry.id)
        .map_err(anyhow::Error::msg)?;
    write_bytes(evidence.root.join("launch-request.json"), &bytes)?;
    state.active_session = Some(accepted);
    state.route = Route::Session;
    Ok(())
}

fn physical_control(button: Button) -> input_profile::PhysicalControl {
    match button {
        Button::Up => input_profile::PhysicalControl::Up,
        Button::Down => input_profile::PhysicalControl::Down,
        Button::Left => input_profile::PhysicalControl::Left,
        Button::Right => input_profile::PhysicalControl::Right,
        Button::Primary => input_profile::PhysicalControl::A,
        Button::Secondary => input_profile::PhysicalControl::B,
        Button::Start => input_profile::PhysicalControl::Start,
        Button::Select => input_profile::PhysicalControl::Select,
        Button::L1 => input_profile::PhysicalControl::L1,
        Button::R1 => input_profile::PhysicalControl::R1,
        Button::Menu => input_profile::PhysicalControl::Home,
    }
}

fn frontend_action(action: input_profile::LogicalAction) -> Option<input_profile::Action> {
    Some(match action {
        input_profile::LogicalAction::MoveUp => input_profile::Action::MoveUp,
        input_profile::LogicalAction::MoveDown => input_profile::Action::MoveDown,
        input_profile::LogicalAction::MoveLeft => input_profile::Action::MoveLeft,
        input_profile::LogicalAction::MoveRight => input_profile::Action::MoveRight,
        input_profile::LogicalAction::Primary => input_profile::Action::Primary,
        input_profile::LogicalAction::Secondary | input_profile::LogicalAction::Escape => {
            input_profile::Action::Secondary
        }
        input_profile::LogicalAction::Start => input_profile::Action::Start,
        input_profile::LogicalAction::Select | input_profile::LogicalAction::Menu => {
            input_profile::Action::Select
        }
        input_profile::LogicalAction::LeftStickClick => input_profile::Action::LeftStickClick,
        input_profile::LogicalAction::RightStickClick => input_profile::Action::RightStickClick,
        input_profile::LogicalAction::JumpNextGroup => input_profile::Action::JumpNextGroup,
        input_profile::LogicalAction::JumpPreviousGroup => input_profile::Action::JumpPreviousGroup,
        input_profile::LogicalAction::F1 => input_profile::Action::F1,
        input_profile::LogicalAction::F2 => input_profile::Action::F2,
        input_profile::LogicalAction::Fn => input_profile::Action::Fn,
        input_profile::LogicalAction::Home => input_profile::Action::Home,
        input_profile::LogicalAction::VolumeUp
        | input_profile::LogicalAction::VolumeDown
        | input_profile::LogicalAction::BrightnessUp
        | input_profile::LogicalAction::BrightnessDown
        | input_profile::LogicalAction::LoadState
        | input_profile::LogicalAction::Quit => return None,
    })
}

fn frontend_button(action: input_profile::Action) -> Option<Button> {
    Some(match action {
        input_profile::Action::MoveUp => Button::Up,
        input_profile::Action::MoveDown => Button::Down,
        input_profile::Action::MoveLeft => Button::Left,
        input_profile::Action::MoveRight => Button::Right,
        input_profile::Action::Primary => Button::Primary,
        input_profile::Action::Secondary => Button::Secondary,
        input_profile::Action::Start => Button::Start,
        input_profile::Action::Select => Button::Select,
        input_profile::Action::Home => Button::Menu,
        input_profile::Action::JumpNextGroup
        | input_profile::Action::JumpPreviousGroup
        | input_profile::Action::LeftStickClick
        | input_profile::Action::RightStickClick
        | input_profile::Action::F1
        | input_profile::Action::F2
        | input_profile::Action::Fn => return None,
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
        "tg4040": platform.tg4040_state().ok(),
        "faults": state.faults,
        "lifecycle": state.lifecycle.evidence(),
        "clock": {"monotonicMs": platform.logical_time_ms(), "wallClockMs": platform.wall_clock_ms()},
        "saveVault": save_vault_json(state),
        "controllerRoute": { "navigatorVisible": false, "selectedIndex": 0, "currentId": canonical_route_id(&state.journey), "expectedCount": CONTROLLER_ROUTE_COUNT },
        "presentation": presentation,
        "power": state.power.evidence(),
        "battery": state.battery.evidence(),
        "focus": {
            "entries": state.persisted.focus,
            "defaultHome": state.persisted.focus_home,
            "kidSafe": state.persisted.kid_safe,
            "missing": state.persisted.focus.iter().filter(|id| !catalog.entries.iter().any(|entry| entry.id == **id)).count(),
        },
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
        "battery": {
            "percent": hardware.battery_percent,
            "charging": hardware.charging,
            "full": hardware.full,
            "health": hardware.battery_health,
        },
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
    input_catalog: &input_profile::Catalog,
    input_mappings: &input_profile::InputMappings,
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
    let mappings = input_catalog
        .launch_mappings_with(input_mappings, Some(&entry.system), Some(&entry.id))
        .map_err(|error| error.to_string())?;

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
            bindings: mappings.bindings,
            hotkeys: mappings.hotkeys,
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
            let surface = state
                .settings
                .scene()
                .map_err(|error| anyhow!(error.to_string()))?
                .surface;
            state
                .settings
                .press(button)
                .map_err(|error| anyhow!(error.to_string()))?;
            sync_settings_ui_size(state, action)?;
            state.sync_visual_preferences()?;
            if action == input_profile::Action::Secondary
                && surface == settings_ui::Surface::SectionList
            {
                state.ui = ui_model::reduce(&state.ui, UiAction::Back);
            }
            return Ok(());
        }
        if matches!(state.ui.route, ui_model::Route::Wifi(_)) {
            let current_route = match &state.ui.route {
                ui_model::Route::Wifi(route) => route.clone(),
                _ => unreachable!(),
            };
            let view = state.wifi.snapshot().view;
            let press_result = state
                .wifi
                .press(button)
                .map_err(|error| anyhow!(error.to_string()));
            let route = wifi_route_for_snapshot(&current_route, &state.wifi.snapshot());
            state.ui =
                ui_model::reduce(&state.ui, UiAction::Navigate(ui_model::Route::Wifi(route)));
            press_result?;
            if action == input_profile::Action::Secondary
                && view == wifi_settings_controller::View::Menu
            {
                state.ui = ui_model::reduce(&state.ui, UiAction::Back);
            }
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
        if self.sequence >= 4096 {
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
