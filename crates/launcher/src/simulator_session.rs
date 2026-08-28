use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use launch_contract::{validate, Catalog, LaunchRequest};
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

const CAPABILITIES: &[u8] =
    include_bytes!("../../../fixtures/session-broker/generated-v1/resume-capabilities.json");

pub(crate) struct SimulatorSessionAdapter {
    active: Option<(SessionHandle, LaunchRequest)>,
    store: ResumeStore,
    vault: SaveVault,
    source_root: PathBuf,
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
        Self {
            active: None,
            store: ResumeStore::for_simulator(root.join("resume"), config)
                .expect("resume store")
                .to_owned(),
            vault: SaveVault::for_simulator(
                root.join("save-vault"),
                root.join("live"),
                SaveCatalog::default(),
            )
            .expect("save vault"),
            source_root: root.join("live"),
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
        let (_, request) = self
            .active
            .as_ref()
            .ok_or_else(|| BrokerError::new("no-active-session"))?;
        self.checkpoint_active(request, reason, fault)
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
            runner_version: format!("generated-{}", preview.runner_version),
            core_version: preview.core_version,
            old_size: preview.old_size,
            new_size: preview.new_size,
            old_hash_status: preview.old_hash_status,
            new_hash_status: preview.new_hash_status,
            reason: format!("{:?}", preview.reason).to_ascii_lowercase(),
            timestamp_ms: preview.timestamp_ms,
        })
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
