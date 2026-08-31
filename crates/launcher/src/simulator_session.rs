use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use launch_contract::{validate, Catalog, LaunchRequest};
use save_sync::{
    Candidate, CandidateStatus, Device, Exchange, Lineage, ResolutionAction, SaveTarget, SyncGate,
    SyncReconciler,
};
use save_vault::{
    Catalog as SaveCatalog, Identity as SaveIdentity, SaveKind, SaveVault, SnapshotFile,
    SnapshotReason,
};
use session_broker::resume::{
    CheckpointReason, CommitFault, ResumeCapabilityConfig, ResumeDecision, ResumeRecord,
    ResumeResult, ResumeStore, ResumeSummary,
};
use session_broker::{
    accepted_handle, BrokerError, SaveVaultPreview, SaveVaultSummary, SessionBrokerClient,
    SessionHandle, SessionResult,
};
use sha2::Digest;

const CAPABILITIES: &[u8] =
    include_bytes!("../../../fixtures/session-broker/generated-v1/resume-capabilities.json");

pub(crate) struct SimulatorSessionAdapter {
    active: Option<(SessionHandle, LaunchRequest)>,
    store: ResumeStore,
    vault: SaveVault,
    source_root: PathBuf,
    sync_exchange: PathBuf,
    sync_gate: SyncGate,
}

impl Default for SimulatorSessionAdapter {
    fn default() -> Self {
        Self::with_root(std::env::temp_dir().join(format!("trimui-resume-{}", std::process::id())))
    }
}

impl SimulatorSessionAdapter {
    pub(crate) fn with_root(root: PathBuf) -> Self {
        let config = ResumeCapabilityConfig::parse(CAPABILITIES).expect("generated resume config");
        fs::create_dir_all(root.join("live/saves")).expect("save source directory");
        fs::create_dir_all(root.join("live/states")).expect("state source directory");
        fs::write(
            root.join("live/saves/active.save"),
            b"synthetic-live-save-v1",
        )
        .expect("save source");
        fs::write(
            root.join("live/states/active.state"),
            b"synthetic-live-state-v1",
        )
        .expect("state source");
        for path in [
            root.join("live"),
            root.join("live/saves"),
            root.join("live/states"),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o777)).expect("save source mode");
        }
        for path in [
            root.join("live/saves/active.save"),
            root.join("live/states/active.state"),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).expect("save source mode");
        }
        let source_root = root.join("live");
        let vault = SaveVault::for_simulator(
            root.join("save-vault"),
            &source_root,
            SaveCatalog::default(),
        )
        .expect("save vault");
        vault
            .snapshot(
                SaveIdentity {
                    content_version: "sim-content",
                    runner_version: "sim-runner",
                    core_version: None,
                },
                &[SnapshotFile {
                    kind: SaveKind::Save,
                    relative: "saves/active.save".into(),
                    source: source_root.join("saves/active.save"),
                }],
                SnapshotReason::NormalExit,
            )
            .expect("initial save snapshot");
        let sync_exchange = root.join("save-sync");
        let exchange = Exchange::new(&sync_exchange).expect("save sync exchange");
        let reconciler = SyncReconciler::new(
            &vault,
            exchange,
            SaveTarget {
                logical_id: "generated-save".into(),
                content_id: "sim-content".into(),
                relative: "saves/active.save".into(),
                kind: SaveKind::Save,
            },
        )
        .expect("save sync reconciler");
        let local = reconciler
            .local_candidate(Device {
                id: "brick-sim".into(),
                name: "Simulator Brick".into(),
            })
            .expect("local save candidate");
        let payload = b"synthetic-remote-save-v1";
        let remote = Candidate {
            schema: save_sync::SCHEMA.into(),
            format: save_sync::FORMAT.into(),
            schema_version: 1,
            logical_id: local.logical_id.clone(),
            content_id: local.content_id.clone(),
            device: Device {
                id: "brick-peer".into(),
                name: "Peer Brick".into(),
            },
            generation: 2,
            hash: format!("{:x}", sha2::Sha256::digest(payload)),
            lineage: Lineage {
                parent_hash: None,
                ancestry: vec![],
            },
            save_kind: SaveKind::Save,
            timestamp_ms: local.timestamp_ms + 1,
            size: payload.len() as u64,
            validator: None,
            status: CandidateStatus::Candidate,
            deleted: false,
        };
        reconciler
            .exchange()
            .stage_remote(remote, payload, false)
            .expect("remote save candidate");
        Self {
            active: None,
            store: ResumeStore::for_simulator(root.join("resume"), config)
                .expect("resume store")
                .to_owned(),
            vault,
            source_root,
            sync_exchange,
            sync_gate: SyncGate::Ready,
        }
    }

    fn checkpoint_active(
        &self,
        request: &LaunchRequest,
        reason: CheckpointReason,
        fault: CommitFault,
    ) -> Result<ResumeRecord, BrokerError> {
        let record = self
            .store
            .checkpoint(
                request,
                reason,
                format!("synthetic-state:{}", request.request_id).as_bytes(),
                b"synthetic-sram-v1",
                b"synthetic-resume-screenshot-v1",
                fault,
            )
            .map_err(|error| BrokerError::new(error.to_string()))?;
        let files = [
            SnapshotFile {
                kind: SaveKind::Save,
                relative: "saves/active.save".into(),
                source: self.source_root.join("saves/active.save"),
            },
            SnapshotFile {
                kind: SaveKind::State,
                relative: "states/active.state".into(),
                source: self.source_root.join("states/active.state"),
            },
        ];
        self.vault
            .snapshot(
                SaveIdentity {
                    content_version: &request.content_sha256,
                    runner_version: &request.runner.version,
                    core_version: request.core.as_ref().map(|core| core.version.as_str()),
                },
                &files,
                SnapshotReason::NormalExit,
            )
            .map_err(|error| BrokerError::new(error.to_string()))?;
        Ok(record)
    }
}

impl SessionBrokerClient for SimulatorSessionAdapter {
    fn submit(
        &mut self,
        request: LaunchRequest,
        catalog: &Catalog,
    ) -> Result<SessionHandle, BrokerError> {
        validate(&request, catalog).map_err(|error| BrokerError::new(error.to_string()))?;
        if self.active.is_some() {
            return Err(BrokerError::new("broker is busy"));
        }
        let handle = accepted_handle(&request);
        self.sync_gate = SyncGate::Gameplay;
        self.active = Some((handle.clone(), request));
        Ok(handle)
    }

    fn complete(&mut self, exit_code: i32, duration_ms: u64) -> Result<SessionResult, BrokerError> {
        let (_, request) = self
            .active
            .as_ref()
            .ok_or_else(|| BrokerError::new("no-active-session"))?;
        let published = exit_code == 0
            && self
                .checkpoint_active(request, CheckpointReason::NormalExit, CommitFault::None)
                .is_ok();
        let (_, request) = self.active.take().expect("active session checked");
        self.sync_gate = SyncGate::Ready;
        Ok(SessionResult {
            result_type: "SessionResult",
            journey: "simulator".into(),
            accepted: true,
            runner: Some(request.runner.id),
            core: request.core.map(|core| core.id),
            reason: if exit_code == 0 {
                "success"
            } else {
                "nonzero-exit"
            }
            .into(),
            duration_ms,
            restored: true,
            safe_default: false,
            persistence_status: if published {
                "durable"
            } else {
                "not-applicable"
            },
            resume_published: published,
            exit_code: Some(exit_code),
            signal: None,
        })
    }

    fn checkpoint(
        &mut self,
        reason: CheckpointReason,
        fault: CommitFault,
    ) -> Result<ResumeRecord, BrokerError> {
        if reason == CheckpointReason::PreSuspend {
            self.sync_gate = SyncGate::SaveFlush;
        }
        let (_, request) = self
            .active
            .as_ref()
            .ok_or_else(|| BrokerError::new("no-active-session"))?;
        let result = self.checkpoint_active(request, reason, fault);
        if reason == CheckpointReason::PreSuspend && result.is_ok() {
            self.sync_gate = SyncGate::Ready;
        }
        result
    }

    fn resume_entries(
        &mut self,
        requests: &[LaunchRequest],
    ) -> Result<Vec<ResumeSummary>, BrokerError> {
        Ok(self.store.list(requests))
    }

    fn resume_choices(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<Vec<ResumeDecision>, BrokerError> {
        Ok(self.store.choices(request))
    }

    fn resume_decision(
        &mut self,
        request: LaunchRequest,
        decision: ResumeDecision,
    ) -> Result<ResumeResult, BrokerError> {
        self.store
            .decide(&request, decision)
            .map_err(|error| BrokerError::new(error.to_string()))
    }

    fn resume_delete(
        &mut self,
        request: LaunchRequest,
        generation: u64,
        confirmed: bool,
    ) -> Result<(), BrokerError> {
        self.store
            .delete(&request, generation, confirmed)
            .map_err(|error| BrokerError::new(error.to_string()))
    }

    fn save_vault_history(&mut self) -> Result<Vec<SaveVaultSummary>, BrokerError> {
        Ok(self
            .vault
            .history()
            .into_iter()
            .map(|manifest| SaveVaultSummary {
                generation: manifest.generation,
                artifact_count: manifest.artifacts.len(),
                protected: manifest.retention == save_vault::RetentionClass::Protected,
            })
            .collect())
    }

    fn save_vault_preview(&mut self) -> Result<SaveVaultPreview, BrokerError> {
        let generation = self
            .vault
            .current_generation()
            .ok_or_else(|| BrokerError::new("save vault has no current generation"))?;
        let preview = self
            .vault
            .preview(generation)
            .map_err(|error| BrokerError::new(error.to_string()))?;
        Ok(SaveVaultPreview {
            generation: preview.generation,
            runner_version: preview.runner_version,
            core_version: preview.core_version,
            old_size: preview.old_size,
            new_size: preview.new_size,
            old_hash_status: preview.old_hash_status,
            new_hash_status: preview.new_hash_status,
            old_hash_prefix: preview.old_hash_prefix,
            new_hash_prefix: preview.new_hash_prefix,
            affected_kinds: preview.affected_kinds,
            reason: format!("{:?}", preview.reason).to_ascii_lowercase(),
            timestamp_ms: preview.timestamp_ms,
        })
    }

    fn save_sync_status(&mut self) -> Result<save_sync::SyncStatus, BrokerError> {
        let reconciler = self.sync_reconciler()?;
        let local = reconciler
            .local_candidate(Device {
                id: "brick-sim".into(),
                name: "Simulator Brick".into(),
            })
            .map_err(|error| BrokerError::new(error.to_string()))?;
        let remote = reconciler
            .exchange()
            .quarantined()
            .map_err(|error| BrokerError::new(error.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| BrokerError::new("no quarantined save candidate"))?;
        reconciler
            .reconcile(&local, &remote, self.sync_gate)
            .map_err(|error| BrokerError::new(error.to_string()))
    }

    fn save_sync_resolve(
        &mut self,
        action: ResolutionAction,
    ) -> Result<save_sync::ResolutionReceipt, BrokerError> {
        let reconciler = self.sync_reconciler()?;
        let local = reconciler
            .local_candidate(Device {
                id: "brick-sim".into(),
                name: "Simulator Brick".into(),
            })
            .map_err(|error| BrokerError::new(error.to_string()))?;
        let remote = reconciler
            .exchange()
            .quarantined()
            .map_err(|error| BrokerError::new(error.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| BrokerError::new("no quarantined save candidate"))?;
        reconciler
            .resolve(&local, &remote, action)
            .map_err(|error| BrokerError::new(error.to_string()))
    }

    fn save_vault_restore(&mut self, confirmed: bool) -> Result<(), BrokerError> {
        let generation = self
            .vault
            .current_generation()
            .ok_or_else(|| BrokerError::new("save vault has no current generation"))?;
        self.vault
            .restore(generation, confirmed)
            .map(|_| ())
            .map_err(|error| BrokerError::new(error.to_string()))
    }
}

impl SimulatorSessionAdapter {
    fn sync_reconciler(&self) -> Result<SyncReconciler<'_>, BrokerError> {
        SyncReconciler::new(
            &self.vault,
            Exchange::new(&self.sync_exchange)
                .map_err(|error| BrokerError::new(error.to_string()))?,
            SaveTarget {
                logical_id: "generated-save".into(),
                content_id: "sim-content".into(),
                relative: "saves/active.save".into(),
                kind: SaveKind::Save,
            },
        )
        .map_err(|error| BrokerError::new(error.to_string()))
    }
}
