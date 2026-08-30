use std::{
    fs,
    path::{Path, PathBuf},
    process,
};

use launcher_presentation::{build_with_sync, SaveSyncCandidateView, SaveSyncView};
use launcher_theme::safe_artbook;
use settings_schema::{ProjectionContext, Registry};
use settings_ui::SettingsUi;
use sim_host_platform::{Backend, HostPlatform};
use sim_platform_contract::Platform;
use ui_model::{
    Action, Button as UiButton, FallbackReason, PlatformCapabilities, Route, ScraperAction,
    UiState, WifiAction,
};
use wifi_manager::{GeneratedWifiBackend, WifiManager};
use wifi_settings_controller::{Metadata as WifiMetadata, WifiSettingsController};

const REGISTRY: &[u8] = include_bytes!("../../../../fixtures/settings-schema/registry-v1.json");
const WIFI_METADATA: &[u8] =
    include_bytes!("../../../../fixtures/wifi-settings-controller/generated-v1/workflow.json");
const WIFI_FIXTURE: &[u8] = include_bytes!("../../../../fixtures/wifi-manager/journeys.json");

fn main() {
    if let Err(error) = run() {
        eprintln!("launcher-presentation-journey: {error}");
        process::exit(1);
    }
    println!("launcher-presentation-journey: deterministic screens passed twice");
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("trimui-presentation-journey-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let profile = PathBuf::from("sim/device/tg4040-host.json");
    let device_profile = PathBuf::from("config/platform/tg4040/compatibility.json");
    let theme = safe_artbook()?;
    let settings = settings()?;
    let mut wifi = wifi()?;
    wifi.scan()?;
    let wifi_snapshot = wifi.snapshot();
    let save_sync = sync_fixture();

    let initial = UiState::generated();
    let splash = initial.clone();
    let mut base = reduce(
        &initial,
        Action::SetCapabilities(PlatformCapabilities {
            catalog: true,
            favorites: true,
            settings_persistence: true,
            session: true,
            scraper: true,
            wifi: true,
        }),
    );
    base = reduce(&base, Action::FinishSplash);
    let systems = reduce(&base, Action::Navigate(Route::Systems));
    let controller_games = controller_press(&systems, UiButton::Primary);
    let controller_game_selection = controller_press(&controller_games, UiButton::Down);
    if systems.route != Route::Systems
        || controller_games.route != Route::Games
        || controller_game_selection.menu.selection.index == controller_games.menu.selection.index
    {
        return Err("controller transition did not change route and selection".into());
    }
    let scraper_progress_2 = reduce(
        &reduce(&base, Action::Scraper(ScraperAction::OpenBulkQueue)),
        Action::Scraper(ScraperAction::SetProgress(bulk_progress(
            2, 0, false, false,
        ))),
    );
    let scraper_progress_4 = reduce(
        &reduce(&base, Action::Scraper(ScraperAction::OpenBulkQueue)),
        Action::Scraper(ScraperAction::SetProgress(bulk_progress(
            4, 1, false, false,
        ))),
    );
    let scraper_paused = reduce(
        &scraper_progress_2,
        Action::Scraper(ScraperAction::PauseForGate {
            reason: "network".into(),
        }),
    );
    let scraper_background = reduce(&scraper_progress_2, Action::Scraper(ScraperAction::Hide));
    let scraper_complete = reduce(
        &scraper_progress_4,
        Action::Scraper(ScraperAction::Complete),
    );

    let states = vec![
        ("splash", splash),
        ("home", base.clone()),
        ("systems", systems),
        ("controller-games", controller_games),
        ("controller-game-selection", controller_game_selection),
        ("games", reduce(&base, Action::Navigate(Route::Games))),
        (
            "favorites",
            reduce(&base, Action::Navigate(Route::Favorites)),
        ),
        (
            "search",
            reduce(
                &base,
                Action::SetSearchQuery {
                    query: "Nebula".into(),
                },
            ),
        ),
        ("settings", reduce(&base, Action::Navigate(Route::Settings))),
        (
            "game-switcher",
            reduce(&base, Action::Navigate(Route::GameSwitcher)),
        ),
        ("theme-garden", base.clone()),
        ("save-vault", base.clone()),
        ("save-sync", base.clone()),
        (
            "keyboard",
            reduce(
                &reduce(
                    &base,
                    Action::Navigate(Route::Wifi(ui_model::WifiRoute::PasswordEntry)),
                ),
                Action::Wifi(WifiAction::RequestMaskedPasswordKeyboard {
                    ssid: "Fixture Network".into(),
                }),
            ),
        ),
        ("portmaster", base.clone()),
        ("controller-help", base.clone()),
        (
            "overlay",
            reduce(
                &base,
                Action::ShowModal(ui_model::ModalState::Info {
                    title: "Overlay".into(),
                    message: "Focused action remains visible".into(),
                }),
            ),
        ),
        (
            "recovery",
            reduce(
                &base,
                Action::ShowFallback {
                    reason: FallbackReason::MissingContent,
                },
            ),
        ),
        ("scraper-progress-2", scraper_progress_2),
        ("scraper-progress-4", scraper_progress_4),
        ("scraper-paused", scraper_paused),
        ("scraper-background", scraper_background),
        ("scraper-complete", scraper_complete),
        (
            "scraper-ambiguity",
            reduce(
                &base,
                Action::Scraper(ScraperAction::OpenAmbiguousChoice(
                    ui_model::AmbiguousChoice {
                        game_id: ui_model::GameId::new("generated-game-01"),
                        candidates: vec!["Generated Match A".into(), "Generated Match B".into()],
                    },
                )),
            ),
        ),
        (
            "wifi",
            reduce(
                &base,
                Action::Wifi(WifiAction::SetAccessPoints {
                    access_points: vec![],
                }),
            ),
        ),
    ];

    for pass in 0..2 {
        let mut platform = HostPlatform::new(&profile, &device_profile, Backend::Dummy)?;
        for (name, state) in &states {
            let mut screen = build_with_sync(
                state,
                &theme,
                (*name == "recovery").then_some(launcher_theme::Reason::MissingTheme),
                Some(&settings),
                Some(&wifi_snapshot),
                &launcher_presentation::IndexView::default(),
                &[],
                Some(&save_sync),
            );
            set_surface_route(&mut screen, name);
            assert_contract(&screen, name)?;
            if screen.ui_size != ui_model::UiSize::Automatic {
                return Err("presentation did not carry the selected UI size".into());
            }
            if *name == "scraper-progress-2"
                && (screen.scraper.progress_percent != Some(0)
                    || screen.scraper.configured_slots != 2
                    || screen.scraper.rows.len() != 2
                    || screen.scraper.total != 4)
            {
                return Err("two-slot scraper popup projection is incomplete".into());
            }
            if *name == "scraper-progress-4"
                && (screen.scraper.configured_slots != 4 || screen.scraper.rows.len() != 4)
            {
                return Err("four-slot scraper popup projection is incomplete".into());
            }
            if *name == "scraper-paused"
                && (!screen.scraper.paused
                    || screen.scraper.paused_reason.as_deref() != Some("network"))
            {
                return Err("paused gate reason is missing from scraper popup".into());
            }
            if *name == "scraper-complete" && screen.scraper.progress_percent != Some(100) {
                return Err("completed scraper popup did not reach 100 percent".into());
            }
            let sync = screen.save_sync.as_ref().ok_or("save sync view missing")?;
            if sync.local.device_name != "Brick A"
                || sync.remote.device_name != "Brick B"
                || sync.local.hash_prefix.len() != 12
                || sync.remote.hash_prefix.len() != 12
                || sync.local.size != 18
                || sync.remote.size != 19
                || sync.actions != ["keep-local", "keep-remote", "keep-both"]
            {
                return Err("save sync presentation contract is incomplete".into());
            }
            if *name == "splash" && screen.splash != "nova8-splash" {
                return Err("splash semantic state is missing".into());
            }
            if *name == "recovery"
                && (screen.theme_fallback.is_none() || screen.splash != "nova8-fallback")
            {
                return Err("fallback semantic state is incomplete".into());
            }
            let semantic = serde_json::to_vec(&screen)?;
            if semantic.windows(5).any(|bytes| bytes == b"/srv/")
                || semantic.windows(12).any(|bytes| bytes == b"secret-value")
                || semantic
                    .windows(16)
                    .any(|bytes| bytes == b"credential-value")
            {
                return Err("semantic artifact contains forbidden private data".into());
            }
            let path = root.join(format!("{name}-{pass}.png"));
            let json_path = root.join(format!("{name}-{pass}.json"));
            platform.present(&screen)?;
            platform.capture_png(&path)?;
            let decoder = png::Decoder::new(fs::File::open(&path)?);
            let reader = decoder.read_info()?;
            if reader.info().width != 1024 || reader.info().height != 768 {
                return Err("PNG dimensions are not 1024x768".into());
            }
            fs::write(json_path, semantic)?;
        }
    }

    for (name, _) in &states {
        let first: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(format!("{name}-0.json")))?)?;
        let second: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(format!("{name}-1.json")))?)?;
        if first != second {
            return Err(format!("normalized semantic artifacts are not stable for {name}").into());
        }
    }
    if fs::read(root.join("splash-0.png"))? == fs::read(root.join("home-0.png"))?
        || fs::read(root.join("recovery-0.png"))? == fs::read(root.join("home-0.png"))?
    {
        return Err("splash or fallback visual is indistinguishable from home".into());
    }
    run_layout_matrix(
        &root,
        &profile,
        &theme,
        &settings,
        &wifi_snapshot,
        &save_sync,
        &states,
    )?;
    println!("evidence={}", root.display());
    Ok(())
}

fn run_layout_matrix(
    root: &Path,
    profile: &Path,
    theme: &launcher_theme::ValidatedTheme,
    settings: &settings_ui::Scene,
    wifi: &wifi_settings_controller::Snapshot,
    save_sync: &SaveSyncView,
    states: &[(&str, UiState)],
) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = [
        (
            "fallback",
            PathBuf::from("config/platform/tg4040/compatibility.json"),
            (1024, 768),
        ),
        (
            "dense-4",
            PathBuf::from("fixtures/platform/ui-density-1024-4/compatibility.json"),
            (1024, 768),
        ),
        (
            "wide-7",
            PathBuf::from("fixtures/platform/ui-density-1024-7/compatibility.json"),
            (1024, 768),
        ),
        (
            "small-3-5",
            PathBuf::from("fixtures/platform/ui-density-640-3-5/compatibility.json"),
            (640, 480),
        ),
        (
            "active-mm",
            PathBuf::from("fixtures/platform/ui-density-active-mm/compatibility.json"),
            (1024, 768),
        ),
    ];
    let sizes = [
        ui_model::UiSize::Automatic,
        ui_model::UiSize::Compact,
        ui_model::UiSize::Comfortable,
        ui_model::UiSize::Large,
        ui_model::UiSize::ExtraLarge,
    ];
    for pass in 0..2 {
        for (profile_name, device_profile, dimensions) in &profiles {
            let mut platform = HostPlatform::new(profile, device_profile, Backend::Dummy)?;
            let automatic_scale_percent = platform.automatic_scale_percent();
            for size in sizes {
                for (route_name, state) in states {
                    let mut sized = state.clone();
                    sized.preferences.ui_size = size;
                    sized.preview_ui_size = None;
                    let mut screen = launcher_presentation::build_with_sync(
                        &sized,
                        theme,
                        None,
                        Some(settings),
                        Some(wifi),
                        &launcher_presentation::IndexView::default(),
                        &[],
                        Some(save_sync),
                    );
                    set_surface_route(&mut screen, route_name);
                    assert_layout_contract(
                        &screen,
                        *dimensions,
                        automatic_scale_percent,
                        route_name,
                        size,
                    )?;
                    let stem = format!("layout-{profile_name}-{size:?}-{route_name}-{pass}");
                    let png = root.join(format!("{stem}.png"));
                    platform.present(&screen)?;
                    platform.capture_png(&png)?;
                    let reader = png::Decoder::new(fs::File::open(&png)?).read_info()?;
                    if reader.info().width != dimensions.0 || reader.info().height != dimensions.1 {
                        return Err(format!("{stem}: PNG dimensions are not {dimensions:?}").into());
                    }
                    let semantic = serde_json::to_vec(&screen)?;
                    let semantic_path = root.join(format!("{stem}.json"));
                    fs::write(&semantic_path, semantic)?;
                    if pass == 1 {
                        let prior = root.join(format!(
                            "layout-{profile_name}-{size:?}-{route_name}-0.json"
                        ));
                        if fs::read(prior)? != fs::read(&semantic_path)? {
                            return Err(
                                format!("{stem}: semantic output was not deterministic").into()
                            );
                        }
                        let prior_png =
                            root.join(format!("layout-{profile_name}-{size:?}-{route_name}-0.png"));
                        if fs::read(prior_png)? != fs::read(&png)? {
                            return Err(format!("{stem}: PNG output was not deterministic").into());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn assert_contract(
    screen: &launcher_presentation::Screen,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if screen.regions.len() < 9
        || screen.identity != "Artbook"
        || screen.schema != "launcher-presentation/v1"
    {
        return Err(format!("presentation contract is incomplete for {name}").into());
    }
    Ok(())
}

fn assert_layout_contract(
    screen: &launcher_presentation::Screen,
    dimensions: (u32, u32),
    automatic_scale_percent: u16,
    route_name: &str,
    size: ui_model::UiSize,
) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = dimensions;
    let geometry =
        launcher_presentation::layout_geometry(screen, width, height, automatic_scale_percent);
    for layout_box in &geometry.boxes {
        if layout_box.x + layout_box.width > width
            || layout_box.y + layout_box.height > height
            || (dimensions == (640, 480) && (layout_box.width == 0 || layout_box.height == 0))
        {
            return Err(format!(
                "{route_name}/{size:?}: {} is outside usable {dimensions:?}",
                layout_box.id
            )
            .into());
        }
    }
    if screen.ui_size != size {
        return Err(format!("{route_name}/{size:?}: UI size was not carried").into());
    }
    let items = if screen.focus == "game-list" {
        &screen.game_rows
    } else {
        &screen.menu
    };
    if screen.splash != "nova8-splash"
        && !items.is_empty()
        && (!items.iter().any(|item| item.selected && item.enabled)
            || geometry
                .focused_action
                .as_ref()
                .is_none_or(|layout_box| layout_box.width == 0 || layout_box.height == 0))
    {
        return Err(format!(
            "{route_name}/{size:?}: focused action is not visible (focus={}, label={:?}, items={})",
            screen.focus,
            screen.selected_label,
            items.len()
        )
        .into());
    }
    Ok(())
}

fn set_surface_route(screen: &mut launcher_presentation::Screen, name: &str) {
    screen.route = match name {
        "theme-garden" | "save-vault" | "save-sync" | "portmaster" | "controller-help" => {
            name.into()
        }
        "keyboard" => "wifi-password-entry".into(),
        _ => screen.route.clone(),
    };
}

fn controller_press(state: &UiState, button: UiButton) -> UiState {
    let action = match button {
        UiButton::Up => Action::MoveSelection(ui_model::Direction::Up),
        UiButton::Down => Action::MoveSelection(ui_model::Direction::Down),
        UiButton::Left => Action::MoveSelection(ui_model::Direction::Left),
        UiButton::Right => Action::MoveSelection(ui_model::Direction::Right),
        UiButton::Primary => Action::ActivateSelected,
        UiButton::Secondary => Action::Back,
        UiButton::Start => Action::Navigate(Route::Home),
        UiButton::Select | UiButton::Menu => Action::SetFocus(ui_model::FocusTarget::Menu),
        UiButton::L1 | UiButton::R1 => Action::SetGroupJump(ui_model::GroupJumpState::default()),
    };
    reduce(state, action)
}

fn reduce(state: &UiState, action: Action) -> UiState {
    ui_model::reduce(state, action)
}

fn bulk_progress(
    slots: u8,
    completed: u16,
    paused: bool,
    background: bool,
) -> ui_model::ScraperProgress {
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
        paused,
        paused_reason: paused.then_some("network".into()),
        background,
        counts: ui_model::ScraperCounts {
            succeeded: completed,
            ..Default::default()
        },
        rows,
    }
}

fn settings() -> Result<settings_ui::Scene, Box<dyn std::error::Error>> {
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
    let registry = Registry::from_json(REGISTRY)?.with_provider_metadata(&providers)?;
    let mut context = ProjectionContext::default();
    context.capabilities.extend([
        "audio".into(),
        "network".into(),
        "theme-engine".into(),
        "wifi".into(),
        "scraper".into(),
    ]);
    let ui = SettingsUi::new(registry, context)?;
    Ok(ui.scene()?)
}

fn sync_fixture() -> SaveSyncView {
    let candidate =
        |device_id: &str, device_name: &str, hash_prefix: &str, size: u64| SaveSyncCandidateView {
            logical_id: "generated-save".into(),
            content_id: "generated-content".into(),
            device_id: device_id.into(),
            device_name: device_name.into(),
            generation: 1,
            hash_prefix: hash_prefix.into(),
            parent_hash_prefix: Some("0123456789ab".into()),
            ancestry: vec!["0123456789ab".into()],
            save_kind: "save".into(),
            timestamp_ms: 1,
            size,
            status: "conflict".into(),
            deleted: false,
        };
    SaveSyncView {
        local: candidate("brick-a", "Brick A", "0123456789ab", 18),
        remote: candidate("brick-b", "Brick B", "abcdef012345", 19),
        state: "conflict".into(),
        transport_outcome: "quarantined".into(),
        actions: vec![
            "keep-local".into(),
            "keep-remote".into(),
            "keep-both".into(),
        ],
    }
}

fn wifi() -> Result<WifiSettingsController, Box<dyn std::error::Error>> {
    let metadata = WifiMetadata::from_json(WIFI_METADATA)?;
    let backend = GeneratedWifiBackend::from_json(WIFI_FIXTURE)?;
    Ok(WifiSettingsController::new(
        metadata,
        WifiManager::new(backend),
        true,
    )?)
}
