use std::{fs, path::PathBuf, process};

use launcher_presentation::build;
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
    let theme = safe_artbook()?;
    let settings = settings()?;
    let mut wifi = wifi()?;
    wifi.scan()?;
    let wifi_snapshot = wifi.snapshot();

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
                    query: "Generated".into(),
                },
            ),
        ),
        ("settings", reduce(&base, Action::Navigate(Route::Settings))),
        (
            "recovery",
            reduce(
                &base,
                Action::ShowFallback {
                    reason: FallbackReason::MissingContent,
                },
            ),
        ),
        (
            "scraper-progress",
            reduce(&base, Action::Scraper(ScraperAction::OpenBulkQueue)),
        ),
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
        let mut platform = HostPlatform::new(&profile, Backend::Dummy)?;
        for (name, state) in &states {
            let screen = build(
                state,
                &theme,
                (*name == "recovery").then_some(launcher_theme::Reason::MissingTheme),
                Some(&settings),
                Some(&wifi_snapshot),
            );
            assert_contract(&screen, name)?;
            if *name == "splash" && screen.splash != "artbook-generated-splash" {
                return Err("splash semantic state is missing".into());
            }
            if *name == "recovery"
                && (screen.theme_fallback.is_none()
                    || screen.splash != "artbook-generated-fallback")
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
    println!("evidence={}", root.display());
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
    };
    reduce(state, action)
}

fn reduce(state: &UiState, action: Action) -> UiState {
    ui_model::reduce(state, action)
}

fn settings() -> Result<settings_ui::Scene, Box<dyn std::error::Error>> {
    let registry = Registry::from_json(REGISTRY)?;
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

fn wifi() -> Result<WifiSettingsController, Box<dyn std::error::Error>> {
    let metadata = WifiMetadata::from_json(WIFI_METADATA)?;
    let backend = GeneratedWifiBackend::from_json(WIFI_FIXTURE)?;
    Ok(WifiSettingsController::new(
        metadata,
        WifiManager::new(backend),
        true,
    )?)
}
