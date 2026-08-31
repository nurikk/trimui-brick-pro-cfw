use serde::{Deserialize, Serialize};

const MAX_PLAUSIBLE_JUMP_PERCENT: u8 = 20;
const REARM_HYSTERESIS_PERCENT: u8 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatteryHealth {
    Good,
    Degraded,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChargingStatus {
    Charging,
    Full,
    NotCharging,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatteryLevel {
    Normal,
    Low,
    Critical,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LowBatteryAction {
    WarnOnly,
    SaveAndExit,
    ExitWithoutSave,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyAction {
    Warn,
    CheckpointAndExit,
    ExitWithoutSave,
    CheckpointAndShutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct BatteryPolicy {
    pub warning_percent: u8,
    pub critical_percent: u8,
    pub low_battery_action: LowBatteryAction,
    pub charging_led: bool,
    pub charging_display: bool,
}

impl Default for BatteryPolicy {
    fn default() -> Self {
        Self {
            warning_percent: 20,
            critical_percent: 10,
            low_battery_action: LowBatteryAction::WarnOnly,
            charging_led: false,
            charging_display: true,
        }
    }
}

impl BatteryPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=50).contains(&self.warning_percent)
            || self.critical_percent == 0
            || self.critical_percent >= self.warning_percent
        {
            return Err("battery thresholds must satisfy 1 <= critical < warning <= 50".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryObservation {
    pub percent: Option<u8>,
    pub charging: Option<bool>,
    pub full: Option<bool>,
    pub external_power: Option<bool>,
    pub health: BatteryHealth,
}

impl BatteryObservation {
    pub fn charging_status(self) -> ChargingStatus {
        match (self.full, self.charging) {
            (Some(true), _) => ChargingStatus::Full,
            (_, Some(true)) => ChargingStatus::Charging,
            (Some(false), Some(false)) => ChargingStatus::NotCharging,
            _ => ChargingStatus::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryDecision {
    pub observation: BatteryObservation,
    pub displayed_percent: Option<u8>,
    pub level: BatteryLevel,
    pub charging_status: ChargingStatus,
    pub action: Option<PolicyAction>,
    pub jump_debounced: bool,
    pub sequence: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryEvidence {
    pub policy: BatteryPolicy,
    pub decision: BatteryDecision,
    pub action_count: u64,
}

pub struct BatteryPolicyController {
    policy: BatteryPolicy,
    stable_percent: Option<u8>,
    pending_percent: Option<u8>,
    low_action_done: bool,
    critical_action_done: bool,
    action_count: u64,
    sequence: u64,
    decision: BatteryDecision,
}

impl BatteryPolicyController {
    pub fn new(policy: BatteryPolicy) -> Result<Self, String> {
        policy.validate()?;
        let observation = BatteryObservation {
            percent: None,
            charging: None,
            full: None,
            external_power: None,
            health: BatteryHealth::Unknown,
        };
        Ok(Self {
            policy,
            stable_percent: None,
            pending_percent: None,
            low_action_done: false,
            critical_action_done: false,
            action_count: 0,
            sequence: 0,
            decision: BatteryDecision {
                observation,
                displayed_percent: None,
                level: BatteryLevel::Unknown,
                charging_status: ChargingStatus::Unknown,
                action: None,
                jump_debounced: false,
                sequence: 0,
            },
        })
    }

    pub fn set_policy(&mut self, policy: BatteryPolicy) -> Result<(), String> {
        policy.validate()?;
        self.policy = policy;
        Ok(())
    }

    pub fn policy(&self) -> &BatteryPolicy {
        &self.policy
    }

    pub fn decision(&self) -> &BatteryDecision {
        &self.decision
    }

    pub fn observe(&mut self, observation: BatteryObservation) -> BatteryDecision {
        self.sequence = self.sequence.saturating_add(1);
        let level = match observation.percent {
            Some(percent) if percent <= self.policy.critical_percent => BatteryLevel::Critical,
            Some(percent) if percent <= self.policy.warning_percent => BatteryLevel::Low,
            Some(_) => BatteryLevel::Normal,
            None => BatteryLevel::Unknown,
        };
        let mut jump_debounced = false;
        if let Some(percent) = observation.percent {
            if self
                .stable_percent
                .is_some_and(|stable| stable.abs_diff(percent) > MAX_PLAUSIBLE_JUMP_PERCENT)
            {
                if self.pending_percent == Some(percent) {
                    self.stable_percent = Some(percent);
                    self.pending_percent = None;
                } else {
                    self.pending_percent = Some(percent);
                    jump_debounced = true;
                }
            } else {
                self.stable_percent = Some(percent);
                self.pending_percent = None;
            }
        } else {
            self.stable_percent = None;
            self.pending_percent = None;
        }

        if observation.percent.is_some_and(|percent| {
            percent
                > self
                    .policy
                    .warning_percent
                    .saturating_add(REARM_HYSTERESIS_PERCENT)
        }) {
            self.low_action_done = false;
            self.critical_action_done = false;
        }

        let action = if observation.external_power == Some(true) || jump_debounced {
            None
        } else {
            match level {
                BatteryLevel::Critical if !self.critical_action_done => {
                    self.critical_action_done = true;
                    self.low_action_done = true;
                    Some(PolicyAction::CheckpointAndShutdown)
                }
                BatteryLevel::Low if !self.low_action_done => {
                    self.low_action_done = true;
                    Some(match self.policy.low_battery_action {
                        LowBatteryAction::WarnOnly => PolicyAction::Warn,
                        LowBatteryAction::SaveAndExit => PolicyAction::CheckpointAndExit,
                        LowBatteryAction::ExitWithoutSave => PolicyAction::ExitWithoutSave,
                    })
                }
                _ => None,
            }
        };
        if action.is_some() {
            self.action_count = self.action_count.saturating_add(1);
        }
        self.decision = BatteryDecision {
            observation,
            displayed_percent: self.stable_percent,
            level,
            charging_status: observation.charging_status(),
            action,
            jump_debounced,
            sequence: self.sequence,
        };
        self.decision.clone()
    }

    pub fn evidence(&self) -> BatteryEvidence {
        BatteryEvidence {
            policy: self.policy.clone(),
            decision: self.decision.clone(),
            action_count: self.action_count,
        }
    }
}
