use std::{
    fmt,
    time::{Duration, Instant},
};

use hex::encode as hex_encode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    HardwareDomain, InputState, Platform, PlatformState, RadiosState, SuspendResult, SuspendState,
};

pub const MAX_TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_SLEEP_DURATION_MINUTES: u16 = 5;
pub const SLEEP_DURATION_OPTIONS_MINUTES: [u16; 6] = [1, 5, 10, 15, 30, 60];
const MAX_JOURNAL_ENTRIES: usize = 64;
const MAX_SHUTDOWN_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecyclePhase {
    Awake,
    PreparingSuspend,
    Suspended,
    ResumedByUser,
    ResumedForDeadline,
    OrderlyShutdown,
    Recovery,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeSource {
    User,
    Deadline,
    StaleAlarm,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeDeadline {
    pub token: u64,
    pub monotonic_deadline_ms: u64,
    pub boot_time_deadline_ms: u64,
    pub duration_minutes: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShutdownReason {
    ArmFailure,
    VerifyFailure,
    AlarmClearFailure,
    CheckpointFailure,
    CrashBeforeSuspend,
    CrashWithArmedJournal,
    Deadline,
    HalLoss,
    LowBattery,
    ColdRecovery,
    BoundedRetry,
    UserRequested,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShutdownStatus {
    Pending,
    Terminal,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownRequest {
    pub reason: ShutdownReason,
    pub status: ShutdownStatus,
    pub attempts: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleMarker {
    pub phase: LifecyclePhase,
    pub reason: String,
    pub checkpoint_generation: Option<u64>,
    pub deadline_ms: u64,
    pub armed_deadline: Option<WakeDeadline>,
    pub wake_source: Option<WakeSource>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
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
    pub armed_deadline: Option<WakeDeadline>,
    pub wake_source: Option<WakeSource>,
    pub wake_reason: Option<String>,
    pub shutdown_request: Option<ShutdownRequest>,
    pub launches_allowed: bool,
    pub background_allowed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleFault {
    HalLoss,
    Checkpoint,
    QuiesceAudio,
    QuiesceInput,
    QuiesceRadios,
    Suspend,
    ResumeRadios,
    ResumeInput,
    ResumeAudio,
    ArmDeadline,
    VerifyDeadline,
    ClearDeadline,
    CrashBeforeSuspend,
    CrashWithArmedJournal,
    Shutdown,
    Deadline,
}

impl LifecycleFault {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "hal-loss" => Self::HalLoss,
            "checkpoint-fail" => Self::Checkpoint,
            "quiesce-audio-fail" => Self::QuiesceAudio,
            "quiesce-input-fail" => Self::QuiesceInput,
            "quiesce-radios-fail" => Self::QuiesceRadios,
            "suspend-fail" => Self::Suspend,
            "resume-radios-fail" => Self::ResumeRadios,
            "resume-input-fail" => Self::ResumeInput,
            "resume-audio-fail" => Self::ResumeAudio,
            "arm-fail" => Self::ArmDeadline,
            "verify-fail" => Self::VerifyDeadline,
            "clear-fail" => Self::ClearDeadline,
            "crash-before-suspend" => Self::CrashBeforeSuspend,
            "crash-armed-journal" => Self::CrashWithArmedJournal,
            "shutdown-fail" => Self::Shutdown,
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
    Shutdown(String),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reentrant { operation, phase } => write!(
                formatter,
                "{operation} rejected while lifecycle is {phase:?}"
            ),
            Self::Deadline => formatter.write_str("lifecycle deadline expired"),
            Self::Checkpoint(reason) => {
                write!(formatter, "pre-suspend checkpoint failed: {reason}")
            }
            Self::Hal(reason) => write!(formatter, "lifecycle HAL unavailable: {reason}"),
            Self::Transition { stage, reason } => {
                write!(formatter, "lifecycle {stage} failed: {reason}")
            }
            Self::Recovery(reason) => write!(formatter, "lifecycle recovery required: {reason}"),
            Self::Shutdown(reason) => {
                write!(formatter, "orderly shutdown request failed: {reason}")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

pub trait CheckpointHook {
    fn checkpoint(&mut self) -> Result<u64, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleClock {
    pub monotonic_ms: u64,
    pub boot_time_ms: u64,
}

pub struct SuspendRequest {
    pub timeout: Duration,
    pub clock: LifecycleClock,
    pub duration_minutes: u16,
    pub fault: Option<LifecycleFault>,
}

pub struct ResumeRequest {
    pub timeout: Duration,
    pub clock: LifecycleClock,
    pub source: Option<WakeSource>,
    pub fault: Option<LifecycleFault>,
}

struct QuiesceRollback<'a> {
    snapshot: &'a PlatformState,
    touched_audio: bool,
    touched_input: bool,
    touched_rumble: bool,
    touched_usb: bool,
    touched_leds: bool,
    touched_radios: bool,
}

pub struct LifecycleController {
    phase: LifecyclePhase,
    marker: Option<LifecycleMarker>,
    saved_state: Option<PlatformState>,
    journal: Vec<LifecycleJournalEntry>,
    armed_deadline: Option<WakeDeadline>,
    wake_source: Option<WakeSource>,
    wake_reason: Option<String>,
    shutdown_request: Option<ShutdownRequest>,
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
            armed_deadline: None,
            wake_source: None,
            wake_reason: None,
            shutdown_request: None,
        }
    }

    pub fn from_pending_marker(marker: LifecycleMarker) -> Self {
        let mut controller = Self::new();
        controller.phase = LifecyclePhase::Recovery;
        controller.armed_deadline = marker.armed_deadline.clone();
        controller.marker = Some(LifecycleMarker {
            phase: LifecyclePhase::Recovery,
            reason: bound(format!("cold-recovery: {}", marker.reason)),
            checkpoint_generation: marker.checkpoint_generation,
            deadline_ms: marker.deadline_ms,
            armed_deadline: marker.armed_deadline,
            wake_source: marker.wake_source,
        });
        controller.shutdown_request = Some(ShutdownRequest {
            reason: ShutdownReason::ColdRecovery,
            status: ShutdownStatus::Pending,
            attempts: 0,
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
    pub fn terminal_shutdown(&self) -> bool {
        self.shutdown_request
            .as_ref()
            .is_some_and(|request| request.status == ShutdownStatus::Terminal)
    }

    pub fn deadline_due(&self, clock: LifecycleClock) -> bool {
        self.phase == LifecyclePhase::Suspended
            && self
                .armed_deadline
                .as_ref()
                .is_some_and(|deadline| clock.boot_time_ms >= deadline.boot_time_deadline_ms)
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
            armed_deadline: self.armed_deadline.clone(),
            wake_source: self.wake_source,
            wake_reason: self.wake_reason.clone(),
            shutdown_request: self.shutdown_request.clone(),
            launches_allowed: self.is_awake(),
            background_allowed: self.is_awake(),
        }
    }

    pub fn suspend<P, H>(
        &mut self,
        platform: &mut P,
        checkpoint: &mut H,
        request: SuspendRequest,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
        H: CheckpointHook,
    {
        self.ensure_awake("suspend")?;
        let SuspendRequest {
            timeout,
            clock,
            duration_minutes,
            fault,
        } = request;
        let started = Instant::now();
        let transition_deadline = self.transition_deadline(clock.monotonic_ms, timeout)?;
        if !SLEEP_DURATION_OPTIONS_MINUTES.contains(&duration_minutes) {
            return Err(self.fail_transition("sleep-duration", "duration is not allowlisted"));
        }
        self.begin(
            LifecyclePhase::PreparingSuspend,
            "suspend-preparing",
            transition_deadline,
        );
        if fault == Some(LifecycleFault::HalLoss) {
            return Err(self.shutdown_failure(
                platform,
                ShutdownReason::HalLoss,
                "HAL loss before suspend",
            ));
        }
        for domain in [
            HardwareDomain::Audio,
            HardwareDomain::Input,
            HardwareDomain::Rumble,
            HardwareDomain::Usb,
            HardwareDomain::Leds,
            HardwareDomain::Radios,
            HardwareDomain::Suspend,
        ] {
            if !platform.capabilities().supports(domain) {
                return Err(self.shutdown_failure(
                    platform,
                    ShutdownReason::HalLoss,
                    &format!("{} capability unavailable", domain.as_str()),
                ));
            }
        }
        let snapshot = match platform.platform_state() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(self.shutdown_failure(
                    platform,
                    ShutdownReason::HalLoss,
                    &error.to_string(),
                ));
            }
        };
        let generation = match checkpoint.checkpoint() {
            Ok(generation) => generation,
            Err(reason) => {
                self.phase = LifecyclePhase::Recovery;
                self.marker =
                    Some(self.marker(LifecyclePhase::Recovery, "checkpoint-failed", None));
                self.record("checkpoint-failed");
                return Err(self.shutdown_failure(
                    platform,
                    ShutdownReason::CheckpointFailure,
                    &bound(reason),
                ));
            }
        };
        self.saved_state = Some(snapshot.clone());
        self.marker = Some(self.marker(
            LifecyclePhase::PreparingSuspend,
            "checkpoint-complete",
            Some(generation),
        ));
        self.record("checkpoint-complete");

        self.clear_wake_deadline(platform, fault)?;
        self.record("alarm-cleared");
        let wake_deadline = WakeDeadline {
            token: generation.max(1),
            monotonic_deadline_ms: clock
                .monotonic_ms
                .saturating_add(u64::from(duration_minutes) * 60_000),
            boot_time_deadline_ms: clock
                .boot_time_ms
                .saturating_add(u64::from(duration_minutes) * 60_000),
            duration_minutes,
        };
        if fault == Some(LifecycleFault::ArmDeadline) || fault == Some(LifecycleFault::Deadline) {
            return Err(self.shutdown_failure(
                platform,
                ShutdownReason::ArmFailure,
                "deadline arm failure",
            ));
        }
        platform
            .arm_wake_deadline(wake_deadline.clone())
            .map_err(|error| {
                self.shutdown_failure(platform, ShutdownReason::ArmFailure, &error.to_string())
            })?;
        if fault == Some(LifecycleFault::VerifyDeadline)
            || platform.verify_wake_deadline(&wake_deadline).is_err()
        {
            return Err(self.shutdown_failure(
                platform,
                ShutdownReason::VerifyFailure,
                "deadline verification failed",
            ));
        }
        self.armed_deadline = Some(wake_deadline.clone());
        self.marker = Some(self.marker(
            LifecyclePhase::PreparingSuspend,
            "deadline-armed",
            Some(generation),
        ));
        self.record("deadline-armed");
        if fault == Some(LifecycleFault::CrashBeforeSuspend) {
            return Err(self.shutdown_failure(
                platform,
                ShutdownReason::CrashBeforeSuspend,
                "crash before suspend",
            ));
        }
        if fault == Some(LifecycleFault::CrashWithArmedJournal) {
            return Err(self.shutdown_failure(
                platform,
                ShutdownReason::CrashWithArmedJournal,
                "crash with armed journal",
            ));
        }

        let mut audio = snapshot.audio;
        audio.active = false;
        audio.enabled = false;
        let mut rollback_state = QuiesceRollback {
            snapshot: &snapshot,
            touched_audio: false,
            touched_input: false,
            touched_rumble: false,
            touched_usb: false,
            touched_leds: false,
            touched_radios: false,
        };
        if fault == Some(LifecycleFault::QuiesceAudio) {
            return Err(self.quiesce_failed(platform, &rollback_state, "audio", "injected fault"));
        }
        if let Err(error) = platform.set_audio(audio) {
            return Err(self.quiesce_failed(
                platform,
                &rollback_state,
                "audio",
                &error.to_string(),
            ));
        }
        rollback_state.touched_audio = true;
        self.record("quiesce-audio");
        if fault == Some(LifecycleFault::QuiesceInput) {
            return Err(self.quiesce_failed(platform, &rollback_state, "input", "injected fault"));
        }
        if let Err(error) = platform.set_input(InputState {
            pressed: Vec::new(),
        }) {
            return Err(self.quiesce_failed(
                platform,
                &rollback_state,
                "input",
                &error.to_string(),
            ));
        }
        rollback_state.touched_input = true;
        self.record("quiesce-input");
        if let Err(error) = platform.set_rumble(crate::RumbleState { active: false }) {
            return Err(self.quiesce_failed(
                platform,
                &rollback_state,
                "rumble",
                &error.to_string(),
            ));
        }
        rollback_state.touched_rumble = true;
        self.record("quiesce-rumble");
        if let Err(error) = platform.set_usb(crate::UsbState {
            connected: snapshot.usb.connected,
            role: crate::UsbRole::None,
        }) {
            return Err(self.quiesce_failed(platform, &rollback_state, "usb", &error.to_string()));
        }
        rollback_state.touched_usb = true;
        self.record("quiesce-usb");
        if let Err(error) = platform.set_leds(crate::LedState {
            on: false,
            brightness_percent: 0,
        }) {
            return Err(self.quiesce_failed(platform, &rollback_state, "leds", &error.to_string()));
        }
        rollback_state.touched_leds = true;
        self.record("quiesce-leds");
        if fault == Some(LifecycleFault::QuiesceRadios) {
            return Err(self.quiesce_failed(platform, &rollback_state, "radios", "injected fault"));
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
                &rollback_state,
                "radios",
                &error.to_string(),
            ));
        }
        rollback_state.touched_radios = true;
        self.record("quiesce-radios");
        if fault == Some(LifecycleFault::Suspend) || started.elapsed() >= timeout {
            return Err(self.quiesce_failed(
                platform,
                &rollback_state,
                "suspend",
                "transition timeout",
            ));
        }
        if let Err(error) = platform.set_suspend((SuspendState::Suspended, SuspendResult::Success))
        {
            return Err(self.quiesce_failed(
                platform,
                &rollback_state,
                "suspend",
                &error.to_string(),
            ));
        }
        self.phase = LifecyclePhase::Suspended;
        self.marker = Some(self.marker(LifecyclePhase::Suspended, "suspended", Some(generation)));
        self.record("suspend-complete");
        Ok(())
    }

    pub fn resume<P>(
        &mut self,
        platform: &mut P,
        request: ResumeRequest,
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
        let ResumeRequest {
            timeout,
            clock,
            source,
            fault,
        } = request;
        let started = Instant::now();
        self.transition_deadline(clock.monotonic_ms, timeout)?;
        let snapshot = self
            .saved_state
            .clone()
            .ok_or_else(|| self.recover("saved state unavailable"))?;
        let due = self.deadline_due(clock);
        if !due && (source == Some(WakeSource::StaleAlarm) || source == Some(WakeSource::Deadline))
        {
            self.wake_source = Some(WakeSource::StaleAlarm);
            self.wake_reason = Some("stale-alarm-ignored".into());
            self.record("stale-alarm");
            return self.restore_user(
                platform,
                snapshot,
                started,
                timeout,
                fault,
                WakeSource::StaleAlarm,
            );
        }
        if due {
            self.phase = LifecyclePhase::ResumedForDeadline;
            self.wake_source = Some(WakeSource::Deadline);
            self.wake_reason = Some("bounded-deadline-reached".into());
            self.marker = Some(self.marker(
                LifecyclePhase::ResumedForDeadline,
                "deadline-resumed",
                self.generation(),
            ));
            self.record("resumed-for-deadline");
            self.clear_wake_deadline(platform, fault)?;
            self.armed_deadline = None;
            self.saved_state = None;
            return self.request_shutdown(platform, ShutdownReason::Deadline, fault);
        }
        self.wake_source = Some(WakeSource::User);
        self.wake_reason = Some("manual-wake-before-deadline".into());
        self.restore_user(
            platform,
            snapshot,
            started,
            timeout,
            fault,
            WakeSource::User,
        )
    }

    pub fn low_battery<P>(&mut self, platform: &mut P) -> Result<(), LifecycleError>
    where
        P: Platform,
    {
        if !self.is_awake() {
            return Ok(());
        }
        self.record("low-battery");
        self.request_shutdown(platform, ShutdownReason::LowBattery, None)
    }

    pub fn orderly_shutdown<P>(
        &mut self,
        platform: &mut P,
        fault: Option<LifecycleFault>,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
    {
        self.request_shutdown(platform, ShutdownReason::UserRequested, fault)
    }

    pub fn retry_shutdown<P>(
        &mut self,
        platform: &mut P,
        fault: Option<LifecycleFault>,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
    {
        let reason = self
            .shutdown_request
            .as_ref()
            .map(|request| request.reason)
            .ok_or_else(|| self.recover("no shutdown request pending"))?;
        self.request_shutdown(platform, reason, fault)
    }

    fn restore_user<P>(
        &mut self,
        platform: &mut P,
        snapshot: PlatformState,
        started: Instant,
        timeout: Duration,
        fault: Option<LifecycleFault>,
        source: WakeSource,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
    {
        self.phase = LifecyclePhase::ResumedByUser;
        self.marker = Some(self.marker(
            LifecyclePhase::ResumedByUser,
            "user-resume",
            self.generation(),
        ));
        self.record("resumed-by-user");
        self.clear_wake_deadline(platform, fault)?;
        self.armed_deadline = None;
        if fault == Some(LifecycleFault::HalLoss) || started.elapsed() >= timeout {
            return Err(self.resume_failed("resume", "transition timeout".into()));
        }
        for domain in [
            HardwareDomain::Suspend,
            HardwareDomain::Radios,
            HardwareDomain::Leds,
            HardwareDomain::Usb,
            HardwareDomain::Rumble,
            HardwareDomain::Input,
            HardwareDomain::Audio,
        ] {
            if !platform.capabilities().supports(domain) {
                return Err(self.recover(format!("{} capability unavailable", domain.as_str())));
            }
        }
        platform
            .set_suspend((SuspendState::Active, SuspendResult::Success))
            .map_err(|error| self.resume_failed("suspend", error.to_string()))?;
        if !matches!(platform.suspend_state(), Ok((SuspendState::Active, _))) {
            return Err(self.recover("HAL did not report active after resume"));
        }
        self.record("resume-active");
        if fault == Some(LifecycleFault::ResumeRadios) {
            return Err(self.resume_failed("radios", "injected fault".into()));
        }
        platform
            .set_radios(snapshot.radios)
            .map_err(|error| self.resume_failed("radios", error.to_string()))?;
        self.record("restore-radios");
        platform
            .set_leds(snapshot.leds)
            .map_err(|error| self.resume_failed("leds", error.to_string()))?;
        self.record("restore-leds");
        platform
            .set_usb(snapshot.usb)
            .map_err(|error| self.resume_failed("usb", error.to_string()))?;
        self.record("restore-usb");
        platform
            .set_rumble(snapshot.rumble)
            .map_err(|error| self.resume_failed("rumble", error.to_string()))?;
        self.record("restore-rumble");
        if fault == Some(LifecycleFault::ResumeInput) {
            return Err(self.resume_failed("input", "injected fault".into()));
        }
        platform
            .set_input(InputState {
                pressed: Vec::new(),
            })
            .map_err(|error| self.resume_failed("input", error.to_string()))?;
        self.record("restore-input");
        if fault == Some(LifecycleFault::ResumeAudio) {
            return Err(self.resume_failed("audio", "injected fault".into()));
        }
        platform
            .set_audio(snapshot.audio)
            .map_err(|error| self.resume_failed("audio", error.to_string()))?;
        self.record("restore-audio");
        self.phase = LifecyclePhase::Awake;
        self.marker = None;
        self.saved_state = None;
        self.wake_source = Some(source);
        self.record("resume-complete");
        Ok(())
    }

    fn clear_wake_deadline<P>(
        &mut self,
        platform: &mut P,
        fault: Option<LifecycleFault>,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
    {
        if fault == Some(LifecycleFault::ClearDeadline) {
            return Err(self.shutdown_failure(
                platform,
                ShutdownReason::AlarmClearFailure,
                "deadline clear failure",
            ));
        }
        platform.clear_wake_deadline().map_err(|error| {
            self.shutdown_failure(
                platform,
                ShutdownReason::AlarmClearFailure,
                &error.to_string(),
            )
        })
    }

    fn request_shutdown<P>(
        &mut self,
        platform: &mut P,
        reason: ShutdownReason,
        fault: Option<LifecycleFault>,
    ) -> Result<(), LifecycleError>
    where
        P: Platform,
    {
        if let Some(request) = self.shutdown_request.as_ref() {
            match request.status {
                ShutdownStatus::Terminal => return Ok(()),
                ShutdownStatus::Failed => {
                    return Err(LifecycleError::Shutdown(
                        "bounded shutdown retry exhausted".into(),
                    ));
                }
                ShutdownStatus::Pending => {}
            }
        }
        self.phase = LifecyclePhase::OrderlyShutdown;
        self.marker = Some(self.marker(
            LifecyclePhase::OrderlyShutdown,
            shutdown_reason(reason),
            self.generation(),
        ));
        let attempts = self
            .shutdown_request
            .as_ref()
            .map_or(0, |request| request.attempts)
            .saturating_add(1);
        let status = if fault == Some(LifecycleFault::Shutdown)
            || platform.request_orderly_shutdown(reason).is_err()
        {
            if attempts >= MAX_SHUTDOWN_ATTEMPTS {
                ShutdownStatus::Failed
            } else {
                ShutdownStatus::Pending
            }
        } else {
            ShutdownStatus::Terminal
        };
        self.shutdown_request = Some(ShutdownRequest {
            reason,
            status,
            attempts: attempts.min(MAX_SHUTDOWN_ATTEMPTS),
        });
        self.record(match status {
            ShutdownStatus::Terminal => "orderly-shutdown-requested",
            ShutdownStatus::Pending => "orderly-shutdown-retry-pending",
            ShutdownStatus::Failed => "orderly-shutdown-retry-exhausted",
        });
        if status == ShutdownStatus::Terminal {
            Ok(())
        } else {
            Err(LifecycleError::Shutdown(
                "bounded shutdown retry pending".into(),
            ))
        }
    }

    fn shutdown_failure<P>(
        &mut self,
        platform: &mut P,
        reason: ShutdownReason,
        message: &str,
    ) -> LifecycleError
    where
        P: Platform,
    {
        self.wake_reason = Some(bound(message.to_string()));
        let _ = platform.clear_wake_deadline();
        self.armed_deadline = None;
        let _ = self.request_shutdown(platform, reason, None);
        LifecycleError::Transition {
            stage: "orderly-shutdown",
            reason: bound(message.to_string()),
        }
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
    fn transition_deadline(&self, now_ms: u64, timeout: Duration) -> Result<u64, LifecycleError> {
        if timeout.is_zero() || timeout > MAX_TRANSITION_TIMEOUT {
            return Err(LifecycleError::Deadline);
        }
        Ok(now_ms.saturating_add(timeout.as_millis() as u64))
    }
    fn begin(&mut self, phase: LifecyclePhase, reason: &str, deadline_ms: u64) {
        self.phase = phase;
        self.marker = Some(self.marker(phase, reason, None));
        if let Some(marker) = self.marker.as_mut() {
            marker.deadline_ms = deadline_ms;
        }
        self.record(reason);
    }
    fn marker(
        &self,
        phase: LifecyclePhase,
        reason: &str,
        generation: Option<u64>,
    ) -> LifecycleMarker {
        LifecycleMarker {
            phase,
            reason: bound(reason.to_string()),
            checkpoint_generation: generation.or_else(|| self.generation()),
            deadline_ms: self.marker.as_ref().map_or(0, |marker| marker.deadline_ms),
            armed_deadline: self.armed_deadline.clone(),
            wake_source: self.wake_source,
        }
    }
    fn generation(&self) -> Option<u64> {
        self.marker
            .as_ref()
            .and_then(|marker| marker.checkpoint_generation)
    }
    fn record(&mut self, event: &str) {
        let sequence = self
            .journal
            .last()
            .map_or(0, |entry| entry.sequence.saturating_add(1));
        self.journal.push(LifecycleJournalEntry {
            sequence,
            event: bound(event.to_string()),
            phase: self.phase,
        });
        if self.journal.len() > MAX_JOURNAL_ENTRIES {
            self.journal.remove(0);
        }
    }
    fn recover(&mut self, reason: impl Into<String>) -> LifecycleError {
        let reason = bound(reason.into());
        self.phase = LifecyclePhase::Recovery;
        self.marker = Some(self.marker(LifecyclePhase::Recovery, &reason, self.generation()));
        self.record("recovery");
        LifecycleError::Recovery(reason)
    }
    fn fail_transition(&mut self, stage: &'static str, reason: &str) -> LifecycleError {
        LifecycleError::Transition {
            stage,
            reason: bound(reason.to_string()),
        }
    }
    fn quiesce_failed<P: Platform>(
        &mut self,
        platform: &mut P,
        rollback_state: &QuiesceRollback<'_>,
        stage: &'static str,
        reason: &str,
    ) -> LifecycleError {
        let rollback = rollback(platform, rollback_state);
        let _ = platform.clear_wake_deadline();
        self.armed_deadline = None;
        if let Err(error) = rollback {
            return self.recover(format!("{stage} failed and rollback failed: {error}"));
        }
        self.phase = LifecyclePhase::Awake;
        self.marker = Some(self.marker(
            LifecyclePhase::Awake,
            &format!("{stage}-failed-rolled-back"),
            None,
        ));
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
        self.marker = Some(self.marker(
            LifecyclePhase::Recovery,
            &format!("resume-{stage}-failed"),
            self.generation(),
        ));
        self.record("resume-failed");
        LifecycleError::Transition { stage, reason }
    }
}

fn rollback<P: Platform>(
    platform: &mut P,
    rollback_state: &QuiesceRollback<'_>,
) -> Result<(), String> {
    platform
        .set_suspend(rollback_state.snapshot.suspend.clone())
        .map_err(|error| error.to_string())?;
    if rollback_state.touched_radios {
        platform
            .set_radios(rollback_state.snapshot.radios)
            .map_err(|error| error.to_string())?;
    }
    if rollback_state.touched_leds {
        platform
            .set_leds(rollback_state.snapshot.leds)
            .map_err(|error| error.to_string())?;
    }
    if rollback_state.touched_usb {
        platform
            .set_usb(rollback_state.snapshot.usb)
            .map_err(|error| error.to_string())?;
    }
    if rollback_state.touched_rumble {
        platform
            .set_rumble(rollback_state.snapshot.rumble)
            .map_err(|error| error.to_string())?;
    }
    if rollback_state.touched_input {
        platform
            .set_input(rollback_state.snapshot.input.clone())
            .map_err(|error| error.to_string())?;
    }
    if rollback_state.touched_audio {
        platform
            .set_audio(rollback_state.snapshot.audio)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn shutdown_reason(reason: ShutdownReason) -> &'static str {
    match reason {
        ShutdownReason::ArmFailure => "arm-failed",
        ShutdownReason::VerifyFailure => "verify-failed",
        ShutdownReason::AlarmClearFailure => "alarm-clear-failed",
        ShutdownReason::CheckpointFailure => "checkpoint-failed",
        ShutdownReason::CrashBeforeSuspend => "crash-before-suspend",
        ShutdownReason::CrashWithArmedJournal => "crash-with-armed-journal",
        ShutdownReason::Deadline => "deadline-expired",
        ShutdownReason::HalLoss => "hal-loss",
        ShutdownReason::LowBattery => "low-battery",
        ShutdownReason::ColdRecovery => "cold-recovery",
        ShutdownReason::BoundedRetry => "bounded-shutdown-retry",
        ShutdownReason::UserRequested => "user-requested",
    }
}

pub fn marker_checksum(marker: &LifecycleMarker) -> String {
    checksum(marker)
}

pub fn journal_checksum(journal: &[LifecycleJournalEntry]) -> String {
    checksum(&journal)
}

fn checksum<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("lifecycle checksum serialization");
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_encode(digest))
}

fn bound(value: String) -> String {
    value.chars().take(160).collect()
}
