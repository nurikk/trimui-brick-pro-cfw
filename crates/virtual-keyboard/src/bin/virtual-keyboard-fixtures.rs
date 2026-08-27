use virtual_keyboard::{
    key_grid, AllowedChars, Button, FieldPolicy, InputResult, KeyAction, Keyboard, LayoutId,
    RejectReason, TypedValue, ValidationState, WrapMode,
};

fn position(keyboard: &Keyboard) -> (usize, usize) {
    let selected_key = keyboard.selected_key();
    let mut parts = selected_key.id.split('-');
    assert_eq!(parts.next(), Some("key"));
    (
        parts.next().unwrap().parse().unwrap(),
        parts.next().unwrap().parse().unwrap(),
    )
}

fn focus_key<F>(keyboard: &mut Keyboard, predicate: F)
where
    F: Fn(&virtual_keyboard::KeyCell) -> bool,
{
    keyboard.set_wrap_mode(WrapMode::None);
    let grid = key_grid(keyboard.layout());
    let (target_row, target_column) = grid
        .iter()
        .enumerate()
        .flat_map(|(row, keys)| {
            keys.iter()
                .enumerate()
                .map(move |(column, key)| (row, column, key))
        })
        .find_map(|(row, column, key)| predicate(key).then_some((row, column)))
        .expect("requested key exists");
    let (mut row, mut column) = position(keyboard);
    while row < target_row {
        keyboard.press(Button::Down);
        row = position(keyboard).0;
    }
    while row > target_row {
        keyboard.press(Button::Up);
        row = position(keyboard).0;
    }
    while column < target_column {
        keyboard.press(Button::Right);
        column = position(keyboard).1;
    }
    while column > target_column {
        keyboard.press(Button::Left);
        column = position(keyboard).1;
    }
    assert_eq!(position(keyboard), (target_row, target_column));
}

fn focus_character(keyboard: &mut Keyboard, character: char) {
    focus_key(
        keyboard,
        |key| matches!(&key.action, KeyAction::Character(value) if *value == character),
    );
}

fn focus_label(keyboard: &mut Keyboard, label: &str) {
    focus_key(keyboard, |key| key.label == label);
}

fn type_controller(keyboard: &mut Keyboard, value: &str) {
    for character in value.chars() {
        focus_character(keyboard, character);
        assert_eq!(keyboard.press(Button::Primary), InputResult::Inserted);
    }
}

fn assert_scene(keyboard: &Keyboard, expected: &[u8]) {
    serde_json::from_slice::<serde_json::Value>(expected).unwrap();
    assert_eq!(keyboard.semantic_scene_snapshot(), expected);
}

fn main() {
    let normal =
        Keyboard::new(FieldPolicy::text("", "SSID", 32, 16, AllowedChars::ascii())).unwrap();
    assert_scene(
        &normal,
        include_bytes!("../../../../fixtures/virtual-keyboard/normal.json"),
    );

    let mut shift =
        Keyboard::new(FieldPolicy::text("", "SSID", 32, 16, AllowedChars::ascii())).unwrap();
    shift.set_layout(LayoutId::UppercaseShift);
    assert_scene(
        &shift,
        include_bytes!("../../../../fixtures/virtual-keyboard/shift.json"),
    );

    let mut symbols =
        Keyboard::new(FieldPolicy::text("", "SSID", 32, 16, AllowedChars::ascii())).unwrap();
    symbols.set_layout(LayoutId::Symbols);
    assert_scene(
        &symbols,
        include_bytes!("../../../../fixtures/virtual-keyboard/symbols.json"),
    );

    let numeric = Keyboard::new(FieldPolicy::numeric("", "port", 5, 5)).unwrap();
    assert_scene(
        &numeric,
        include_bytes!("../../../../fixtures/virtual-keyboard/numeric.json"),
    );

    let error = Keyboard::new(
        FieldPolicy::text("", "search", 32, 16, AllowedChars::ascii())
            .with_validation(ValidationState::Invalid),
    )
    .unwrap();
    assert_scene(
        &error,
        include_bytes!("../../../../fixtures/virtual-keyboard/error.json"),
    );

    let max = Keyboard::new(FieldPolicy::text("abc", "", 3, 3, AllowedChars::ascii())).unwrap();
    assert_scene(
        &max,
        include_bytes!("../../../../fixtures/virtual-keyboard/max-length.json"),
    );

    let mut masked =
        Keyboard::new(FieldPolicy::secret("", "", 32, 16, AllowedChars::ascii())).unwrap();
    let fixture_token: String = ['q', 'j', 'v'].into_iter().collect();
    type_controller(&mut masked, &fixture_token);
    assert_eq!(masked.selected_key().id, "key-2-3");
    assert_scene(
        &masked,
        include_bytes!("../../../../fixtures/virtual-keyboard/masked-secret.json"),
    );

    let mut ssid =
        Keyboard::new(FieldPolicy::text("", "SSID", 32, 16, AllowedChars::ascii())).unwrap();
    type_controller(&mut ssid, "manual");
    assert_eq!(
        ssid.press(Button::Start),
        InputResult::Confirmed(TypedValue::Text("manual".to_owned()))
    );

    let mut passphrase =
        Keyboard::new(FieldPolicy::secret("", "", 32, 16, AllowedChars::ascii())).unwrap();
    let token: String = ['q', 'j', 'v'].into_iter().collect();
    type_controller(&mut passphrase, &token);
    let semantic = String::from_utf8(passphrase.semantic_scene_snapshot()).unwrap();
    let events = format!("{:?}", passphrase.drain_events());
    let debug = format!("{passphrase:?}");
    assert!(!semantic.contains(&token));
    assert!(!events.contains(&token));
    assert!(!debug.contains(&token));
    assert_eq!(
        passphrase.press(Button::Start),
        InputResult::Confirmed(TypedValue::Secret(token))
    );

    let mut username = Keyboard::new(FieldPolicy::text(
        "",
        "username",
        32,
        16,
        AllowedChars::ascii(),
    ))
    .unwrap();
    type_controller(&mut username, "scraper");
    assert_eq!(
        username.press(Button::Start),
        InputResult::Confirmed(TypedValue::Text("scraper".to_owned()))
    );

    let mut port = Keyboard::new(FieldPolicy::numeric("", "port", 5, 5)).unwrap();
    type_controller(&mut port, "8080");
    assert_eq!(
        port.press(Button::Start),
        InputResult::Confirmed(TypedValue::Numeric(8080))
    );

    let mut game_search = Keyboard::new(FieldPolicy::text(
        "",
        "search",
        32,
        16,
        AllowedChars::ascii(),
    ))
    .unwrap();
    type_controller(&mut game_search, "zelda");
    assert_eq!(
        game_search.press(Button::Start),
        InputResult::Confirmed(TypedValue::Text("zelda".to_owned()))
    );

    let mut wrap = Keyboard::new(FieldPolicy::text("", "", 8, 8, AllowedChars::ascii())).unwrap();
    wrap.set_wrap_mode(WrapMode::None);
    wrap.press(Button::Left);
    assert_eq!(wrap.selected_key().label, "q");
    wrap.set_wrap_mode(WrapMode::Both);
    wrap.press(Button::Left);
    assert_eq!(wrap.selected_key().label, "p");
    assert_eq!(wrap.press(Button::Select), InputResult::Ignored);
    focus_character(&mut wrap, 'q');
    assert_eq!(wrap.press(Button::Menu), InputResult::Cancelled);

    let mut cancelled =
        Keyboard::new(FieldPolicy::text("go", "", 8, 8, AllowedChars::ascii())).unwrap();
    type_controller(&mut cancelled, "q");
    assert_eq!(cancelled.press(Button::Secondary), InputResult::Cancelled);
    assert_eq!(cancelled.semantic_snapshot().display, "go");

    let mut unicode =
        Keyboard::new(FieldPolicy::text("aé界", "", 32, 8, AllowedChars::any())).unwrap();
    unicode.press(Button::LeftShoulder);
    focus_label(&mut unicode, "DEL");
    assert_eq!(unicode.press(Button::Primary), InputResult::Inserted);
    unicode.press(Button::LeftShoulder);
    assert_eq!(unicode.press(Button::Primary), InputResult::Inserted);
    assert_eq!(unicode.semantic_snapshot().display, "a");
    assert_eq!(unicode.semantic_snapshot().cursor_byte, 1);

    let mut backspace =
        Keyboard::new(FieldPolicy::text("é", "", 32, 8, AllowedChars::any())).unwrap();
    focus_label(&mut backspace, "BACK");
    assert_eq!(backspace.press(Button::Primary), InputResult::Inserted);
    assert_eq!(backspace.semantic_snapshot().display, "");

    let mut byte_limited =
        Keyboard::new(FieldPolicy::text("", "", 2, 8, AllowedChars::ascii())).unwrap();
    type_controller(&mut byte_limited, "qq");
    focus_character(&mut byte_limited, 'q');
    assert_eq!(
        byte_limited.press(Button::Primary),
        InputResult::Rejected(RejectReason::MaxBytes)
    );

    let mut scalar_limited =
        Keyboard::new(FieldPolicy::text("", "", 8, 1, AllowedChars::ascii())).unwrap();
    type_controller(&mut scalar_limited, "q");
    focus_character(&mut scalar_limited, 'q');
    assert_eq!(
        scalar_limited.press(Button::Primary),
        InputResult::Rejected(RejectReason::MaxScalars)
    );

    let mut url =
        Keyboard::new(FieldPolicy::text("", "", 32, 32, AllowedChars::url_safe())).unwrap();
    url.set_layout(LayoutId::Symbols);
    focus_character(&mut url, '?');
    assert_eq!(url.press(Button::Primary), InputResult::Inserted);
    focus_character(&mut url, '{');
    assert_eq!(
        url.press(Button::Primary),
        InputResult::Rejected(RejectReason::DisallowedCharacter)
    );
    let mut normal_punctuation =
        Keyboard::new(FieldPolicy::text("", "", 32, 32, AllowedChars::ascii())).unwrap();
    normal_punctuation.set_layout(LayoutId::Symbols);
    focus_character(&mut normal_punctuation, '{');
    assert_eq!(
        normal_punctuation.press(Button::Primary),
        InputResult::Inserted
    );

    let evidence = include_bytes!("../../../../fixtures/virtual-keyboard/wifi-journeys.json");
    serde_json::from_slice::<serde_json::Value>(evidence).unwrap();
    assert!(!String::from_utf8_lossy(evidence).contains(&fixture_token));
    println!("virtual-keyboard-fixtures: all deterministic journeys and semantic fixtures passed");
}
