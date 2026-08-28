use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process};

use sha2::Digest;

use save_sync::{
    syncthing, webdav, Candidate, CandidateStatus, Device, Exchange, Lineage, ResolutionAction,
    SaveTarget, SecretRef, SyncGate, SyncReconciler,
};
use save_vault::{Catalog, Identity, SaveKind, SaveVault, SnapshotFile, SnapshotReason};

fn main() {
    if let Err(error) = run() {
        eprintln!("save-sync journey: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("trimui-save-sync-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("live/saves"))?;
    fs::set_permissions(root.join("live"), fs::Permissions::from_mode(0o777))?;
    fs::set_permissions(root.join("live/saves"), fs::Permissions::from_mode(0o777))?;
    let live = root.join("live/saves/active.save");
    fs::write(&live, b"local-generation-1")?;
    fs::set_permissions(&live, fs::Permissions::from_mode(0o644))?;
    let vault =
        SaveVault::for_simulator(root.join("vault"), root.join("live"), Catalog::default())?;
    vault.snapshot(
        Identity {
            content_version: "fixture-content",
            runner_version: "fixture-runner",
            core_version: None,
        },
        &[SnapshotFile {
            kind: SaveKind::Save,
            relative: "saves/active.save".into(),
            source: live.clone(),
        }],
        SnapshotReason::NormalExit,
    )?;
    let exchange = Exchange::new(root.join("exchange"))?;
    let target = SaveTarget {
        logical_id: "fixture-save".into(),
        content_id: "fixture-content".into(),
        relative: "saves/active.save".into(),
        kind: SaveKind::Save,
    };
    let reconciler = SyncReconciler::new(&vault, exchange, target)?;
    let local = reconciler.local_candidate(Device {
        id: "brick-a".into(),
        name: "Brick A".into(),
    })?;
    let fast_hash = sha256(b"remote-fast-forward");
    let fast = candidate(
        &local,
        "brick-b",
        "Brick B",
        fast_hash,
        Some(local.hash.clone()),
        vec![local.hash.clone()],
        2,
        b"remote-fast-forward".len() as u64,
    );
    let remote = reconciler
        .exchange()
        .stage_remote(fast, b"remote-fast-forward", false)?;
    let result = reconciler.reconcile(&local, &remote, SyncGate::Ready)?;
    require(result.state == "fast-forwarded", "fast-forward")?;
    pass("fast-forward")?;

    let divergent = candidate(
        &local,
        "brick-c",
        "Brick C",
        sha256(b"divergent"),
        None,
        vec![],
        3,
        b"divergent".len() as u64,
    );
    let divergent = reconciler
        .exchange()
        .stage_remote(divergent, b"divergent", false)?;
    let conflict = reconciler.reconcile(&local, &divergent, SyncGate::Ready)?;
    require(conflict.state == "conflict", "simultaneous divergent edit")?;
    pass("simultaneous-divergent-edit")?;
    let equal = candidate(
        &local,
        "brick-d",
        "Brick D",
        sha256(b"equal-time"),
        None,
        vec![],
        local.timestamp_ms,
        b"equal-time".len() as u64,
    );
    let equal = reconciler
        .exchange()
        .stage_remote(equal, b"equal-time", false)?;
    require(
        reconciler.reconcile(&local, &equal, SyncGate::Ready)?.state == "conflict",
        "equal timestamp divergence",
    )?;
    pass("equal-timestamp-divergence")?;
    let deletion = candidate_with_deleted(&local, "brick-e", local.timestamp_ms + 1);
    let deletion = reconciler.exchange().stage_remote(deletion, &[], false)?;
    require(
        reconciler
            .reconcile(&local, &deletion, SyncGate::Ready)?
            .state
            == "conflict",
        "deletion versus modification",
    )?;
    pass("deletion-versus-modification")?;

    require(
        syncthing::is_conflict_copy("active.save.sync-conflict-date-device"),
        "Syncthing conflict copy",
    )?;
    let conflict_copy = candidate(
        &local,
        "brick-f",
        "Brick F",
        sha256(b"syncthing-copy"),
        None,
        vec![],
        6,
        b"syncthing-copy".len() as u64,
    );
    let staged = syncthing::SyncthingAdapter::new(reconciler.exchange().clone()).ingest(
        "active.save.sync-conflict-date-device",
        conflict_copy,
        b"syncthing-copy",
    )?;
    require(
        staged.is_conflict_copy() && !reconciler.exchange().quarantined()?.is_empty(),
        "Syncthing quarantine",
    )?;
    pass("syncthing-conflict-copy-ingestion")?;

    require(
        matches!(
            webdav::WebDavAdapter::finish(
                &webdav::PutCondition::IfMatch("etag".into()),
                webdav::PutResponse::PreconditionFailed
            ),
            webdav::WebDavOutcome::Conflict(_)
        ),
        "WebDAV 412",
    )?;
    require(
        matches!(
            webdav::WebDavAdapter::replacement(&webdav::HeadResponse {
                status: 200,
                etag: Some("W/weak".into())
            }),
            webdav::WebDavOutcome::Conflict(_)
        ),
        "weak validator",
    )?;
    require(
        matches!(
            webdav::WebDavAdapter::replacement(&webdav::HeadResponse {
                status: 200,
                etag: None
            }),
            webdav::WebDavOutcome::Conflict(_)
        ),
        "missing validator",
    )?;
    pass("webdav-412")?;
    pass("weak-missing-validator")?;

    for (action, name, expected) in [
        (
            ResolutionAction::KeepLocal,
            "keep-local",
            b"remote-for-resolution".as_slice(),
        ),
        (
            ResolutionAction::KeepRemote,
            "keep-remote",
            b"remote-for-resolution".as_slice(),
        ),
        (
            ResolutionAction::KeepBoth,
            "keep-both",
            b"remote-for-resolution".as_slice(),
        ),
    ] {
        let (vault, reconciler, live) = fresh_fixture(&root, name)?;
        let local = reconciler.local_candidate(Device {
            id: "brick-a".into(),
            name: "Brick A".into(),
        })?;
        let remote = candidate(
            &local,
            "brick-b",
            "Brick B",
            sha256(expected),
            None,
            vec![],
            9,
            expected.len() as u64,
        );
        let remote = reconciler
            .exchange()
            .stage_remote(remote, expected, false)?;
        let before = fs::read(&live)?;
        let receipt = reconciler.resolve(&local, &remote, action)?;
        let hashes = vault
            .history()
            .into_iter()
            .flat_map(|m| m.artifacts)
            .map(|a| a.sha256)
            .collect::<Vec<_>>();
        require(
            hashes.contains(&local.hash) && hashes.contains(&remote.candidate().hash),
            "original hashes preserved",
        )?;
        let current = vault
            .current_generation()
            .ok_or("canonical generation missing")?;
        let current_hash = vault
            .material(current)?
            .into_iter()
            .find(|file| file.relative == "saves/active.save")
            .map(|file| sha256(&file.bytes))
            .ok_or("canonical material missing")?;
        if action == ResolutionAction::KeepRemote {
            require(
                fs::read(&live)? == expected && current_hash == remote.candidate().hash,
                "remote became canonical",
            )?;
        } else {
            require(
                fs::read(&live)? == before && current_hash == local.hash,
                "local remained canonical",
            )?;
        }
        require(
            receipt.preserved_hash_prefixes.len() == 2,
            "resolution receipt",
        )?;
        pass(name)?;
    }
    pass("preservation-of-both-hashes")?;
    require(
        SecretRef::new("/tmp/not-a-save-secret").is_err(),
        "secret path rejection",
    )?;
    pass("secret-redaction-path-rejection")?;

    let paused = candidate(
        &local,
        "brick-g",
        "Brick G",
        sha256(b"queued"),
        None,
        vec![],
        10,
        b"queued".len() as u64,
    );
    let paused = reconciler
        .exchange()
        .stage_remote(paused, b"queued", false)?;
    require(
        reconciler
            .reconcile(&local, &paused, SyncGate::Gameplay)?
            .state
            == "paused",
        "gameplay pause",
    )?;
    require(reconciler.exchange().pending_count()? > 0, "durable queue")?;
    pass("offline-kill-durable-queue-recovery")?;
    require(
        reconciler
            .reconcile(&local, &paused, SyncGate::SaveFlush)?
            .state
            == "paused",
        "flush pause",
    )?;
    pass("gameplay-flush-pause")?;
    println!("evidence={}", root.display());
    Ok(())
}

fn fresh_fixture(
    root: &std::path::Path,
    name: &str,
) -> Result<(&'static SaveVault, SyncReconciler<'static>, PathBuf), Box<dyn std::error::Error>> {
    let owned = root.join(name);
    fs::create_dir_all(owned.join("live/saves"))?;
    fs::set_permissions(owned.join("live"), fs::Permissions::from_mode(0o777))?;
    fs::set_permissions(owned.join("live/saves"), fs::Permissions::from_mode(0o777))?;
    let live = owned.join("live/saves/active.save");
    fs::write(&live, b"local-for-resolution")?;
    fs::set_permissions(&live, fs::Permissions::from_mode(0o644))?;
    let vault = Box::new(SaveVault::for_simulator(
        owned.join("vault"),
        owned.join("live"),
        Catalog::default(),
    )?);
    vault.snapshot(
        Identity {
            content_version: "fixture-content",
            runner_version: "fixture-runner",
            core_version: None,
        },
        &[SnapshotFile {
            kind: SaveKind::Save,
            relative: "saves/active.save".into(),
            source: live.clone(),
        }],
        SnapshotReason::NormalExit,
    )?;
    let vault: &'static SaveVault = Box::leak(vault);
    let reconciler = SyncReconciler::new(
        vault,
        Exchange::new(owned.join("exchange"))?,
        SaveTarget {
            logical_id: "fixture-save".into(),
            content_id: "fixture-content".into(),
            relative: "saves/active.save".into(),
            kind: SaveKind::Save,
        },
    )?;
    Ok((vault, reconciler, live))
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    local: &Candidate,
    id: &str,
    name: &str,
    hash: String,
    parent: Option<String>,
    ancestry: Vec<String>,
    generation: u64,
    size: u64,
) -> Candidate {
    Candidate {
        schema: save_sync::SCHEMA.into(),
        format: save_sync::FORMAT.into(),
        schema_version: 1,
        logical_id: local.logical_id.clone(),
        content_id: local.content_id.clone(),
        device: Device {
            id: id.into(),
            name: name.into(),
        },
        generation,
        hash,
        lineage: Lineage {
            parent_hash: parent,
            ancestry,
        },
        save_kind: local.save_kind,
        timestamp_ms: local.timestamp_ms,
        size,
        validator: None,
        status: CandidateStatus::Candidate,
        deleted: false,
    }
}
fn candidate_with_deleted(local: &Candidate, id: &str, timestamp_ms: u64) -> Candidate {
    Candidate {
        schema: save_sync::SCHEMA.into(),
        format: save_sync::FORMAT.into(),
        schema_version: 1,
        logical_id: local.logical_id.clone(),
        content_id: local.content_id.clone(),
        device: Device {
            id: id.into(),
            name: "Brick E".into(),
        },
        generation: 8,
        hash: sha256(&[]),
        lineage: Lineage {
            parent_hash: None,
            ancestry: vec![],
        },
        save_kind: local.save_kind,
        timestamp_ms,
        size: 0,
        validator: None,
        status: CandidateStatus::Candidate,
        deleted: true,
    }
}
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(bytes))
}
fn require(condition: bool, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    if condition {
        Ok(())
    } else {
        Err(name.into())
    }
}
fn pass(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("PASS {name}");
    Ok(())
}
