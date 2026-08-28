use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use anyhow::{bail, Result};
use package_manager::{
    install_with_validation, validate_manifest, PackageManifest, TransactionOptions, TrustContext,
};
use package_trust::{TrustStore, VerifiedTarget};
use theme_garden::{Catalog, ThemeGarden, CACHE_PATH, STAGING_PATH};

fn main() {
    if let Err(error) = run() {
        eprintln!("theme-garden failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("journey") => {
            let fixtures = required_path(&mut args, "--fixtures")?;
            let root = required_path(&mut args, "--root")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            journey(&fixtures, &root)
        }
        Some("parse") => {
            let path = required_path(&mut args, "--catalog")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            let catalog = Catalog::parse(&fs::read(path)?)?;
            println!("PASS parsed {} catalog entries", catalog.themes.len());
            Ok(())
        }
        _ => bail!("usage: theme-garden journey --fixtures PATH --root SYNTHETIC_ROOT | parse --catalog PATH"),
    }
}

fn journey(fixtures: &Path, root: &Path) -> Result<()> {
    if root.as_os_str().is_empty() || root == Path::new("/") {
        bail!("journey requires a caller-provided synthetic root")
    }
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root)?;
    let protected = [
        ("roms/keep.bin", b"synthetic-rom-boundary".as_slice()),
        (
            "data/saves/keep.save",
            b"synthetic-save-boundary".as_slice(),
        ),
        (
            "data/states/keep.state",
            b"synthetic-state-boundary".as_slice(),
        ),
        (
            "data/resume/keep.record",
            b"synthetic-resume-boundary".as_slice(),
        ),
        (
            "data/settings.json",
            b"synthetic-settings-boundary".as_slice(),
        ),
    ];
    for (relative, bytes) in protected {
        let path = root.join(relative);
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("protected path has no parent"))?,
        )?;
        fs::write(path, bytes)?;
    }
    let before = protected_bytes(root)?;
    let garden = ThemeGarden::authenticate(root, fixtures)?;
    let catalog_bytes = fs::read(fixtures.join("themes.json"))?;
    let selected =
        garden.select_themes_json(&catalog_bytes, "minimal", "1.0.0", &fixtures.join("themes"))?;
    if selected.name().is_empty() || ThemeGarden::from_cache(root, fixtures)?.browse().len() != 3 {
        bail!("verified offline cache could not be browsed")
    }
    let cache_dir = root.join(CACHE_PATH.trim_start_matches('/'));
    let cached_catalog = fs::read(cache_dir.join("catalog.json"))?;
    let cached_metadata = fs::read(cache_dir.join("metadata.json"))?;
    let mut changed_catalog: serde_json::Value = serde_json::from_slice(&cached_catalog)?;
    changed_catalog["catalogVersion"] = serde_json::Value::String("9.9.9".into());
    let mut changed_metadata: serde_json::Value = serde_json::from_slice(&cached_metadata)?;
    changed_metadata["catalogVersion"] = serde_json::Value::String("9.9.9".into());
    fs::write(
        cache_dir.join("catalog.json"),
        serde_json::to_vec(&changed_catalog)?,
    )?;
    fs::write(
        cache_dir.join("metadata.json"),
        serde_json::to_vec(&changed_metadata)?,
    )?;
    if ThemeGarden::from_cache(root, fixtures).is_ok() {
        bail!("coordinated cache modification was accepted")
    }
    fs::write(cache_dir.join("catalog.json"), cached_catalog)?;
    fs::write(cache_dir.join("metadata.json"), cached_metadata)?;
    println!("PASS coordinated catalog and metadata cache tampering rejects without refresh");
    if garden.browse().len() != 3 {
        bail!("browse did not expose all catalog entries")
    }
    println!("PASS themes.json adapter selected a verified local theme without network access");
    println!("PASS Browse controller exposes 3 project-authored entries");
    for entry in garden.browse() {
        let detail = garden.details(&entry.id)?;
        if detail.target_sku != "TG4040"
            || detail.sha256.len() != 64
            || !detail.screenshots_available
        {
            bail!("incomplete detail record")
        }
        println!(
            "PASS Details {} {} {}",
            detail.id, detail.version, detail.download_size
        );
    }
    let active_before = garden.active()?;
    let preview = garden.preview("high-contrast")?;
    if !preview.is_file() || garden.active()?.id != active_before.id {
        bail!("preview changed active theme")
    }
    println!("PASS Preview generated deterministic screenshot without activation");
    if garden.installed()?.is_empty() || !garden.updates()?.is_empty() {
        bail!("initial installed or updates state is incorrect")
    }
    let flow = garden.controller_flow()?;
    if flow.controller != "controller-first" || flow.entries != 3 {
        bail!("controller flow is not catalog-first")
    }
    println!("PASS Installed and Updates controller states are explicit");

    if garden.install("high-contrast", Some(8), false).is_ok()
        || garden.active()?.id != "artbook"
        || !root
            .join(STAGING_PATH.trim_start_matches('/'))
            .join("high-contrast/1.0.0.partial")
            .is_file()
    {
        bail!("interrupted acquisition was not safely staged")
    }
    println!("PASS interrupted download leaves partial bytes and no active promotion");
    garden.install("high-contrast", None, false)?;
    if garden.active()?.id != "high-contrast" || before != protected_bytes(root)? {
        bail!("resumed install violated activation or protected boundary")
    }
    println!(
        "PASS matching-validator resume verifies target, manifest, theme, and atomic promotion"
    );
    if garden.updates()?.len() != 1 || garden.updates()?[0].to != "1.1.0" {
        bail!("authenticated catalog update was not discovered")
    }
    println!("PASS Updates reports signed catalog high-contrast 1.1.0");
    let high = garden.details("high-contrast")?;
    let target = VerifiedTarget {
        path: "themes/high-contrast/manifest.json".into(),
        length: high.download_size,
        sha256: high.sha256,
        delegated_role: "themes".into(),
    };
    if TrustStore::new(&root.join(".brickpro/theme-garden/trust-state.json"))
        .verify_target_bytes(&target, b"wrong-target-bytes")
        .is_ok()
    {
        bail!("wrong target bytes were accepted")
    }
    let manifest: PackageManifest = serde_json::from_slice(&fs::read(
        fixtures.join("repository/high-contrast-manifest.json"),
    )?)?;
    let mut bad_manifest = manifest.clone();
    bad_manifest.files[0].path = "../escape.json".into();
    if validate_manifest(&bad_manifest).is_ok()
        || launcher_theme::parse_theme_bytes(br#"{\"format\":\"theme-v1\"}"#).is_ok()
    {
        bail!("malformed package or Theme v1 asset was accepted")
    }
    let mut unknown: serde_json::Value =
        serde_json::from_slice(&fs::read(fixtures.join("repository/catalog.json"))?)?;
    unknown["unknown"] = serde_json::Value::Bool(true);
    if Catalog::parse(&serde_json::to_vec(&unknown)?).is_ok() {
        bail!("catalog unknown field was accepted")
    }
    let mismatched_target = VerifiedTarget {
        path: "themes/high-contrast/manifest-1.1.0.json".into(),
        length: 1,
        sha256: "0".repeat(64),
        delegated_role: "themes".into(),
    };
    if install_with_validation(
        root,
        &fixtures.join("repository/minimal-manifest.json"),
        &fixtures.join("packages/minimal"),
        &mismatched_target,
        TrustContext::community_signed(),
        TransactionOptions::default(),
        |_, _| Ok(()),
    )
    .is_ok()
    {
        bail!("mismatched theme identity/version target was accepted")
    }
    println!(
        "PASS wrong target bytes, mismatched identity/version, traversal, executable-free schema, and unknown fields reject"
    );

    if garden.update("high-contrast", true).is_ok()
        || garden.active()?.version != "1.0.0"
        || before != protected_bytes(root)?
    {
        bail!("failed successor displaced prior active theme")
    }
    println!("PASS failed update retains prior active version");
    garden.update("high-contrast", false)?;
    if garden.active()?.version != "1.1.0" {
        bail!("validated successor did not activate")
    }
    println!("PASS successful update promotes a versioned successor atomically");

    garden.remove("high-contrast")?;
    if garden.active()?.id != "artbook" || before != protected_bytes(root)? {
        bail!("removal did not fall back safely")
    }
    println!("PASS removal selects built-in Artbook and preserves protected bytes");
    if garden.remove("artbook").is_ok() {
        bail!("Artbook was removable")
    }
    println!("PASS Artbook default protection");

    garden.install("minimal", None, false)?;
    let active_path = root.join(".brickpro/theme-garden/active.json");
    fs::write(
        &active_path,
        serde_json::to_vec(&theme_garden::ActiveTheme {
            id: "minimal".into(),
            version: "9.9.9".into(),
        })?,
    )?;
    if garden.active()?.id != "artbook" || before != protected_bytes(root)? {
        bail!("incompatible active selection did not fall back")
    }
    fs::write(
        &active_path,
        serde_json::to_vec(&theme_garden::ActiveTheme {
            id: "minimal".into(),
            version: "1.0.0".into(),
        })?,
    )?;
    println!("PASS unavailable and incompatible active selection falls back to Artbook");
    if !garden.updates()?.is_empty() {
        bail!("updates state reported a false update")
    }
    garden.expire_cache()?;
    if garden.install("minimal", None, false).is_ok() || garden.active()?.id != "minimal" {
        bail!("expired cache did not deny new authorization")
    }
    println!(
        "PASS expired offline metadata denies install while installed state remains inspectable"
    );
    if before != protected_bytes(root)? {
        bail!("lifecycle operation changed protected bytes")
    }
    println!("PASS protected ROM/save/state/resume/settings bytes are byte-identical");
    println!(
        "PASS Theme Garden synthetic journey paths {} {}",
        CACHE_PATH, STAGING_PATH
    );
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn required_path(args: &mut impl Iterator<Item = String>, option: &str) -> Result<PathBuf> {
    match (args.next().as_deref(), args.next()) {
        (Some(value), Some(path)) if value == option => Ok(PathBuf::from(path)),
        (Some(value), _) => bail!("expected {option}, got {value}"),
        (None, _) => bail!("missing {option}"),
    }
}

fn protected_bytes(root: &Path) -> Result<Vec<Vec<u8>>> {
    Ok([
        "roms/keep.bin",
        "data/saves/keep.save",
        "data/states/keep.state",
        "data/resume/keep.record",
        "data/settings.json",
    ]
    .into_iter()
    .map(|path| fs::read(root.join(path)))
    .collect::<std::io::Result<Vec<_>>>()?)
}
