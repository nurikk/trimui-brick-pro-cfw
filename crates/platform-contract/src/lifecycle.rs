use std::{
    fmt,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::{
    HardwareDomain, InputState, Platform, PlatformError, PlatformState, RadiosState, SuspendResult,
    SuspendState,
};

pub const MAX_TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_JOURNAL_ENTRIES: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecyclePhase {
    Awake,
    Preparing,
    Suspended,
    Resuming,
    Recovery,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleMarker {
    pub phase: LifecyclePhase,
    pub reason: String,
    pub checkpoint_generation: Option<u64>,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleJournalEntry {
    pub sequence: u64,
    pub event: String,
    pub phase: LifecyclePhase,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleEvidence {
    pub phase: LifecyclePhase,
    pub marker: Option<LifecycleMarker>,
    pub saved_state: Option<PlatformState>,
    pub journal: Vec<LifecycleJournalEntry>,
    pub launches_allowed: bool,
    pub background_allowed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleFault {
    HalLoss,
    QuiesceAudio,
    QuiesceInput,
    QuiesceRadios,
    Suspend,
    ResumeRadios,
    ResumeInput,
    ResumeAudio,
    Deadline,
}

impl LifecycleFault {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "hal-loss" => Self::HalLoss,
            "quiesce-audio-fail" => Self::QuiesceAudio,
            "quiesce-input-fail" => Self::QuiesceInput,
            "quiesce-radios-fail" => Self::QuiesceRadios,
            "suspend-fail" => Self::Suspend,
            "resume-radios-fail" => Self::ResumeRadios,
            "resume-input-fail" => Self::ResumeInput,
            "resume-audio-fail" => Self::ResumeAudio,
            "deadline" => Self::Deadline,
            _ => return None,
        })
    }
}

#[derive(Debug)]
pub enum LifecycleError {
    Reentrant {
        operation: &'static str,
        phase: LifecyclePhase,
    },
    Deadline,
    Checkpoint(String),
    Hal(String),
    Transition {
        stage: &'static str,
        reason: String,
    },
    Recovery(String),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reentrant { operation, phase } => {
                write!(
                    formatter,
                    "{operation} rejected while lifecycle is {phase:?}"
                )
            }
            Self::Deadline => formatter.write_str("lifecycle deadline expired"),
            Self::Checkpoint(reason) => {
                write!(formatter, "pre-suspend checkpoint failed: {reason}")
            }
            Self::Hal(reason) => write!(formatter, "lifecycle HAL unavailable: {reason}"),
            Self::Transition { stage, reason } => {
                write!(formatter, "lifecycle {stage} failed: {reason}")
            }
            Self::Recovery(reason) => write!(formatter, "lifecycle recovery required: {reason}"),
        }
    }
}

impl std::error::Error for LifecycleError {}

pub trait CheckpointHook {
    fn checkpoint(&mut self) -> Result<u64, String>;
}

pub struct LifecycleController {
    phase: LifecyclePhase,
    marker: Option<LifecycleMarker>,
    saved_state: Option<PlatformState>,
    journal: Vec<LifecycleJournalEntry>,
}

impl Default for LifecycleController {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleController {
    pub fn new() -> Self {
        Self {
            phase: LifecyclePhase::Awake,
            marker: None,
            saved_state: None,
            journal: Vec::new(),
        }
    }

    pub fn from_pending_marker(marker: LifecycleMarker) -> Self {
        let mut controller = Self::new();
        controller.phase = LifecyclePhase::Recovery;
        controller.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Recovery,
            reason: bound(format!("cold-recovery: {}", marker.reason)),
            checkpoint_generation: marker.checkpoint_generation,
            deadline_ms: marker.deadline_ms,
        });
        controller.record("cold-recovery");
        controller
    }

    pub fn phase(&self) -> LifecyclePhase {
        self.phase
    }

    pub fn is_awake(&self) -> bool {
        self.phase == LifecyclePhase::Awake
    }

    pub fn gate(&self, operation: &'static str) -> Result<(), String> {
        if self.is_awake() {
            Ok(())
        } else {
            Err(format!(
                "{operation} blocked while lifecycle is {:?}",
                self.phase
            ))
        }
    }

    pub fn evidence(&self) -> LifecycleEvidence {
        LifecycleEvidence {
            phase: self.phase,
            marker: self.marker.clone(),
            saved_state: self.saved_state.clone(),
            journal: self.journal.clone(),
            launches_allowed: self.is_awake(),
            background_allowed: self.is_awake(),
        }
    }

    pub fn suspend<P, H>(
        &mut self,
        platform: &mut P,
        checkpoint: &mut H,
        timeout: Duration,
        now_ms: u64,
        fault: Option<LifecycleFault>,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
        H: CheckpointHook,
    {
        self.ensure_awake("suspend")?;
        let started = Instant::now();
        let deadline_ms = self.deadline(now_ms, timeout)?;
        self.begin(LifecyclePhase::Preparing, "suspend-preparing", deadline_ms);
        if fault == Some(LifecycleFault::HalLoss) {
            return Err(self.recover("HAL loss before suspend"));
        }
        if fault == Some(LifecycleFault::Deadline) {
            return Err(self.fail_deadline());
        }
        for domain in [
            HardwareDomain::Audio,
            HardwareDomain::Input,
            HardwareDomain::Radios,
            HardwareDomain::Suspend,
        ] {
            if !platform.capabilities().supports(domain) {
                return Err(self.recover(format!("{} capability unavailable", domain.as_str())));
            }
        }
        let snapshot = match platform.platform_state() {
            Ok(snapshot) => snapshot,
            Err(error) => return Err(self.hal(error)),
        };
        let generation = match checkpoint.checkpoint() {
            Ok(generation) => generation,
            Err(reason) => {
                self.marker = Some(LifecycleMarker {
                    phase: LifecyclePhase::Recovery,
                    reason: "checkpoint-failed".into(),
                    checkpoint_generation: None,
                    deadline_ms,
                });
                self.phase = LifecyclePhase::Recovery;
                self.record("checkpoint-failed");
                return Err(LifecycleError::Checkpoint(bound(reason)));
            }
        };
        self.saved_state = Some(snapshot.clone());
        self.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Preparing,
            reason: "checkpoint-complete".into(),
            checkpoint_generation: Some(generation),
            deadline_ms,
        });
        self.record("checkpoint-complete");

        let mut audio = snapshot.audio;
        audio.active = false;
        audio.enabled = false;
        let mut touched_audio = false;
        let mut touched_input = false;
        let mut touched_radios = false;
        if fault == Some(LifecycleFault::QuiesceAudio) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "audio",
                "injected fault",
            ));
        }
        if let Err(error) = platform.set_audio(audio) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "audio",
                &error.to_string(),
            ));
        }
        touched_audio = true;
        self.record("quiesce-audio");

        if fault == Some(LifecycleFault::QuiesceInput) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "input",
                "injected fault",
            ));
        }
        if let Err(error) = platform.set_input(InputState {
            pressed: Vec::new(),
        }) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "input",
                &error.to_string(),
            ));
        }
        touched_input = true;
        self.record("quiesce-input");

        if fault == Some(LifecycleFault::QuiesceRadios) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "radios",
                "injected fault",
            ));
        }
        if let Err(error) = platform.set_radios(RadiosState {
            wifi: crate::RadioState {
                enabled: false,
                connected: false,
            },
            bluetooth: crate::RadioState {
                enabled: false,
                connected: false,
            },
        }) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "radios",
                &error.to_string(),
            ));
        }
        touched_radios = true;
        self.record("quiesce-radios");

        if fault == Some(LifecycleFault::Suspend) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "suspend",
                "injected fault",
            ));
        }
        if self.expired(started, timeout) {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "suspend",
                "deadline expired",
            ));
        }
        if let Err(error) = platform.set_suspend((SuspendState::Suspended, SuspendResult::Success))
        {
            return Err(self.quiesce_failed(
                platform,
                &snapshot,
                touched_audio,
                touched_input,
                touched_radios,
                "suspend",
                &error.to_string(),
            ));
        }
        self.phase = LifecyclePhase::Suspended;
        self.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Suspended,
            reason: "suspended".into(),
            checkpoint_generation: Some(generation),
            deadline_ms,
        });
        self.record("suspend-complete");
        Ok(())
    }

    pub fn resume<P>(
        &mut self,
        platform: &mut P,
        timeout: Duration,
        now_ms: u64,
        fault: Option<LifecycleFault>,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
    {
        if self.phase != LifecyclePhase::Suspended {
            return Err(LifecycleError::Reentrant {
                operation: "resume",
                phase: self.phase,
            });
        }
        let started = Instant::now();
        let deadline_ms = self.deadline(now_ms, timeout)?;
        let snapshot = match self.saved_state.clone() {
            Some(snapshot) => snapshot,
            None => return Err(self.recover("saved state unavailable")),
        };
        self.phase = LifecyclePhase::Resuming;
        self.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Resuming,
            reason: "resume-preparing".into(),
            checkpoint_generation: self
                .marker
                .as_ref()
                .and_then(|marker| marker.checkpoint_generation),
            deadline_ms,
        });
        self.record("resume-preparing");
        if fault == Some(LifecycleFault::HalLoss) {
            return Err(self.recover("HAL loss before resume"));
        }
        if fault == Some(LifecycleFault::Deadline) || self.expired(started, timeout) {
            return Err(self.fail_deadline());
        }
        for domain in [
            HardwareDomain::Suspend,
            HardwareDomain::Radios,
            HardwareDomain::Input,
            HardwareDomain::Audio,
        ] {
            if !platform.capabilities().supports(domain) {
                return Err(self.recover(format!("{} capability unavailable", domain.as_str())));
            }
        }
        if let Err(error) = platform.set_suspend((SuspendState::Active, SuspendResult::Success)) {
            return Err(self.resume_failed("suspend", error.to_string()));
        }
        if !matches!(platform.suspend_state(), Ok((SuspendState::Active, _))) {
            return Err(self.recover("HAL did not report active after resume"));
        }
        self.record("resume-active");

        if fault == Some(LifecycleFault::ResumeRadios) {
            return Err(self.resume_failed("radios", "injected fault".into()));
        }
        if let Err(error) = platform.set_radios(snapshot.radios) {
            return Err(self.resume_failed("radios", error.to_string()));
        }
        self.record("restore-radios");
        if fault == Some(LifecycleFault::ResumeInput) {
            return Err(self.resume_failed("input", "injected fault".into()));
        }
        if let Err(error) = platform.set_input(snapshot.input.clone()) {
            return Err(self.resume_failed("input", error.to_string()));
        }
        self.record("restore-input");
        if fault == Some(LifecycleFault::ResumeAudio) {
            return Err(self.resume_failed("audio", "injected fault".into()));
        }
        if let Err(error) = platform.set_audio(snapshot.audio) {
            return Err(self.resume_failed("audio", error.to_string()));
        }
        self.record("restore-audio");
        if let Err(error) = platform.set_suspend(snapshot.suspend.clone()) {
            return Err(self.resume_failed("suspend-finalize", error.to_string()));
        }
        self.phase = LifecyclePhase::Awake;
        self.marker = None;
        self.saved_state = None;
        self.record("resume-complete");
        Ok(())
    }

    fn ensure_awake(&self, operation: &'static str) -> Result<(), LifecycleError> {
        if self.is_awake() {
            Ok(())
        } else {
            Err(LifecycleError::Reentrant {
                operation,
                phase: self.phase,
            })
        }
    }

    fn deadline(&mut self, now_ms: u64, timeout: Duration) -> Result<u64, LifecycleError> {
        if timeout.is_zero() || timeout > MAX_TRANSITION_TIMEOUT {
            return Err(self.fail_deadline());
        }
        Ok(now_ms.saturating_add(timeout.as_millis() as u64))
    }

    fn expired(&self, started: Instant, timeout: Duration) -> bool {
        started.elapsed() >= timeout || timeout.is_zero() || timeout > MAX_TRANSITION_TIMEOUT
    }

    fn begin(&mut self, phase: LifecyclePhase, reason: &str, deadline_ms: u64) {
        self.phase = phase;
        self.marker = Some(LifecycleMarker {
            phase,
            reason: reason.into(),
            checkpoint_generation: None,
            deadline_ms,
        });
        self.record(reason);
    }

    fn record(&mut self, event: &str) {
        let sequence = self.journal.last().map_or(0, |entry| entry.sequence + 1);
        self.journal.push(LifecycleJournalEntry {
            sequence,
            event: event.into(),
            phase: self.phase,
        });
        if self.journal.len() > MAX_JOURNAL_ENTRIES {
            self.journal.remove(0);
        }
    }

    fn recover(&mut self, reason: impl Into<String>) -> LifecycleError {
        let reason = bound(reason.into());
        self.phase = LifecyclePhase::Recovery;
        self.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Recovery,
            reason: reason.clone(),
            checkpoint_generation: self
                .marker
                .as_ref()
                .and_then(|marker| marker.checkpoint_generation),
            deadline_ms: self.marker.as_ref().map_or(0, |marker| marker.deadline_ms),
        });
        self.record("recovery");
        LifecycleError::Hal(reason)
    }

    fn hal(&mut self, error: PlatformError) -> LifecycleError {
        self.recover(error.to_string())
    }

    fn fail_deadline(&mut self) -> LifecycleError {
        self.phase = LifecyclePhase::Recovery;
        self.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Recovery,
            reason: "deadline-expired".into(),
            checkpoint_generation: self
                .marker
                .as_ref()
                .and_then(|marker| marker.checkpoint_generation),
            deadline_ms: self.marker.as_ref().map_or(0, |marker| marker.deadline_ms),
        });
        self.record("deadline-expired");
        LifecycleError::Deadline
    }

    fn quiesce_failed<P: Platform>(
        &mut self,
        platform: &mut P,
        snapshot: &PlatformState,
        touched_audio: bool,
        touched_input: bool,
        touched_radios: bool,
        stage: &'static str,
        reason: &str,
    ) -> LifecycleError {
        let rollback = rollback(
            platform,
            snapshot,
            touched_audio,
            touched_input,
            touched_radios,
        );
        if let Err(error) = rollback {
            return self.recover(format!("{stage} failed and rollback failed: {error}"));
        }
        self.phase = LifecyclePhase::Awake;
        self.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Awake,
            reason: format!("{stage}-failed-rolled-back"),
            checkpoint_generation: None,
            deadline_ms: self.marker.as_ref().map_or(0, |marker| marker.deadline_ms),
        });
        self.saved_state = None;
        self.record("rollback-complete");
        LifecycleError::Transition {
            stage,
            reason: bound(reason.to_string()),
        }
    }

    fn resume_failed(&mut self, stage: &'static str, reason: String) -> LifecycleError {
        let reason = bound(reason);
        self.phase = LifecyclePhase::Recovery;
        self.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Recovery,
            reason: format!("resume-{stage}-failed"),
            checkpoint_generation: self
                .marker
                .as_ref()
                .and_then(|marker| marker.checkpoint_generation),
            deadline_ms: self.marker.as_ref().map_or(0, |marker| marker.deadline_ms),
        });
        self.record("resume-failed");
        LifecycleError::Transition { stage, reason }
    }
}

fn rollback<P: Platform>(
    platform: &mut P,
    snapshot: &PlatformState,
    touched_audio: bool,
    touched_input: bool,
    touched_radios: bool,
) -> Result<(), String> {
    platform
        .set_suspend(snapshot.suspend.clone())
        .map_err(|error| error.to_string())?;
    if touched_radios {
        platform
            .set_radios(snapshot.radios)
            .map_err(|error| error.to_string())?;
    }
    if touched_input {
        platform
            .set_input(snapshot.input.clone())
            .map_err(|error| error.to_string())?;
    }
    if touched_audio {
        platform
            .set_audio(snapshot.audio)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn bound(value: String) -> String {
    value.chars().take(160).collect()
}
