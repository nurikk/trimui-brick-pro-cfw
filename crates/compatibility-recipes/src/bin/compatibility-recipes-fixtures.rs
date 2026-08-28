use compatibility_recipes::{
    launcher_dispatch, match_recipe, parse_recipe, LauncherAction, LauncherResponse,
    LocalOverrides, ValidationContext,
};
use serde::Deserialize;
use std::{
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
    println!(
        "compatibility-recipes fixture journey: local match, preview, apply, and rollback passed"
    );
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() > 2 || args.first().is_some_and(|arg| arg != "journey") {
        return Err("usage: compatibility-recipes-fixtures [journey] [fixtures-root]".into());
    }
    let root = PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| format!("/tmp/compatibility-recipes-fixtures-{}", process::id())),
    );
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/compatibility-recipes/generated-v1");
    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/compat-recipe-v1.schema.json");
    check_static_fixtures(&fixtures, &schema_path)?;
    let journey: Journey = read_json(&fixtures.join("journey.json"))?;
    if journey.schema != "https://example.invalid/trimui-compat-recipe-journey-v1.schema.json"
        || journey.format != "trimui-compatibility-recipe-journey"
        || journey.schema_version != 1
        || !journey.synthetic
        || journey.cases.is_empty()
    {
        return Err("journey manifest identity is invalid".into());
    }
    prepare_root(&root)?;
    let catalog = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    let context = ValidationContext::new(catalog);
    let matched = match_recipe(
        &fixtures,
        "recipe-synthetic-valid",
        &"a".repeat(64),
        &context,
    )
    .map_err(|error| error.to_string())?;
    let before = protected_snapshot(&root)?;
    let preview = match launcher_dispatch(
        &root,
        &matched,
        &context,
        "recipe-synthetic-valid",
        LauncherAction::Preview {
            local_overrides: LocalOverrides::default(),
        },
    )
    .map_err(|error| error.to_string())?
    {
        LauncherResponse::Preview(value) => value,
        _ => return Err("recipe preview did not return a preview".into()),
    };
    if preview.source != "local" || preview.recipe_target != "recipe-synthetic-valid" {
        return Err("preview source or target is invalid".into());
    }
    let applied = launcher_dispatch(
        &root,
        &matched,
        &context,
        "recipe-synthetic-valid",
        LauncherAction::Apply {
            local_overrides: LocalOverrides::default(),
            replace_collisions: Default::default(),
        },
    )
    .map_err(|error| error.to_string())?;
    if !matches!(applied, LauncherResponse::Applied(_))
        || !root
            .join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json")
            .is_file()
    {
        return Err("explicit apply did not publish the recipe layer".into());
    }
    if !matches!(
        launcher_dispatch(
            &root,
            &matched,
            &context,
            "recipe-synthetic-valid",
            LauncherAction::Rollback
        )
        .map_err(|error| error.to_string())?,
        LauncherResponse::RolledBack
    ) || root
        .join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json")
        .exists()
    {
        return Err("rollback did not remove the recipe layer".into());
    }
    if before != protected_snapshot(&root)? {
        return Err("recipe flow changed protected data".into());
    }
    println!("PASS local recipe match, preview, explicit apply, and rollback");
    if parse_recipe(
        &fs::read(fixtures.join("recipes/recipe-synthetic-valid.json"))
            .map_err(|e| e.to_string())?,
    )
    .is_err()
    {
        return Err("valid recipe did not parse".into());
    }
    if match_recipe(&fixtures, "recipe-wrong-rom", &"a".repeat(64), &context).is_ok() {
        return Err("wrong ROM matched a recipe".into());
    }
    if match_recipe(&fixtures, "recipe-wrong-device", &"a".repeat(64), &context).is_ok() {
        return Err("wrong device matched a recipe".into());
    }
    println!("PASS versioned catalog matching and ordinary validation reject mismatches");
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn check_static_fixtures(directory: &Path, schema_path: &Path) -> Result<(), String> {
    let schema: serde_json::Value = read_json(schema_path)?;
    if schema.get("additionalProperties") != Some(&serde_json::Value::Bool(false)) {
        return Err("recipe schema is not closed".into());
    }
    for entry in walk_files(directory)? {
        let bytes = fs::read(&entry).map_err(|e| e.to_string())?;
        let _: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", entry.display()))?;
        if bytes.len() > compatibility_recipes::MAX_RECIPE_BYTES {
            return Err(format!(
                "fixture exceeds recipe size bound: {}",
                entry.display()
            ));
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
fn prepare_root(root: &Path) -> Result<(), String> {
    let _ = fs::remove_dir_all(root);
    for (relative, bytes) in [
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
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("protected path has no parent")?)
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
