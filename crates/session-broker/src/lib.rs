pub mod resume;

use std::{fmt, time::Duration};

use launch_contract::{Catalog, LaunchRequest};
use save_vault::SaveKind;
use serde::Serialize;

pub const HANDLE_SCHEMA: &str = "trimui-session-broker-handle/v1";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveVaultSummary {
    pub generation: u64,
    pub artifact_count: usize,
    pub protected: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveVaultPreview {
    pub generation: u64,
    pub runner_version: String,
    pub core_version: Option<String>,
    pub old_size: u64,
    pub new_size: u64,
    pub old_hash_status: String,
    pub new_hash_status: String,
    pub old_hash_prefix: String,
    pub new_hash_prefix: String,
    pub affected_kinds: Vec<SaveKind>,
    pub reason: String,
    pub timestamp_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleCheckpointPolicy {
    periodic_interval: Duration,
}

impl Default for LifecycleCheckpointPolicy {
    fn default() -> Self {
        Self {
            periodic_interval: Duration::from_secs(30),
        }
    }
}

impl LifecycleCheckpointPolicy {
    pub const fn periodic_interval(self) -> Duration {
        self.periodic_interval
    }
}

pub type BrokerRequest = LaunchRequest;

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionHandle {
    pub schema: String,
    pub session_id: String,
    pub content_id: String,
    pub phase: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionResult {
    #[serde(rename = "type")]
    pub result_type: &'static str,
    pub journey: String,
    pub accepted: bool,
    pub runner: Option<String>,
    pub core: Option<String>,
    pub reason: String,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    pub restored: bool,
    #[serde(rename = "safeDefault")]
    pub safe_default: bool,
    #[serde(rename = "persistenceStatus")]
    pub persistence_status: &'static str,
    #[serde(rename = "resumePublished")]
    pub resume_published: bool,
    #[serde(skip_serializing_if = "Option::is_none", rename = "exitCode")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokerError(String);

impl BrokerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BrokerError {}

pub trait SessionBrokerClient {
    fn submit(
        &mut self,
        request: BrokerRequest,
        catalog: &Catalog,
    ) -> Result<SessionHandle, BrokerError>;

    fn complete(&mut self, exit_code: i32, duration_ms: u64) -> Result<SessionResult, BrokerError>;

    fn checkpoint(
        &mut self,
        _reason: resume::CheckpointReason,
        _fault: resume::CommitFault,
    ) -> Result<resume::ResumeRecord, BrokerError> {
        Err(BrokerError::new("resume checkpoint is unavailable"))
    }

    fn resume_entries(
        &mut self,
        requests: &[BrokerRequest],
    ) -> Result<Vec<resume::ResumeSummary>, BrokerError> {
        let _ = requests;
        Err(BrokerError::new("resume listing is unavailable"))
    }

    fn resume_choices(
        &mut self,
        _request: &BrokerRequest,
    ) -> Result<Vec<resume::ResumeDecision>, BrokerError> {
        Err(BrokerError::new("resume choices are unavailable"))
    }

    fn resume_decision(
        &mut self,
        _request: BrokerRequest,
        _decision: resume::ResumeDecision,
    ) -> Result<resume::ResumeResult, BrokerError> {
        Err(BrokerError::new("resume decision is unavailable"))
    }

    fn save_vault_history(&mut self) -> Result<Vec<SaveVaultSummary>, BrokerError> {
        Err(BrokerError::new("save vault is unavailable"))
    }

    fn save_vault_preview(&mut self) -> Result<SaveVaultPreview, BrokerError> {
        Err(BrokerError::new("save vault is unavailable"))
    }

    fn save_vault_restore(&mut self, _confirmed: bool) -> Result<(), BrokerError> {
        Err(BrokerError::new("save vault is unavailable"))
    }

    fn save_sync_status(&mut self) -> Result<save_sync::SyncStatus, BrokerError> {
        Err(BrokerError::new("save synchronization is unavailable"))
    }

    fn save_sync_resolve(
        &mut self,
        _action: save_sync::ResolutionAction,
    ) -> Result<save_sync::ResolutionReceipt, BrokerError> {
        Err(BrokerError::new("save synchronization is unavailable"))
    }
}

pub fn accepted_handle(request: &BrokerRequest) -> SessionHandle {
    SessionHandle {
        schema: HANDLE_SCHEMA.into(),
        session_id: request.request_id.clone(),
        content_id: request.content_id.clone(),
        phase: "active",
    }
}
