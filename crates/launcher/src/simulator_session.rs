use std::path::PathBuf;

use launch_contract::{validate, Catalog, LaunchRequest};
use session_broker::resume::{
    CheckpointReason, CommitFault, ResumeCapabilityConfig, ResumeDecision, ResumeRecord,
    ResumeResult, ResumeStore, ResumeSummary,
};
use session_broker::{
    accepted_handle, BrokerError, SessionBrokerClient, SessionHandle, SessionResult,
};

const CAPABILITIES: &[u8] =
    include_bytes!("../../../fixtures/session-broker/generated-v1/resume-capabilities.json");

pub(crate) struct SimulatorSessionAdapter {
    active: Option<(SessionHandle, LaunchRequest)>,
    store: ResumeStore,
}

impl Default for SimulatorSessionAdapter {
    fn default() -> Self {
        Self::with_root(std::env::temp_dir().join(format!("trimui-resume-{}", std::process::id())))
    }
}

impl SimulatorSessionAdapter {
    pub(crate) fn with_root(root: PathBuf) -> Self {
        let config = ResumeCapabilityConfig::parse(CAPABILITIES).expect("generated resume config");
        Self {
            active: None,
            store: ResumeStore::new(root.join("resume"), config)
                .expect("resume store")
                .to_owned(),
        }
    }

    fn checkpoint_active(
        &self,
        request: &LaunchRequest,
        reason: CheckpointReason,
        fault: CommitFault,
    ) -> Result<ResumeRecord, BrokerError> {
        self.store
            .checkpoint(
                request,
                reason,
                format!("synthetic-state:{}", request.request_id).as_bytes(),
                b"synthetic-sram-v1",
                b"synthetic-resume-screenshot-v1",
                fault,
            )
            .map_err(|error| BrokerError::new(error.to_string()))
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
}
