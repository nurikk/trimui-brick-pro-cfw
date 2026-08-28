use std::process;

use settings_schema::{ApplyMode, FieldKind, ProjectionContext, SettingValue, ValidationMetadata};
use settings_ui::{ControllerAction, EventKind, SettingsUi};
use virtual_keyboard::{Button, InputResult, TypedValue};

const REGISTRY: &[u8] = include_bytes!("../../../../fixtures/settings-schema/registry-v1.json");
const JOURNEY: &str = include_str!("../../../../fixtures/settings-ui/journey.json");

fn main() {
    if let Err(error) = run() {
        eprintln!("settings-ui-fixtures: {error}");
        process::exit(1);
    }
    println!("settings-ui-fixtures: deterministic generic controller journey passed");
}

fn run() -> Result<(), String> {
    let fixture: serde_json::Value =
        serde_json::from_str(JOURNEY).map_err(|error| error.to_string())?;
    let fixture_bytes = serde_json::to_vec(&fixture).map_err(|error| error.to_string())?;
    if fixture_bytes != serde_json::to_vec(&fixture).map_err(|error| error.to_string())? {
        return Err("fixture serialization is not deterministic".into());
    }
    for forbidden in [
        "credentialRef",
        "credential-ref",
        "credential reference",
        "wifi-password",
        "scraper-api-key",
    ] {
        if JOURNEY.contains(forbidden) {
            return Err(format!(
                "fixture contains forbidden secret reference: {forbidden}"
            ));
        }
    }

    let registry = synthetic_registry()?;
    let mut context = ProjectionContext::default();
    context.capabilities.extend([
        "audio".into(),
        "network".into(),
        "theme-engine".into(),
        "wifi".into(),
    ]);
    let mut ui = SettingsUi::new(registry.clone(), context).map_err(|error| error.to_string())?;
    let initial = ui.scene().map_err(|error| error.to_string())?;
    if initial.width != 1024 || initial.height != 768 {
        return Err("scene dimensions are not 1024x768".into());
    }
    let section_ids: Vec<_> = initial
        .sections
        .iter()
        .map(|section| section.id.as_str())
        .collect();
    if section_ids
        != [
            "display", "audio", "input", "scraper", "theme", "system", "network", "wifi",
        ]
    {
        return Err(format!("unexpected section order: {section_ids:?}"));
    }
    let kinds: Vec<_> = initial
        .sections
        .iter()
        .flat_map(|section| section.groups.iter())
        .flat_map(|group| group.controls.iter())
        .map(|control| control.kind)
        .collect();
    for kind in [
        FieldKind::Boolean,
        FieldKind::EnumSingle,
        FieldKind::EnumMulti,
        FieldKind::Integer,
        FieldKind::Decimal,
        FieldKind::Text,
        FieldKind::Secret,
        FieldKind::Action,
        FieldKind::ReadOnly,
        FieldKind::Status,
    ] {
        if !kinds.contains(&kind) {
            return Err(format!("missing projected control kind: {kind:?}"));
        }
    }
    let synthetic = control(&initial, "core.system.synthetic-label")?;
    if synthetic.kind != FieldKind::Text
        || synthetic.label_key != "settings.synthetic.label"
        || synthetic.apply != ApplyMode::OnConfirm
    {
        return Err("synthetic descriptor did not use the generic projection path".into());
    }
    let regex_control = control(&initial, "core.system.synthetic-pattern")?;
    if regex_control.kind != FieldKind::Text {
        return Err("regex-shaped text descriptor did not project as text".into());
    }
    ui.set_value(
        "core.system.synthetic-pattern",
        SettingValue::Text("beta".into()),
    )
    .map_err(|error| error.to_string())?;
    if ui
        .set_value(
            "core.system.synthetic-pattern",
            SettingValue::Text("gamma".into()),
        )
        .is_ok()
    {
        return Err("regex-shaped descriptor accepted an invalid value".into());
    }
    if ui
        .scene()
        .map_err(|error| error.to_string())?
        .validation_errors
        .len()
        != 1
    {
        return Err("regex-shaped descriptor did not report invalid input".into());
    }
    ui.cancel();
    let night_mode = control(&initial, "core.display.night-mode")?;
    if !night_mode
        .badges
        .contains(&settings_ui::ApplyBadge::RestartLauncher)
    {
        return Err("restart-launcher badge was not projected".into());
    }
    let volume = control(&initial, "core.audio.volume")?;
    if !volume
        .badges
        .contains(&settings_ui::ApplyBadge::RebootCandidate)
    {
        return Err("reboot-candidate badge was not projected".into());
    }
    let secret = control(&initial, "provider.scraper.api-key")?;
    if !matches!(secret.value, settings_ui::SemanticValue::Masked { .. }) || !secret.redacted {
        return Err("secret control was not masked".into());
    }
    let parallel = control(&initial, "core.scraper.parallel-jobs")?;
    let options: Vec<_> = parallel
        .constraints
        .as_ref()
        .ok_or("parallelism options missing")?
        .options
        .iter()
        .map(|option| option.value.as_str())
        .collect();
    if parallel.kind != FieldKind::EnumSingle || options != ["1", "2", "4"] {
        return Err("parallelism did not project exact controller options".into());
    }
    for id in [
        "provider.scraper.fixture-primary-enabled",
        "provider.scraper.fixture-secondary-enabled",
        "provider.scraper.fixture-tertiary-enabled",
        "provider.scraper.priority",
        "provider.scraper.fixture-primary-credentials",
        "provider.scraper.fixture-secondary-credentials",
        "provider.scraper.fixture-tertiary-credentials",
    ] {
        control(&initial, id)?;
    }
    for (id, expected) in [
        ("provider.scraper.fixture-primary-limit", "1"),
        ("provider.scraper.fixture-secondary-limit", "2"),
        ("provider.scraper.fixture-tertiary-limit", "2"),
    ] {
        if control(&initial, id)?.value != settings_ui::SemanticValue::Text(expected.into()) {
            return Err(format!(
                "provider limit metadata was not projected for {id}"
            ));
        }
    }
    if initial
        .sections
        .iter()
        .flat_map(|section| section.groups.iter())
        .flat_map(|group| group.controls.iter())
        .any(|control| control.setting_id.contains("credential-ref"))
    {
        return Err("provider projection exposed a credential reference".into());
    }
    if initial
        .sections
        .iter()
        .flat_map(|section| section.groups.iter())
        .flat_map(|group| group.controls.iter())
        .any(|control| {
            control.disabled_reason.as_deref()
                == Some("Audio output is unavailable on this platform")
        })
    {
        return Err("supported context unexpectedly disabled audio".into());
    }

    ui.press(Button::Primary)
        .map_err(|error| error.to_string())?;
    let form = ui.scene().map_err(|error| error.to_string())?;
    if form.surface != settings_ui::Surface::Form || form.selected_help.is_none() {
        return Err("section list did not open a form with selected help".into());
    }
    ui.press(Button::Right).map_err(|error| error.to_string())?;
    ui.press(Button::Down).map_err(|error| error.to_string())?;
    ui.press(Button::Right).map_err(|error| error.to_string())?;
    let pending = ui.scene().map_err(|error| error.to_string())?;
    if pending.pending.count != 1
        || pending.pending.changes[0].setting_id != "core.display.gamma"
        || !matches!(
            pending.pending.changes[0].value,
            settings_ui::SemanticValue::Decimal(value) if (value - 1.3).abs() < 1e-9
        )
    {
        return Err("decimal controller stepping did not produce the pending value".into());
    }
    ui.cancel();
    if ui.scene().map_err(|error| error.to_string())?.pending.count != 0 {
        return Err("cancel did not discard pending changes".into());
    }
    ui.set_value("core.display.gamma", SettingValue::Decimal(1.3))
        .map_err(|error| error.to_string())?;
    ui.confirm().map_err(|error| error.to_string())?;
    ui.dispatch(ControllerAction::SetValue {
        setting_id: "core.system.synthetic-label".into(),
        value: SettingValue::Text("updated".into()),
    })
    .map_err(|error| error.to_string())?;
    ui.dispatch(ControllerAction::Apply)
        .map_err(|error| error.to_string())?;
    if ui.scene().map_err(|error| error.to_string())?.pending.count != 0 {
        return Err("apply did not commit pending changes".into());
    }

    ui.set_value("core.display.night-mode", SettingValue::Boolean(false))
        .map_err(|error| error.to_string())?;
    ui.set_value(
        "core.audio.output",
        SettingValue::EnumSingle("speaker".into()),
    )
    .map_err(|error| error.to_string())?;
    ui.set_value(
        "core.input.turbo-buttons",
        SettingValue::EnumMulti(vec!["b".into(), "x".into()]),
    )
    .map_err(|error| error.to_string())?;
    ui.confirm().map_err(|error| error.to_string())?;
    ui.set_value("core.system.version", SettingValue::Text("no-write".into()))
        .unwrap_err();
    ui.set_value("core.network.state", SettingValue::Text("no-write".into()))
        .unwrap_err();
    ui.cancel();
    ui.set_value("core.display.gamma", SettingValue::Decimal(9.0))
        .unwrap_err();
    let invalid = ui.scene().map_err(|error| error.to_string())?;
    if invalid.validation_errors.len() != 1 {
        return Err("invalid value did not produce a validation error".into());
    }
    ui.cancel();
    let text_keyboard = ui
        .open_keyboard("core.input.hotkey")
        .map_err(|error| error.to_string())?;
    if text_keyboard.scene().field != virtual_keyboard::FieldKind::Text {
        return Err("text keyboard policy was not used".into());
    }
    ui.accept_keyboard("core.input.hotkey", InputResult::Cancelled)
        .map_err(|error| error.to_string())?;
    if ui.open_keyboard("core.display.gamma").is_ok() {
        return Err("decimal control exposed an unusable keyboard".into());
    }
    ui.cancel();
    let integer_keyboard = ui
        .open_keyboard("core.display.brightness")
        .map_err(|error| error.to_string())?;
    if integer_keyboard.scene().field != virtual_keyboard::FieldKind::Numeric {
        return Err("integer keyboard policy was not used".into());
    }
    ui.accept_keyboard("core.display.brightness", InputResult::Cancelled)
        .map_err(|error| error.to_string())?;
    let secret_keyboard = ui
        .open_keyboard("provider.scraper.api-key")
        .map_err(|error| error.to_string())?;
    if secret_keyboard.scene().field != virtual_keyboard::FieldKind::Secret
        || secret_keyboard
            .scene()
            .display
            .chars()
            .any(|character| character != '*')
    {
        return Err("secret keyboard was not masked".into());
    }
    let secret_debug = format!("{secret_keyboard:?}");
    if secret_debug.contains("credential") {
        return Err("secret keyboard debug leaked a reference".into());
    }
    ui.accept_keyboard(
        "provider.scraper.api-key",
        InputResult::Confirmed(TypedValue::Secret(String::new())),
    )
    .map_err(|error| error.to_string())?;

    ui.back();
    for _ in 0..5 {
        ui.press(Button::Down).map_err(|error| error.to_string())?;
    }
    ui.press(Button::Primary)
        .map_err(|error| error.to_string())?;
    ui.press(Button::Primary)
        .map_err(|error| error.to_string())?;
    ui.press(Button::Down).map_err(|error| error.to_string())?;
    ui.press(Button::Down).map_err(|error| error.to_string())?;
    ui.press(Button::Down).map_err(|error| error.to_string())?;
    ui.press(Button::Primary)
        .map_err(|error| error.to_string())?;
    let external = ui.scene().map_err(|error| error.to_string())?;
    if external.external_operations.len() != 1
        || external.external_operations[0].operation != ApplyMode::ExternalOperation
    {
        return Err("action did not remain a semantic external-operation request".into());
    }
    if !ui
        .drain_events()
        .iter()
        .any(|event| event.kind == EventKind::ExternalOperationRequested)
    {
        return Err("external operation event was not recorded".into());
    }

    let unsupported = SettingsUi::new(registry, ProjectionContext::default())
        .map_err(|error| error.to_string())?
        .scene()
        .map_err(|error| error.to_string())?;
    let audio = control(&unsupported, "core.audio.output")?;
    if audio.enabled || audio.disabled_reason.is_none() {
        return Err("capability-disabled control did not expose a reason".into());
    }
    let wifi = control(&unsupported, "core.wifi.enabled")?;
    if wifi.enabled || wifi.disabled_reason.is_none() {
        return Err("placeholder Wi-Fi was not capability-gated".into());
    }
    if unsupported
        .sections
        .iter()
        .any(|section| section.id == "theme")
    {
        return Err("capability visibility predicate did not hide theme".into());
    }

    let ui_debug = format!("{ui:?}");
    let ui_json = serde_json::to_string(&ui).map_err(|error| error.to_string())?;
    if ui_debug.contains("credentialRef") || ui_json.contains("credentialRef") {
        return Err("settings UI evidence leaked a credential reference".into());
    }
    let stable = ui.scene().map_err(|error| error.to_string())?;
    let first = serde_json::to_vec(&stable).map_err(|error| error.to_string())?;
    let second = serde_json::to_vec(&ui.scene().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    if first != second {
        return Err("repeated scene serialization changed bytes".into());
    }
    Ok(())
}

fn synthetic_registry() -> Result<settings_schema::Registry, String> {
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
    let mut registry = settings_schema::Registry::from_json(REGISTRY)
        .and_then(|registry| registry.with_provider_metadata(&providers))
        .map_err(|error| error.to_string())?;
    let template = registry
        .settings
        .iter()
        .find(|setting| setting.kind == FieldKind::Text)
        .cloned()
        .ok_or("text descriptor missing")?;
    let mut synthetic = template;
    synthetic.id = "core.system.synthetic-label".into();
    synthetic.section = "system".into();
    synthetic.group = "generated".into();
    synthetic.order = 30;
    synthetic.label_key = "settings.synthetic.label".into();
    synthetic.description_key = "settings.synthetic.description".into();
    synthetic.default = Some(SettingValue::Text("generated".into()));
    synthetic.current = Some(SettingValue::Text("generated".into()));
    synthetic.pending = None;
    synthetic.apply = vec![ApplyMode::OnConfirm];
    synthetic.validation = ValidationMetadata {
        required: true,
        trim: false,
        allow_empty: false,
    };
    registry.settings.push(synthetic.clone());
    let mut regex_synthetic = synthetic;
    regex_synthetic.id = "core.system.synthetic-pattern".into();
    regex_synthetic.label_key = "settings.synthetic.pattern".into();
    regex_synthetic.description_key = "settings.synthetic.pattern.description".into();
    regex_synthetic.order = 40;
    let constraints = regex_synthetic
        .constraints
        .as_mut()
        .and_then(|constraints| constraints.text.as_mut())
        .ok_or("text descriptor has no text constraints")?;
    constraints.pattern = Some("^alpha|beta$".into());
    regex_synthetic.default = Some(SettingValue::Text("alpha".into()));
    regex_synthetic.current = Some(SettingValue::Text("alpha".into()));
    registry.settings.push(regex_synthetic);
    registry.validate().map_err(|error| error.to_string())?;
    Ok(registry)
}

fn control<'a>(
    scene: &'a settings_ui::Scene,
    id: &str,
) -> Result<&'a settings_ui::SettingControl, String> {
    scene
        .sections
        .iter()
        .flat_map(|section| section.groups.iter())
        .flat_map(|group| group.controls.iter())
        .find(|control| control.setting_id == id)
        .ok_or_else(|| format!("missing projected control: {id}"))
}
