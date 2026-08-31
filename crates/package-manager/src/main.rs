use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use anyhow::{bail, Context, Result};
use package_manager::{
    bounded_log, install, install_for_device, load_manifest, package_status, preflight,
    set_enabled, simple_launcher_visible, uninstall, upgrade, validate_manifest, DeviceProfile,
    PackageStatus, TransactionOptions,
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
    fs::write(
        root.join(".brickpro/packages/demo-theme/1.0.0/writable/preferences.json"),
        b"user preference",
    )?;
    if package_status(
        &root,
        &load_manifest(&manifest_path)?.0,
        &DeviceProfile::brick_pro(),
    ) != PackageStatus::Installed
    {
        bail!("installed package did not project its UI state")
    }

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
    if upgraded.version != "1.1.0"
        || root.join(".brickpro/packages/demo-theme/1.0.0").exists()
        || fs::read(root.join(".brickpro/packages/demo-theme/1.1.0/writable/preferences.json"))?
            != b"user preference"
    {
        bail!("update did not preserve declared user data")
    }
    println!("PASS interrupted update retains prior activation; update preserves user data");

    uninstall(&root, "demo-theme", TransactionOptions::default())?;
    if root
        .join(".brickpro/package-state/demo-theme.json")
        .exists()
        || root.join(".brickpro/packages/demo-theme").exists()
    {
        bail!("uninstall left package activation")
    }
    if protected_before != protected_bytes(&root)?
        || fs::read(root.join("data/packages/demo-theme/preferences.json"))? != b"user preference"
    {
        bail!("uninstall did not retain declared user data")
    }
    println!("PASS uninstall preserves protected and declared user data");

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

    let (manifest, _) = load_manifest(&manifest_path)?;
    let mut bad_path = manifest.clone();
    bad_path.files[0].path = "../escape.json".into();
    if validate_manifest(&bad_path).is_ok() {
        bail!("traversal path was accepted")
    }
    let mut wrong_sku = DeviceProfile::brick_pro();
    wrong_sku.sku = "TG5050".into();
    let mut wrong_abi = DeviceProfile::brick_pro();
    wrong_abi.abi = "armv7-unknown-linux-musleabihf".into();
    let mut missing_library = manifest.clone();
    missing_library
        .dependencies
        .push(package_manager::Dependency {
            id: "libcurl".into(),
            version: "8.7.1".into(),
        });
    let missing_library_path = root.join("missing-library.json");
    fs::write(&missing_library_path, serde_json::to_vec(&missing_library)?)?;
    let mut no_space = DeviceProfile::brick_pro();
    no_space.free_bytes = 1;
    for device in [&wrong_sku, &wrong_abi, &no_space] {
        if install_for_device(
            &root,
            &manifest_path,
            &payload_root,
            device,
            TransactionOptions::default(),
        )
        .is_ok()
        {
            bail!("incompatible package reached activation")
        }
    }
    if install_for_device(
        &root,
        &missing_library_path,
        &payload_root,
        &DeviceProfile::brick_pro(),
        TransactionOptions::default(),
    )
    .is_ok()
        || preflight(&manifest, &wrong_sku).ready()
        || package_status(&root, &manifest, &wrong_sku) != PackageStatus::Incompatible
    {
        bail!("package preflight did not block incompatible activation")
    }
    install(
        &root,
        &manifest_path,
        &payload_root,
        TransactionOptions::default(),
    )?;
    set_enabled(&root, "demo-theme", false)?;
    if simple_launcher_visible(&root, "demo-theme")
        || bounded_log((0..40).map(|line| "x".repeat(line + 1))).len() != 32
    {
        bail!("disabled package remained visible or package log was unbounded")
    }
    set_enabled(&root, "demo-theme", true)?;
    uninstall(&root, "demo-theme", TransactionOptions::default())?;
    println!("PASS preflight rejects SKU ABI dependency space and traversal; disabled package hides safely");
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
