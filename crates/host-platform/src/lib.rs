use std::{fs, path::Path};

use png::{BitDepth, ColorType, Encoder};
use sdl2::{pixels::PixelFormatEnum, rect::Rect, render::Canvas, video::Window, Sdl};
use serde::Deserialize;
use sim_platform_contract::{
    Button, ButtonAction, ButtonEvent, Platform, PlatformResult, PlatformSnapshot, Screen,
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
    _external_power: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Led {
    state: LedState,
    #[serde(rename = "brightnessPercent")]
    _brightness_percent: u8,
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
    _volume_percent: u8,
    #[serde(rename = "active")]
    _active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Radio {
    enabled: bool,
    #[serde(rename = "connected")]
    _connected: bool,
    #[serde(rename = "rxPackets")]
    _rx_packets: u64,
    #[serde(rename = "txPackets")]
    _tx_packets: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suspend {
    state: SuspendState,
    #[serde(rename = "wakeReason")]
    _wake_reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SuspendState {
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
    _initial_pressed: Vec<Button>,
    events: Vec<ButtonEvent>,
    event_index: usize,
    logical_time_ms: u64,
    snapshot: PlatformSnapshot,
    target_sku: String,
}

impl HostPlatform {
    pub fn new(profile_path: &Path, backend: Backend) -> PlatformResult<Self> {
        let bytes = fs::read(profile_path).map_err(|error| error.to_string())?;
        let profile: Profile = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if profile.contract_version != "1.0.0" || profile.target_sku != "TG4040" {
            return Err("unsupported synthetic profile".to_string());
        }
        if profile.display.logical_width != 1024 || profile.display.logical_height != 768 {
            return Err("unsupported logical display".to_string());
        }
        if !profile.virtual_storage.read_only {
            return Err("virtual storage must be read-only".to_string());
        }
        if profile.faults.iter().any(|fault| fault == "startup") {
            return Err("injected startup fault".to_string());
        }
        for file in &profile.virtual_storage.files {
            if file.logical_key.is_empty()
                || file.logical_key.len() > 32
                || !file.logical_key.chars().all(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
                })
                || !file.logical_key.as_bytes()[0].is_ascii_lowercase()
            {
                return Err("invalid virtual storage key".to_string());
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
        fs::create_dir_all(&virtual_sd).map_err(|error| error.to_string())?;
        for file in profile.virtual_storage.files {
            fs::write(virtual_sd.join(file.logical_key), file.content.as_str())
                .map_err(|error| error.to_string())?;
        }

        let video_driver = match backend {
            Backend::X11 => "x11",
            Backend::Dummy => "dummy",
        };
        std::env::set_var("SDL_VIDEODRIVER", video_driver);
        let sdl = sdl2::init().map_err(|error| error.to_string())?;
        let video = sdl.video().map_err(|error| error.to_string())?;
        let window = video
            .window("TG4040 host-native userspace simulator", 1024, 768)
            .position_centered()
            .build()
            .map_err(|error| error.to_string())?;
        let canvas = window
            .into_canvas()
            .software()
            .build()
            .map_err(|error| error.to_string())?;
        let snapshot = PlatformSnapshot {
            battery_level_percent: profile.battery.level_percent,
            charging: profile.battery.charging,
            led_on: matches!(profile.led.state, LedState::On),
            audio_enabled: profile.audio.enabled,
            radio_enabled: profile.radio.enabled,
            suspended: matches!(profile.suspend.state, SuspendState::Suspended),
        };
        Ok(Self {
            _sdl: sdl,
            canvas,
            _initial_pressed: initial_pressed,
            events,
            event_index: 0,
            logical_time_ms: profile.clock.start_ms,
            snapshot,
            target_sku: profile.target_sku,
        })
    }

    pub fn target_sku(&self) -> &str {
        &self.target_sku
    }
}

impl Platform for HostPlatform {
    fn next_button_event(&mut self) -> PlatformResult<Option<ButtonEvent>> {
        let Some(event) = self.events.get(self.event_index).copied() else {
            return Ok(None);
        };
        self.event_index += 1;
        self.logical_time_ms = event.at_ms;
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
            .map_err(|error| error.to_string())?;
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
                .map_err(|error| error.to_string())?;
        }
        self.canvas.set_draw_color(match screen.route {
            sim_domain::Route::Catalog => sdl2::pixels::Color::RGB(96, 210, 130),
            sim_domain::Route::Session => sdl2::pixels::Color::RGB(240, 180, 75),
        });
        self.canvas
            .fill_rect(Rect::new(96, 680, 832, 32))
            .map_err(|error| error.to_string())?;
        self.canvas.present();
        Ok(())
    }

    fn capture_png(&mut self, path: &Path) -> PlatformResult<()> {
        let pixels = self
            .canvas
            .read_pixels(None, PixelFormatEnum::RGBA8888)
            .map_err(|error| error.to_string())?;
        let file = fs::File::create(path).map_err(|error| error.to_string())?;
        let mut encoder = Encoder::new(file, 1024, 768);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&pixels)
            .map_err(|error| error.to_string())
    }

    fn logical_time_ms(&self) -> u64 {
        self.logical_time_ms
    }

    fn snapshot(&self) -> PlatformSnapshot {
        self.snapshot.clone()
    }
}
