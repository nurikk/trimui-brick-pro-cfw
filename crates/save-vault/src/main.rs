use std::{env, fs, os::unix::fs::PermissionsExt, path::Path};

use save_vault::{
    Catalog, Identity, RestorePreview, SaveKind, SaveKindPolicy, SaveVault, SnapshotFile,
    SnapshotReason,
};
use sha2::{Digest, Sha256};

fn main() {
    if let Err(error) = journey() {
        eprintln!("save-vault journey failed: {error}");
        std::process::exit(1);
    }
}

fn journey() -> Result<(), String> {
    let root = env::temp_dir().join(format!("trimui-save-vault-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("live/saves")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join("live/states")).map_err(|e| e.to_string())?;
    fs::write(root.join("live/saves/slot.sav"), b"old-save-bytes").map_err(|e| e.to_string())?;
    fs::write(root.join("live/states/slot.state"), b"old-state-bytes")
        .map_err(|e| e.to_string())?;
    let source = root.join("live");
    let vault_root = root.join("vault");
    let catalog = Catalog::default().with_policy(
        SaveKind::DeclaredState,
        SaveKindPolicy {
            allow_empty: true,
            max_shrink_percent: 75,
        },
    );
    let vault =
        SaveVault::for_simulator(&vault_root, &source, catalog).map_err(|e| e.to_string())?;
    let files = files(&source);
    let identity = Identity {
        content_version: "content-v1",
        runner_version: "runner-v1",
        core_version: Some("core-v1"),
    };
    let first = vault
        .snapshot(identity.clone(), &files, SnapshotReason::NormalExit)
        .map_err(|e| e.to_string())?;
    if !first.committed || first.generation != 1 {
        return Err("initial generation was not atomically published".into());
    }
    check_mode(&vault_root, 0o777, 0o644)?;
    println!("PASS simulator policy 0777/0644 and committed generation");

    fs::write(source.join("live-placeholder"), b"outside-boundary").map_err(|e| e.to_string())?;
    if vault
        .snapshot(
            identity.clone(),
            &[SnapshotFile {
                kind: SaveKind::Save,
                relative: "saves/../escape.sav".into(),
                source: source.join("live-placeholder"),
            }],
            SnapshotReason::NormalExit,
        )
        .is_ok()
    {
        return Err("traversal was accepted".into());
    }
    if vault
        .snapshot(
            identity.clone(),
            &[SnapshotFile {
                kind: SaveKind::Save,
                relative: "roms/game.sav".into(),
                source: source.join("live-placeholder"),
            }],
            SnapshotReason::NormalExit,
        )
        .is_ok()
    {
        return Err("ROM boundary was accepted".into());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source.join("saves/slot.sav"), source.join("saves/link.sav"))
            .map_err(|e| e.to_string())?;
        if vault
            .snapshot(identity.clone(), &files, SnapshotReason::NormalExit)
            .is_ok()
        {
            return Err("source symlink was accepted".into());
        }
        fs::remove_file(source.join("saves/link.sav")).map_err(|e| e.to_string())?;
    }
    println!("PASS traversal, ROM/BIOS, and symlink rejection");

    fs::write(source.join("saves/slot.sav"), b"new-save-bytes").map_err(|e| e.to_string())?;
    let second = vault
        .snapshot(identity.clone(), &files, SnapshotReason::PreCoreChange)
        .map_err(|e| e.to_string())?;
    if second.status != save_vault::AnomalyStatus::Valid {
        return Err("valid changed save was quarantined".into());
    }
    fs::write(source.join("saves/slot.sav"), b"").map_err(|e| e.to_string())?;
    let zero = vault
        .snapshot(identity.clone(), &files, SnapshotReason::NormalExit)
        .map_err(|e| e.to_string())?;
    if zero.status != save_vault::AnomalyStatus::Quarantined || zero.committed {
        return Err("zero-length save was published as current".into());
    }
    fs::write(source.join("saves/slot.sav"), b"x").map_err(|e| e.to_string())?;
    let before_failure = vault
        .current_generation()
        .ok_or("current pointer missing")?;
    let object_hash = format!("{:x}", Sha256::digest(b"x"));
    let object = vault_root
        .join("objects")
        .join(format!("{object_hash}.bin"));
    fs::create_dir_all(&object).map_err(|e| e.to_string())?;
    if vault
        .snapshot(identity.clone(), &files, SnapshotReason::NormalExit)
        .is_ok()
    {
        return Err("publication storage failure was accepted".into());
    }
    if vault.current_generation() != Some(before_failure) {
        return Err("publication failure changed current pointer".into());
    }
    fs::remove_dir_all(&object).map_err(|e| e.to_string())?;
    let anomaly = vault
        .snapshot(identity.clone(), &files, SnapshotReason::NormalExit)
        .map_err(|e| e.to_string())?;
    if anomaly.status != save_vault::AnomalyStatus::Quarantined || anomaly.committed {
        return Err("shrink anomaly was published as current".into());
    }
    if vault
        .history()
        .iter()
        .all(|item| item.generation != second.generation)
    {
        return Err("known-good generation was lost".into());
    }
    fs::write(source.join("saves/slot.sav"), b"new-save-bytes").map_err(|e| e.to_string())?;
    println!("PASS zero/shrink anomaly quarantine preserves last known good");
    println!("PASS staged/publication failure keeps current generation");
    let second_object_hash = format!("{:x}", Sha256::digest(b"new-save-bytes"));
    let second_object = vault_root
        .join("objects")
        .join(format!("{second_object_hash}.bin"));
    let original_object = fs::read(&second_object).map_err(|e| e.to_string())?;
    fs::write(&second_object, b"corrupt").map_err(|e| e.to_string())?;
    if vault.preview(second.generation).is_ok() {
        return Err("corrupt object was accepted".into());
    }
    fs::write(&second_object, original_object).map_err(|e| e.to_string())?;
    println!("PASS corrupt generation is ignored and prior current remains");

    let stable = vault
        .snapshot(identity.clone(), &files, SnapshotReason::PreUpdate)
        .map_err(|e| e.to_string())?;
    if !stable.committed || stable.status != save_vault::AnomalyStatus::Valid {
        return Err(format!(
            "retention baseline was not valid: {:?}",
            stable.status
        ));
    }
    for _ in 0..12 {
        let result = vault
            .snapshot(identity.clone(), &files, SnapshotReason::NormalExit)
            .map_err(|e| e.to_string())?;
        if !result.committed {
            return Err(format!(
                "retention fixture did not commit generation {} status {:?}",
                result.generation, result.status
            ));
        }
    }
    let history = vault.history();
    let protected_count = history
        .iter()
        .filter(|item| item.retention == save_vault::RetentionClass::Protected)
        .count();
    if history
        .iter()
        .filter(|item| item.retention == save_vault::RetentionClass::Recent)
        .count()
        > save_vault::MAX_GENERATIONS + 1
        || history.len() > save_vault::MAX_GENERATIONS + protected_count + 1
        || protected_count == 0
        || history.iter().any(|item| {
            item.parent_generation.is_some_and(|parent| {
                !history
                    .iter()
                    .any(|candidate| candidate.generation == parent)
            })
        })
    {
        return Err("retention was not bounded with protected points and parents".into());
    }
    println!("PASS deterministic retention keeps recent, protected, and parent generations");
    concurrency_journey(&root)?;

    fs::write(source.join("saves/slot.sav"), b"changed-save-x").map_err(|e| e.to_string())?;
    let preview = vault
        .preview(second.generation)
        .map_err(|e| e.to_string())?;
    if preview.old_hash_status != "verified"
        || preview.content_version != "content-v1"
        || preview.new_size == 0
        || preview.old_hash_prefix.len() != 12
        || preview.new_hash_prefix.len() != 12
        || preview.affected_kinds != vec![SaveKind::Save, SaveKind::State]
    {
        return Err("restore preview is incomplete".into());
    }
    let live_before_cancel = fs::read(source.join("saves/slot.sav")).map_err(|e| e.to_string())?;
    if vault.restore(second.generation, false).is_ok() {
        return Err("restore without confirmation succeeded".into());
    }
    if fs::read(source.join("saves/slot.sav")).map_err(|e| e.to_string())? != live_before_cancel {
        return Err("cancel changed live bytes".into());
    }
    println!("PASS restore preview requires explicit confirmation and cancel is unchanged");

    let mut controller = Controller::new(&vault, second.generation)?;
    for button in [
        Button::History,
        Button::Preview,
        Button::Confirm,
        Button::Restore,
    ] {
        controller.press(button)?;
    }
    let presentation = controller.presentation();
    for forbidden in [
        "sha256:",
        "/",
        "old-save-bytes",
        "old-state-bytes",
        "content-v1",
    ] {
        if presentation.contains(forbidden) {
            return Err(format!("presentation exposed {forbidden}"));
        }
    }
    if fs::read(source.join("saves/slot.sav")).map_err(|e| e.to_string())? != b"new-save-bytes" {
        return Err("confirmed restore did not promote verified replacement".into());
    }
    println!("PASS controller buttons history/preview/confirm/restore are sanitized");

    let blocked = root.join("protected-failure");
    fs::create_dir_all(blocked.join("data/saves")).map_err(|e| e.to_string())?;
    fs::create_dir_all(blocked.join("data/states")).map_err(|e| e.to_string())?;
    fs::write(blocked.join("data/saves/a.sav"), b"").map_err(|e| e.to_string())?;
    fs::write(blocked.join("data/states/a.state"), b"state").map_err(|e| e.to_string())?;
    if SaveVault::snapshot_standard(&blocked, SnapshotReason::PreUpdate).is_ok() {
        return Err("protected anomaly was allowed to continue".into());
    }
    if fs::read(blocked.join("data/saves/a.sav")).map_err(|e| e.to_string())? != b"" {
        return Err("protected snapshot failure changed live data".into());
    }
    println!("PASS protected pre-operation anomaly blocks replacement");

    let production = root.join("production");
    fs::create_dir_all(production.join("data/saves")).map_err(|e| e.to_string())?;
    fs::create_dir_all(production.join("data/states")).map_err(|e| e.to_string())?;
    fs::write(production.join("data/saves/a.sav"), b"a").map_err(|e| e.to_string())?;
    fs::write(production.join("data/states/a.state"), b"b").map_err(|e| e.to_string())?;
    let outcome = SaveVault::snapshot_standard(&production, SnapshotReason::PreUpdate)
        .map_err(|e| e.to_string())?;
    if !outcome.committed {
        return Err("production standard snapshot did not commit".into());
    }
    check_mode(&production.join(".brickpro/save-vault"), 0o700, 0o600)?;
    println!("PASS production policy 0700/0600 and pre-update snapshot");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

fn concurrency_journey(root: &Path) -> Result<(), String> {
    let root = root.join("concurrency");
    fs::create_dir_all(root.join("live/saves")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join("live/states")).map_err(|e| e.to_string())?;
    fs::write(root.join("live/saves/slot.sav"), b"concurrent-save").map_err(|e| e.to_string())?;
    fs::write(root.join("live/states/slot.state"), b"concurrent-state")
        .map_err(|e| e.to_string())?;
    let source = root.join("live");
    let vault_a = SaveVault::for_simulator(root.join("vault"), &source, Catalog::default())
        .map_err(|e| e.to_string())?;
    let vault_b = SaveVault::for_simulator(root.join("vault"), &source, Catalog::default())
        .map_err(|e| e.to_string())?;
    let files = files(&source);
    let (left, right) = std::thread::scope(|scope| -> Result<_, String> {
        let left = scope.spawn(|| {
            vault_a.snapshot(
                Identity {
                    content_version: "concurrent",
                    runner_version: "runner",
                    core_version: None,
                },
                &files,
                SnapshotReason::PreUpdate,
            )
        });
        let right = scope.spawn(|| {
            vault_b.snapshot(
                Identity {
                    content_version: "concurrent",
                    runner_version: "runner",
                    core_version: None,
                },
                &files,
                SnapshotReason::PrePackage,
            )
        });
        Ok((
            left.join()
                .map_err(|_| "left snapshot thread panicked".to_string())?,
            right
                .join()
                .map_err(|_| "right snapshot thread panicked".to_string())?,
        ))
    })?;
    let left = left.map_err(|e| e.to_string())?;
    let right = right.map_err(|e| e.to_string())?;
    if !left.committed || !right.committed || left.generation == right.generation {
        return Err("concurrent snapshots reused or lost a generation".into());
    }
    let history = vault_a.history();
    if history.len() != 2
        || vault_a.current_generation() != Some(left.generation.max(right.generation))
        || history.iter().any(|manifest| {
            manifest.parent_generation.is_some_and(|parent| {
                !history
                    .iter()
                    .any(|candidate| candidate.generation == parent)
            })
        })
    {
        return Err("concurrent publication corrupted current history".into());
    }
    println!("PASS concurrent snapshots serialize generation and parent publication");
    Ok(())
}

fn files(source: &Path) -> Vec<SnapshotFile> {
    vec![
        SnapshotFile {
            kind: SaveKind::Save,
            relative: "saves/slot.sav".into(),
            source: source.join("saves/slot.sav"),
        },
        SnapshotFile {
            kind: SaveKind::State,
            relative: "states/slot.state".into(),
            source: source.join("states/slot.state"),
        },
    ]
}

fn check_mode(root: &Path, directory: u32, file: u32) -> Result<(), String> {
    for path in [
        root.to_path_buf(),
        root.join("objects"),
        root.join("generations"),
        root.join("quarantine"),
        root.join(".staging"),
    ] {
        let mode = fs::metadata(path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o7777;
        if mode != directory {
            return Err(format!("directory mode {mode:o} != {directory:o}"));
        }
    }
    let current = root.join("current.json");
    if current.exists()
        && fs::metadata(current)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o7777
            != file
    {
        return Err("pointer file mode is invalid".into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Button {
    History,
    Preview,
    Confirm,
    Restore,
}
struct Controller<'a> {
    vault: &'a SaveVault,
    generation: u64,
    screen: &'static str,
    confirmed: bool,
    preview: Option<RestorePreview>,
}
impl<'a> Controller<'a> {
    fn new(vault: &'a SaveVault, generation: u64) -> Result<Self, String> {
        Ok(Self {
            vault,
            generation,
            screen: "home",
            confirmed: false,
            preview: None,
        })
    }
    fn press(&mut self, button: Button) -> Result<(), String> {
        match button {
            Button::History => self.screen = "history",
            Button::Preview => {
                self.preview = Some(
                    self.vault
                        .preview(self.generation)
                        .map_err(|e| e.to_string())?,
                );
                self.screen = "preview";
            }
            Button::Confirm => {
                if self.preview.is_none() {
                    return Err("confirmation before preview".into());
                }
                self.confirmed = true;
                self.screen = "confirm";
            }
            Button::Restore => {
                if !self.confirmed {
                    return Err("restore before confirmation".into());
                }
                self.vault
                    .restore(self.generation, true)
                    .map_err(|e| e.to_string())?;
                self.screen = "restored";
            }
        }
        Ok(())
    }
    fn presentation(&self) -> String {
        format!(
            "screen={} generation={} oldSize={} newSize={} oldHashPrefix={} newHashPrefix={} hashStatus={} runner={} core={:?} affectedKinds={:?} reason={:?} timestampMs={}",
            self.screen,
            self.generation,
            self.preview.as_ref().map_or(0, |p| p.old_size),
            self.preview.as_ref().map_or(0, |p| p.new_size),
            self.preview.as_ref().map_or("hidden", |p| p.old_hash_prefix.as_str()),
            self.preview.as_ref().map_or("hidden", |p| p.new_hash_prefix.as_str()),
            self.preview
                .as_ref()
                .map_or("hidden", |p| p.old_hash_status.as_str()),
            self.preview
                .as_ref()
                .map_or("hidden", |p| p.runner_version.as_str()),
            self.preview.as_ref().and_then(|p| p.core_version.as_deref()),
            self.preview
                .as_ref()
                .map_or_else(|| "hidden".into(), |p| format!("{:?}", p.affected_kinds)),
            self.preview
                .as_ref()
                .map_or(SnapshotReason::NormalExit, |p| p.reason),
            self.preview.as_ref().map_or(0, |p| p.timestamp_ms)
        )
    }
}
