//! Source-derived TG4040 capability model. This contains no device I/O.

use serde::{Deserialize, Serialize};

pub const OFFICIAL_ASSET_SHA256: &str =
    "36189b7966717c520901e6ef7318d1b092067598959b030cc0fd66b0747e5585";
pub const CAPABILITY_PROFILE: &str = "tg4040-v1.1.1-source-derived";
pub const LED_CLASS_PATH: &str = "/sys/class/led_anim";
pub const MOTOR_VOLTAGE_PATH: &str = "/sys/class/motor/voltage";
pub const MOTOR_ENABLE_PATH: &str = "/sys/class/gpio/gpio227/value";
pub const BLUETOOTH_READY_PATH: &str = "/sys/class/bluetooth/hci0";
pub const BLUETOOTH_REQUEST_PATH: &str = "/tmp/bluetooth_request";
pub const BLUETOOTH_RESULT_PATH: &str = "/tmp/bluetooth_result";
pub const BLUETOOTH_READY_FILE: &str = "/tmp/bluetooth_ready";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LedZone {
    ShoulderLr,
    Middle,
    F1,
    F2,
    Rear,
}

impl LedZone {
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::ShoulderLr => "lr",
            Self::Middle => "m",
            Self::F1 => "f1",
            Self::F2 => "f2",
            Self::Rear => "rear",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LedEffect {
    Off,
    LowBattery,
}

impl LedEffect {
    pub const fn official_id(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::LowBattery => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LedBrightnessTarget {
    Top,
    Shoulder,
    Joystick,
    F1F2,
}

impl LedBrightnessTarget {
    pub const fn attribute(self) -> &'static str {
        match self {
            Self::Top => "max_scale",
            Self::Shoulder => "max_scale_rear",
            Self::Joystick => "max_scale_lr",
            Self::F1F2 => "max_scale_f1f2",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BluetoothRole {
    Controller,
    Audio,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BluetoothPhase {
    Idle,
    Scanning,
    Pairing,
    Paired,
    Reconnecting,
    Connected,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputSignal {
    /// Observed at GPIO 243; Fn/slider semantics are not assigned by the source inventory.
    Gpio243,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BatteryImpact {
    Unmeasured,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Tg4040Capabilities {
    pub led_zones: [LedZone; 5],
    pub led_effects: [LedEffect; 2],
    pub brightness_targets: [LedBrightnessTarget; 4],
    pub bluetooth_roles: [BluetoothRole; 2],
    pub input_signals: [InputSignal; 1],
    pub motor_voltage_path: &'static str,
    pub motor_enable_path: &'static str,
    pub bluetooth_ready_path: &'static str,
    pub battery_impact: BatteryImpact,
}

impl Tg4040Capabilities {
    pub const fn source_derived() -> Self {
        Self {
            led_zones: [
                LedZone::ShoulderLr,
                LedZone::Middle,
                LedZone::F1,
                LedZone::F2,
                LedZone::Rear,
            ],
            // The official image exposes effect 0 (off) and effect 6 (low battery).
            led_effects: [LedEffect::Off, LedEffect::LowBattery],
            brightness_targets: [
                LedBrightnessTarget::Top,
                LedBrightnessTarget::Shoulder,
                LedBrightnessTarget::Joystick,
                LedBrightnessTarget::F1F2,
            ],
            bluetooth_roles: [BluetoothRole::Controller, BluetoothRole::Audio],
            input_signals: [InputSignal::Gpio243],
            motor_voltage_path: MOTOR_VOLTAGE_PATH,
            motor_enable_path: MOTOR_ENABLE_PATH,
            bluetooth_ready_path: BLUETOOTH_READY_PATH,
            battery_impact: BatteryImpact::Unmeasured,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LedSettings {
    pub enabled: bool,
    pub brightness_percent: u8,
}

impl Default for LedSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            brightness_percent: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BluetoothState {
    pub role: Option<BluetoothRole>,
    pub phase: BluetoothPhase,
    pub local_input_enabled: bool,
}

impl Default for BluetoothState {
    fn default() -> Self {
        Self {
            role: None,
            phase: BluetoothPhase::Idle,
            local_input_enabled: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Tg4040State {
    pub capabilities: Tg4040Capabilities,
    pub persisted_led: LedSettings,
    pub effective_led_enabled: bool,
    pub effective_led_effect: LedEffect,
    pub low_battery_override: bool,
    pub rumble_active: bool,
    pub bluetooth: BluetoothState,
    pub observed_inputs: Vec<InputSignal>,
    pub ownership_active: bool,
    suspended: bool,
}

impl Tg4040State {
    pub const fn source_derived() -> Self {
        Self {
            capabilities: Tg4040Capabilities::source_derived(),
            persisted_led: LedSettings {
                enabled: false,
                brightness_percent: 100,
            },
            effective_led_enabled: false,
            effective_led_effect: LedEffect::Off,
            low_battery_override: false,
            rumble_active: false,
            bluetooth: BluetoothState {
                role: None,
                phase: BluetoothPhase::Idle,
                local_input_enabled: true,
            },
            observed_inputs: Vec::new(),
            ownership_active: false,
            suspended: false,
        }
    }

    pub fn set_led(&mut self, settings: LedSettings) {
        self.persisted_led = settings;
        self.ownership_active = true;
        self.refresh_leds();
    }

    pub fn set_low_battery(&mut self, active: bool) {
        self.low_battery_override = active;
        self.refresh_leds();
    }

    pub fn set_rumble_active(&mut self, active: bool) {
        self.rumble_active = active;
    }

    pub fn suspend(&mut self) {
        self.suspended = true;
        self.rumble_active = false;
        self.refresh_leds();
    }

    pub fn resume(&mut self) {
        self.suspended = false;
        self.refresh_leds();
    }

    pub fn stop_session_effects(&mut self) {
        self.rumble_active = false;
        self.effective_led_effect = LedEffect::Off;
    }

    pub fn observe_input(&mut self, signal: InputSignal) {
        if !self.observed_inputs.contains(&signal) {
            self.observed_inputs.push(signal);
        }
    }

    pub fn scan(&mut self, role: BluetoothRole) {
        self.bluetooth.role = Some(role);
        self.bluetooth.phase = BluetoothPhase::Scanning;
    }

    pub fn pair(&mut self) {
        if self.bluetooth.role.is_some() && self.bluetooth.phase == BluetoothPhase::Scanning {
            self.bluetooth.phase = BluetoothPhase::Pairing;
        }
    }

    pub fn paired(&mut self) {
        if self.bluetooth.role.is_some() && self.bluetooth.phase == BluetoothPhase::Pairing {
            self.bluetooth.phase = BluetoothPhase::Paired;
        }
    }

    pub fn connected(&mut self) {
        if self.bluetooth.role.is_some() {
            self.bluetooth.phase = BluetoothPhase::Connected;
        }
    }

    pub fn reconnect(&mut self) {
        if self.bluetooth.role.is_some() {
            self.bluetooth.phase = BluetoothPhase::Reconnecting;
        }
    }

    pub fn reboot(&mut self) {
        self.low_battery_override = false;
        self.rumble_active = false;
        self.bluetooth = BluetoothState {
            role: self.bluetooth.role,
            phase: if self.bluetooth.role.is_some() {
                BluetoothPhase::Paired
            } else {
                BluetoothPhase::Idle
            },
            local_input_enabled: true,
        };
        self.observed_inputs.clear();
        self.suspended = false;
        self.refresh_leds();
    }

    pub fn reset_to_baseline(&mut self) {
        self.persisted_led = LedSettings::default();
        self.low_battery_override = false;
        self.rumble_active = false;
        self.bluetooth = BluetoothState::default();
        self.observed_inputs.clear();
        self.ownership_active = false;
        self.suspended = false;
        self.refresh_leds();
    }

    fn refresh_leds(&mut self) {
        self.effective_led_enabled =
            self.suspended == false && (self.persisted_led.enabled || self.low_battery_override);
        self.effective_led_effect = if self.suspended {
            LedEffect::Off
        } else if self.low_battery_override {
            LedEffect::LowBattery
        } else {
            LedEffect::Off
        };
    }
}
