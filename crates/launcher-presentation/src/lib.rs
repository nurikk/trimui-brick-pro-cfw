use launcher_theme::{scene as theme_scene, Reason as ThemeReason, ValidatedTheme};
use serde::Serialize;
use ui_model::{AssetKind, GeneratedAssetRef, Route, UiState};

pub const SCHEMA: &str = "launcher-presentation/v1";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub background: [u8; 4],
    pub surface: [u8; 4],
    pub accent: [u8; 4],
    pub text: [u8; 4],
    pub muted: [u8; 4],
    pub highlight: [u8; 4],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenItem {
    pub id: String,
    pub label: String,
    pub selected: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenRegion {
    pub id: String,
    pub kind: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub visible: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDetails {
    pub id: String,
    pub title: String,
    pub description: String,
    pub rating: Option<u8>,
    pub release_date: Option<String>,
    pub box_art: GeneratedAssetRef,
    pub screenshot: GeneratedAssetRef,
    pub favorite: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Affordances {
    pub clock: String,
    pub battery_percent: u8,
    pub charging: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScraperView {
    pub route: String,
    pub status: String,
    pub queue_count: usize,
    pub progress_percent: Option<u8>,
    pub paused: bool,
    pub ambiguous_candidates: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub width: u16,
    pub height: u16,
    pub surface: settings_ui::Surface,
    pub sections: Vec<SettingsSection>,
    pub selected_setting_id: Option<String>,
    pub pending_count: usize,
    pub validation_error_count: usize,
    pub keyboard_masked: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    pub id: String,
    pub label_key: String,
    pub controls: Vec<SettingsControl>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsControl {
    pub setting_id: String,
    pub label_key: String,
    pub kind: settings_schema::FieldKind,
    pub value: settings_ui::SemanticValue,
    pub enabled: bool,
    pub redacted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub schema: &'static str,
    pub identity: &'static str,
    pub route: String,
    pub focus: String,
    pub title: String,
    pub selected_label: String,
    pub menu: Vec<ScreenItem>,
    pub game_rows: Vec<ScreenItem>,
    pub selected_game: Option<GameDetails>,
    pub regions: Vec<ScreenRegion>,
    pub palette: Palette,
    pub theme: launcher_theme::Scene,
    pub theme_fallback: Option<String>,
    pub generated_assets: Vec<GeneratedAssetRef>,
    pub settings: Option<SettingsView>,
    pub wifi: Option<wifi_settings_controller::Snapshot>,
    pub scraper: ScraperView,
    pub modal: Option<String>,
    pub splash: String,
    pub affordances: Affordances,
    pub controller_help: Vec<ui_model::HelpBinding>,
}

pub fn build(
    state: &UiState,
    theme: &ValidatedTheme,
    theme_fallback: Option<ThemeReason>,
    settings: Option<&settings_ui::Scene>,
    wifi: Option<&wifi_settings_controller::Snapshot>,
) -> Screen {
    let selected_id = state
        .menu
        .selection
        .item_id
        .as_ref()
        .map(|id| id.0.as_str());
    let menu = state
        .menu
        .entries
        .iter()
        .map(|entry| ScreenItem {
            id: entry.id.0.clone(),
            label: entry.label.clone(),
            selected: entry.selected,
            enabled: entry.enabled,
        })
        .collect::<Vec<_>>();
    let game_rows = state
        .games
        .iter()
        .filter(|game| {
            state
                .selected_system
                .as_ref()
                .is_none_or(|id| id == &game.system_id)
        })
        .filter(|game| !matches!(state.route, Route::Favorites) || game.favorite)
        .filter(|game| {
            state.search_query.is_empty()
                || game
                    .title
                    .to_ascii_lowercase()
                    .contains(&state.search_query.to_ascii_lowercase())
        })
        .map(|game| ScreenItem {
            id: game.id.0.clone(),
            label: game.title.clone(),
            selected: selected_id == Some(game.id.0.as_str()),
            enabled: true,
        })
        .collect::<Vec<_>>();
    let selected_game = selected_id.and_then(|id| {
        state
            .games
            .iter()
            .find(|game| game.id.0 == id)
            .map(|game| GameDetails {
                id: game.id.0.clone(),
                title: game.title.clone(),
                description: game.description.clone(),
                rating: game.rating,
                release_date: game.release_date.clone(),
                box_art: game.box_art.clone(),
                screenshot: game.screenshot.clone(),
                favorite: game.favorite,
            })
    });
    let theme_data = theme.theme();
    let palette = Palette {
        background: color(&theme_data.colors.background),
        surface: color(&theme_data.colors.surface),
        accent: color(&theme_data.colors.accent),
        text: color(&theme_data.colors.text),
        muted: color(&theme_data.colors.muted),
        highlight: color(&theme_data.colors.highlight),
    };
    let scene = theme_scene(theme);
    let generated_assets = state
        .systems
        .iter()
        .flat_map(|system| [&system.artwork, &system.logo])
        .chain(
            state
                .games
                .iter()
                .flat_map(|game| [&game.box_art, &game.screenshot]),
        )
        .cloned()
        .collect();
    Screen {
        schema: SCHEMA,
        identity: ui_model::ARTBOOK_IDENTITY,
        route: route_name(&state.route),
        focus: focus_name(&state.focus),
        title: title(&state.route),
        selected_label: state
            .menu
            .selected()
            .map(|entry| entry.label.clone())
            .unwrap_or_default(),
        menu,
        game_rows,
        selected_game,
        regions: scene
            .regions
            .iter()
            .map(|region| ScreenRegion {
                id: region.id.clone(),
                kind: region_kind_name(region.kind),
                x: region.bounds.x,
                y: region.bounds.y,
                width: region.bounds.width,
                height: region.bounds.height,
                visible: true,
            })
            .collect(),
        palette,
        theme: scene,
        theme_fallback: theme_fallback.map(format_reason),
        generated_assets,
        settings: settings.map(settings_view),
        wifi: wifi.cloned(),
        scraper: scraper_view(state),
        modal: state.modal.as_ref().map(modal_text),
        splash: splash_text(&state.splash),
        affordances: Affordances {
            clock: state.affordances.clock.value.clone(),
            battery_percent: state.affordances.battery.percent,
            charging: state.affordances.battery.charging,
        },
        controller_help: state.controller_help.bindings.clone(),
    }
}

fn settings_view(scene: &settings_ui::Scene) -> SettingsView {
    SettingsView {
        width: scene.width,
        height: scene.height,
        surface: scene.surface,
        sections: scene
            .sections
            .iter()
            .map(|section| SettingsSection {
                id: section.id.clone(),
                label_key: section.label_key.clone(),
                controls: section
                    .groups
                    .iter()
                    .flat_map(|group| group.controls.iter())
                    .map(|control| SettingsControl {
                        setting_id: control.setting_id.clone(),
                        label_key: control.label_key.clone(),
                        kind: control.kind,
                        value: control.value.clone(),
                        enabled: control.enabled,
                        redacted: control.redacted,
                    })
                    .collect(),
            })
            .collect(),
        selected_setting_id: scene
            .form
            .as_ref()
            .and_then(|form| form.selected_setting_id.clone()),
        pending_count: scene.pending.count,
        validation_error_count: scene.validation_errors.len(),
        keyboard_masked: scene
            .keyboard
            .as_ref()
            .is_some_and(|keyboard| keyboard.masked),
    }
}

fn color(value: &str) -> [u8; 4] {
    let mut result = [0, 0, 0, 255];
    for (index, slot) in result[..3].iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[1 + index * 2..3 + index * 2], 16).unwrap_or(0);
    }
    result
}

fn route_name(route: &Route) -> String {
    match route {
        Route::Home => "home".into(),
        Route::Systems => "systems".into(),
        Route::Games => "games".into(),
        Route::Search => "search".into(),
        Route::Favorites => "favorites".into(),
        Route::Settings => "settings".into(),
        Route::GameSwitcher => "game-switcher".into(),
        Route::Recovery => "recovery".into(),
        Route::Scraper(route) => format!("scraper-{}", scraper_route_name(route)),
        Route::Wifi(route) => format!("wifi-{}", wifi_route_name(route)),
    }
}

fn format_debug<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:?}").to_ascii_lowercase().replace('_', "-")
}

fn title(route: &Route) -> String {
    match route {
        Route::Home => "Artbook Home".into(),
        Route::Systems => "Systems".into(),
        Route::Games => "Games".into(),
        Route::Search => "Search".into(),
        Route::Favorites => "Favorites".into(),
        Route::Settings => "Settings".into(),
        Route::GameSwitcher => "Game Switcher".into(),
        Route::Recovery => "Recovery".into(),
        Route::Scraper(_) => "Scraper".into(),
        Route::Wifi(_) => "Wi-Fi".into(),
    }
}

fn scraper_route_name(route: &ui_model::ScraperRoute) -> &'static str {
    match route {
        ui_model::ScraperRoute::Settings => "settings",
        ui_model::ScraperRoute::Game => "game",
        ui_model::ScraperRoute::BulkQueue => "bulk-queue",
        ui_model::ScraperRoute::AmbiguousChoice => "ambiguous-choice",
    }
}

fn wifi_route_name(route: &ui_model::WifiRoute) -> &'static str {
    match route {
        ui_model::WifiRoute::Scan => "scan",
        ui_model::WifiRoute::AccessPointSelection => "access-point-selection",
        ui_model::WifiRoute::HiddenNetwork => "hidden-network",
        ui_model::WifiRoute::ManualSsid => "manual-ssid",
        ui_model::WifiRoute::PasswordEntry => "password-entry",
        ui_model::WifiRoute::Progress => "progress",
        ui_model::WifiRoute::Error => "error",
    }
}

fn focus_name(focus: &ui_model::FocusTarget) -> String {
    match focus {
        ui_model::FocusTarget::Menu => "menu",
        ui_model::FocusTarget::SearchInput => "search-input",
        ui_model::FocusTarget::GameList => "game-list",
        ui_model::FocusTarget::Artwork => "artwork",
        ui_model::FocusTarget::Modal => "modal",
        ui_model::FocusTarget::ControllerHelp => "controller-help",
    }
    .into()
}

fn format_reason(reason: ThemeReason) -> String {
    format_debug(&reason)
}

fn splash_text(splash: &ui_model::SplashState) -> String {
    match splash {
        ui_model::SplashState::Visible(asset) => asset.id.clone(),
        ui_model::SplashState::Ready => "ready".into(),
        ui_model::SplashState::Fallback { asset, .. } => asset.id.clone(),
    }
}

fn modal_text(modal: &ui_model::ModalState) -> String {
    match modal {
        ui_model::ModalState::Unavailable(error) => error.code.clone(),
        ui_model::ModalState::Info { title, .. } => title.clone(),
        ui_model::ModalState::Confirm { title, .. } => title.clone(),
        ui_model::ModalState::MaskedPasswordKeyboard { .. } => "masked-password-keyboard".into(),
    }
}

fn scraper_view(state: &UiState) -> ScraperView {
    let scraper = &state.scraper;
    let progress_percent = scraper.progress.as_ref().map(|progress| {
        if progress.total == 0 {
            0
        } else {
            ((u32::from(progress.completed) * 100) / u32::from(progress.total)) as u8
        }
    });
    ScraperView {
        route: format_debug(&scraper.route),
        status: format_debug(&scraper.status),
        queue_count: scraper.queue.len(),
        progress_percent,
        paused: scraper
            .progress
            .as_ref()
            .is_some_and(|progress| progress.paused),
        ambiguous_candidates: scraper
            .ambiguous_choice
            .as_ref()
            .map_or_else(Vec::new, |choice| choice.candidates.clone()),
    }
}

fn region_kind_name(kind: launcher_theme::RegionKind) -> String {
    match kind {
        launcher_theme::RegionKind::SystemArt => "system-art",
        launcher_theme::RegionKind::GameList => "game-list",
        launcher_theme::RegionKind::BoxArtPlaceholder => "box-art-placeholder",
        launcher_theme::RegionKind::ScreenshotPlaceholder => "screenshot-placeholder",
        launcher_theme::RegionKind::Metadata => "metadata",
        launcher_theme::RegionKind::Menu => "menu",
        launcher_theme::RegionKind::HelpStrip => "help-strip",
        launcher_theme::RegionKind::Clock => "clock",
        launcher_theme::RegionKind::Battery => "battery",
    }
    .into()
}

pub fn is_generated_asset(asset: &GeneratedAssetRef) -> bool {
    asset.id.starts_with("generated-") || asset.id == "artbook-generated-splash"
}

pub fn is_presented_asset_kind(asset: &GeneratedAssetRef) -> bool {
    matches!(
        asset.kind,
        AssetKind::SystemArtwork
            | AssetKind::SystemLogo
            | AssetKind::BoxArt
            | AssetKind::Screenshot
            | AssetKind::Splash
            | AssetKind::Fallback
    )
}
