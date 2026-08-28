use compatibility_recipes::{
    apply, authenticate_and_match, launcher_dispatch, parse_recipe, preview, ApplyOptions,
    FailurePoint, LauncherAction, LauncherResponse, LocalOverrides, ValidationContext,
};
use package_trust::{TrustedMetadataState, VerificationTime};
use serde::Deserialize;
use serde_json::Value;
use std::io::Write;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Journey {
    #[serde(rename = "$schema")]
    schema: String,
    format: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    synthetic: bool,
    cases: Vec<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("compatibility-recipes-fixtures failed: {error}");
        process::exit(1);
    }
    println!("compatibility-recipes fixture journey: signed match, launcher flow, rollback, and 15 rejection cases passed");
}

fn run() -> Result<(), String> {
    let fixtures = env::args().skip(1).collect::<Vec<_>>();
    if fixtures.len() > 2 || fixtures.first().is_some_and(|arg| arg != "journey") {
        return Err("usage: compatibility-recipes-fixtures [journey] [fixtures-root]".into());
    }
    let root = PathBuf::from(
        fixtures
            .get(1)
            .cloned()
            .unwrap_or_else(|| format!("/tmp/compatibility-recipes-fixtures-{}", process::id())),
    );
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/compatibility-recipes/generated-v1");
    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/compat-recipe-v1.schema.json");
    check_static_fixtures(&fixture_dir, &schema_path)?;
    let journey: Journey = read_json(&fixture_dir.join("journey.json"))?;
    if journey.schema != "https://example.invalid/trimui-compat-recipe-journey-v1.schema.json"
        || journey.format != "trimui-compatibility-recipe-journey"
        || journey.schema_version != 1
        || !journey.synthetic
        || journey.cases.len() != 15
    {
        return Err("journey manifest identity or coverage is invalid".into());
    }

    prepare_root(&root)?;
    let catalog_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    let context = ValidationContext::new(&catalog_root);
    let repository = &fixture_dir;
    let valid_hash = "a".repeat(64);
    let state = root.join("states/valid.json");
    let authenticated = authenticate(
        repository,
        &state,
        "recipe-synthetic-valid",
        &valid_hash,
        &context,
    )?;
    if authenticated.delegated_role() != "recipes" || authenticated.repository_versions().root != 1
    {
        return Err("signed repository summary is incomplete".into());
    }
    let mut local = LocalOverrides::default();
    local.folder.insert("core.audio.volume".into(), integer(90));
    local.game.insert("core.audio.volume".into(), integer(92));
    local
        .session
        .insert("core.audio.volume".into(), integer(94));
    let preview_result = launcher_dispatch(
        &root,
        &authenticated,
        &context,
        "recipe-synthetic-valid",
        LauncherAction::Preview {
            local_overrides: local.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    let LauncherResponse::Preview(preview_view) = preview_result else {
        return Err("launcher preview did not return a preview".into());
    };
    let volume = preview_view
        .setting_changes
        .iter()
        .find(|change| change.key == "core.audio.volume")
        .ok_or_else(|| "preview omitted effective volume".to_string())?;
    if volume.after != Some(integer(94))
        || volume.after_source != "session"
        || preview_view.collisions.len() != 3
    {
        return Err("precedence or collision projection is incomplete".into());
    }
    if apply(
        &root,
        &authenticated,
        &context,
        &local,
        ApplyOptions::default(),
    )
    .is_ok()
    {
        return Err("apply did not require explicit collision choices".into());
    }
    let choices = preview_view
        .collisions
        .iter()
        .map(|collision| collision.key.clone())
        .collect();
    let applied = launcher_dispatch(
        &root,
        &authenticated,
        &context,
        "recipe-synthetic-valid",
        LauncherAction::Apply {
            local_overrides: local.clone(),
            replace_collisions: choices,
        },
    )
    .map_err(|error| error.to_string())?;
    let LauncherResponse::Applied(receipt) = applied else {
        return Err("launcher apply did not return a receipt".into());
    };
    if receipt.generation != 1
        || !root
            .join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json")
            .is_file()
    {
        return Err("explicit apply did not publish a named recipe layer".into());
    }
    let before_rollback = protected_snapshot(&root)?;
    if !matches!(
        launcher_dispatch(
            &root,
            &authenticated,
            &context,
            "recipe-synthetic-valid",
            LauncherAction::Rollback,
        )
        .map_err(|error| error.to_string())?,
        LauncherResponse::RolledBack
    ) || root
        .join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json")
        .exists()
    {
        return Err("launcher rollback did not remove only the recipe layer".into());
    }
    let local_preview =
        preview(&authenticated, &context, &local).map_err(|error| error.to_string())?;
    if before_rollback != protected_snapshot(&root)? || local_preview.collisions.len() != 3 {
        return Err("recipe flow changed protected data or hid later local choices".into());
    }
    println!(
        "PASS valid signed match, preview precedence, explicit apply, and recipe-layer rollback"
    );

    let failure_root = root.join("failure");
    let failure_local = LocalOverrides::default();
    for (point, label) in [
        (FailurePoint::AfterVault, "vault"),
        (FailurePoint::AfterLayer, "publication"),
    ] {
        let failure_state = root.join(format!("states/failure-{}.json", label));
        let failure_auth = authenticate(
            repository,
            &failure_state,
            "recipe-synthetic-valid",
            &valid_hash,
            &context,
        )?;
        let error = apply(
            &failure_root,
            &failure_auth,
            &context,
            &failure_local,
            ApplyOptions {
                replace_collisions: Default::default(),
                failure: Some(point),
            },
        )
        .err()
        .ok_or_else(|| format!("injected {label} failure was accepted"))?;
        if error.code != "publication-failed"
            || failure_root
                .join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json")
                .exists()
        {
            return Err(format!("injected {label} failure was not fail-closed"));
        }
    }
    let prior_auth = authenticate(
        repository,
        &root.join("states/failure-prior.json"),
        "recipe-synthetic-valid",
        &valid_hash,
        &context,
    )?;
    apply(
        &failure_root,
        &prior_auth,
        &context,
        &failure_local,
        ApplyOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let layer_path =
        failure_root.join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json");
    let prior_layer = fs::read(&layer_path).map_err(|e| e.to_string())?;
    let failure = apply(
        &failure_root,
        &prior_auth,
        &context,
        &failure_local,
        ApplyOptions {
            replace_collisions: Default::default(),
            failure: Some(FailurePoint::AfterLayer),
        },
    );
    if failure.is_ok() || prior_layer != fs::read(&layer_path).map_err(|e| e.to_string())? {
        return Err("partial publication did not restore the preceding layer".into());
    }
    println!("PASS injected partial publication failures restore prior recipe-layer state");

    expect_code(
        authenticate(
            repository,
            &root.join("states/no-match.json"),
            "recipe-synthetic-valid",
            &"b".repeat(64),
            &context,
        ),
        "no-match",
    )?;
    let tampered = copy_repository(repository, &root.join("tampered-repository"))?;
    fs::OpenOptions::new()
        .append(true)
        .open(tampered.join("recipes/recipe-synthetic-valid.json"))
        .map_err(|e| e.to_string())?
        .write_all(b" ")
        .map_err(|e| e.to_string())?;
    expect_code(
        authenticate(
            &tampered,
            &root.join("states/tamper.json"),
            "recipe-synthetic-valid",
            &valid_hash,
            &context,
        ),
        "target-integrity",
    )?;
    let expired = copy_repository(repository, &root.join("expired-repository"))?;
    fs::copy(
        expired.join("timestamp-expired.json"),
        expired.join("timestamp.json"),
    )
    .map_err(|e| e.to_string())?;
    expect_code(
        authenticate(
            &expired,
            &root.join("states/expired.json"),
            "recipe-synthetic-valid",
            &valid_hash,
            &context,
        ),
        "expired",
    )?;
    let rollback_state = root.join("states/rollback.json");
    write_json(
        &rollback_state,
        &TrustedMetadataState {
            format: package_trust::TRUST_STATE_FORMAT.into(),
            schema_version: 1,
            root_version: 2,
            timestamp_version: 2,
            snapshot_version: 2,
            targets_version: 2,
            delegated: BTreeMap::new(),
        },
    )?;
    expect_code(
        authenticate(
            repository,
            &rollback_state,
            "recipe-synthetic-valid",
            &valid_hash,
            &context,
        ),
        "rollback",
    )?;
    expect_code(
        authenticate(
            repository,
            &root.join("states/wrong-device.json"),
            "recipe-wrong-device",
            &valid_hash,
            &context,
        ),
        "identity-invalid",
    )?;
    expect_code(
        authenticate(
            repository,
            &root.join("states/wrong-rom.json"),
            "recipe-wrong-rom",
            &valid_hash,
            &context,
        ),
        "no-match",
    )?;
    expect_code(
        authenticate(
            repository,
            &root.join("states/blocked-core.json"),
            "recipe-blocked-core",
            &valid_hash,
            &context,
        ),
        "unavailable-core",
    )?;
    expect_code(
        authenticate(
            repository,
            &root.join("states/invalid-config.json"),
            "recipe-invalid-config",
            &valid_hash,
            &context,
        ),
        "unsafe-value",
    )?;
    expect_code(
        authenticate(
            repository,
            &root.join("states/unknown-config.json"),
            "recipe-unknown-config",
            &valid_hash,
            &context,
        ),
        "invalid-config-key",
    )?;
    expect_code(
        parse_recipe(br#"{"format":"x","format":"y"}"#),
        "malformed-recipe",
    )?;
    expect_code(
        parse_recipe(&vec![b'x'; compatibility_recipes::MAX_RECIPE_BYTES + 1]),
        "oversized-input",
    )?;
    expect_code(
        authenticate(
            repository,
            &root.join("states/unsafe-path.json"),
            "../escape",
            &valid_hash,
            &context,
        ),
        "invalid-identifier",
    )?;
    if parse_recipe(br#"{"#).is_ok() {
        return Err("malformed JSON was accepted".into());
    }
    let _ = fs::remove_dir_all(&root);
    println!("PASS no-match, tamper, expiry, rollback, identity, blocked-core, config, duplicate, size, and path rejection matrix");
    Ok(())
}

fn authenticate(
    repository: &Path,
    state: &Path,
    target: &str,
    hash: &str,
    context: &ValidationContext,
) -> Result<compatibility_recipes::AuthenticatedRecipe, String> {
    authenticate_and_match(
        repository,
        state,
        target,
        hash,
        VerificationTime {
            now_rfc3339: "2030-01-01T00:00:00Z",
            uncertainty_seconds: 0,
        },
        context,
    )
    .map_err(|error| format!("{}", error))
}

fn expect_code<T, E: std::fmt::Display>(
    result: std::result::Result<T, E>,
    expected: &str,
) -> Result<(), String> {
    match result {
        Ok(_) => Err(format!("expected {expected} rejection")),
        Err(error)
            if error.to_string().starts_with(expected) || error.to_string().contains(expected) =>
        {
            Ok(())
        }
        Err(error) => Err(format!("expected {expected}, got {error}")),
    }
}

fn integer(value: i64) -> settings_schema::SettingValue {
    settings_schema::SettingValue::Integer(value)
}

fn check_static_fixtures(directory: &Path, schema_path: &Path) -> Result<(), String> {
    let schema: Value = read_json(schema_path)?;
    if schema.get("additionalProperties") != Some(&Value::Bool(false))
        || schema.get("$defs").is_none()
    {
        return Err("recipe schema is not closed".into());
    }
    for entry in walk_files(directory)? {
        let bytes = fs::read(&entry).map_err(|e| e.to_string())?;
        let _: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", entry.display()))?;
        if entry
            .extension()
            .is_some_and(|extension| extension == "json")
            && bytes.len() > compatibility_recipes::MAX_RECIPE_BYTES
        {
            return Err(format!(
                "fixture exceeds recipe size bound: {}",
                entry.display()
            ));
        }
    }
    for name in [
        "recipe-synthetic-valid",
        "recipe-wrong-device",
        "recipe-wrong-rom",
        "recipe-blocked-core",
        "recipe-invalid-config",
        "recipe-unknown-config",
    ] {
        if !directory
            .join("recipes")
            .join(format!("{name}.json"))
            .is_file()
        {
            return Err(format!("missing generated target {name}"));
        }
    }
    Ok(())
}

fn walk_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            files.extend(walk_files(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

fn copy_repository(source: &Path, destination: &Path) -> Result<PathBuf, String> {
    let _ = fs::remove_dir_all(destination);
    for path in walk_files(source)? {
        let relative = path.strip_prefix(source).map_err(|e| e.to_string())?;
        let target = destination.join(relative);
        fs::create_dir_all(
            target
                .parent()
                .ok_or_else(|| "repository target has no parent".to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::copy(path, target).map_err(|e| e.to_string())?;
    }
    Ok(destination.to_path_buf())
}

fn prepare_root(root: &Path) -> Result<(), String> {
    let _ = fs::remove_dir_all(root);
    for (path, bytes) in [
        ("roms/keep.bin", b"synthetic-rom-boundary".as_slice()),
        (
            "data/saves/keep.save",
            b"synthetic-save-boundary".as_slice(),
        ),
        (
            "data/states/keep.state",
            b"synthetic-state-boundary".as_slice(),
        ),
    ] {
        let path = root.join(path);
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| "protected path has no parent".to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn protected_snapshot(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    [
        "roms/keep.bin",
        "data/saves/keep.save",
        "data/states/keep.state",
    ]
    .into_iter()
    .map(|path| fs::read(root.join(path)).map_err(|e| e.to_string()))
    .collect()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| "JSON path has no parent".to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
