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
    wall_clock_ms: u64,
    wake_deadline: Option<sim_platform_contract::lifecycle::WakeDeadline>,
    orderly_shutdown: Option<sim_platform_contract::lifecycle::ShutdownReason>,
    target_sku: String,
    initial_splash_pending: bool,
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
            wall_clock_ms: profile.clock.start_ms,
            wake_deadline: None,
            orderly_shutdown: None,
            target_sku: profile.target_sku,
            initial_splash_pending: true,
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
        if self.initial_splash_pending && screen.splash == "artbook-generated-splash" {
            self.canvas.set_draw_color(rgb(screen.palette.background));
            self.canvas.clear();
            draw_splash(&mut self.canvas, screen);
            self.initial_splash_pending = false;
            self.canvas.present();
            return Ok(());
        }
        self.canvas.set_draw_color(rgb(screen.palette.background));
        self.canvas.clear();
        for region in &screen.regions {
            if !region.visible {
                continue;
            }
            let fill = match region.kind.as_str() {
                "system-art" => screen.palette.accent,
                "game-list" | "menu" => screen.palette.surface,
                "metadata" | "help-strip" => screen.palette.muted,
                "clock" | "battery" => screen.palette.highlight,
                _ => screen.palette.text,
            };
            self.canvas.set_draw_color(rgb(fill));
            self.canvas
                .fill_rect(Rect::new(
                    i32::from(region.x),
                    i32::from(region.y),
                    u32::from(region.width),
                    u32::from(region.height),
                ))
                .map_err(backend_error)?;
        }
        draw_text(
            &mut self.canvas,
            48,
            28,
            &screen.title,
            screen.palette.text,
            3,
        );
        draw_text(
            &mut self.canvas,
            768,
            26,
            &screen.affordances.clock,
            screen.palette.background,
            2,
        );
        let battery = format!(
            "{}%{}",
            screen.affordances.battery_percent,
            if screen.affordances.charging { "+" } else { "" }
        );
        draw_text(
            &mut self.canvas,
            888,
            26,
            &battery,
            screen.palette.background,
            2,
        );

        if screen.theme_fallback.is_some() || screen.splash == "artbook-generated-fallback" {
            draw_fallback(&mut self.canvas, screen);
        } else if screen.route == "settings" {
            draw_settings(&mut self.canvas, screen);
        } else if screen.route.starts_with("wifi-") {
            draw_wifi(&mut self.canvas, screen);
        } else if screen.route.starts_with("scraper-") {
            draw_scraper(&mut self.canvas, screen);
        } else {
            draw_catalog(&mut self.canvas, screen);
        }
        if let Some(modal) = &screen.modal {
            self.canvas.set_draw_color(rgb(screen.palette.background));
            self.canvas
                .fill_rect(Rect::new(180, 190, 664, 300))
                .map_err(backend_error)?;
            draw_text(
                &mut self.canvas,
                224,
                236,
                "MODAL",
                screen.palette.highlight,
                3,
            );
            draw_text(&mut self.canvas, 224, 292, modal, screen.palette.text, 2);
        }
        draw_text(
            &mut self.canvas,
            48,
            706,
            &screen
                .controller_help
                .iter()
                .map(|binding| format!("{}:{}", button_label(binding.button), binding.label))
                .collect::<Vec<_>>()
                .join("  "),
            screen.palette.text,
            2,
        );
        self.canvas.present();
        Ok(())
    }

    fn capture_png(&mut self, path: &Path) -> PlatformResult<()> {
        let pixels = self
            .canvas
            .read_pixels(None, PixelFormatEnum::ABGR8888)
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

    fn wall_clock_ms(&self) -> u64 {
        self.wall_clock_ms
    }

    fn semantic_clock(&mut self, monotonic_ms: u64, wall_clock_ms: u64) -> PlatformResult<()> {
        if monotonic_ms < self.logical_time_ms {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Power,
                reason: "semantic monotonic clock cannot move backwards".into(),
            });
        }
        self.logical_time_ms = monotonic_ms;
        self.wall_clock_ms = wall_clock_ms;
        Ok(())
    }

    fn arm_wake_deadline(
        &mut self,
        deadline: sim_platform_contract::lifecycle::WakeDeadline,
    ) -> PlatformResult<()> {
        if deadline.token == 0 || deadline.monotonic_deadline_ms < self.logical_time_ms {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Suspend,
                reason: "invalid semantic wake deadline".into(),
            });
        }
        self.wake_deadline = Some(deadline);
        Ok(())
    }

    fn verify_wake_deadline(
        &self,
        deadline: &sim_platform_contract::lifecycle::WakeDeadline,
    ) -> PlatformResult<()> {
        if self.wake_deadline.as_ref() == Some(deadline) {
            Ok(())
        } else {
            Err(PlatformError::Invalid {
                domain: HardwareDomain::Suspend,
                reason: "semantic wake deadline mismatch".into(),
            })
        }
    }

    fn clear_wake_deadline(&mut self) -> PlatformResult<()> {
        self.wake_deadline = None;
        Ok(())
    }

    fn request_orderly_shutdown(
        &mut self,
        reason: sim_platform_contract::lifecycle::ShutdownReason,
    ) -> PlatformResult<()> {
        self.orderly_shutdown = Some(reason);
        Ok(())
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
            external_power: self.state.power.external_power,
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
        if let Some(value) = changes.external_power {
            self.state.power.external_power = value;
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

    fn set_input(&mut self, state: InputState) -> PlatformResult<()> {
        self.state.input = state;
        Ok(())
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

fn rgb(color: [u8; 4]) -> sdl2::pixels::Color {
    sdl2::pixels::Color::RGBA(color[0], color[1], color[2], color[3])
}

fn draw_splash(canvas: &mut Canvas<Window>, screen: &Screen) {
    draw_text(canvas, 332, 250, "ARTBOOK", screen.palette.accent, 8);
    draw_text(canvas, 350, 360, "GENERATED SPLASH", screen.palette.text, 3);
    draw_text(canvas, 368, 420, &screen.splash, screen.palette.muted, 2);
}

fn draw_fallback(canvas: &mut Canvas<Window>, screen: &Screen) {
    draw_text(
        canvas,
        292,
        238,
        "SAFE FALLBACK",
        screen.palette.highlight,
        5,
    );
    draw_text(
        canvas,
        330,
        330,
        "GENERATED CONTENT",
        screen.palette.text,
        3,
    );
    draw_text(canvas, 360, 400, &screen.splash, screen.palette.muted, 2);
    if let Some(reason) = &screen.theme_fallback {
        draw_text(canvas, 350, 460, reason, screen.palette.highlight, 2);
    }
}

fn draw_catalog(canvas: &mut Canvas<Window>, screen: &Screen) {
    if screen.route == "systems" {
        draw_text(
            canvas,
            64,
            120,
            &screen.selected_label,
            screen.palette.background,
            3,
        );
    }
    let items = if screen.route == "systems" || screen.game_rows.is_empty() {
        &screen.menu
    } else {
        &screen.game_rows
    };
    for (index, item) in items.iter().take(8).enumerate() {
        let y = 92 + index as i32 * 26;
        let color = if item.selected {
            screen.palette.highlight
        } else {
            screen.palette.text
        };
        draw_text(canvas, 448, y, &item.label, color, 2);
    }
    if let Some(game) = &screen.selected_game {
        draw_text(canvas, 548, 372, &game.title, screen.palette.text, 3);
        draw_text(canvas, 548, 412, &game.description, screen.palette.text, 1);
        let metadata = format!(
            "rating {:?}  release {:?}{}",
            game.rating,
            game.release_date,
            if game.favorite { "  favorite" } else { "" }
        );
        draw_text(canvas, 548, 468, &metadata, screen.palette.highlight, 2);
        draw_text(canvas, 62, 372, "BOX ART", screen.palette.background, 3);
        draw_text(canvas, 260, 404, "SCREENSHOT", screen.palette.background, 2);
    }
    if let Some(fallback) = &screen.theme_fallback {
        draw_text(
            canvas,
            64,
            612,
            &format!("SAFE FALLBACK {fallback}"),
            screen.palette.highlight,
            2,
        );
    }
}

fn draw_settings(canvas: &mut Canvas<Window>, screen: &Screen) {
    let Some(settings) = &screen.settings else {
        return;
    };
    let mut y = 96;
    for section in &settings.sections {
        draw_text(
            canvas,
            64,
            y,
            &section.label_key,
            screen.palette.highlight,
            2,
        );
        y += 24;
        for control in &section.controls {
            let value = format!("{} = {:?}", control.label_key, control.value);
            draw_text(canvas, 96, y, &value, screen.palette.text, 1);
            y += 18;
            if y > 650 {
                return;
            }
        }
        y += 10;
    }
}

fn draw_wifi(canvas: &mut Canvas<Window>, screen: &Screen) {
    if let Some(wifi) = &screen.wifi {
        let mut y = 108;
        for network in wifi.networks.iter().take(8) {
            let marker = if network.selected { ">" } else { " " };
            let row = format!(
                "{} {} {}%",
                marker, network.display_ssid, network.signal_quality
            );
            draw_text(canvas, 64, y, &row, screen.palette.text, 2);
            y += 30;
        }
        if let Some(keyboard) = &wifi.keyboard {
            let label = if keyboard.masked {
                "PASSWORD •••"
            } else {
                "NETWORK NAME"
            };
            draw_text(canvas, 64, 560, label, screen.palette.highlight, 3);
        }
        if wifi.open_confirmation {
            draw_text(
                canvas,
                64,
                610,
                "CONFIRM OPEN NETWORK",
                screen.palette.highlight,
                2,
            );
        }
    }
}

fn draw_scraper(canvas: &mut Canvas<Window>, screen: &Screen) {
    let scraper = &screen.scraper;
    canvas.set_draw_color(rgb(screen.palette.background));
    let _ = canvas.fill_rect(Rect::new(32, 72, 960, 610));
    draw_text(
        canvas,
        64,
        104,
        "BULK COVER SCRAPER",
        screen.palette.highlight,
        3,
    );
    draw_text(
        canvas,
        64,
        148,
        &format!("STATUS {}", scraper.status),
        screen.palette.text,
        2,
    );
    draw_text(
        canvas,
        64,
        180,
        &format!(
            "COMPLETED {}/{}  SLOTS {}",
            scraper.completed, scraper.total, scraper.configured_slots
        ),
        screen.palette.text,
        2,
    );
    if let Some(progress) = scraper.progress_percent {
        canvas.set_draw_color(rgb(screen.palette.muted));
        let _ = canvas.fill_rect(Rect::new(64, 214, 896, 18));
        canvas.set_draw_color(rgb(screen.palette.highlight));
        let _ = canvas.fill_rect(Rect::new(64, 214, 896 * u32::from(progress) / 100, 18));
        draw_text(
            canvas,
            64,
            252,
            &format!("PROGRESS {}%", progress),
            screen.palette.highlight,
            3,
        );
    }
    let counts = &scraper.counts;
    draw_text(
        canvas,
        64,
        294,
        &format!(
            "FOUND {}  FALLBACK {}  NOT FOUND {}  AMBIGUOUS {}  FAILED {}",
            counts.succeeded, counts.fallback, counts.not_found, counts.ambiguous, counts.failed
        ),
        screen.palette.text,
        1,
    );
    if scraper.paused {
        draw_text(
            canvas,
            64,
            326,
            &format!(
                "PAUSED {}",
                scraper.paused_reason.as_deref().unwrap_or("user-paused")
            ),
            screen.palette.highlight,
            2,
        );
    } else if scraper.background {
        draw_text(
            canvas,
            64,
            326,
            "RUNNING IN BACKGROUND",
            screen.palette.highlight,
            2,
        );
    }
    for (index, row) in scraper
        .rows
        .iter()
        .enumerate()
        .take(scraper.configured_slots as usize)
    {
        draw_text(
            canvas,
            64,
            366 + index as i32 * 48,
            &format!(
                "{}  {}  {}",
                row.title,
                row.provider.as_deref().unwrap_or("-"),
                format_debug(&row.phase)
            ),
            screen.palette.text,
            2,
        );
        if let Some(transition) = &row.fallback_transition {
            draw_text(
                canvas,
                96,
                394 + index as i32 * 48,
                transition,
                screen.palette.muted,
                1,
            );
        }
    }
    for (index, candidate) in scraper.ambiguous_candidates.iter().enumerate() {
        draw_text(
            canvas,
            96,
            420 + index as i32 * 30,
            candidate,
            screen.palette.text,
            2,
        );
    }
}

fn format_debug<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:?}").to_ascii_lowercase().replace('_', "-")
}

fn button_label(button: ui_model::Button) -> &'static str {
    match button {
        ui_model::Button::Up => "UP",
        ui_model::Button::Down => "DOWN",
        ui_model::Button::Left => "LEFT",
        ui_model::Button::Right => "RIGHT",
        ui_model::Button::Primary => "A",
        ui_model::Button::Secondary => "B",
        ui_model::Button::Start => "START",
        ui_model::Button::Select => "SELECT",
        ui_model::Button::Menu => "MENU",
    }
}

fn draw_text(canvas: &mut Canvas<Window>, x: i32, y: i32, text: &str, color: [u8; 4], scale: i32) {
    let mut cursor = x;
    for character in text.chars().take(72) {
        if character == ' ' {
            cursor += 6 * scale;
            continue;
        }
        let glyph = glyph(character);
        canvas.set_draw_color(rgb(color));
        for (row, line) in glyph.iter().enumerate() {
            for (column, value) in line.bytes().enumerate() {
                if value == b'#' {
                    let _ = canvas.fill_rect(Rect::new(
                        cursor + column as i32 * scale,
                        y + row as i32 * scale,
                        scale as u32,
                        scale as u32,
                    ));
                }
            }
        }
        cursor += 6 * scale;
    }
}

fn glyph(character: char) -> [&'static str; 7] {
    match character.to_ascii_uppercase() {
        'A' => [
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'B' => [
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
        'C' => [
            ".####", "#....", "#....", "#....", "#....", "#....", ".####",
        ],
        'D' => [
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ],
        'E' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        'F' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#....",
        ],
        'G' => [
            ".####", "#....", "#....", "#.###", "#...#", "#...#", ".###.",
        ],
        'H' => [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'I' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####",
        ],
        'J' => [
            "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
        ],
        'K' => [
            "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
        ],
        'L' => [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
        'M' => [
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
        'N' => [
            "#...#", "##..#", "##..#", "#.#.#", "#..##", "#..##", "#...#",
        ],
        'O' => [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'P' => [
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
        'Q' => [
            ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#",
        ],
        'R' => [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
        'S' => [
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
        'T' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        'U' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'V' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..",
        ],
        'W' => [
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ],
        'X' => [
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ],
        'Y' => [
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
        'Z' => [
            "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####",
        ],
        '0' => [
            ".###.", "#..##", "#.#.#", "##..#", "#...#", "#...#", ".###.",
        ],
        '1' => [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
        '2' => [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
        '3' => [
            "####.", "....#", "....#", ".###.", "....#", "....#", "####.",
        ],
        '4' => [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
        '5' => [
            "#####", "#....", "#....", "####.", "....#", "....#", "####.",
        ],
        '6' => [
            ".###.", "#....", "#....", "####.", "#...#", "#...#", ".###.",
        ],
        '7' => [
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
        '8' => [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
        '9' => [
            ".###.", "#...#", "#...#", ".####", "....#", "....#", ".###.",
        ],
        '-' => [
            ".....", ".....", ".....", ".###.", ".....", ".....", ".....",
        ],
        '%' => [
            "#...#", "...#.", "..#..", ".#...", "#...#", ".....", ".....",
        ],
        '+' => [
            ".....", "..#..", "..#..", ".###.", "..#..", "..#..", ".....",
        ],
        '.' => [
            ".....", ".....", ".....", ".....", ".....", ".#...", ".#...",
        ],
        ':' => [
            ".....", ".#...", ".#...", ".....", ".#...", ".#...", ".....",
        ],
        '>' => [
            "#....", ".#...", "..#..", "...#.", "..#..", ".#...", "#....",
        ],
        _ => [
            "#####", "#...#", "...#.", "..#..", "...#.", "#...#", "#####",
        ],
    }
}
