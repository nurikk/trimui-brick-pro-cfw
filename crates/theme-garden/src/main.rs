use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use anyhow::{bail, Context, Result};
use launcher_theme::DirectCatalogTransport;
use package_manager::{validate_manifest, PackageManifest};
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
            let device_profile = required_path(&mut args, "--device-profile")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            journey(
                &fixtures,
                &root,
                device_profile::DeviceProfile::from_path(&device_profile)?,
            )
        }
        Some("parse") => {
            let path = required_path(&mut args, "--catalog")?;
            let device_profile = required_path(&mut args, "--device-profile")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            let device = device_profile::DeviceProfile::from_path(&device_profile)?;
            println!(
                "PASS parsed {} catalog entries",
                Catalog::parse(&fs::read(path)?, &device)?.themes.len()
            );
            Ok(())
        }
        Some("source-install") => {
            let fixtures = required_path(&mut args, "--fixtures")?;
            let root = required_path(&mut args, "--root")?;
            let catalog = required_path(&mut args, "--catalog")?;
            let id = required_value(&mut args, "--id")?;
            let device_profile = required_path(&mut args, "--device-profile")?;
            if args.next().is_some() { bail!("unexpected argument") }
            let garden = ThemeGarden::load(
                &root,
                &fixtures,
                device_profile::DeviceProfile::from_path(&device_profile)?,
            )?;
            let active = garden.install_upstream_source(
                &fs::read(catalog)?,
                &id,
                &DirectCatalogTransport,
                None,
                false,
            )?;
            println!("PASS installed {} {}", active.id, active.version);
            Ok(())
        }
        _ => bail!("usage: theme-garden journey --fixtures PATH --root SYNTHETIC_ROOT --device-profile PATH | parse --catalog PATH --device-profile PATH | source-install --fixtures PATH --root PATH --catalog PATH --id ID --device-profile PATH"),
    }
}

fn journey(fixtures: &Path, root: &Path, device: device_profile::DeviceProfile) -> Result<()> {
    if root.as_os_str().is_empty() || root == Path::new("/") {
        bail!("journey requires a caller-provided synthetic root")
    }
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
        (
            "data/resume/keep.record",
            b"synthetic-resume-boundary".as_slice(),
        ),
        (
            "data/settings.json",
            b"synthetic-settings-boundary".as_slice(),
        ),
    ] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().context("protected path has no parent")?)?;
        fs::write(path, bytes)?;
    }
    let before = protected_bytes(root)?;
    let garden = ThemeGarden::load(root, fixtures, device.clone())?;
    let catalog_bytes = fs::read(fixtures.join("themes.json"))?;
    let selected =
        garden.select_themes_json(&catalog_bytes, "minimal", "1.0.0", &fixtures.join("themes"))?;
    if selected.name().is_empty()
        || ThemeGarden::from_cache(root, fixtures, device.clone())?
            .browse()
            .len()
            != 3
    {
        bail!("offline cache could not be browsed")
    }
    let cache = root.join(CACHE_PATH.trim_start_matches('/'));
    let catalog = fs::read(cache.join("catalog.json"))?;
    let metadata = fs::read(cache.join("metadata.json"))?;
    let mut changed: serde_json::Value = serde_json::from_slice(&metadata)?;
    changed["catalogVersion"] = serde_json::Value::String("9.9.9".into());
    fs::write(cache.join("metadata.json"), serde_json::to_vec(&changed)?)?;
    if ThemeGarden::from_cache(root, fixtures, device.clone()).is_ok() {
        bail!("tampered cache was accepted")
    }
    fs::write(cache.join("catalog.json"), catalog)?;
    fs::write(cache.join("metadata.json"), metadata)?;
    println!("PASS local catalog cache detects metadata tampering");
    if garden.browse().len() != 3 {
        bail!("browse did not expose all catalog entries")
    }
    println!("PASS themes.json adapter selected a local theme without network access");
    println!("PASS Browse exposes 3 project-authored entries");
    for entry in garden.browse() {
        let detail = garden.details(&entry.id)?;
        if detail.target_sku != device.target_sku() || detail.sha256.len() != 64 {
            bail!("incomplete detail record")
        }
        if !detail.screenshots_available {
            bail!("incomplete detail record")
        }
    }
    let active_before = garden.active()?;
    let preview = garden.preview("high-contrast")?;
    if !preview.is_file() || garden.active()?.id != active_before.id {
        bail!("preview changed active theme")
    }
    if garden.installed()?.is_empty() || !garden.updates()?.is_empty() {
        bail!("initial installed or updates state is incorrect")
    }
    if garden.controller_flow()?.cache_state != "available" {
        bail!("catalog flow has an invalid cache state")
    }
    println!("PASS browse, details, preview, installed, and updates are available");

    if garden.install("high-contrast", Some(8), false).is_ok()
        || garden.active()?.id != "artbook"
        || !root
            .join(STAGING_PATH.trim_start_matches('/'))
            .join("high-contrast/1.0.0.partial")
            .is_file()
    {
        bail!("interrupted acquisition was not safely staged")
    }
    garden.install("high-contrast", None, false)?;
    if garden.active()?.id != "high-contrast" || before != protected_bytes(root)? {
        bail!("install violated activation or protected boundary")
    }
    if garden.updates()?.len() != 1 || garden.updates()?[0].to != "1.1.0" {
        bail!("catalog update was not discovered")
    }
    println!("PASS interrupted download resumes and update is discovered");

    let manifest: PackageManifest = serde_json::from_slice(&fs::read(
        fixtures.join("repository/high-contrast-manifest.json"),
    )?)?;
    let mut bad_manifest = manifest.clone();
    bad_manifest.files[0].path = "../escape.json".into();
    if validate_manifest(&bad_manifest).is_ok()
        || launcher_theme::parse_theme_bytes(br#"{\"format\":\"theme-v1\"}"#).is_ok()
    {
        bail!("malformed package or theme was accepted")
    }
    let mut unknown: serde_json::Value =
        serde_json::from_slice(&fs::read(fixtures.join("repository/catalog.json"))?)?;
    unknown["unknown"] = serde_json::Value::Bool(true);
    if Catalog::parse(&serde_json::to_vec(&unknown)?, &device).is_ok() {
        bail!("catalog unknown field was accepted")
    }
    println!("PASS bad archive path, invalid theme data, and unknown fields reject");

    if garden.update("high-contrast", true).is_ok()
        || garden.active()?.version != "1.0.0"
        || before != protected_bytes(root)?
    {
        bail!("failed successor displaced prior active theme")
    }
    garden.update("high-contrast", false)?;
    garden.remove("high-contrast")?;
    if garden.active()?.id != "artbook" || before != protected_bytes(root)? {
        bail!("removal did not fall back safely")
    }
    if garden.remove("artbook").is_ok() {
        bail!("Artbook was removable")
    }
    garden.install("minimal", None, false)?;
    let active_path = root.join(".brickpro/theme-garden/active.json");
    fs::write(
        &active_path,
        serde_json::to_vec(&theme_garden::ActiveTheme {
            id: "minimal".into(),
            version: "9.9.9".into(),
        })?,
    )?;
    if garden.active()?.id != "artbook" {
        bail!("incompatible active selection did not fall back")
    }
    println!("PASS failed update, atomic activation, removal, and bundled fallback");
    if before != protected_bytes(root)? {
        bail!("theme lifecycle changed protected bytes")
    }
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
fn required_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String> {
    match (args.next().as_deref(), args.next()) {
        (Some(value), Some(item)) if value == option => Ok(item),
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
