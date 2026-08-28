use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use anyhow::{bail, Context, Result};
use package_manager::{
    install, load_manifest, uninstall, upgrade, validate_manifest, TransactionOptions,
};
use serde_json::Value;

fn main() {
    if let Err(error) = run() {
        eprintln!("package-manager failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("demo") => {
            let fixtures = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("fixtures/packages"));
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            demo(&fixtures)
        }
        _ => bail!("usage: package-manager demo [fixtures/packages]"),
    }
}

fn demo(fixtures: &Path) -> Result<()> {
    let manifest_path = fixtures.join("payload/manifest.json");
    let payload_root = fixtures.join("payload");
    let root = env::temp_dir().join(format!("brickpro-package-demo-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    for (relative, bytes) in [
        ("roms/keep.txt", b"generated-rom-boundary".as_slice()),
        ("data/saves/keep.sav", b"generated-save-boundary".as_slice()),
        (
            "data/states/keep.state",
            b"generated-state-boundary".as_slice(),
        ),
        (
            "data/resume/keep.record",
            b"generated-resume-boundary".as_slice(),
        ),
        (
            "data/settings.json",
            b"generated-settings-boundary".as_slice(),
        ),
        (
            ".brickpro/save-vault/keep.record",
            b"generated-save-vault-boundary".as_slice(),
        ),
    ] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().context("protected path has no parent")?)?;
        fs::write(path, bytes)?;
    }
    let protected_before = protected_bytes(&root)?;
    let blocked = fixtures.join("payload/blocked-core-pack-manifest.json");
    if install(
        &root,
        &blocked,
        &payload_root,
        TransactionOptions::default(),
    )
    .is_ok()
    {
        bail!("blocked core-pack was installed")
    }
    println!("PASS blocked core-pack rejected before installation");
    let activation = install(
        &root,
        &manifest_path,
        &payload_root,
        TransactionOptions::default(),
    )?;
    if activation.id != "demo-theme"
        || !root
            .join(".brickpro/package-state/demo-theme.json")
            .is_file()
    {
        bail!("install did not create activation")
    }
    if protected_before != protected_bytes(&root)? {
        bail!("install changed protected data")
    }
    println!(
        "PASS safe install promoted {} {}",
        activation.id, activation.version
    );

    let (manifest, _) = load_manifest(&manifest_path)?;
    let mut update = manifest;
    update.version = "1.1.0".into();
    let update_path = root.join("demo-theme-update.json");
    fs::write(&update_path, serde_json::to_vec_pretty(&update)?)?;
    if upgrade(
        &root,
        &update_path,
        &payload_root,
        TransactionOptions {
            interrupt_after_files: Some(1),
            interrupt_after_removals: None,
        },
    )
    .is_ok()
        || serde_json::from_slice::<Value>(&fs::read(
            root.join(".brickpro/package-state/demo-theme.json"),
        )?)?["version"]
            != "1.0.0"
    {
        bail!("interrupted update did not retain prior activation")
    }
    let upgraded = upgrade(
        &root,
        &update_path,
        &payload_root,
        TransactionOptions::default(),
    )?;
    if upgraded.version != "1.1.0" || root.join(".brickpro/packages/demo-theme/1.0.0").exists() {
        bail!("update did not promote exactly one active version")
    }
    println!("PASS interrupted update retains prior activation; update promotes 1.1.0");

    uninstall(&root, "demo-theme", TransactionOptions::default())?;
    if root
        .join(".brickpro/package-state/demo-theme.json")
        .exists()
        || root.join(".brickpro/packages/demo-theme").exists()
    {
        bail!("uninstall left package activation")
    }
    if protected_before != protected_bytes(&root)? {
        bail!("uninstall changed protected data")
    }
    println!("PASS uninstall preserves protected data");

    if install(
        &root,
        &manifest_path,
        &payload_root,
        TransactionOptions {
            interrupt_after_files: Some(1),
            interrupt_after_removals: None,
        },
    )
    .is_ok()
        || root
            .join(".brickpro/package-state/demo-theme.json")
            .exists()
    {
        bail!("interrupted install activated a partial package")
    }
    println!("PASS interrupted install leaves no activation");
    install(
        &root,
        &manifest_path,
        &payload_root,
        TransactionOptions::default(),
    )?;
    if uninstall(
        &root,
        "demo-theme",
        TransactionOptions {
            interrupt_after_files: None,
            interrupt_after_removals: Some(0),
        },
    )
    .is_ok()
    {
        bail!("interrupted uninstall unexpectedly succeeded")
    }
    uninstall(&root, "demo-theme", TransactionOptions::default())?;
    println!("PASS interrupted uninstall preserves protected data");

    let (_, _) = load_manifest(&manifest_path)?;
    let mut bad_path: package_manager::PackageManifest =
        serde_json::from_slice(&fs::read(&manifest_path)?)?;
    bad_path.files[0].path = "../escape.json".into();
    if validate_manifest(&bad_path).is_ok() {
        bail!("traversal path was accepted")
    }
    let mut raw: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    raw["capabilities"]["network"] = Value::Array(vec![Value::String("raw-shell".into())]);
    if serde_json::from_value::<package_manager::PackageManifest>(raw).is_ok() {
        bail!("unsupported capability was accepted")
    }
    println!("PASS bad archive path, executable-free themes, and unsupported fields reject");
    if protected_before != protected_bytes(&root)? {
        bail!("package lifecycle changed protected data")
    }
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn protected_bytes(root: &Path) -> Result<Vec<Vec<u8>>> {
    Ok([
        "roms/keep.txt",
        "data/saves/keep.sav",
        "data/states/keep.state",
        "data/resume/keep.record",
        "data/settings.json",
        ".brickpro/save-vault/keep.record",
    ]
    .into_iter()
    .map(|path| fs::read(root.join(path)))
    .collect::<std::io::Result<Vec<_>>>()?)
}
