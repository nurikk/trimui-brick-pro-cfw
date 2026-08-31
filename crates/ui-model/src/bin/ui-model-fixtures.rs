use ui_model::{
    reduce, Action, AmbiguousChoice, GameId, MenuCommand, ModalState, NightSchedule,
    PlatformCapabilities, PreferenceChange, Route, ScraperAction, SsidEntryMode, UiSize, UiState,
    VisualPreset, WifiAction, WifiRoute, WifiStatus, MAX_INPUT_CHARS,
};

const FIXTURES: &[(&str, &str); 14] = &[
    (
        "artbook-4x3.json",
        include_str!("../../../../fixtures/ui-model/artbook-4x3.json"),
    ),
    (
        "controller-help.json",
        include_str!("../../../../fixtures/ui-model/controller-help.json"),
    ),
    (
        "favorite-toggle.json",
        include_str!("../../../../fixtures/ui-model/favorite-toggle.json"),
    ),
    (
        "modal-handling.json",
        include_str!("../../../../fixtures/ui-model/modal-handling.json"),
    ),
    (
        "navigation.json",
        include_str!("../../../../fixtures/ui-model/navigation.json"),
    ),
    (
        "preferences.json",
        include_str!("../../../../fixtures/ui-model/preferences.json"),
    ),
    (
        "scraper-contract.json",
        include_str!("../../../../fixtures/ui-model/scraper-contract.json"),
    ),
    (
        "search-query.json",
        include_str!("../../../../fixtures/ui-model/search-query.json"),
    ),
    (
        "settings-edits.json",
        include_str!("../../../../fixtures/ui-model/settings-edits.json"),
    ),
    (
        "snapshot-home.json",
        include_str!("../../../../fixtures/ui-model/snapshot-home.json"),
    ),
    (
        "snapshot-recovery.json",
        include_str!("../../../../fixtures/ui-model/snapshot-recovery.json"),
    ),
    (
        "splash-fallback.json",
        include_str!("../../../../fixtures/ui-model/splash-fallback.json"),
    ),
    (
        "unavailable-capability.json",
        include_str!("../../../../fixtures/ui-model/unavailable-capability.json"),
    ),
    (
        "wifi-contract.json",
        include_str!("../../../../fixtures/ui-model/wifi-contract.json"),
    ),
];

fn check_fixtures() -> Result<(), String> {
    for (name, source) in FIXTURES {
        let value: serde_json::Value =
            serde_json::from_str(source).map_err(|error| format!("{name}: {error}"))?;
        let first = serde_json::to_vec(&value).map_err(|error| format!("{name}: {error}"))?;
        let second = serde_json::to_vec(&value).map_err(|error| format!("{name}: {error}"))?;
        if first != second {
            return Err(format!("{name}: repeated serialization changed bytes"));
        }
    }
    Ok(())
}

fn check_journey() {
    let state = UiState::generated();
    assert_eq!(state.identity, "Artbook");
    assert_eq!(
        state.layout.aspect_ratio,
        ui_model::AspectRatio::FourByThree
    );
    assert_eq!(
        1024u32 * state.layout.logical_height as u32,
        768u32 * state.layout.logical_width as u32
    );

    let state = reduce(&state, Action::Navigate(Route::Systems));
    let state = reduce(&state, Action::ActivateSelected);
    assert_eq!(state.route, Route::Games);
    assert_eq!(
        state.selected_system.as_ref().map(|id| id.0.as_str()),
        Some("generated-system-alpha")
    );

    let disabled = reduce(
        &UiState::generated(),
        Action::SetCapabilities(PlatformCapabilities {
            favorites: false,
            ..Default::default()
        }),
    );
    let favorites = disabled
        .menu
        .entries
        .iter()
        .find(|entry| entry.id.0 == "favorites")
        .unwrap();
    assert!(!favorites.enabled);
    assert_eq!(
        favorites.disabled_reason,
        Some(ui_model::Capability::Favorites)
    );

    let state = reduce(
        &state,
        Action::ToggleFavorite {
            game_id: GameId::new("generated-game-01"),
        },
    );
    assert!(
        state
            .games
            .iter()
            .find(|game| game.id.0 == "generated-game-01")
            .unwrap()
            .favorite
    );
    let state = reduce(
        &state,
        Action::SetSearchQuery {
            query: "Nebula".into(),
        },
    );
    assert_eq!(state.route, Route::Search);
    assert_eq!(state.menu.entries.len(), 1);
    let state = reduce(
        &state,
        Action::SetPreference(PreferenceChange::UiSize(UiSize::Large)),
    );
    assert_eq!(state.preferences.ui_size, UiSize::Large);
    let state = reduce(
        &state,
        Action::SetPreference(PreferenceChange::VisualPreset(VisualPreset::DenseList)),
    );
    let dense = state.preferences.visual_profile(0);
    assert_eq!(dense.brightness_floor_percent, 20);
    assert_eq!(dense.list_density, ui_model::ListDensity::Dense);
    assert!(!dense.status_bar_visible);
    let scheduled = reduce(
        &reduce(
            &state,
            Action::SetPreference(PreferenceChange::VisualPreset(VisualPreset::NightWarm)),
        ),
        Action::SetPreference(PreferenceChange::NightSchedule(NightSchedule::LocalTime)),
    );
    let noon = 12 * 60 * 60 * 1000;
    assert_eq!(
        reduce(
            &scheduled,
            Action::SetVisualClock {
                wall_clock_ms: noon,
            },
        )
        .preferences
        .visual_profile(noon)
        .preset,
        VisualPreset::Default
    );
    assert_eq!(
        scheduled
            .preferences
            .visual_profile(22 * 60 * 60 * 1000)
            .preset,
        VisualPreset::NightWarm
    );
    let restored: ui_model::UiPreferences = serde_json::from_slice(
        &serde_json::to_vec(&scheduled.preferences).expect("serialize visual preferences"),
    )
    .expect("restore visual preferences");
    assert_eq!(restored, scheduled.preferences);
    assert_eq!(
        reduce(
            &scheduled,
            Action::SetPreference(PreferenceChange::VisualPreset(VisualPreset::Default)),
        )
        .preferences
        .visual_profile(22 * 60 * 60 * 1000)
        .preset,
        VisualPreset::Default
    );

    let state = reduce(
        &state,
        Action::ShowModal(ModalState::Confirm {
            title: "Confirm".into(),
            message: "Open settings".into(),
            command: MenuCommand::Navigate(Route::Settings),
        }),
    );
    let state = reduce(&state, Action::ConfirmModal);
    assert_eq!(state.route, Route::Settings);
    assert!(state.modal.is_none());

    let state = reduce(
        &state,
        Action::ShowFallback {
            reason: ui_model::FallbackReason::MissingContent,
        },
    );
    assert_eq!(state.route, Route::Recovery);

    let mut state = UiState::generated();
    state.capabilities.wifi = true;
    let state = reduce(
        &state,
        Action::Wifi(WifiAction::EnterSsid {
            mode: SsidEntryMode::Manual,
            ssid: "x".repeat(MAX_INPUT_CHARS + 1),
        }),
    );
    assert_eq!(state.route, Route::Wifi(WifiRoute::PasswordEntry));
    assert_eq!(
        state.wifi.selected_ssid.as_ref().unwrap().len(),
        MAX_INPUT_CHARS
    );
    assert_eq!(state.wifi.status, WifiStatus::AwaitingPassword);
    assert!(state.wifi.keyboard_request.is_none());
    let state = reduce(
        &state,
        Action::Wifi(WifiAction::RequestMaskedPasswordKeyboard {
            ssid: "generated-manual-ssid".into(),
        }),
    );
    assert!(state.modal.is_some());

    let mut state = UiState::generated();
    state.capabilities.scraper = true;
    let state = reduce(
        &state,
        Action::Scraper(ScraperAction::OpenAmbiguousChoice(AmbiguousChoice {
            game_id: GameId::new("generated-game-01"),
            candidates: vec![
                "generated-candidate-a".into(),
                "generated-candidate-b".into(),
            ],
        })),
    );
    let state = reduce(
        &state,
        Action::Scraper(ScraperAction::SelectAmbiguousCandidate { index: 1 }),
    );
    assert_eq!(
        state.scraper.selected_candidate.as_deref(),
        Some("generated-candidate-b")
    );

    let progress = ui_model::ScraperProgress {
        completed: 0,
        total: 4,
        percent: 0,
        configured_slots: 2,
        paused: false,
        paused_reason: None,
        background: false,
        counts: ui_model::ScraperCounts::default(),
        rows: vec![
            ui_model::ScraperRow {
                game_id: GameId::new("generated-game-01"),
                title: "Nebula Notes".into(),
                provider: Some("fixture-secondary".into()),
                phase: ui_model::ScraperPhase::FallingBack,
                fallback_transition: Some("fixture-primary: not found → fixture-secondary".into()),
            },
            ui_model::ScraperRow {
                game_id: GameId::new("generated-game-02"),
                title: "Mirror Museum".into(),
                provider: Some("fixture-tertiary".into()),
                phase: ui_model::ScraperPhase::Searching,
                fallback_transition: None,
            },
        ],
    };
    let state = reduce(&state, Action::Scraper(ScraperAction::OpenBulkQueue));
    let state = reduce(
        &state,
        Action::Scraper(ScraperAction::SetProgress(progress)),
    );
    assert_eq!(state.scraper.progress.as_ref().unwrap().percent, 0);
    assert_eq!(state.scraper.progress.as_ref().unwrap().rows.len(), 2);
    let state = reduce(&state, Action::Scraper(ScraperAction::Pause));
    assert!(state.scraper.progress.as_ref().unwrap().paused);
    let state = reduce(&state, Action::Scraper(ScraperAction::Hide));
    assert!(state.scraper.progress.as_ref().unwrap().background);
    let state = reduce(&state, Action::Scraper(ScraperAction::Resume));
    assert!(!state.scraper.progress.as_ref().unwrap().paused);
    let state = reduce(&state, Action::Scraper(ScraperAction::Cancel));
    assert!(state.scraper.cancel_requested);
    let state = reduce(&state, Action::Scraper(ScraperAction::ConfirmCancel));
    assert_eq!(state.scraper.progress.as_ref().unwrap().percent, 100);
    assert_eq!(state.scraper.status, ui_model::ScraperStatus::Cancelled);

    let serialized = serde_json::to_vec(&state).unwrap();
    assert_eq!(serialized, serde_json::to_vec(&state).unwrap());
}

fn main() -> Result<(), String> {
    check_fixtures()?;
    check_journey();
    println!("ui-model-fixtures: 14 fixtures loaded; journey and deterministic checks passed");
    Ok(())
}
