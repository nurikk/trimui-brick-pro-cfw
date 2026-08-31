use launcher_theme::{scene as theme_scene, Reason as ThemeReason, ValidatedTheme};
use serde::Serialize;
use ui_model::{
    AssetKind, GeneratedAssetRef, ListDensity, Route, UiSize, UiState, VisualPreset, VisualProfile,
};

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
pub struct ScreenMedia {
    pub content_id: String,
    pub kind: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing)]
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Affordances {
    pub clock: String,
    pub battery_percent: Option<u8>,
    pub charging_status: String,
    pub external_power: Option<bool>,
    pub battery_health: String,
    pub battery_level: String,
    pub show_charging_status: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScraperView {
    pub route: String,
    pub status: String,
    pub queue_count: usize,
    pub completed: u16,
    pub total: u16,
    pub progress_percent: Option<u8>,
    pub configured_slots: u8,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub background: bool,
    pub counts: ui_model::ScraperCounts,
    pub rows: Vec<ui_model::ScraperRow>,
    pub actions: Vec<String>,
    pub cancel_requested: bool,
    pub ambiguous_candidates: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub width: u16,
    pub height: u16,
    pub surface: settings_ui::Surface,
    pub selected_section_id: Option<String>,
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
pub struct BoundaryStatus {
    pub available: bool,
    pub state: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryViews {
    pub metadata_scraper: BoundaryStatus,
    pub media_cache: BoundaryStatus,
    pub wifi_controller: BoundaryStatus,
    pub theme_garden: BoundaryStatus,
    pub update_agent: BoundaryStatus,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexView {
    pub status: String,
    pub entry_count: usize,
    pub visible_rows: usize,
    pub search_results: usize,
    pub queue_depth: usize,
}

impl Default for IndexView {
    fn default() -> Self {
        Self {
            status: "ready".into(),
            entry_count: 0,
            visible_rows: 12,
            search_results: 64,
            queue_depth: 32,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSyncCandidateView {
    pub logical_id: String,
    pub content_id: String,
    pub device_id: String,
    pub device_name: String,
    pub generation: u64,
    pub hash_prefix: String,
    pub parent_hash_prefix: Option<String>,
    pub ancestry: Vec<String>,
    pub save_kind: String,
    pub timestamp_ms: u64,
    pub size: u64,
    pub status: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSyncView {
    pub local: SaveSyncCandidateView,
    pub remote: SaveSyncCandidateView,
    pub state: String,
    pub transport_outcome: String,
    pub actions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutBox {
    pub id: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutGeometry {
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub boxes: Vec<LayoutBox>,
    pub focused_action: Option<LayoutBox>,
    pub visible_menu_items: usize,
}

impl LayoutGeometry {
    pub fn box_by_id(&self, id: &str) -> Option<&LayoutBox> {
        self.boxes.iter().find(|layout_box| layout_box.id == id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowMetrics {
    pub scale_percent: u16,
    pub row_height: u32,
    pub row_step: u32,
}

pub fn row_metrics(screen: &Screen, automatic_scale_percent: u16) -> RowMetrics {
    let scale_percent = screen
        .ui_size
        .preset_scale_percent()
        .unwrap_or(automatic_scale_percent);
    let density_percent = match screen.visual_profile.list_density {
        ListDensity::Normal => 100,
        ListDensity::Dense => 80,
    };
    let row_height = (26 * u32::from(scale_percent) * density_percent / 10_000).max(18);
    let spacing = (12 * u32::from(scale_percent) * density_percent / 10_000).max(6);
    RowMetrics {
        scale_percent,
        row_height,
        row_step: row_height + spacing,
    }
}

pub fn layout_geometry(
    screen: &Screen,
    viewport_width: u32,
    viewport_height: u32,
    automatic_scale_percent: u16,
) -> LayoutGeometry {
    let reflow = |id: &str, x: u32, y: u32, width: u32, height: u32| LayoutBox {
        id: id.into(),
        x: x.saturating_mul(viewport_width) / 1024,
        y: y.saturating_mul(viewport_height) / 768,
        width: (x + width).saturating_mul(viewport_width) / 1024
            - x.saturating_mul(viewport_width) / 1024,
        height: (y + height).saturating_mul(viewport_height) / 768
            - y.saturating_mul(viewport_height) / 768,
    };
    let mut boxes = screen
        .regions
        .iter()
        .map(|region| {
            reflow(
                &region.id,
                region.x.into(),
                region.y.into(),
                region.width.into(),
                region.height.into(),
            )
        })
        .collect::<Vec<_>>();
    let rows = row_metrics(screen, automatic_scale_percent);
    let row_height = rows.row_height;
    let row_step = rows.row_step;
    let menu = reflow("route-content", 32, 72, 960, 576);
    let items = if screen.focus == "game-list" {
        &screen.game_rows
    } else {
        &screen.menu
    };
    let physical_row_step = row_step.saturating_mul(viewport_height) / 768;
    let visible_menu_items = (menu.height / physical_row_step.max(1)).max(1) as usize;
    let focused_action = items
        .iter()
        .position(|item| item.selected && item.enabled)
        .map(|index| {
            let start = index
                .saturating_sub(visible_menu_items / 2)
                .min(items.len().saturating_sub(visible_menu_items));
            reflow(
                "focused-action",
                32,
                72 + (index.saturating_sub(start) as u32) * row_step,
                960,
                row_height,
            )
        });
    boxes.push(menu);
    match screen.route.as_str() {
        "games" | "games-no-metadata" | "favorites" | "recent" | "search" => {
            boxes.push(reflow("library-list", 0, 0, 400, 768));
            boxes.push(reflow("game-details", 400, 0, 624, 768));
        }
        "settings" | "theme-garden" | "save-vault" | "save-sync" | "portmaster"
        | "controller-routes" | "game-switcher" | "recovery" => {
            boxes.push(reflow("surface", 40, 48, 944, 632));
        }
        route if route.starts_with("wifi-") => {
            boxes.push(reflow("wifi-surface", 40, 48, 944, 632));
        }
        route if route.starts_with("scraper-") => {
            boxes.push(reflow("scraper-surface", 32, 72, 960, 610));
        }
        _ => {}
    }
    if screen.route == "wifi-password-entry" {
        boxes.push(reflow("keyboard", 64, 300, 896, 320));
    }
    if screen.modal.is_some() {
        boxes.push(reflow("modal", 160, 108, 704, 552));
    }
    if let Some(focused_action) = &focused_action {
        boxes.push(focused_action.clone());
    }
    LayoutGeometry {
        viewport_width,
        viewport_height,
        boxes,
        focused_action,
        visible_menu_items,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub schema: &'static str,
    pub identity: &'static str,
    pub route: String,
    pub ui_size: UiSize,
    pub focus: String,
    pub title: String,
    pub selected_label: String,
    pub menu: Vec<ScreenItem>,
    pub game_rows: Vec<ScreenItem>,
    pub resume: Vec<ui_model::ResumeProjection>,
    pub selected_game: Option<GameDetails>,
    pub game_media: Vec<ScreenMedia>,
    pub system_media: Option<ScreenMedia>,
    pub regions: Vec<ScreenRegion>,
    pub palette: Palette,
    pub visual_profile: VisualProfile,
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
    pub group_jump: ui_model::GroupJumpState,
    pub index: IndexView,
    pub boundaries: BoundaryViews,
    pub save_sync: Option<SaveSyncView>,
}

pub fn build(
    state: &UiState,
    theme: &ValidatedTheme,
    theme_fallback: Option<ThemeReason>,
    settings: Option<&settings_ui::Scene>,
    wifi: Option<&wifi_settings_controller::Snapshot>,
) -> Screen {
    build_with_index(
        state,
        theme,
        theme_fallback,
        settings,
        wifi,
        &IndexView::default(),
    )
}

pub fn build_with_index(
    state: &UiState,
    theme: &ValidatedTheme,
    theme_fallback: Option<ThemeReason>,
    settings: Option<&settings_ui::Scene>,
    wifi: Option<&wifi_settings_controller::Snapshot>,
    index: &IndexView,
) -> Screen {
    build_with_recent(state, theme, theme_fallback, settings, wifi, index, &[])
}

pub fn build_with_recent(
    state: &UiState,
    theme: &ValidatedTheme,
    theme_fallback: Option<ThemeReason>,
    settings: Option<&settings_ui::Scene>,
    wifi: Option<&wifi_settings_controller::Snapshot>,
    index: &IndexView,
    recent: &[String],
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
            !matches!(state.route, Route::Recent) || recent.iter().any(|id| id == &game.id.0)
        })
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
    let visual_profile = state.preferences.visual_profile(state.visual_clock_ms);
    let theme_data = theme.theme();
    let palette = visual_palette(
        Palette {
            background: color(&theme_data.colors.background),
            surface: color(&theme_data.colors.surface),
            accent: color(&theme_data.colors.accent),
            text: color(&theme_data.colors.text),
            muted: color(&theme_data.colors.muted),
            highlight: color(&theme_data.colors.highlight),
        },
        &visual_profile,
    );
    let scene = theme_scene(theme);
    let game_media = state
        .games
        .iter()
        .flat_map(|game| media_for_content(&game.id.0))
        .collect();
    let system_media = state
        .selected_system
        .as_ref()
        .and_then(|system| media_for_system(&system.0));
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
        ui_size: state.preview_ui_size.unwrap_or(state.preferences.ui_size),
        focus: focus_name(&state.focus),
        title: title(&state.route),
        selected_label: state
            .menu
            .selected()
            .map(|entry| entry.label.clone())
            .unwrap_or_default(),
        menu,
        game_rows,
        resume: state.resume_entries.clone(),
        selected_game,
        game_media,
        system_media,
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
                visible: visual_profile.status_bar_visible
                    || !matches!(
                        region.kind,
                        launcher_theme::RegionKind::Clock | launcher_theme::RegionKind::Battery
                    ),
            })
            .collect(),
        palette,
        visual_profile,
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
            charging_status: state.affordances.battery.charging_status.clone(),
            external_power: state.affordances.battery.external_power,
            battery_health: state.affordances.battery.health.clone(),
            battery_level: state.affordances.battery.level.clone(),
            show_charging_status: state.affordances.battery.show_charging_status,
        },
        controller_help: state.controller_help.bindings.clone(),
        group_jump: state.group_jump.clone(),
        index: index.clone(),
        boundaries: BoundaryViews {
            metadata_scraper: BoundaryStatus {
                available: true,
                state: "fixture-contract".into(),
            },
            media_cache: BoundaryStatus {
                available: true,
                state: "bounded-cache".into(),
            },
            wifi_controller: BoundaryStatus {
                available: wifi.is_some(),
                state: "controller-contract".into(),
            },
            theme_garden: BoundaryStatus {
                available: true,
                state: "built-in-preview".into(),
            },
            update_agent: BoundaryStatus {
                available: false,
                state: "unavailable".into(),
            },
        },
        save_sync: None,
    }
}

fn visual_palette(mut palette: Palette, profile: &VisualProfile) -> Palette {
    let brightness = profile
        .requested_brightness_percent
        .max(profile.brightness_floor_percent);
    for color in [
        &mut palette.background,
        &mut palette.surface,
        &mut palette.accent,
        &mut palette.text,
        &mut palette.muted,
        &mut palette.highlight,
    ] {
        if profile.preset == VisualPreset::NightWarm {
            color[0] = color[0].saturating_add(16);
            color[2] = color[2].saturating_sub(24);
        }
        for channel in &mut color[..3] {
            *channel = (*channel as u16 * u16::from(brightness) / 100) as u8;
        }
    }
    palette
}

pub fn media_for_content(content_id: &str) -> Vec<ScreenMedia> {
    let (cover, screenshot) = match content_id {
        "generated-game-01" | "nebula-nes" => (
            (
                "themes/media/games/nebula-cover.png",
                include_bytes!("../../../themes/media/games/nebula-cover.png").as_slice(),
                1024,
                1536,
            ),
            (
                "themes/media/games/nebula-screen.png",
                include_bytes!("../../../themes/media/games/nebula-screen.png").as_slice(),
                1536,
                1024,
            ),
        ),
        "generated-game-02" | "mirror-ps1" => (
            (
                "themes/media/games/mirror-cover.png",
                include_bytes!("../../../themes/media/games/mirror-cover.png").as_slice(),
                420,
                560,
            ),
            (
                "themes/media/games/mirror-screen.png",
                include_bytes!("../../../themes/media/games/mirror-screen.png").as_slice(),
                640,
                360,
            ),
        ),
        "generated-game-03" | "orbit-garden" => (
            (
                "themes/media/games/orbit-cover.png",
                include_bytes!("../../../themes/media/games/orbit-cover.png").as_slice(),
                420,
                560,
            ),
            (
                "themes/media/games/orbit-screen.png",
                include_bytes!("../../../themes/media/games/orbit-screen.png").as_slice(),
                640,
                360,
            ),
        ),
        "signal-workshop" => (
            (
                "themes/media/games/signal-cover.png",
                include_bytes!("../../../themes/media/games/signal-cover.png").as_slice(),
                420,
                560,
            ),
            (
                "themes/media/games/signal-screen.png",
                include_bytes!("../../../themes/media/games/signal-screen.png").as_slice(),
                640,
                360,
            ),
        ),
        _ => return Vec::new(),
    };
    vec![
        ScreenMedia {
            content_id: content_id.into(),
            kind: "box-art".into(),
            path: cover.0.into(),
            width: cover.2,
            height: cover.3,
            pixels: cover.1.to_vec(),
        },
        ScreenMedia {
            content_id: content_id.into(),
            kind: "screenshot".into(),
            path: screenshot.0.into(),
            width: screenshot.2,
            height: screenshot.3,
            pixels: screenshot.1.to_vec(),
        },
    ]
}

pub fn media_for_system(system_id: &str) -> Option<ScreenMedia> {
    let (path, pixels) = match system_id {
        "generated-system-alpha" | "nes" | "ps1" => (
            "themes/media/systems/nova8.png",
            include_bytes!("../../../themes/media/systems/nova8.png").as_slice(),
        ),
        "generated-system-beta" | "portmaster" => (
            "themes/media/systems/luma.png",
            include_bytes!("../../../themes/media/systems/luma.png").as_slice(),
        ),
        _ => return None,
    };
    Some(ScreenMedia {
        content_id: system_id.into(),
        kind: "system-art".into(),
        path: path.into(),
        width: 640,
        height: 400,
        pixels: pixels.to_vec(),
    })
}

pub fn catalog_game_details(content_id: &str, title: &str, _system: &str) -> Option<GameDetails> {
    let (description, rating, release_date) = match content_id {
        "nebula-nes" => (
            "Chart a quiet starship through forgotten constellations.",
            Some(92),
            Some("1994-04-12"),
        ),
        "mirror-ps1" => (
            "Restore a living gallery where every reflection hides a room.",
            Some(88),
            Some("1998-09-21"),
        ),
        "orbit-garden" => (
            "Cultivate tidal gardens on a drifting orbital station.",
            Some(86),
            Some("2001-06-17"),
        ),
        "signal-workshop" => (
            "Tune a pocket laboratory of switches, waves, and light.",
            Some(90),
            Some("2003-02-08"),
        ),
        _ => return None,
    };
    Some(GameDetails {
        id: content_id.into(),
        title: title.into(),
        description: description.into(),
        rating,
        release_date: release_date.map(str::to_owned),
        box_art: GeneratedAssetRef {
            id: format!("media-{content_id}-cover"),
            kind: AssetKind::BoxArt,
            alt_text: format!("{title} original cover"),
        },
        screenshot: GeneratedAssetRef {
            id: format!("media-{content_id}-screen"),
            kind: AssetKind::Screenshot,
            alt_text: format!("{title} gameplay screenshot"),
        },
        favorite: false,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_with_sync(
    state: &UiState,
    theme: &ValidatedTheme,
    theme_fallback: Option<ThemeReason>,
    settings: Option<&settings_ui::Scene>,
    wifi: Option<&wifi_settings_controller::Snapshot>,
    index: &IndexView,
    recent: &[String],
    save_sync: Option<&SaveSyncView>,
) -> Screen {
    let mut screen = build_with_recent(state, theme, theme_fallback, settings, wifi, index, recent);
    screen.save_sync = save_sync.cloned();
    screen
}

fn settings_view(scene: &settings_ui::Scene) -> SettingsView {
    let surface = scene.surface;
    SettingsView {
        width: scene.width,
        height: scene.height,
        surface,
        selected_section_id: scene.selected_section_id.clone(),
        sections: scene
            .sections
            .iter()
            .filter(|section| {
                scene
                    .form
                    .as_ref()
                    .is_none_or(|form| form.section_id == section.id)
            })
            .map(|section| SettingsSection {
                id: section.id.clone(),
                label_key: section.label_key.clone(),
                controls: if surface == settings_ui::Surface::SectionList {
                    Vec::new()
                } else {
                    section
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
                        .collect()
                },
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
        Route::Recent => "recent".into(),
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
        Route::Home => "NOVA/8 LIBRARY".into(),
        Route::Systems => "SYSTEM SELECT".into(),
        Route::Games => "GAME LIBRARY".into(),
        Route::Search => "Search".into(),
        Route::Favorites => "Favorites".into(),
        Route::Recent => "Recent".into(),
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
    let progress = scraper.progress.as_ref();
    ScraperView {
        route: format_debug(&scraper.route),
        status: format_debug(&scraper.status),
        queue_count: scraper.queue.len(),
        completed: progress.map_or(0, |value| value.completed),
        total: progress.map_or(0, |value| value.total),
        progress_percent: progress.map(|value| value.percent),
        configured_slots: progress.map_or(0, |value| value.configured_slots),
        paused: progress.is_some_and(|value| value.paused),
        paused_reason: progress.and_then(|value| value.paused_reason.clone()),
        background: progress.is_some_and(|value| value.background),
        counts: progress.map_or_else(ui_model::ScraperCounts::default, |value| {
            value.counts.clone()
        }),
        rows: progress.map_or_else(Vec::new, |value| value.rows.clone()),
        actions: vec![
            "hide".into(),
            "pause".into(),
            "resume".into(),
            "cancel".into(),
            "close".into(),
            "inspect-results".into(),
        ],
        cancel_requested: scraper.cancel_requested,
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
    asset.id.starts_with("generated-") || asset.id == "nova8-splash"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_game_media_has_real_role_paths() {
        let media = media_for_content("nebula-nes");
        assert_eq!(media.len(), 2);
        assert_ne!(media[0].path, media[1].path);
        assert!(media[0].path.contains("games/nebula-cover.png"));
        assert!(media[1].path.contains("games/nebula-screen.png"));
        assert_eq!((media[0].width, media[0].height), (1024, 1536));
        assert_eq!((media[1].width, media[1].height), (1536, 1024));
    }

    #[test]
    fn low_brightness_palette_is_dimmed() {
        let preferences = ui_model::UiPreferences {
            visual_preset: VisualPreset::LowBrightness,
            ..Default::default()
        };
        let palette = visual_palette(
            Palette {
                background: [100, 100, 100, 255],
                surface: [100, 100, 100, 255],
                accent: [100, 100, 100, 255],
                text: [100, 100, 100, 255],
                muted: [100, 100, 100, 255],
                highlight: [100, 100, 100, 255],
            },
            &preferences.visual_profile(0),
        );
        assert_eq!(palette.background, [20, 20, 20, 255]);
    }

    #[test]
    fn authored_home_and_system_identity_are_not_debug_labels() {
        let state = ui_model::UiState::generated();
        assert!(state
            .systems
            .iter()
            .any(|system| system.name == "NOVA/8 HANDHELD"));
        assert!(state.games.iter().any(|game| game.title == "Nebula Notes"));
        assert!(!state
            .games
            .iter()
            .any(|game| game.title.starts_with("Generated ")));
    }
}
