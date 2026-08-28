use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use anyhow::{bail, Result};
use package_manager::{
    install, load_manifest, uninstall, upgrade, validate_manifest, TransactionOptions, TrustContext,
};
use package_trust::{
    RecoveryStatus, RepositoryMetadata, TrustStore, TrustedMetadataState, VerificationTime,
    VerifiedTarget,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

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
    let repository = fixtures.join("repository");
    let manifest_path = fixtures.join("payload/manifest.json");
    let payload_root = fixtures.join("payload");
    let target_path = "packages/demo-theme/manifest.json";
    let root = unique_temp("brickpro-package-demo");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("roms"))?;
    fs::create_dir_all(root.join("data/saves"))?;
    fs::create_dir_all(root.join("data/states"))?;
    fs::create_dir_all(root.join("data/resume"))?;
    fs::create_dir_all(root.join(".brickpro/save-vault"))?;
    fs::write(root.join("roms/keep.txt"), b"generated-rom-boundary")?;
    fs::write(root.join("data/saves/keep.sav"), b"generated-save-boundary")?;
    fs::write(
        root.join("data/states/keep.state"),
        b"generated-state-boundary",
    )?;
    fs::write(
        root.join("data/resume/keep.record"),
        b"generated-resume-boundary",
    )?;
    fs::write(
        root.join("data/settings.json"),
        b"generated-settings-boundary",
    )?;
    fs::write(
        root.join(".brickpro/save-vault/keep.record"),
        b"generated-save-vault-boundary",
    )?;
    let protected_before = protected_bytes(&root)?;
    let state = root.join(".brickpro/trust-state.json");
    let report = verify_fixture(&repository, &state, target_path)?;
    println!("PASS signed delegated package target progression");

    let (manifest, _) = load_manifest(&manifest_path)?;
    let blocked_manifest = fixtures.join("payload/blocked-core-pack-manifest.json");
    if install(
        &root,
        &blocked_manifest,
        &payload_root,
        &report.target,
        TrustContext::community_signed(),
        TransactionOptions::default(),
    )
    .is_ok()
        || root
            .join(".brickpro/package-state/tg4040-stable-core-pack.json")
            .exists()
        || protected_before != protected_bytes(&root)?
    {
        bail!("blocked core-pack was installed or changed protected data")
    }
    println!("PASS blocked core-pack rejected before installation");
    let activation = install(
        &root,
        &manifest_path,
        &payload_root,
        &report.target,
        TrustContext::community_signed(),
        TransactionOptions::default(),
    )?;
    if !root
        .join(".brickpro/package-state/demo-theme.json")
        .is_file()
    {
        bail!("install did not create activation")
    }
    println!(
        "PASS install promoted immutable activation {} {}",
        activation.id, activation.version
    );
    if protected_before != protected_bytes(&root)? {
        bail!("install changed protected data")
    }
    let prior_activation = fs::read(root.join(".brickpro/package-state/demo-theme.json"))?;
    let mut update = manifest.clone();
    update.version = "1.1.0".to_string();
    let update_path = root.join("demo-theme-update.json");
    let update_bytes = serde_json::to_vec_pretty(&update)?;
    fs::write(&update_path, &update_bytes)?;
    let update_target = VerifiedTarget {
        path: target_path.to_string(),
        length: update_bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&update_bytes)),
        delegated_role: "packages".to_string(),
    };
    if upgrade(
        &root,
        &update_path,
        &payload_root,
        &update_target,
        TrustContext::community_signed(),
        TransactionOptions {
            interrupt_after_files: Some(1),
            interrupt_after_removals: None,
        },
    )
    .is_ok()
        || fs::read(root.join(".brickpro/package-state/demo-theme.json"))? != prior_activation
        || protected_before != protected_bytes(&root)?
    {
        bail!("interrupted update did not retain the prior activation")
    }
    let upgraded = upgrade(
        &root,
        &update_path,
        &payload_root,
        &update_target,
        TrustContext::community_signed(),
        TransactionOptions::default(),
    )?;
    if upgraded.version != "1.1.0"
        || root.join(".brickpro/packages/demo-theme/1.0.0").exists()
        || protected_before != protected_bytes(&root)?
    {
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
    println!("PASS uninstall preserves ROM/save/state/resume/settings/Save Vault bytes");

    let interrupted = install(
        &root,
        &manifest_path,
        &payload_root,
        &report.target,
        TrustContext::community_signed(),
        TransactionOptions {
            interrupt_after_files: Some(1),
            interrupt_after_removals: None,
        },
    );
    if interrupted.is_ok()
        || root
            .join(".brickpro/package-state/demo-theme.json")
            .exists()
    {
        bail!("interrupted install activated a partial package")
    }
    if protected_before != protected_bytes(&root)? {
        bail!("interrupted install changed protected data")
    }
    println!("PASS interrupted install leaves no activation");

    install(
        &root,
        &manifest_path,
        &payload_root,
        &report.target,
        TrustContext::community_signed(),
        TransactionOptions::default(),
    )?;
    let interrupted_uninstall = uninstall(
        &root,
        "demo-theme",
        TransactionOptions {
            interrupt_after_files: None,
            interrupt_after_removals: Some(0),
        },
    );
    if interrupted_uninstall.is_ok() || protected_before != protected_bytes(&root)? {
        bail!("interrupted uninstall violated preservation")
    }
    uninstall(&root, "demo-theme", TransactionOptions::default())?;
    println!("PASS interrupted uninstall preserves protected data");

    let valid_target = fs::read(&manifest_path)?;
    let retry_state = root.join(".brickpro/retry-state.json");
    let mut corrupt_target = valid_target.clone();
    corrupt_target.push(b'!');
    expect_status(
        verify_fixture_with_target(&repository, &retry_state, target_path, &corrupt_target),
        RecoveryStatus::CorruptTrustedState,
        "corrupt target retry",
    )?;
    if retry_state.exists() {
        bail!("corrupt target created trusted state")
    }
    let retry_report =
        verify_fixture_with_target(&repository, &retry_state, target_path, &valid_target)?;
    let state_after_retry = fs::read(&retry_state)?;
    let retry_again =
        verify_fixture_with_target(&repository, &retry_state, target_path, &valid_target)?;
    if retry_report.root_version != 1
        || retry_again.root_version != 1
        || state_after_retry != fs::read(&retry_state)?
    {
        bail!("valid target retry did not publish exactly once")
    }
    println!("PASS corrupt target leaves state unchanged and valid retry publishes once");

    let publication_state = root.join(".brickpro/publication-state.json");
    verify_fixture(&repository, &publication_state, target_path)?;
    let state_before_failure = fs::read(&publication_state)?;
    expect_status(
        verify_fixture_with_store(
            &repository,
            &TrustStore::new(&publication_state).with_publication_failure(),
            target_path,
            &valid_target,
            None,
            0,
        ),
        RecoveryStatus::CorruptTrustedState,
        "interrupted state publication",
    )?;
    if state_before_failure != fs::read(&publication_state)? {
        bail!("publication failure changed prior trusted state")
    }
    println!("PASS interrupted state publication preserves prior state");

    let mut bad_path = manifest.clone();
    bad_path.files[0].path = "../escape.json".to_string();
    expect_error(
        validate_manifest(&bad_path),
        "capability/path manifest rejection",
    )?;
    let mut case_collision = manifest.clone();
    let mut duplicate = case_collision.files[0].clone();
    duplicate.path = "immutable/THEME.JSON".to_string();
    case_collision.files.push(duplicate);
    expect_error(
        validate_manifest(&case_collision),
        "case collision rejection",
    )?;
    let mut raw: Value = serde_json::from_slice(&valid_target)?;
    raw["capabilities"]["network"] = Value::Array(vec![Value::String("raw-shell".to_string())]);
    if serde_json::from_value::<package_manager::PackageManifest>(raw).is_ok() {
        bail!("unsupported capability was accepted")
    }
    println!("PASS capability, traversal, and case-collision rejection");

    let unsigned_root = without_signatures(&fs::read(repository.join("root.json"))?)?;
    let unsigned_state = root.join(".brickpro/unsigned-state.json");
    expect_status(
        verify_fixture_with_root(&repository, &unsigned_state, target_path, &unsigned_root),
        RecoveryStatus::SignatureFailure,
        "unsigned metadata rejection",
    )?;
    println!("PASS unsigned metadata rejection");

    expect_status(
        verify_fixture_with_target_path(
            &repository,
            &root.join(".brickpro/scope-state.json"),
            "other/manifest.json",
            &valid_target,
        ),
        RecoveryStatus::SignatureFailure,
        "delegated scope rejection",
    )?;
    println!("PASS delegated scope rejection");

    let mut bad_snapshot = fs::read(repository.join("snapshot.json"))?;
    bad_snapshot.push(b'!');
    expect_status(
        verify_fixture_with_store(
            &repository,
            &TrustStore::new(&root.join(".brickpro/freeze-state.json")),
            target_path,
            &valid_target,
            Some(&bad_snapshot),
            0,
        ),
        RecoveryStatus::Freeze,
        "freeze/integrity rejection",
    )?;
    println!("PASS freeze/integrity rejection");

    expect_status(
        verify_fixture_with_store(
            &repository,
            &TrustStore::new(&root.join(".brickpro/clock-state.json")),
            target_path,
            &valid_target,
            None,
            301,
        ),
        RecoveryStatus::ClockUncertain,
        "clock uncertainty rejection",
    )?;
    println!("PASS clock uncertainty rejection");

    let rollback_state = root.join(".brickpro/rollback-state.json");
    fs::create_dir_all(rollback_state.parent().unwrap())?;
    fs::write(
        &rollback_state,
        serde_json::to_vec(&TrustedMetadataState {
            format: package_trust::TRUST_STATE_FORMAT.to_string(),
            schema_version: 1,
            root_version: 2,
            timestamp_version: 2,
            snapshot_version: 2,
            targets_version: 2,
            delegated: Default::default(),
        })?,
    )?;
    expect_status(
        verify_fixture(&repository, &rollback_state, target_path),
        RecoveryStatus::Rollback,
        "persisted rollback rejection",
    )?;
    println!("PASS rollback rejection");

    let expired = repository.join("expired-timestamp.json");
    expect_status(
        verify_fixture_with_timestamp(
            &repository,
            &root.join(".brickpro/expired-state.json"),
            target_path,
            &fs::read(expired)?,
        ),
        RecoveryStatus::Expired,
        "expired metadata rejection",
    )?;
    println!("PASS expired metadata rejection");

    #[cfg(unix)]
    {
        let symlink_state = root.join(".brickpro/symlink-state.json");
        symlink(root.join("roms/keep.txt"), &symlink_state)?;
        expect_status(
            verify_fixture(&repository, &symlink_state, target_path),
            RecoveryStatus::CorruptTrustedState,
            "symlinked state rejection",
        )?;
        let temp_link = root.join(".brickpro/state-temp-link");
        symlink(&publication_state, &temp_link)?;
        expect_status(
            verify_fixture_with_store(
                &repository,
                &TrustStore::new(&root.join(".brickpro/temp-state.json"))
                    .with_temp_path(&temp_link),
                target_path,
                &valid_target,
                None,
                0,
            ),
            RecoveryStatus::CorruptTrustedState,
            "symlinked temp rejection",
        )?;
        println!("PASS symlinked state and temp rejection");
    }

    let corrupt_state = root.join(".brickpro/corrupt-state.json");
    fs::write(&corrupt_state, b"not-json")?;
    expect_status(
        verify_fixture(&repository, &corrupt_state, target_path),
        RecoveryStatus::CorruptTrustedState,
        "corrupt trusted state rejection",
    )?;
    println!("PASS corrupt trusted state rejection");
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn verify_fixture(
    repository: &Path,
    state: &Path,
    target: &str,
) -> Result<package_trust::VerificationReport> {
    let target_bytes = fs::read(repository.parent().unwrap().join("payload/manifest.json"))?;
    verify_fixture_with_target(repository, state, target, &target_bytes)
}

fn verify_fixture_with_target(
    repository: &Path,
    state: &Path,
    target: &str,
    target_bytes: &[u8],
) -> Result<package_trust::VerificationReport> {
    verify_fixture_with_store(
        repository,
        &TrustStore::new(state),
        target,
        target_bytes,
        None,
        0,
    )
}

fn verify_fixture_with_root(
    repository: &Path,
    state: &Path,
    target: &str,
    root: &[u8],
) -> Result<package_trust::VerificationReport> {
    let target_bytes = fs::read(repository.parent().unwrap().join("payload/manifest.json"))?;
    verify_fixture_with_store_and_root(
        repository,
        &TrustStore::new(state),
        target,
        FixtureMetadata {
            root,
            timestamp: &fs::read(repository.join("timestamp.json"))?,
            target_bytes: &target_bytes,
            snapshot_override: None,
            uncertainty_seconds: 0,
        },
    )
}

fn verify_fixture_with_timestamp(
    repository: &Path,
    state: &Path,
    target: &str,
    timestamp: &[u8],
) -> Result<package_trust::VerificationReport> {
    let target_bytes = fs::read(repository.parent().unwrap().join("payload/manifest.json"))?;
    verify_fixture_with_store_and_root(
        repository,
        &TrustStore::new(state),
        target,
        FixtureMetadata {
            root: &fs::read(repository.join("root.json"))?,
            timestamp,
            target_bytes: &target_bytes,
            snapshot_override: None,
            uncertainty_seconds: 0,
        },
    )
}

fn verify_fixture_with_target_path(
    repository: &Path,
    state: &Path,
    target: &str,
    target_bytes: &[u8],
) -> Result<package_trust::VerificationReport> {
    verify_fixture_with_store(
        repository,
        &TrustStore::new(state),
        target,
        target_bytes,
        None,
        0,
    )
}

struct FixtureMetadata<'a> {
    root: &'a [u8],
    timestamp: &'a [u8],
    target_bytes: &'a [u8],
    snapshot_override: Option<&'a [u8]>,
    uncertainty_seconds: u64,
}

fn verify_fixture_with_store(
    repository: &Path,
    store: &TrustStore<'_>,
    target: &str,
    target_bytes: &[u8],
    snapshot_override: Option<&[u8]>,
    uncertainty_seconds: u64,
) -> Result<package_trust::VerificationReport> {
    verify_fixture_with_store_and_root(
        repository,
        store,
        target,
        FixtureMetadata {
            root: &fs::read(repository.join("root.json"))?,
            timestamp: &fs::read(repository.join("timestamp.json"))?,
            target_bytes,
            snapshot_override,
            uncertainty_seconds,
        },
    )
}

fn verify_fixture_with_store_and_root(
    repository: &Path,
    store: &TrustStore<'_>,
    target: &str,
    metadata: FixtureMetadata<'_>,
) -> Result<package_trust::VerificationReport> {
    let snapshot = metadata
        .snapshot_override
        .map(|bytes| bytes.to_vec())
        .unwrap_or(fs::read(repository.join("snapshot.json"))?);
    let targets = fs::read(repository.join("targets.json"))?;
    let delegated = fs::read(repository.join("packages.json"))?;
    let root_updates: [&[u8]; 0] = [];
    store
        .verify_repository(
            RepositoryMetadata {
                root_bytes: metadata.root,
                root_updates: &root_updates,
                timestamp_bytes: metadata.timestamp,
                snapshot_bytes: &snapshot,
                targets_bytes: &targets,
                delegated_role: "packages",
                delegated_bytes: &delegated,
                target_bytes: metadata.target_bytes,
            },
            target,
            VerificationTime {
                now_rfc3339: "2030-01-01T00:00:00Z",
                uncertainty_seconds: metadata.uncertainty_seconds,
            },
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn without_signatures(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(bytes)?;
    value["signatures"] = Value::Array(Vec::new());
    Ok(serde_json::to_vec(&value)?)
}

fn expect_status<T>(result: Result<T>, expected: RecoveryStatus, label: &str) -> Result<()> {
    match result {
        Err(error) if error.to_string().starts_with(expected.as_str()) => Ok(()),
        Err(error) => bail!("{label}: expected {}, got {error}", expected.as_str()),
        Ok(_) => bail!("{label}: unexpectedly succeeded"),
    }
}

fn expect_error<T>(result: Result<T>, label: &str) -> Result<()> {
    if result.is_ok() {
        bail!("{label}: unexpectedly succeeded")
    }
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

fn unique_temp(prefix: &str) -> PathBuf {
    env::temp_dir().join(format!("{prefix}-{}", std::process::id()))
}
