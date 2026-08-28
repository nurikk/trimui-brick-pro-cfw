//! Generic platform HAL for UI and launcher code.
//!
//! The contract describes typed logical operations only. A backend may expose
//! synthetic state, but unknown target hardware must use explicit unavailable
//! errors and must not probe or access device nodes.

use std::{fmt, path::Path};

pub use launcher_presentation::Screen;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Primary,
    Secondary,
    Start,
    Select,
    Menu,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonAction {
    Press,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ButtonEvent {
    pub at_ms: u64,
    pub button: Button,
    pub action: ButtonAction,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HardwareDomain {
    Display,
    Input,
    HallCalibration,
    Power,
    Battery,
    Suspend,
    Radios,
    Audio,
    Leds,
    Rumble,
    Usb,
    Storage,
}

impl HardwareDomain {
    pub const ALL: [Self; 12] = [
        Self::Display,
        Self::Input,
        Self::HallCalibration,
        Self::Power,
        Self::Battery,
        Self::Suspend,
        Self::Radios,
        Self::Audio,
        Self::Leds,
        Self::Rumble,
        Self::Usb,
        Self::Storage,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Input => "input",
            Self::HallCalibration => "hall-calibration",
            Self::Power => "power",
            Self::Battery => "battery",
            Self::Suspend => "suspend",
            Self::Radios => "radios",
            Self::Audio => "audio",
            Self::Leds => "leds",
            Self::Rumble => "rumble",
            Self::Usb => "usb",
            Self::Storage => "storage",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CapabilityStatus {
    Supported,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capability {
    pub domain: HardwareDomain,
    pub status: CapabilityStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformCapabilities {
    pub domains: Vec<Capability>,
}

impl PlatformCapabilities {
    pub fn all(status: CapabilityStatus) -> Self {
        Self {
            domains: HardwareDomain::ALL
                .into_iter()
                .map(|domain| Capability { domain, status })
                .collect(),
        }
    }

    pub fn status(&self, domain: HardwareDomain) -> CapabilityStatus {
        self.domains
            .iter()
            .find(|capability| capability.domain == domain)
            .map_or(CapabilityStatus::Unavailable, |capability| {
                capability.status
            })
    }

    pub fn supports(&self, domain: HardwareDomain) -> bool {
        self.status(domain) == CapabilityStatus::Supported
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformIdentity {
    pub target_sku: String,
    pub lane: String,
}

impl PlatformIdentity {
    pub fn unknown() -> Self {
        Self {
            target_sku: "unknown".to_string(),
            lane: "unknown".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformError {
    Unsupported {
        domain: HardwareDomain,
        operation: String,
    },
    Unavailable {
        domain: HardwareDomain,
        operation: String,
        reason: String,
    },
    Invalid {
        domain: HardwareDomain,
        reason: String,
    },
    Backend(String),
}

impl PlatformError {
    pub fn unsupported(domain: HardwareDomain, operation: impl Into<String>) -> Self {
        Self::Unsupported {
            domain,
            operation: operation.into(),
        }
    }

    pub fn unavailable(
        domain: HardwareDomain,
        operation: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::Unavailable {
            domain,
            operation: operation.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { domain, operation } => {
                write!(
                    formatter,
                    "{} is unsupported for {}",
                    operation,
                    domain.as_str()
                )
            }
            Self::Unavailable {
                domain,
                operation,
                reason,
            } => write!(
                formatter,
                "{} is unavailable for {}: {}",
                operation,
                domain.as_str(),
                reason
            ),
            Self::Invalid { domain, reason } => {
                write!(formatter, "invalid {} state: {}", domain.as_str(), reason)
            }
            Self::Backend(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PlatformError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    Available,
    Full,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuspendState {
    Active,
    Suspended,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuspendResult {
    None,
    Success,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayState {
    pub logical_width: u32,
    pub logical_height: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InputState {
    pub pressed: Vec<Button>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HallCalibrationState {
    pub calibrated: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PowerState {
    pub external_power: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BatteryState {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RadioState {
    pub enabled: bool,
    pub connected: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RadiosState {
    pub wifi: RadioState,
    pub bluetooth: RadioState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioState {
    pub enabled: bool,
    pub volume_percent: u8,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LedState {
    pub on: bool,
    pub brightness_percent: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RumbleState {
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum UsbRole {
    None,
    Host,
    Device,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UsbState {
    pub connected: bool,
    pub role: UsbRole,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlatformState {
    pub display: DisplayState,
    pub input: InputState,
    pub hall_calibration: HallCalibrationState,
    pub power: PowerState,
    pub battery: BatteryState,
    pub suspend: (SuspendState, SuspendResult),
    pub radios: RadiosState,
    pub audio: AudioState,
    pub leds: LedState,
    pub rumble: RumbleState,
    pub usb: UsbState,
    pub storage: StorageMode,
}

#[derive(Clone, Debug, Serialize)]
pub struct HardwareState {
    pub battery_percent: u8,
    pub charging: bool,
    pub storage_mode: StorageMode,
    pub radio_enabled: bool,
    pub radio_connected: bool,
    pub suspend_state: SuspendState,
    pub suspend_result: SuspendResult,
}

#[derive(Clone, Debug, Default)]
pub struct HardwareChanges {
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub storage_mode: Option<StorageMode>,
    pub radio_enabled: Option<bool>,
    pub radio_connected: Option<bool>,
    pub suspend_state: Option<SuspendState>,
    pub suspend_result: Option<SuspendResult>,
}

#[derive(Clone, Debug)]
pub struct PlatformSnapshot {
    pub battery_level_percent: u8,
    pub charging: bool,
    pub led_on: bool,
    pub audio_enabled: bool,
    pub radio_enabled: bool,
    pub suspended: bool,
}

pub type PlatformResult<T> = Result<T, PlatformError>;

pub trait Platform {
    fn identity(&self) -> PlatformIdentity {
        PlatformIdentity::unknown()
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::all(CapabilityStatus::Unsupported)
    }

    fn next_button_event(&mut self) -> PlatformResult<Option<ButtonEvent>>;
    fn present(&mut self, screen: &Screen) -> PlatformResult<()>;
    fn capture_png(&mut self, path: &Path) -> PlatformResult<()>;
    fn logical_time_ms(&self) -> u64;
    fn snapshot(&self) -> PlatformResult<PlatformSnapshot>;
    fn platform_state(&self) -> PlatformResult<PlatformState>;
    fn hardware_state(&self) -> PlatformResult<HardwareState>;
    fn mutate_hardware(&mut self, changes: HardwareChanges) -> PlatformResult<()>;

    fn display_state(&self) -> PlatformResult<DisplayState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Display,
            "read state",
        ))
    }

    fn input_state(&self) -> PlatformResult<InputState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Input,
            "read state",
        ))
    }

    fn hall_calibration_state(&self) -> PlatformResult<HallCalibrationState> {
        Err(PlatformError::unsupported(
            HardwareDomain::HallCalibration,
            "read state",
        ))
    }

    fn power_state(&self) -> PlatformResult<PowerState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Power,
            "read state",
        ))
    }

    fn battery_state(&self) -> PlatformResult<BatteryState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Battery,
            "read state",
        ))
    }

    fn suspend_state(&self) -> PlatformResult<(SuspendState, SuspendResult)> {
        Err(PlatformError::unsupported(
            HardwareDomain::Suspend,
            "read state",
        ))
    }

    fn radios_state(&self) -> PlatformResult<RadiosState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Radios,
            "read state",
        ))
    }

    fn audio_state(&self) -> PlatformResult<AudioState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Audio,
            "read state",
        ))
    }

    fn leds_state(&self) -> PlatformResult<LedState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Leds,
            "read state",
        ))
    }

    fn rumble_state(&self) -> PlatformResult<RumbleState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Rumble,
            "read state",
        ))
    }

    fn usb_state(&self) -> PlatformResult<UsbState> {
        Err(PlatformError::unsupported(
            HardwareDomain::Usb,
            "read state",
        ))
    }

    fn set_hall_calibration(&mut self, _calibrated: bool) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::HallCalibration,
            "set calibration",
        ))
    }

    fn set_power(&mut self, _state: PowerState) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::Power,
            "set state",
        ))
    }

    fn set_battery(&mut self, _state: BatteryState) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::Battery,
            "set state",
        ))
    }

    fn set_suspend(&mut self, _state: (SuspendState, SuspendResult)) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::Suspend,
            "set state",
        ))
    }

    fn set_radios(&mut self, _state: RadiosState) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::Radios,
            "set state",
        ))
    }

    fn set_audio(&mut self, _state: AudioState) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::Audio,
            "set state",
        ))
    }

    fn set_leds(&mut self, _state: LedState) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::Leds,
            "set state",
        ))
    }

    fn set_rumble(&mut self, _state: RumbleState) -> PlatformResult<()> {
        Err(PlatformError::unsupported(
            HardwareDomain::Rumble,
            "set state",
        ))
    }

    fn set_usb(&mut self, _state: UsbState) -> PlatformResult<()> {
        Err(PlatformError::unsupported(HardwareDomain::Usb, "set state"))
    }
}

pub struct UnavailableTg4040Platform;

impl UnavailableTg4040Platform {
    pub fn new() -> Self {
        Self
    }

    fn unavailable<T>(domain: HardwareDomain, operation: &str) -> PlatformResult<T> {
        Err(PlatformError::unavailable(
            domain,
            operation,
            "no-device contract has no backend",
        ))
    }
}

impl Default for UnavailableTg4040Platform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for UnavailableTg4040Platform {
    fn identity(&self) -> PlatformIdentity {
        PlatformIdentity::unknown()
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::all(CapabilityStatus::Unavailable)
    }

    fn next_button_event(&mut self) -> PlatformResult<Option<ButtonEvent>> {
        Self::unavailable(HardwareDomain::Input, "read event")
    }

    fn present(&mut self, _screen: &Screen) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Display, "present frame")
    }

    fn capture_png(&mut self, _path: &Path) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Display, "capture frame")
    }

    fn logical_time_ms(&self) -> u64 {
        0
    }

    fn snapshot(&self) -> PlatformResult<PlatformSnapshot> {
        Self::unavailable(HardwareDomain::Display, "read snapshot")
    }

    fn platform_state(&self) -> PlatformResult<PlatformState> {
        Self::unavailable(HardwareDomain::Display, "read platform state")
    }

    fn hardware_state(&self) -> PlatformResult<HardwareState> {
        Self::unavailable(HardwareDomain::Battery, "read hardware state")
    }

    fn mutate_hardware(&mut self, _changes: HardwareChanges) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Storage, "mutate hardware state")
    }

    fn display_state(&self) -> PlatformResult<DisplayState> {
        Self::unavailable(HardwareDomain::Display, "read state")
    }

    fn input_state(&self) -> PlatformResult<InputState> {
        Self::unavailable(HardwareDomain::Input, "read state")
    }

    fn hall_calibration_state(&self) -> PlatformResult<HallCalibrationState> {
        Self::unavailable(HardwareDomain::HallCalibration, "read state")
    }

    fn power_state(&self) -> PlatformResult<PowerState> {
        Self::unavailable(HardwareDomain::Power, "read state")
    }

    fn battery_state(&self) -> PlatformResult<BatteryState> {
        Self::unavailable(HardwareDomain::Battery, "read state")
    }

    fn suspend_state(&self) -> PlatformResult<(SuspendState, SuspendResult)> {
        Self::unavailable(HardwareDomain::Suspend, "read state")
    }

    fn radios_state(&self) -> PlatformResult<RadiosState> {
        Self::unavailable(HardwareDomain::Radios, "read state")
    }

    fn audio_state(&self) -> PlatformResult<AudioState> {
        Self::unavailable(HardwareDomain::Audio, "read state")
    }

    fn leds_state(&self) -> PlatformResult<LedState> {
        Self::unavailable(HardwareDomain::Leds, "read state")
    }

    fn rumble_state(&self) -> PlatformResult<RumbleState> {
        Self::unavailable(HardwareDomain::Rumble, "read state")
    }

    fn usb_state(&self) -> PlatformResult<UsbState> {
        Self::unavailable(HardwareDomain::Usb, "read state")
    }

    fn set_hall_calibration(&mut self, _calibrated: bool) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::HallCalibration, "set calibration")
    }

    fn set_power(&mut self, _state: PowerState) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Power, "set state")
    }

    fn set_battery(&mut self, _state: BatteryState) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Battery, "set state")
    }

    fn set_suspend(&mut self, _state: (SuspendState, SuspendResult)) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Suspend, "set state")
    }

    fn set_radios(&mut self, _state: RadiosState) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Radios, "set state")
    }

    fn set_audio(&mut self, _state: AudioState) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Audio, "set state")
    }

    fn set_leds(&mut self, _state: LedState) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Leds, "set state")
    }

    fn set_rumble(&mut self, _state: RumbleState) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Rumble, "set state")
    }

    fn set_usb(&mut self, _state: UsbState) -> PlatformResult<()> {
        Self::unavailable(HardwareDomain::Usb, "set state")
    }
}
