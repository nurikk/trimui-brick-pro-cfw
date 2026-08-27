use std::process;

use settings_schema::{
    MigrationMetadata, Predicate, ProjectionContext, Registry, RegistryError, SettingValue,
    MAX_OPTIONS, MAX_PREDICATE_DEPTH, MAX_PREDICATE_NODES, MAX_REGISTRY_BYTES, MAX_SETTINGS,
};

const FIXTURE: &[u8] = include_bytes!("../../../../fixtures/settings-schema/registry-v1.json");

fn main() {
    if let Err(error) = run() {
        eprintln!("settings-schema-fixtures: {error}");
        process::exit(1);
    }
    println!("settings-schema-fixtures: all bounded checks passed");
}

fn run() -> Result<(), String> {
    let registry = fixture()?;
    check_projection(&registry)?;
    check_rejected_cases(&registry)?;
    Ok(())
}

fn fixture() -> Result<Registry, String> {
    Registry::from_json(FIXTURE).map_err(|error| format!("checked-in fixture rejected: {error}"))
}

fn check_projection(registry: &Registry) -> Result<(), String> {
    let mut context = ProjectionContext::default();
    context
        .capabilities
        .extend(["audio".into(), "theme-engine".into(), "wifi".into()]);
    let model = registry
        .project(&context)
        .map_err(|error| format!("projection failed: {error}"))?;
    let section_ids: Vec<_> = model
        .sections
        .iter()
        .map(|section| section.id.as_str())
        .collect();
    let expected = [
        "display", "audio", "input", "scraper", "theme", "system", "network", "wifi",
    ];
    if section_ids != expected {
        return Err(format!("projection order was {section_ids:?}"));
    }
    let json = model
        .to_canonical_json()
        .map_err(|error| format!("canonical projection failed: {error}"))?;
    if json.contains("wifi-password") || json.contains("scraper-api-key") {
        return Err("canonical projection exposed a credential reference".into());
    }
    let unavailable = registry
        .project(&ProjectionContext::default())
        .map_err(|error| format!("unavailable projection failed: {error}"))?;
    let audio = control(&unavailable, "core.audio.output")?;
    if audio.enabled || audio.unsupported_reason.is_none() {
        return Err("missing capability did not disable audio output".into());
    }
    if unavailable
        .sections
        .iter()
        .any(|section| section.id == "theme")
    {
        return Err("failed visibility predicate did not omit theme".into());
    }
    Ok(())
}

fn check_rejected_cases(registry: &Registry) -> Result<(), String> {
    let mut duplicate_id = registry.clone();
    duplicate_id.settings.push(duplicate_id.settings[0].clone());
    expect_rejected("duplicate setting ID", duplicate_id.validate())?;

    let duplicate_key =
        br#"{"format":"brickpro-settings-registry","format":"brickpro-settings-registry"}"#;
    expect_rejected("duplicate JSON key", Registry::from_json(duplicate_key))?;

    let mut namespace_collision = registry.clone();
    namespace_collision.settings[0].namespace = "provider.scraper".into();
    expect_rejected(
        "provider namespace collision",
        namespace_collision.validate(),
    )?;

    let mut invalid_default = registry.clone();
    invalid_default.settings[0].default = Some(SettingValue::Text("wrong".into()));
    expect_rejected("invalid default", invalid_default.validate())?;

    let mut cycle = registry.clone();
    cycle.settings[0].visibility = Some(Predicate::Present {
        setting: "core.display.brightness".into(),
    });
    expect_rejected("circular predicate", cycle.validate())?;

    let mut secret_without_redaction = registry.clone();
    secret_without_redaction.settings[7].redacted = false;
    expect_rejected("unredacted secret", secret_without_redaction.validate())?;

    let mut secret_bytes = registry.clone();
    secret_bytes.settings[7].default = Some(SettingValue::Text("secret-bytes".into()));
    expect_rejected("secret bytes", secret_bytes.validate())?;

    let mut executable = FIXTURE[..FIXTURE.len() - 2].to_vec();
    executable.extend_from_slice(
        br#","script":"sh"}
"#,
    );
    expect_rejected("unknown executable field", Registry::from_json(&executable))?;

    let mut migration = registry.clone();
    migration.migrations.push(MigrationMetadata {
        id: "future-migration".into(),
        from_version: 1,
        to_version: 2,
        changes: vec![],
    });
    expect_rejected("migration version bounds", migration.validate())?;

    let mut oversized = FIXTURE.to_vec();
    oversized.resize(MAX_REGISTRY_BYTES + 1, b' ');
    expect_rejected("registry size budget", Registry::from_json(&oversized))?;

    let mut too_many_settings = registry.clone();
    let template = too_many_settings.settings[0].clone();
    for index in 0..=MAX_SETTINGS {
        let mut setting = template.clone();
        setting.id = format!("core.display.extra-{index}");
        setting.order = index as i32 + 100;
        too_many_settings.settings.push(setting);
    }
    expect_rejected("setting count budget", too_many_settings.validate())?;

    let mut too_many_options = registry.clone();
    let options = &mut too_many_options.settings[4]
        .constraints
        .as_mut()
        .ok_or("enum fixture has no constraints")?
        .options;
    for index in 0..=MAX_OPTIONS {
        options.push(settings_schema::OptionDescriptor {
            value: format!("extra-{index}"),
            label_key: format!("settings.extra.{index}"),
        });
    }
    expect_rejected("option count budget", too_many_options.validate())?;

    let mut too_deep = registry.clone();
    let mut predicate = Predicate::Present {
        setting: "core.display.brightness".into(),
    };
    for _ in 0..=MAX_PREDICATE_DEPTH {
        predicate = Predicate::Not {
            predicate: Box::new(predicate),
        };
    }
    too_deep.settings[0].visibility = Some(predicate);
    expect_rejected("predicate depth budget", too_deep.validate())?;

    let mut too_many_nodes = registry.clone();
    too_many_nodes.settings[0].visibility = Some(Predicate::All {
        predicates: (0..MAX_PREDICATE_NODES)
            .map(|_| Predicate::Present {
                setting: "core.display.brightness".into(),
            })
            .collect(),
    });
    expect_rejected("predicate node budget", too_many_nodes.validate())?;

    Ok(())
}

fn control<'a>(
    model: &'a settings_schema::MenuModel,
    id: &str,
) -> Result<&'a settings_schema::FormControl, String> {
    model
        .sections
        .iter()
        .flat_map(|section| section.groups.iter())
        .flat_map(|group| group.controls.iter())
        .find(|control| control.setting_id == id)
        .ok_or_else(|| format!("projection omitted expected control {id}"))
}

fn expect_rejected<T>(label: &str, result: Result<T, RegistryError>) -> Result<(), String> {
    if result.is_ok() {
        return Err(format!("{label} was accepted"));
    }
    Ok(())
}
