use launcher_presentation::{build, Screen};
use launcher_theme::safe_artbook;
use serde_json::{json, Value};
use settings_schema::{ProjectionContext, Registry};
use settings_ui::SettingsUi;
use ui_model::{Action, PlatformCapabilities, Route, UiState};
use wifi_manager::{GeneratedWifiBackend, WifiManager};
use wifi_settings_controller::{Metadata as WifiMetadata, WifiSettingsController};

const REGISTRY: &[u8] = include_bytes!("../../../../fixtures/settings-schema/registry-v1.json");
const WIFI_METADATA: &[u8] =
    include_bytes!("../../../../fixtures/wifi-settings-controller/generated-v1/workflow.json");
const WIFI_FIXTURE: &[u8] = include_bytes!("../../../../fixtures/wifi-manager/journeys.json");

fn main() {
    if let Err(error) = run() {
        eprintln!("aarch64 launcher journey failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let theme = safe_artbook()?;
    let settings = settings()?;
    let mut wifi = wifi()?;
    wifi.scan()?;
    let wifi_snapshot = wifi.snapshot();

    let initial = UiState::generated();
    let home = ui_model::reduce(
        &ui_model::reduce(
            &initial,
            Action::SetCapabilities(PlatformCapabilities {
                catalog: true,
                favorites: true,
                settings_persistence: true,
                session: true,
                scraper: true,
                wifi: true,
            }),
        ),
        Action::FinishSplash,
    );
    let systems = ui_model::reduce(&home, Action::Navigate(Route::Systems));
    let games = ui_model::reduce(&systems, Action::ActivateSelected);
    let selected_games = ui_model::reduce(&games, Action::MoveSelection(ui_model::Direction::Down));
    let states = [
        ("home", home),
        ("systems", systems),
        ("games", games),
        ("selected-games", selected_games),
    ];
    let screens: Vec<Screen> = states
        .iter()
        .map(|(_, state)| build(state, &theme, None, Some(&settings), Some(&wifi_snapshot)))
        .collect();
    if screens.iter().any(|screen| {
        screen.schema != "launcher-presentation/v1"
            || screen.identity != "Artbook"
            || screen.regions.len() < 9
            || screen
                .settings
                .as_ref()
                .is_none_or(|settings| settings.sections.is_empty())
    }) {
        return Err("launcher presentation contract is incomplete".into());
    }
    if screens[0].route != "home"
        || screens[0].selected_label != "Systems"
        || screens[1].route != "systems"
        || screens[2].route != "games"
        || screens[3].route != "games"
        || screens[2].selected_label == screens[3].selected_label
    {
        return Err("launcher route or selection journey did not progress".into());
    }
    let first = serde_json::to_value(&screens[0])?;
    let second = serde_json::to_value(&screens[0])?;
    if first != second {
        return Err("launcher semantic projection is not deterministic".into());
    }
    let result = json!({
        "schema": "trimui-tg4040-aarch64-launcher-journey/v1",
        "result": "pass",
        "entryPoint": "project-authored-launcher-presentation",
        "routes": screens.iter().map(|screen| screen.route.clone()).collect::<Vec<_>>(),
        "screenCount": screens.len(),
        "regionCount": screens.iter().map(|screen| screen.regions.len()).collect::<Vec<_>>(),
        "settingsSections": screens[0].settings.as_ref().map_or(0, |settings| settings.sections.len()),
        "wifiPhase": wifi_snapshot.phase,
        "boundaryStates": boundary_states(&screens[0]),
    });
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
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

fn boundary_states(screen: &Screen) -> Value {
    json!({
        "metadataScraper": screen.boundaries.metadata_scraper.state,
        "mediaCache": screen.boundaries.media_cache.state,
        "wifiController": screen.boundaries.wifi_controller.state,
        "themeGarden": screen.boundaries.theme_garden.state,
        "updateAgent": screen.boundaries.update_agent.state,
    })
}
