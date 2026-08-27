use std::{fmt, fs, path::Path};

use png::{BitDepth, ColorType, Encoder};
use sdl2::{pixels::PixelFormatEnum, rect::Rect, render::Canvas, video::Window, Sdl};
use serde::Deserialize;
use sim_platform_contract::{
    AudioState, BatteryState, Button, ButtonAction, ButtonEvent, DisplayState,
    HallCalibrationState, HardwareChanges, HardwareDomain, HardwareState, InputState,
    LedState as PlatformLedState, Platform, PlatformCapabilities, PlatformError, PlatformIdentity,
    PlatformResult, PlatformSnapshot, PlatformState, PowerState, RadioState, RadiosState,
    RumbleState, Screen, StorageMode, SuspendResult, SuspendState, UsbRole, UsbState,
};

#[derive(Clone, Copy, Debug)]
pub enum Backend {
    X11,
    Dummy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    #[serde(rename = "contractVersion")]
    contract_version: String,
    #[serde(rename = "targetSku")]
    target_sku: String,
    display: Display,
    controls: Controls,
    battery: Battery,
    led: Led,
    audio: Audio,
    radio: Radio,
    suspend: Suspend,
    #[serde(rename = "virtualStorage")]
    virtual_storage: VirtualStorage,
    clock: Clock,
    faults: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Display {
    #[serde(rename = "logicalWidth")]
    logical_width: u32,
    #[serde(rename = "logicalHeight")]
    logical_height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Controls {
    #[serde(rename = "initialPressed")]
    initial_pressed: Vec<Button>,
    events: Vec<ConfiguredEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfiguredEvent {
    #[serde(rename = "atMs")]
    at_ms: u64,
    control: Button,
    action: ButtonAction,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Battery {
    #[serde(rename = "levelPercent")]
    level_percent: u8,
    charging: bool,
    #[serde(rename = "externalPower")]
    external_power: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Led {
    state: LedState,
    #[serde(rename = "brightnessPercent")]
    brightness_percent: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LedState {
    Off,
    On,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Audio {
    enabled: bool,
    #[serde(rename = "volumePercent")]
    volume_percent: u8,
    #[serde(rename = "active")]
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Radio {
    enabled: bool,
    connected: bool,
    #[serde(rename = "rxPackets")]
    _rx_packets: u64,
    #[serde(rename = "txPackets")]
    _tx_packets: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suspend {
    state: ProfileSuspendState,
    #[serde(rename = "wakeReason")]
    wake_reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ProfileSuspendState {
    Active,
    Suspended,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VirtualStorage {
    #[serde(rename = "readOnly")]
    read_only: bool,
    files: Vec<StorageFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageFile {
    #[serde(rename = "logicalKey")]
    logical_key: String,
    content: StorageContent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StorageContent {
    GeneratedStateV1,
    GeneratedSaveV1,
}

impl StorageContent {
    fn as_str(&self) -> &'static str {
        match self {
            Self::GeneratedStateV1 => "generated-state-v1",
            Self::GeneratedSaveV1 => "generated-save-v1",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Clock {
    #[serde(rename = "startMs")]
    start_ms: u64,
    #[serde(rename = "stepMs")]
    _step_ms: u64,
}

pub struct HostPlatform {
    _sdl: Sdl,
    canvas: Canvas<Window>,
    state: PlatformState,
    events: Vec<ButtonEvent>,
    event_index: usize,
    logical_time_ms: u64,
    target_sku: String,
}

impl HostPlatform {
    pub fn new(profile_path: &Path, backend: Backend) -> PlatformResult<Self> {
        let bytes = fs::read(profile_path).map_err(backend_error)?;
        let profile: Profile = serde_json::from_slice(&bytes).map_err(backend_error)?;
        if profile.contract_version != "1.0.0" || profile.target_sku != "TG4040" {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Display,
                reason: "unsupported synthetic profile".to_string(),
            });
        }
        if profile.display.logical_width != 1024 || profile.display.logical_height != 768 {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Display,
                reason: "unsupported logical display".to_string(),
            });
        }
        if !profile.virtual_storage.read_only {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Storage,
                reason: "virtual storage must be read-only".to_string(),
            });
        }
        if profile.faults.iter().any(|fault| fault == "startup") {
            return Err(PlatformError::Backend("injected startup fault".to_string()));
        }
        for file in &profile.virtual_storage.files {
            if file.logical_key.is_empty()
                || file.logical_key.len() > 32
                || !file.logical_key.chars().all(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
                })
                || !file.logical_key.as_bytes()[0].is_ascii_lowercase()
            {
                return Err(PlatformError::Invalid {
                    domain: HardwareDomain::Storage,
                    reason: "invalid virtual storage key".to_string(),
                });
            }
        }
        let Controls {
            initial_pressed,
            events: configured_events,
        } = profile.controls;
        let events = configured_events
            .into_iter()
            .map(|event| ButtonEvent {
                at_ms: event.at_ms,
                button: event.control,
                action: event.action,
            })
            .collect();
        let virtual_sd = std::env::temp_dir().join("trimui-virtual-sd");
        fs::create_dir_all(&virtual_sd).map_err(backend_error)?;
        for file in profile.virtual_storage.files {
            fs::write(virtual_sd.join(file.logical_key), file.content.as_str())
                .map_err(backend_error)?;
        }

        let video_driver = match backend {
            Backend::X11 => "x11",
            Backend::Dummy => "dummy",
        };
        std::env::set_var("SDL_VIDEODRIVER", video_driver);
        let sdl = sdl2::init().map_err(backend_error)?;
        let video = sdl.video().map_err(backend_error)?;
        let window = video
            .window("host-native userspace simulator", 1024, 768)
            .position_centered()
            .build()
            .map_err(backend_error)?;
        let canvas = window
            .into_canvas()
            .software()
            .build()
            .map_err(backend_error)?;
        let suspend_state = match profile.suspend.state {
            ProfileSuspendState::Active => SuspendState::Active,
            ProfileSuspendState::Suspended => SuspendState::Suspended,
        };
        let suspend_result = match profile.suspend.wake_reason.as_str() {
            "control" => SuspendResult::Success,
            _ => SuspendResult::None,
        };
        let state = PlatformState {
            display: DisplayState {
                logical_width: profile.display.logical_width,
                logical_height: profile.display.logical_height,
            },
            input: InputState {
                pressed: initial_pressed,
            },
            hall_calibration: HallCalibrationState { calibrated: false },
            power: PowerState {
                external_power: profile.battery.external_power,
            },
            battery: BatteryState {
                percent: profile.battery.level_percent,
                charging: profile.battery.charging,
            },
            suspend: (suspend_state, suspend_result),
            radios: RadiosState {
                wifi: RadioState {
                    enabled: profile.radio.enabled,
                    connected: profile.radio.connected,
                },
                bluetooth: RadioState {
                    enabled: false,
                    connected: false,
                },
            },
            audio: AudioState {
                enabled: profile.audio.enabled,
                volume_percent: profile.audio.volume_percent,
                active: profile.audio.active,
            },
            leds: PlatformLedState {
                on: matches!(profile.led.state, LedState::On),
                brightness_percent: profile.led.brightness_percent,
            },
            rumble: RumbleState { active: false },
            usb: UsbState {
                connected: false,
                role: UsbRole::None,
            },
            storage: StorageMode::Available,
        };
        Ok(Self {
            _sdl: sdl,
            canvas,
            state,
            events,
            event_index: 0,
            logical_time_ms: profile.clock.start_ms,
            target_sku: profile.target_sku,
        })
    }

    pub fn target_sku(&self) -> &str {
        &self.target_sku
    }
}

impl Platform for HostPlatform {
    fn identity(&self) -> PlatformIdentity {
        PlatformIdentity {
            target_sku: self.target_sku.clone(),
            lane: "host-native userspace simulator".to_string(),
        }
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::all(sim_platform_contract::CapabilityStatus::Supported)
    }

    fn next_button_event(&mut self) -> PlatformResult<Option<ButtonEvent>> {
        let Some(event) = self.events.get(self.event_index).copied() else {
            return Ok(None);
        };
        self.event_index += 1;
        self.logical_time_ms = event.at_ms;
        match event.action {
            ButtonAction::Press if !self.state.input.pressed.contains(&event.button) => {
                self.state.input.pressed.push(event.button);
            }
            ButtonAction::Release => self
                .state
                .input
                .pressed
                .retain(|button| *button != event.button),
            ButtonAction::Press => {}
        }
        Ok(Some(event))
    }

    fn present(&mut self, screen: &Screen) -> PlatformResult<()> {
        self.canvas
            .set_draw_color(sdl2::pixels::Color::RGB(18, 22, 31));
        self.canvas.clear();
        self.canvas
            .set_draw_color(sdl2::pixels::Color::RGB(40, 52, 75));
        self.canvas
            .fill_rect(Rect::new(48, 40, 928, 96))
            .map_err(backend_error)?;
        for index in 0..screen.entry_count {
            let y = 176 + (index as i32 * 104);
            let selected = index == screen.selected_index;
            self.canvas.set_draw_color(if selected {
                sdl2::pixels::Color::RGB(70, 150, 220)
            } else {
                sdl2::pixels::Color::RGB(42, 48, 61)
            });
            self.canvas
                .fill_rect(Rect::new(96, y, 832, 72))
                .map_err(backend_error)?;
        }
        self.canvas.set_draw_color(match screen.route {
            sim_domain::Route::Library => sdl2::pixels::Color::RGB(96, 210, 130),
            sim_domain::Route::Systems => sdl2::pixels::Color::RGB(100, 170, 230),
            sim_domain::Route::Games => sdl2::pixels::Color::RGB(140, 120, 230),
            sim_domain::Route::Catalog => sdl2::pixels::Color::RGB(96, 210, 130),
            sim_domain::Route::Session => sdl2::pixels::Color::RGB(240, 180, 75),
        });
        self.canvas
            .fill_rect(Rect::new(96, 680, 832, 32))
            .map_err(backend_error)?;
        self.canvas.present();
        Ok(())
    }

    fn capture_png(&mut self, path: &Path) -> PlatformResult<()> {
        let pixels = self
            .canvas
            .read_pixels(None, PixelFormatEnum::RGBA8888)
            .map_err(backend_error)?;
        let file = fs::File::create(path).map_err(backend_error)?;
        let mut encoder = Encoder::new(file, 1024, 768);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(backend_error)?;
        writer.write_image_data(&pixels).map_err(backend_error)
    }

    fn logical_time_ms(&self) -> u64 {
        self.logical_time_ms
    }

    fn snapshot(&self) -> PlatformResult<PlatformSnapshot> {
        Ok(PlatformSnapshot {
            battery_level_percent: self.state.battery.percent,
            charging: self.state.battery.charging,
            led_on: self.state.leds.on,
            audio_enabled: self.state.audio.enabled,
            radio_enabled: self.state.radios.wifi.enabled,
            suspended: matches!(self.state.suspend.0, SuspendState::Suspended),
        })
    }

    fn platform_state(&self) -> PlatformResult<PlatformState> {
        Ok(self.state.clone())
    }

    fn hardware_state(&self) -> PlatformResult<HardwareState> {
        Ok(HardwareState {
            battery_percent: self.state.battery.percent,
            charging: self.state.battery.charging,
            storage_mode: self.state.storage.clone(),
            radio_enabled: self.state.radios.wifi.enabled,
            radio_connected: self.state.radios.wifi.connected,
            suspend_state: self.state.suspend.0.clone(),
            suspend_result: self.state.suspend.1.clone(),
        })
    }

    fn mutate_hardware(&mut self, changes: HardwareChanges) -> PlatformResult<()> {
        if let Some(value) = changes.battery_percent {
            self.state.battery.percent = value;
        }
        if let Some(value) = changes.charging {
            self.state.battery.charging = value;
        }
        if let Some(value) = changes.storage_mode {
            self.state.storage = value;
        }
        if let Some(value) = changes.radio_enabled {
            self.state.radios.wifi.enabled = value;
        }
        if let Some(value) = changes.radio_connected {
            self.state.radios.wifi.connected = value;
        }
        if let Some(value) = changes.suspend_state {
            self.state.suspend.0 = value;
        }
        if let Some(value) = changes.suspend_result {
            self.state.suspend.1 = value;
        }
        Ok(())
    }

    fn display_state(&self) -> PlatformResult<DisplayState> {
        Ok(self.state.display.clone())
    }

    fn input_state(&self) -> PlatformResult<InputState> {
        Ok(self.state.input.clone())
    }

    fn hall_calibration_state(&self) -> PlatformResult<HallCalibrationState> {
        Ok(self.state.hall_calibration)
    }

    fn power_state(&self) -> PlatformResult<PowerState> {
        Ok(self.state.power)
    }

    fn battery_state(&self) -> PlatformResult<BatteryState> {
        Ok(self.state.battery)
    }

    fn suspend_state(&self) -> PlatformResult<(SuspendState, SuspendResult)> {
        Ok(self.state.suspend.clone())
    }

    fn radios_state(&self) -> PlatformResult<RadiosState> {
        Ok(self.state.radios)
    }

    fn audio_state(&self) -> PlatformResult<AudioState> {
        Ok(self.state.audio)
    }

    fn leds_state(&self) -> PlatformResult<PlatformLedState> {
        Ok(self.state.leds)
    }

    fn rumble_state(&self) -> PlatformResult<RumbleState> {
        Ok(self.state.rumble)
    }

    fn usb_state(&self) -> PlatformResult<UsbState> {
        Ok(self.state.usb)
    }

    fn set_hall_calibration(&mut self, calibrated: bool) -> PlatformResult<()> {
        self.state.hall_calibration.calibrated = calibrated;
        Ok(())
    }

    fn set_power(&mut self, state: PowerState) -> PlatformResult<()> {
        self.state.power = state;
        Ok(())
    }

    fn set_battery(&mut self, state: BatteryState) -> PlatformResult<()> {
        if state.percent > 100 {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Battery,
                reason: "percent must be between 0 and 100".to_string(),
            });
        }
        self.state.battery = state;
        Ok(())
    }

    fn set_suspend(&mut self, state: (SuspendState, SuspendResult)) -> PlatformResult<()> {
        self.state.suspend = state;
        Ok(())
    }

    fn set_radios(&mut self, state: RadiosState) -> PlatformResult<()> {
        self.state.radios = state;
        Ok(())
    }

    fn set_audio(&mut self, state: AudioState) -> PlatformResult<()> {
        if state.volume_percent > 100 {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Audio,
                reason: "volume must be between 0 and 100".to_string(),
            });
        }
        self.state.audio = state;
        Ok(())
    }

    fn set_leds(&mut self, state: PlatformLedState) -> PlatformResult<()> {
        if state.brightness_percent > 100 {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Leds,
                reason: "brightness must be between 0 and 100".to_string(),
            });
        }
        self.state.leds = state;
        Ok(())
    }

    fn set_rumble(&mut self, state: RumbleState) -> PlatformResult<()> {
        self.state.rumble = state;
        Ok(())
    }

    fn set_usb(&mut self, state: UsbState) -> PlatformResult<()> {
        self.state.usb = state;
        Ok(())
    }
}

fn backend_error(error: impl fmt::Display) -> PlatformError {
    PlatformError::Backend(error.to_string())
}
