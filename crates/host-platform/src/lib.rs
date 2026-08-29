use std::{fmt, fs, path::Path, sync::OnceLock};

use png::{BitDepth, ColorType, Decoder, Encoder, Transformations};
use sdl2::{
    event::Event,
    keyboard::Keycode,
    pixels::PixelFormatEnum,
    rect::Rect,
    render::{BlendMode, Canvas},
    video::Window,
    EventPump, Sdl,
};
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
    logical_width: u32,
    logical_height: u32,
    initial_splash_pending: bool,
    event_pump: EventPump,
}

impl HostPlatform {
    pub fn new(
        profile_path: &Path,
        device_profile_path: &Path,
        backend: Backend,
    ) -> PlatformResult<Self> {
        let bytes = fs::read(profile_path).map_err(backend_error)?;
        let profile: Profile = serde_json::from_slice(&bytes).map_err(backend_error)?;
        let device =
            device_profile::DeviceProfile::from_path(device_profile_path).map_err(backend_error)?;
        let viewport = device.virtual_viewport();
        if profile.contract_version != "1.0.0" {
            return Err(PlatformError::Invalid {
                domain: HardwareDomain::Display,
                reason: "unsupported synthetic profile".to_string(),
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
            .window(
                "Host-native simulator acceptance — not physical TG4040 evidence",
                u32::from(viewport.width),
                u32::from(viewport.height),
            )
            .position_centered()
            .build()
            .map_err(backend_error)?;
        let canvas = window
            .into_canvas()
            .software()
            .build()
            .map_err(backend_error)?;
        let event_pump = sdl.event_pump().map_err(backend_error)?;
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
                logical_width: u32::from(viewport.width),
                logical_height: u32::from(viewport.height),
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
            target_sku: viewport.target_sku,
            logical_width: u32::from(viewport.width),
            logical_height: u32::from(viewport.height),
            initial_splash_pending: true,
            event_pump,
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
        let event = if let Some(event) = self.events.get(self.event_index).copied() {
            self.event_index += 1;
            event
        } else if let Some((button, action)) =
            self.event_pump.poll_iter().find_map(sdl_button_event)
        {
            ButtonEvent {
                at_ms: self.logical_time_ms.saturating_add(1),
                button,
                action,
            }
        } else {
            return Ok(None);
        };
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
        if self.initial_splash_pending && screen.splash == "nova8-splash" {
            self.canvas.set_draw_color(rgb(screen.palette.background));
            self.canvas.clear();
            draw_splash(&mut self.canvas, screen);
            self.initial_splash_pending = false;
            self.canvas.present();
            return Ok(());
        }
        let art_book = is_art_book_next(screen);
        self.canvas
            .set_logical_size(
                if art_book { 1024 } else { self.logical_width },
                if art_book { 768 } else { self.logical_height },
            )
            .map_err(backend_error)?;
        draw_backdrop(&mut self.canvas, screen);
        if art_book {
            match screen.route.as_str() {
                "controller-routes" => draw_controller_routes(&mut self.canvas, screen)?,
                "theme-garden" => draw_theme_garden(&mut self.canvas, screen)?,
                "home" | "systems" | "games" | "games-no-metadata" | "session" => {
                    draw_art_book_next(&mut self.canvas, screen)?;
                }
                "settings" => draw_settings(&mut self.canvas, screen),
                route if route.starts_with("wifi-") => draw_wifi(&mut self.canvas, screen),
                route if route.starts_with("scraper-") => draw_scraper(&mut self.canvas, screen),
                "game-switcher" | "recovery" => {
                    if screen.theme_fallback.is_some() {
                        draw_fallback(&mut self.canvas, screen);
                    } else {
                        draw_route_surface(&mut self.canvas, screen);
                    }
                }
                _ if is_auxiliary_route(&screen.route) => {
                    draw_route_surface(&mut self.canvas, screen)
                }
                _ => draw_art_book_next(&mut self.canvas, screen)?,
            }
        } else {
            draw_theme_components(&mut self.canvas, screen)?;
            draw_theme_media(&mut self.canvas, screen)?;
            draw_text(
                &mut self.canvas,
                48,
                28,
                &screen.title,
                screen.palette.text,
                3,
            );
            let (viewport_width, _) = self.canvas.output_size().map_err(backend_error)?;
            draw_text(
                &mut self.canvas,
                viewport_width.saturating_sub(256) as i32,
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
                viewport_width.saturating_sub(136) as i32,
                26,
                &battery,
                screen.palette.background,
                2,
            );

            if screen.theme_fallback.is_some() || screen.splash == "nova8-fallback" {
                draw_fallback(&mut self.canvas, screen);
            } else if screen.route == "theme-garden" {
                draw_theme_garden(&mut self.canvas, screen)?;
            } else if screen.route == "settings" {
                draw_settings(&mut self.canvas, screen);
            } else if screen.route.starts_with("wifi-") {
                draw_wifi(&mut self.canvas, screen);
            } else if screen.route.starts_with("scraper-") {
                draw_scraper(&mut self.canvas, screen);
            } else if screen.route == "game-switcher"
                || screen.route == "recovery"
                || is_auxiliary_route(&screen.route)
            {
                draw_route_surface(&mut self.canvas, screen);
            } else {
                draw_catalog(&mut self.canvas, screen);
            }
        }
        if !is_art_book_next(screen) {
            if let Some(modal) = &screen.modal {
                let dialog = Rect::new(160, 174, 704, 340);
                self.canvas.set_draw_color(rgb(screen.palette.background));
                self.canvas.fill_rect(dialog).map_err(backend_error)?;
                self.canvas.set_draw_color(rgb(screen.palette.highlight));
                self.canvas.draw_rect(dialog).map_err(backend_error)?;
                draw_text(&mut self.canvas, 208, 240, modal, screen.palette.text, 2);
                for (index, binding) in screen.controller_help.iter().take(2).enumerate() {
                    draw_text(
                        &mut self.canvas,
                        208 + index as i32 * 260,
                        432,
                        &binding.label,
                        screen.palette.highlight,
                        1,
                    );
                }
            }
        }
        let mut help_x = 48;
        for binding in screen.controller_help.iter().take(5) {
            let label = button_label(binding.button);
            draw_controller_badge(
                &mut self.canvas,
                help_x,
                704,
                screen.palette.highlight,
                label,
            );
            draw_text(
                &mut self.canvas,
                help_x + 78,
                709,
                &binding.label,
                screen.palette.text,
                1,
            );
            help_x += 180;
        }
        self.canvas.present();
        Ok(())
    }

    fn capture_png(&mut self, path: &Path) -> PlatformResult<()> {
        let pixels = self
            .canvas
            .read_pixels(None, PixelFormatEnum::ABGR8888)
            .map_err(backend_error)?;
        let file = fs::File::create(path).map_err(backend_error)?;
        let mut encoder = Encoder::new(file, self.logical_width, self.logical_height);
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

fn sdl_button_event(event: Event) -> Option<(Button, ButtonAction)> {
    let (keycode, action) = match event {
        Event::KeyDown {
            keycode: Some(keycode),
            repeat: false,
            ..
        } => (keycode, ButtonAction::Press),
        Event::KeyUp {
            keycode: Some(keycode),
            repeat: false,
            ..
        } => (keycode, ButtonAction::Release),
        _ => return None,
    };
    let button = match keycode {
        Keycode::Up => Button::Up,
        Keycode::Down => Button::Down,
        Keycode::Left => Button::Left,
        Keycode::Right => Button::Right,
        Keycode::Return | Keycode::Z | Keycode::A => Button::Primary,
        Keycode::Escape | Keycode::X | Keycode::B => Button::Secondary,
        Keycode::Space => Button::Start,
        Keycode::Tab => Button::Select,
        Keycode::Home => Button::Menu,
        Keycode::PageUp => Button::L1,
        Keycode::PageDown => Button::R1,
        _ => return None,
    };
    Some((button, action))
}

fn backend_error(error: impl fmt::Display) -> PlatformError {
    PlatformError::Backend(error.to_string())
}

fn rgb(color: [u8; 4]) -> sdl2::pixels::Color {
    sdl2::pixels::Color::RGBA(color[0], color[1], color[2], color[3])
}

fn draw_backdrop(canvas: &mut Canvas<Window>, screen: &Screen) {
    canvas.set_draw_color(rgb(screen.palette.background));
    canvas.clear();
}

fn is_art_book_next(screen: &Screen) -> bool {
    screen.theme.theme == "Art Book Next (Batocera ES Edition)"
}

fn draw_controller_routes(canvas: &mut Canvas<Window>, screen: &Screen) -> PlatformResult<()> {
    canvas.set_draw_color(rgb([12, 12, 12, 255]));
    canvas.clear();
    draw_text(canvas, 48, 36, "CONTROLLER ROUTES", screen.palette.text, 4);
    draw_text(
        canvas,
        48,
        88,
        &screen.selected_label,
        screen.palette.highlight,
        2,
    );
    let selected = screen
        .menu
        .iter()
        .position(|item| item.selected)
        .unwrap_or(0);
    let start = selected
        .saturating_sub(5)
        .min(screen.menu.len().saturating_sub(12));
    for (index, item) in screen.menu.iter().skip(start).take(12).enumerate() {
        let y = 142 + index as i32 * 44;
        if item.selected {
            canvas.set_draw_color(rgb([48, 48, 48, 255]));
            canvas
                .fill_rect(Rect::new(40, y - 7, 944, 38))
                .map_err(backend_error)?;
        }
        draw_text(
            canvas,
            64,
            y,
            &item.label,
            if item.selected {
                screen.palette.highlight
            } else {
                screen.palette.text
            },
            2,
        );
    }
    Ok(())
}

fn art_book_asset<'a>(screen: &'a Screen, path: &str) -> Option<(u32, u32, &'a [u8])> {
    screen
        .theme
        .assets
        .iter()
        .find(|asset| asset.path == path)
        .map(|asset| (asset.width, asset.height, asset.pixels.as_slice()))
}

fn draw_art_book_logo(
    canvas: &mut Canvas<Window>,
    screen: &Screen,
    bounds: Rect,
) -> PlatformResult<()> {
    if let Some((width, height, pixels)) =
        art_book_asset(screen, "./_inc/systems/logos/genesis.png")
    {
        draw_theme_asset(canvas, width, height, pixels, bounds)?;
    }
    Ok(())
}

fn menu_icon_line(
    canvas: &mut Canvas<Window>,
    x: i32,
    y: i32,
    end_x: i32,
    end_y: i32,
) -> PlatformResult<()> {
    canvas
        .draw_line(
            sdl2::rect::Point::new(x, y),
            sdl2::rect::Point::new(end_x, end_y),
        )
        .map_err(backend_error)
}

fn draw_menu_icon(
    canvas: &mut Canvas<Window>,
    index: usize,
    x: i32,
    y: i32,
    selected: bool,
) -> PlatformResult<()> {
    canvas.set_draw_color(rgb(if selected {
        [20, 20, 20, 255]
    } else {
        [190, 190, 190, 255]
    }));
    match index {
        0 => {
            canvas
                .draw_rect(Rect::new(x + 5, y + 3, 12, 16))
                .map_err(backend_error)?;
            menu_icon_line(canvas, x + 8, y + 3, x + 8, y + 7)?;
            menu_icon_line(canvas, x + 14, y + 3, x + 14, y + 7)?;
            menu_icon_line(canvas, x + 8, y + 13, x + 14, y + 13)?;
        }
        1 => {
            for (offset_x, offset_y, width, height) in
                [(3, 7, 15, 11), (5, 4, 15, 11), (7, 1, 12, 10)]
            {
                canvas
                    .draw_rect(Rect::new(x + offset_x, y + offset_y, width, height))
                    .map_err(backend_error)?;
            }
        }
        2 => {
            canvas
                .draw_rect(Rect::new(x + 6, y + 3, 10, 9))
                .map_err(backend_error)?;
            menu_icon_line(canvas, x + 6, y + 5, x + 3, y + 5)?;
            menu_icon_line(canvas, x + 3, y + 5, x + 3, y + 9)?;
            menu_icon_line(canvas, x + 3, y + 9, x + 6, y + 9)?;
            menu_icon_line(canvas, x + 16, y + 5, x + 19, y + 5)?;
            menu_icon_line(canvas, x + 19, y + 5, x + 19, y + 9)?;
            menu_icon_line(canvas, x + 19, y + 9, x + 16, y + 9)?;
            menu_icon_line(canvas, x + 11, y + 12, x + 11, y + 17)?;
            menu_icon_line(canvas, x + 7, y + 19, x + 15, y + 19)?;
            menu_icon_line(canvas, x + 8, y + 17, x + 14, y + 17)?;
        }
        3 => {
            canvas
                .draw_rect(Rect::new(x + 8, y + 8, 6, 6))
                .map_err(backend_error)?;
            menu_icon_line(canvas, x + 11, y + 1, x + 11, y + 8)?;
            menu_icon_line(canvas, x + 11, y + 14, x + 11, y + 21)?;
            menu_icon_line(canvas, x + 1, y + 11, x + 8, y + 11)?;
            menu_icon_line(canvas, x + 14, y + 11, x + 21, y + 11)?;
            menu_icon_line(canvas, x + 4, y + 4, x + 8, y + 8)?;
            menu_icon_line(canvas, x + 14, y + 14, x + 18, y + 18)?;
        }
        4 => {
            canvas
                .draw_rect(Rect::new(x + 3, y + 3, 16, 11))
                .map_err(backend_error)?;
            menu_icon_line(canvas, x + 8, y + 17, x + 14, y + 17)?;
            menu_icon_line(canvas, x + 11, y + 14, x + 11, y + 17)?;
            menu_icon_line(canvas, x + 6, y + 20, x + 16, y + 20)?;
        }
        5 => {
            canvas
                .draw_rect(Rect::new(x + 4, y + 6, 14, 10))
                .map_err(backend_error)?;
            menu_icon_line(canvas, x + 8, y + 8, x + 8, y + 14)?;
            menu_icon_line(canvas, x + 5, y + 11, x + 11, y + 11)?;
            menu_icon_line(canvas, x + 14, y + 9, x + 14, y + 9)?;
            menu_icon_line(canvas, x + 17, y + 12, x + 17, y + 12)?;
        }
        6 => {
            menu_icon_line(canvas, x + 3, y + 8, x + 7, y + 8)?;
            menu_icon_line(canvas, x + 7, y + 8, x + 12, y + 4)?;
            menu_icon_line(canvas, x + 12, y + 4, x + 12, y + 18)?;
            menu_icon_line(canvas, x + 12, y + 18, x + 7, y + 14)?;
            menu_icon_line(canvas, x + 7, y + 14, x + 3, y + 14)?;
            menu_icon_line(canvas, x + 16, y + 8, x + 19, y + 11)?;
            menu_icon_line(canvas, x + 19, y + 11, x + 16, y + 14)?;
        }
        7 => {
            menu_icon_line(canvas, x + 11, y + 8, x + 5, y + 14)?;
            menu_icon_line(canvas, x + 11, y + 8, x + 17, y + 14)?;
            menu_icon_line(canvas, x + 5, y + 14, x + 17, y + 14)?;
            canvas
                .draw_rect(Rect::new(x + 9, y + 5, 4, 4))
                .map_err(backend_error)?;
            canvas
                .draw_rect(Rect::new(x + 3, y + 14, 4, 4))
                .map_err(backend_error)?;
            canvas
                .draw_rect(Rect::new(x + 15, y + 14, 4, 4))
                .map_err(backend_error)?;
        }
        8 => {
            canvas
                .draw_rect(Rect::new(x + 4, y + 3, 11, 11))
                .map_err(backend_error)?;
            menu_icon_line(canvas, x + 6, y + 3, x + 13, y + 3)?;
            menu_icon_line(canvas, x + 4, y + 6, x + 4, y + 11)?;
            menu_icon_line(canvas, x + 15, y + 6, x + 15, y + 11)?;
            menu_icon_line(canvas, x + 7, y + 14, x + 12, y + 14)?;
            menu_icon_line(canvas, x + 14, y + 14, x + 20, y + 20)?;
        }
        _ => {
            canvas
                .draw_rect(Rect::new(x + 8, y + 3, 10, 16))
                .map_err(backend_error)?;
            menu_icon_line(canvas, x + 3, y + 11, x + 13, y + 11)?;
            menu_icon_line(canvas, x + 9, y + 7, x + 13, y + 11)?;
            menu_icon_line(canvas, x + 9, y + 15, x + 13, y + 11)?;
        }
    }
    Ok(())
}

fn draw_art_book_next(canvas: &mut Canvas<Window>, screen: &Screen) -> PlatformResult<()> {
    canvas.set_draw_color(rgb([0, 0, 0, 255]));
    canvas.clear();
    let system_assets = [
        "./_inc/systems/artwork-default/nes.png",
        "./_inc/systems/artwork-default/genesis.png",
        "./_inc/systems/artwork-default/snes.png",
        "./_inc/systems/artwork-default/psx.png",
    ];
    if screen.route == "systems" || screen.route == "home" {
        for (index, path) in system_assets.iter().enumerate() {
            if let Some((width, height, pixels)) = art_book_asset(screen, path) {
                draw_theme_asset(
                    canvas,
                    width,
                    height,
                    pixels,
                    Rect::new(index as i32 * 256 - 12, 0, 280, 768),
                )?;
            }
        }
        canvas.set_draw_color(rgb([0, 0, 0, 255]));
        for x in [244, 500, 756] {
            canvas
                .fill_rect(Rect::new(x, 0, 12, 768))
                .map_err(backend_error)?;
        }
        draw_art_book_logo(canvas, screen, Rect::new(352, 346, 320, 76))?;
    } else {
        canvas.set_draw_color(rgb([17, 17, 17, 255]));
        canvas
            .fill_rect(Rect::new(0, 0, 400, 768))
            .map_err(backend_error)?;
        draw_art_book_logo(canvas, screen, Rect::new(32, 30, 240, 57))?;
        draw_text(canvas, 32, 96, "GAME LIBRARY", [238, 238, 238, 255], 2);
        for (index, item) in screen.game_rows.iter().take(9).enumerate() {
            draw_text(
                canvas,
                32,
                150 + index as i32 * 48,
                &item.label,
                if item.selected {
                    [255, 255, 255, 255]
                } else {
                    [105, 105, 105, 255]
                },
                if item.selected { 3 } else { 2 },
            );
        }
        let screenshot = screen.selected_game.as_ref().and_then(|game| {
            screen
                .game_media
                .iter()
                .find(|media| media.content_id == game.id && media.kind == "screenshot")
        });
        if let Some(media) = screenshot {
            let bounds = if screen.route == "games-no-metadata" {
                Rect::new(400, 0, 624, 768)
            } else {
                Rect::new(400, 0, 624, 500)
            };
            draw_screen_media(canvas, media, bounds)?;
        }
        if screen.route != "games-no-metadata" {
            if let Some(media) = screen.selected_game.as_ref().and_then(|game| {
                screen
                    .game_media
                    .iter()
                    .find(|media| media.content_id == game.id && media.kind == "box-art")
            }) {
                draw_screen_media(canvas, media, Rect::new(800, 352, 192, 140))?;
            }
        }
        if screen.route != "games-no-metadata" {
            canvas.set_draw_color(rgb([34, 34, 34, 255]));
            canvas
                .fill_rect(Rect::new(400, 500, 624, 268))
                .map_err(backend_error)?;
            if let Some(game) = &screen.selected_game {
                draw_text(canvas, 432, 530, &game.title, [255, 255, 255, 255], 3);
                draw_text_in_bounds(
                    canvas,
                    Rect::new(432, 588, 360, 120),
                    &game.description,
                    [238, 238, 238, 255],
                    20,
                );
                draw_text(canvas, 832, 570, "RATING 4 / 5", [180, 180, 180, 255], 1);
                draw_text(canvas, 832, 610, "RELEASE 1994", [150, 150, 150, 255], 1);
                draw_text(canvas, 832, 650, "PLAYERS 1", [150, 150, 150, 255], 1);
                draw_text(canvas, 832, 690, "LAST PLAYED —", [150, 150, 150, 255], 1);
            }
        } else if let Some(game) = &screen.selected_game {
            draw_text(canvas, 580, 338, &game.title, [255, 255, 255, 255], 5);
        }
    }
    canvas.set_draw_color(rgb([255, 255, 255, 255]));
    for (x, y, width) in [(900, 18, 24), (904, 23, 16), (908, 28, 8)] {
        canvas
            .draw_line(
                sdl2::rect::Point::new(x, y),
                sdl2::rect::Point::new(x + width, y),
            )
            .map_err(backend_error)?;
    }
    canvas
        .fill_rect(Rect::new(910, 33, 4, 4))
        .map_err(backend_error)?;
    canvas
        .draw_rect(Rect::new(950, 18, 24, 16))
        .map_err(backend_error)?;
    canvas
        .fill_rect(Rect::new(954, 22, 14, 8))
        .map_err(backend_error)?;
    canvas
        .fill_rect(Rect::new(974, 23, 4, 6))
        .map_err(backend_error)?;
    draw_text(
        canvas,
        812,
        18,
        &screen.affordances.clock,
        [255, 255, 255, 255],
        1,
    );
    if let Some(modal) = &screen.modal {
        canvas.set_blend_mode(BlendMode::Blend);
        canvas.set_draw_color(sdl2::pixels::Color::RGBA(0, 0, 0, 190));
        canvas
            .fill_rect(Rect::new(0, 0, 1024, 768))
            .map_err(backend_error)?;
        canvas.set_draw_color(rgb([17, 17, 17, 255]));
        let dialog = Rect::new(208, 108, 608, 552);
        canvas.fill_rect(dialog).map_err(backend_error)?;
        if modal == "Generated notice" {
            draw_text(canvas, 440, 142, "MAIN MENU", [255, 255, 255, 255], 3);
            let rows = [
                "GAME SETTINGS >",
                "GAME COLLECTION SETTINGS >",
                "RETROACHIEVEMENTS >",
                "SYSTEM SETTINGS >",
                "UI SETTINGS >",
                "CONTROLLER & BLUETOOTH SETTINGS >",
                "SOUND SETTINGS >",
                "NETWORK SETTINGS >",
                "SCRAPER >",
                "QUIT >",
            ];
            for (index, row) in rows.iter().enumerate() {
                if index == 1 {
                    canvas.set_draw_color(rgb([145, 145, 145, 255]));
                    canvas
                        .fill_rect(Rect::new(208, 190 + index as i32 * 48, 608, 44))
                        .map_err(backend_error)?;
                }
                draw_menu_icon(canvas, index, 224, 201 + index as i32 * 48, index == 1)?;
                draw_text(
                    canvas,
                    260,
                    198 + index as i32 * 48,
                    row,
                    if index == 1 {
                        [255, 255, 255, 255]
                    } else {
                        [145, 145, 145, 255]
                    },
                    2,
                );
            }
        } else {
            draw_text(canvas, 344, 250, modal, [255, 255, 255, 255], 3);
        }
    }
    Ok(())
}

fn draw_theme_components(canvas: &mut Canvas<Window>, screen: &Screen) -> PlatformResult<()> {
    for component in &screen.theme.components {
        let bounds = Rect::new(
            i32::from(component.bounds.x),
            i32::from(component.bounds.y),
            u32::from(component.bounds.width),
            u32::from(component.bounds.height),
        );
        let color = component
            .color
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or(screen.palette.text);
        let point_size = component.font_size.unwrap_or(16);
        let line_height = i32::from(point_size).saturating_add(4);
        match component.kind.as_str() {
            "image" => {
                let Some(path) = component.path.as_deref() else {
                    continue;
                };
                let Some(asset) = screen.theme.assets.iter().find(|asset| asset.path == path)
                else {
                    continue;
                };
                draw_theme_asset(canvas, asset.width, asset.height, &asset.pixels, bounds)?;
            }
            "text" => {
                if let Some(text) = &component.text {
                    draw_text_in_bounds(canvas, bounds, text, color, point_size);
                }
            }
            "textlist" => {
                if let Some(text) = &component.text {
                    draw_text_in_bounds(canvas, bounds, text, color, point_size);
                }
                let items = if screen.game_rows.is_empty() {
                    &screen.menu
                } else {
                    &screen.game_rows
                };
                for (index, item) in items
                    .iter()
                    .take((bounds.height() as i32 / line_height) as usize)
                    .enumerate()
                {
                    let item_color = if item.selected {
                        screen.palette.highlight
                    } else {
                        color
                    };
                    let row = Rect::new(
                        bounds.x(),
                        bounds.y() + line_height * (index as i32 + 1),
                        bounds.width(),
                        line_height.max(1) as u32,
                    );
                    draw_text_in_bounds(canvas, row, &item.label, item_color, point_size);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn draw_theme_media(canvas: &mut Canvas<Window>, screen: &Screen) -> PlatformResult<()> {
    for region in &screen.regions {
        let bounds = Rect::new(
            i32::from(region.x),
            i32::from(region.y),
            u32::from(region.width),
            u32::from(region.height),
        );
        let media = match region.kind.as_str() {
            "system-art" => screen.system_media.as_ref(),
            "box-art-placeholder" => screen.selected_game.as_ref().and_then(|game| {
                screen
                    .game_media
                    .iter()
                    .find(|media| media.content_id == game.id && media.kind == "box-art")
            }),
            "screenshot-placeholder" => screen.selected_game.as_ref().and_then(|game| {
                screen
                    .game_media
                    .iter()
                    .find(|media| media.content_id == game.id && media.kind == "screenshot")
            }),
            _ => None,
        };
        if let Some(media) = media {
            draw_screen_media(canvas, media, bounds)?;
            continue;
        }
        let fallback_path = match region.kind.as_str() {
            "system-art" => "assets/art.png",
            "box-art-placeholder" => "assets/box-art.png",
            "screenshot-placeholder" => "assets/screenshot.png",
            _ => continue,
        };
        if let Some(asset) = screen
            .theme
            .assets
            .iter()
            .find(|asset| asset.path == fallback_path)
        {
            draw_theme_asset(canvas, asset.width, asset.height, &asset.pixels, bounds)?;
        }
    }
    Ok(())
}

fn draw_screen_media(
    canvas: &mut Canvas<Window>,
    media: &launcher_presentation::ScreenMedia,
    bounds: Rect,
) -> PlatformResult<()> {
    let mut decoder = Decoder::new(std::io::Cursor::new(&media.pixels));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(backend_error)?;
    let mut raw = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut raw).map_err(backend_error)?;
    let pixels = match reader.output_color_type() {
        (ColorType::Rgba, BitDepth::Eight) => raw[..frame.buffer_size()].to_vec(),
        (ColorType::Rgb, BitDepth::Eight) => raw[..frame.buffer_size()]
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        _ => return Err(backend_error("unsupported game media color format")),
    };
    draw_theme_asset(
        canvas,
        reader.info().width,
        reader.info().height,
        &pixels,
        bounds,
    )
}

fn draw_theme_asset(
    canvas: &mut Canvas<Window>,
    width: u32,
    height: u32,
    pixels: &[u8],
    bounds: Rect,
) -> PlatformResult<()> {
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(PixelFormatEnum::ABGR8888, width, height)
        .map_err(backend_error)?;
    texture
        .update(None, pixels, (width * 4) as usize)
        .map_err(backend_error)?;
    canvas.copy(&texture, None, bounds).map_err(backend_error)
}

fn parse_hex_color(value: &str) -> Option<[u8; 4]> {
    if value.len() != 7 || !value.starts_with('#') {
        return None;
    }
    Some([
        u8::from_str_radix(&value[1..3], 16).ok()?,
        u8::from_str_radix(&value[3..5], 16).ok()?,
        u8::from_str_radix(&value[5..7], 16).ok()?,
        255,
    ])
}

fn is_auxiliary_route(route: &str) -> bool {
    [
        "theme-garden",
        "save-vault",
        "save-sync",
        "portmaster",
        "update",
        "modal",
        "modals",
        "fault",
        "settings-form",
    ]
    .iter()
    .any(|prefix| route == *prefix || route.starts_with(&format!("{prefix}-")))
}

fn draw_theme_garden(canvas: &mut Canvas<Window>, screen: &Screen) -> PlatformResult<()> {
    let panel = Rect::new(48, 92, 928, 500);
    canvas.set_draw_color(rgb(screen.palette.surface));
    canvas.fill_rect(panel).map_err(backend_error)?;
    canvas.set_draw_color(rgb(screen.palette.accent));
    canvas.draw_rect(panel).map_err(backend_error)?;
    draw_text(canvas, 84, 128, "PREVIEW", screen.palette.highlight, 2);
    draw_text(canvas, 84, 178, &screen.theme.theme, screen.palette.text, 4);
    draw_text(
        canvas,
        84,
        246,
        "A living skin for your library",
        screen.palette.text,
        2,
    );
    draw_text(
        canvas,
        84,
        286,
        "Distinct layout, colour, and artwork treatment",
        screen.palette.muted,
        1,
    );
    draw_text(
        canvas,
        84,
        354,
        "A  switch theme",
        screen.palette.highlight,
        2,
    );
    draw_text(
        canvas,
        84,
        402,
        "B  return to library",
        screen.palette.text,
        2,
    );
    if matches!(screen.theme.theme.as_str(), "SimpleLife" | "Techdweeb") {
        if let Some(asset) = screen
            .theme
            .assets
            .iter()
            .find(|asset| asset.path == "assets/hero.png")
        {
            draw_theme_asset(
                canvas,
                asset.width,
                asset.height,
                &asset.pixels,
                Rect::new(570, 160, 350, 230),
            )?;
        }
        draw_text(
            canvas,
            570,
            430,
            "UPSTREAM DATA-ONLY IMPORT",
            screen.palette.highlight,
            1,
        );
    } else if let Some(media) = &screen.system_media {
        draw_screen_media(canvas, media, Rect::new(630, 190, 290, 182))?;
    }
    draw_text(
        canvas,
        84,
        482,
        "NOVA/8 THEME GARDEN  /  CURATED FOR 4:3",
        screen.palette.accent,
        1,
    );
    Ok(())
}

fn draw_route_surface(canvas: &mut Canvas<Window>, screen: &Screen) {
    draw_text(canvas, 64, 112, &screen.title, screen.palette.highlight, 3);
    draw_text(
        canvas,
        64,
        158,
        &screen.selected_label,
        screen.palette.text,
        2,
    );
    draw_text(canvas, 64, 208, &screen.focus, screen.palette.muted, 1);
    draw_text(canvas, 64, 250, &screen.route, screen.palette.text, 1);
}

fn draw_splash(canvas: &mut Canvas<Window>, screen: &Screen) {
    draw_text(
        canvas,
        332,
        250,
        &screen.theme.theme,
        screen.palette.accent,
        6,
    );
    draw_text(canvas, 368, 360, &screen.splash, screen.palette.muted, 2);
}

fn draw_fallback(canvas: &mut Canvas<Window>, screen: &Screen) {
    draw_text(
        canvas,
        292,
        238,
        &screen.theme.theme,
        screen.palette.highlight,
        5,
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
    let items = if screen.route == "systems"
        || screen.route == "controller-routes"
        || screen.game_rows.is_empty()
    {
        &screen.menu
    } else {
        &screen.game_rows
    };
    let has_theme_textlist = screen
        .theme
        .components
        .iter()
        .any(|component| component.kind == "textlist");
    if !has_theme_textlist {
        let selected = items.iter().position(|item| item.selected).unwrap_or(0);
        let start = selected
            .saturating_sub(3)
            .min(items.len().saturating_sub(8));
        for (index, item) in items.iter().skip(start).take(8).enumerate() {
            let y = 92 + index as i32 * 26;
            let color = if item.selected {
                screen.palette.highlight
            } else {
                screen.palette.text
            };
            draw_text(canvas, 448, y, &item.label, color, 2);
        }
    } else {
        for (index, item) in screen.menu.iter().take(4).enumerate() {
            let color = if item.selected {
                screen.palette.highlight
            } else {
                screen.palette.text
            };
            draw_text_in_bounds(
                canvas,
                Rect::new(48 + index as i32 * 230, 600, 210, 32),
                &item.label,
                color,
                16,
            );
        }
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
    }
    if let Some(fallback) = &screen.theme_fallback {
        draw_text(canvas, 64, 612, fallback, screen.palette.highlight, 2);
    }
    if screen.group_jump.visible {
        let current = screen.group_jump.current.as_deref().unwrap_or("…");
        let target = screen.group_jump.target.as_deref().unwrap_or("");
        draw_text(
            canvas,
            64,
            650,
            &format!("GROUP {current} -> {target}"),
            screen.palette.highlight,
            2,
        );
    }
}

fn draw_settings(canvas: &mut Canvas<Window>, screen: &Screen) {
    let Some(settings) = &screen.settings else {
        return;
    };
    let panel = Rect::new(40, 48, 944, 632);
    canvas.set_draw_color(rgb(screen.palette.surface));
    let _ = canvas.fill_rect(panel);
    canvas.set_draw_color(rgb(screen.palette.accent));
    let _ = canvas.draw_rect(panel);
    draw_text(canvas, 72, 76, &screen.title, screen.palette.highlight, 4);
    let surface = match settings.surface {
        settings_ui::Surface::SectionList => "SETTINGS CATEGORIES",
        settings_ui::Surface::Form => "EDIT SETTINGS",
    };
    draw_text(canvas, 72, 126, surface, screen.palette.muted, 1);
    let mut y = 164;
    for section in &settings.sections {
        let selected = settings
            .selected_section_id
            .as_deref()
            .is_some_and(|id| id == section.id);
        if selected {
            canvas.set_draw_color(rgb(screen.palette.accent));
            let _ = canvas.fill_rect(Rect::new(64, y - 5, 896, 30));
        }
        let color = if selected {
            screen.palette.highlight
        } else {
            screen.palette.accent
        };
        draw_text(canvas, 72, y, &user_label(&section.label_key), color, 2);
        y += 30;
        for control in &section.controls {
            if y + 28 > 648 {
                return;
            }
            let selected = settings
                .selected_setting_id
                .as_deref()
                .is_some_and(|id| id == control.setting_id);
            if selected {
                canvas.set_draw_color(rgb(screen.palette.accent));
                let _ = canvas.fill_rect(Rect::new(64, y - 5, 896, 30));
            }
            let color = if selected {
                screen.palette.highlight
            } else {
                screen.palette.text
            };
            draw_text(canvas, 88, y, &user_label(&control.label_key), color, 1);
            draw_text(canvas, 560, y, &setting_value(&control.value), color, 1);
            y += 34;
        }
        y += 12;
    }
}

fn user_label(value: &str) -> String {
    let value = value
        .strip_prefix("settings.")
        .unwrap_or(value)
        .strip_suffix(".label")
        .unwrap_or(value);
    let value = value.rsplit('.').next().unwrap_or(value);
    let mut label = String::new();
    for character in value.chars() {
        if character == '-' || character == '_' {
            label.push(' ');
        } else if character.is_uppercase()
            && label
                .chars()
                .last()
                .is_some_and(|last| last.is_ascii_lowercase())
        {
            label.push(' ');
            label.push(character);
        } else {
            label.push(character);
        }
    }
    label
        .split_whitespace()
        .map(|word| match word.to_ascii_lowercase().as_str() {
            "api" => "API".into(),
            "psk" => "PSK".into(),
            "ssid" => "SSID".into(),
            "ui" => "UI".into(),
            "wifi" => "Wi-Fi".into(),
            _ => {
                let mut chars = word.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn setting_value(value: &settings_ui::SemanticValue) -> String {
    match value {
        settings_ui::SemanticValue::Boolean(value) => {
            if *value {
                "On".into()
            } else {
                "Off".into()
            }
        }
        settings_ui::SemanticValue::Integer(value) => value.to_string(),
        settings_ui::SemanticValue::Decimal(value) => value.to_string(),
        settings_ui::SemanticValue::Text(value) | settings_ui::SemanticValue::EnumSingle(value) => {
            value.clone()
        }
        settings_ui::SemanticValue::EnumMulti(values) => {
            if values.is_empty() {
                "—".into()
            } else {
                values.join(", ")
            }
        }
        settings_ui::SemanticValue::Masked { .. } => "•••".into(),
        settings_ui::SemanticValue::Empty => "—".into(),
    }
}

fn draw_wifi(canvas: &mut Canvas<Window>, screen: &Screen) {
    let Some(wifi) = &screen.wifi else {
        return;
    };
    let panel = Rect::new(40, 48, 944, 632);
    canvas.set_draw_color(rgb(screen.palette.surface));
    let _ = canvas.fill_rect(panel);
    canvas.set_draw_color(rgb(screen.palette.accent));
    let _ = canvas.draw_rect(panel);
    draw_text(canvas, 72, 76, &screen.title, screen.palette.highlight, 4);
    draw_text(
        canvas,
        72,
        126,
        wifi_surface_label(&screen.route),
        screen.palette.muted,
        1,
    );
    match screen.route.as_str() {
        "wifi-scan" | "wifi-access-point-selection" => draw_wifi_networks(canvas, screen, wifi),
        "wifi-hidden-network" | "wifi-manual-ssid" | "wifi-password-entry" => {
            draw_wifi_keyboard(canvas, screen, wifi)
        }
        "wifi-progress" => {
            draw_text(canvas, 88, 220, "STATUS CONNECTING", screen.palette.text, 2);
            if let Some(network) = &wifi.selected_network {
                draw_text(
                    canvas,
                    88,
                    274,
                    &network.display_ssid,
                    screen.palette.highlight,
                    3,
                );
            }
            draw_text(
                canvas,
                88,
                340,
                &format!("PHASE {}", format_debug(&wifi.phase)),
                screen.palette.muted,
                1,
            );
        }
        "wifi-error" => {
            draw_text(
                canvas,
                88,
                220,
                "CONNECTION ERROR",
                screen.palette.highlight,
                2,
            );
            draw_text(
                canvas,
                88,
                274,
                &format!(
                    "REASON {}",
                    wifi.reason
                        .map(|reason| format_debug(&reason))
                        .unwrap_or_else(|| "unknown".into())
                ),
                screen.palette.text,
                1,
            );
        }
        _ => draw_wifi_networks(canvas, screen, wifi),
    }
}

fn draw_wifi_networks(
    canvas: &mut Canvas<Window>,
    screen: &Screen,
    wifi: &wifi_settings_controller::Snapshot,
) {
    if wifi.networks.is_empty() {
        draw_text(canvas, 88, 188, "No networks found", screen.palette.text, 2);
    } else {
        for (index, network) in wifi.networks.iter().take(8).enumerate() {
            let y = 170 + index as i32 * 48;
            if network.selected {
                canvas.set_draw_color(rgb(screen.palette.accent));
                let _ = canvas.fill_rect(Rect::new(64, y - 6, 896, 40));
            }
            let color = if network.selected {
                screen.palette.highlight
            } else {
                screen.palette.text
            };
            draw_text(canvas, 88, y, &network.display_ssid, color, 2);
            draw_text(canvas, 640, y, security_label(network.security), color, 1);
            draw_text(
                canvas,
                850,
                y,
                &format!("{}%", network.signal_quality),
                color,
                1,
            );
        }
    }
}

fn draw_wifi_keyboard(
    canvas: &mut Canvas<Window>,
    screen: &Screen,
    wifi: &wifi_settings_controller::Snapshot,
) {
    let (label, value) = match screen.route.as_str() {
        "wifi-password-entry" => ("NETWORK KEY", "•••"),
        "wifi-hidden-network" | "wifi-manual-ssid" => ("NETWORK NAME", "TYPE NETWORK NAME"),
        _ => ("NETWORK INPUT", "WAITING FOR INPUT"),
    };
    draw_text(canvas, 88, 220, label, screen.palette.accent, 2);
    draw_text(canvas, 88, 276, value, screen.palette.highlight, 2);
    if let Some(keyboard) = &wifi.keyboard {
        draw_text(
            canvas,
            88,
            332,
            &format!("{} CHARACTERS", keyboard.length_scalars),
            screen.palette.muted,
            1,
        );
    }
}

fn wifi_surface_label(route: &str) -> &'static str {
    match route {
        "wifi-scan" => "AVAILABLE NETWORKS",
        "wifi-access-point-selection" => "SELECT NETWORK",
        "wifi-hidden-network" => "HIDDEN NETWORK",
        "wifi-manual-ssid" => "ENTER NETWORK NAME",
        "wifi-password-entry" => "ENTER NETWORK KEY",
        "wifi-progress" => "CONNECTING",
        "wifi-error" => "CONNECTION ERROR",
        _ => "NETWORK SETTINGS",
    }
}

fn security_label(security: wifi_manager::Security) -> &'static str {
    match security {
        wifi_manager::Security::Open => "Open",
        wifi_manager::Security::Wpa2Psk => "WPA2",
        wifi_manager::Security::Wpa3Sae => "WPA3",
        wifi_manager::Security::Unsupported => "Unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::wifi_surface_label;

    #[test]
    fn wifi_surface_labels_follow_the_route() {
        for (route, label) in [
            ("wifi-scan", "AVAILABLE NETWORKS"),
            ("wifi-access-point-selection", "SELECT NETWORK"),
            ("wifi-hidden-network", "HIDDEN NETWORK"),
            ("wifi-manual-ssid", "ENTER NETWORK NAME"),
            ("wifi-password-entry", "ENTER NETWORK KEY"),
            ("wifi-progress", "CONNECTING"),
            ("wifi-error", "CONNECTION ERROR"),
        ] {
            assert_eq!(wifi_surface_label(route), label);
        }
    }
}

fn draw_scraper(canvas: &mut Canvas<Window>, screen: &Screen) {
    let scraper = &screen.scraper;
    canvas.set_draw_color(rgb(screen.palette.background));
    let _ = canvas.fill_rect(Rect::new(32, 72, 960, 610));
    draw_text(canvas, 64, 104, &screen.title, screen.palette.highlight, 3);
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
            &scraper.status,
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
        ui_model::Button::L1 => "L1",
        ui_model::Button::R1 => "R1",
        ui_model::Button::Menu => "MENU",
    }
}

fn ttf_context() -> Option<&'static sdl2::ttf::Sdl2TtfContext> {
    static CONTEXT: OnceLock<Option<sdl2::ttf::Sdl2TtfContext>> = OnceLock::new();
    CONTEXT.get_or_init(|| sdl2::ttf::init().ok()).as_ref()
}

fn draw_text(canvas: &mut Canvas<Window>, x: i32, y: i32, text: &str, color: [u8; 4], scale: i32) {
    let (width, height) = canvas.logical_size();
    draw_text_in_bounds(
        canvas,
        Rect::new(
            x,
            y,
            (width as i32 - x).max(1) as u32,
            (height as i32 - y).max(1) as u32,
        ),
        text,
        color,
        (scale.max(1) * 8) as u16,
    );
}

fn draw_text_in_bounds(
    canvas: &mut Canvas<Window>,
    bounds: Rect,
    text: &str,
    color: [u8; 4],
    point_size: u16,
) {
    let Some(ttf) = ttf_context() else {
        return;
    };
    let Ok(rwops) =
        sdl2::rwops::RWops::from_bytes(include_bytes!("../assets/fonts/Lato-Regular.ttf"))
    else {
        return;
    };
    let Ok(font) = ttf.load_font_from_rwops(rwops, point_size.clamp(8, 96)) else {
        return;
    };
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        let fits = font
            .size_of(&candidate)
            .map(|(width, _)| width <= bounds.width())
            .unwrap_or(false);
        if !line.is_empty() && !fits {
            lines.push(std::mem::take(&mut line));
        }
        if line.is_empty() {
            line.push_str(word);
        } else if fits {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line.push_str(word);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    canvas.set_clip_rect(Some(bounds));
    let texture_creator = canvas.texture_creator();
    let mut cursor_y = bounds.y();
    for line in lines {
        let Ok(surface) = font.render(&line).blended(rgb(color)) else {
            continue;
        };
        let Ok(texture) = texture_creator.create_texture_from_surface(&surface) else {
            continue;
        };
        let target = Rect::new(bounds.x(), cursor_y, surface.width(), surface.height());
        let _ = canvas.copy(&texture, None, target);
        cursor_y += surface.height() as i32;
        if cursor_y >= bounds.y() + bounds.height() as i32 {
            break;
        }
    }
    canvas.set_clip_rect(None);
}

fn draw_controller_badge(canvas: &mut Canvas<Window>, x: i32, y: i32, color: [u8; 4], label: &str) {
    canvas.set_draw_color(rgb(color));
    let _ = canvas.fill_rect(Rect::new(x + 2, y + 2, 24, 24));
    canvas.set_draw_color(rgb([0, 0, 0, 255]));
    let _ = canvas.fill_rect(Rect::new(x + 7, y + 7, 14, 14));
    canvas.set_draw_color(rgb(color));
    let _ = canvas.draw_line((x + 8, y + 14), (x + 20, y + 14));
    let _ = canvas.draw_line((x + 14, y + 8), (x + 14, y + 20));
    draw_text(canvas, x + 32, y + 4, label, color, 1);
}
