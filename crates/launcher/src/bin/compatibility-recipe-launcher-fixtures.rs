use anyhow::{bail, Result};
use compatibility_recipes::{LocalOverrides, ValidationContext};
use package_trust::VerificationTime;
use sim_launcher::{
    CompatibilityRecipeAction, CompatibilityRecipeController, CompatibilityRecipeResult,
};
use std::{env, fs, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("compatibility-recipe-launcher-fixtures failed: {error}");
        std::process::exit(1);
    }
    println!("compatibility-recipe-launcher fixture journey: typed preview/apply/rollback passed");
}

fn run() -> Result<()> {
    let root = env::temp_dir().join(format!(
        "compatibility-recipe-launcher-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
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
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, bytes)?;
    }
    let protected_before = protected_snapshot(&root)?;
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/compatibility-recipes/generated-v1");
    let catalog = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    let controller = CompatibilityRecipeController::new(
        &root,
        &fixtures,
        &root.join("trust-state.json"),
        "recipe-synthetic-valid",
        &"a".repeat(64),
        ValidationContext::new(catalog),
        VerificationTime {
            now_rfc3339: "2030-01-01T00:00:00Z",
            uncertainty_seconds: 0,
        },
    )?;
    if !matches!(
        controller.dispatch(CompatibilityRecipeAction::Preview {
            local_overrides: LocalOverrides::default(),
        })?,
        CompatibilityRecipeResult::Preview(_)
    ) {
        bail!("launcher typed preview was not returned");
    }
    let applied = controller.dispatch(CompatibilityRecipeAction::Apply {
        local_overrides: LocalOverrides::default(),
        replace_collisions: Default::default(),
    })?;
    if !matches!(applied, CompatibilityRecipeResult::Applied(_))
        || !root
            .join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json")
            .is_file()
    {
        bail!("launcher typed apply did not publish the recipe layer");
    }
    if !matches!(
        controller.dispatch(CompatibilityRecipeAction::Rollback)?,
        CompatibilityRecipeResult::RolledBack
    ) || root
        .join(".brickpro/config/compatibility-recipes/recipe-synthetic-valid.json")
        .exists()
    {
        bail!("launcher typed rollback did not remove the recipe layer");
    }
    if protected_before != protected_snapshot(&root)? {
        bail!("launcher recipe flow changed protected data");
    }
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn protected_snapshot(root: &std::path::Path) -> Result<Vec<Vec<u8>>> {
    Ok([
        "roms/keep.bin",
        "data/saves/keep.save",
        "data/states/keep.state",
    ]
    .into_iter()
    .map(|path| fs::read(root.join(path)))
    .collect::<std::io::Result<Vec<_>>>()?)
}
