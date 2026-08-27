use launch_contract::{InputLayout, LaunchRequest, SuspendMode};
use sim_platform_contract::{
    AudioState, BatteryState, Button, DisplayState, HallCalibrationState, InputState, LedState,
    PlatformState, PowerState, RadioState, RadiosState, RumbleState, StorageMode, SuspendResult,
    SuspendState, UsbRole, UsbState,
};

pub struct LogicalPlatform {
    state: PlatformState,
}

impl LogicalPlatform {
    pub fn new() -> Self {
        Self {
            state: PlatformState {
                display: DisplayState {
                    logical_width: 1024,
                    logical_height: 768,
                },
                input: InputState {
                    pressed: vec![Button::Menu],
                },
                hall_calibration: HallCalibrationState { calibrated: true },
                power: PowerState {
                    external_power: true,
                },
                battery: BatteryState {
                    percent: 73,
                    charging: true,
                },
                suspend: (SuspendState::Active, SuspendResult::None),
                radios: RadiosState {
                    wifi: RadioState {
                        enabled: true,
                        connected: false,
                    },
                    bluetooth: RadioState {
                        enabled: false,
                        connected: false,
                    },
                },
                audio: AudioState {
                    enabled: true,
                    volume_percent: 65,
                    active: false,
                },
                leds: LedState {
                    on: true,
                    brightness_percent: 40,
                },
                rumble: RumbleState { active: true },
                usb: UsbState {
                    connected: false,
                    role: UsbRole::None,
                },
                storage: StorageMode::Available,
            },
        }
    }

    pub fn snapshot(&self) -> PlatformState {
        self.state.clone()
    }

    pub fn apply_profile(&mut self, request: &LaunchRequest) {
        self.state.display = DisplayState {
            logical_width: request.display.width as u32,
            logical_height: request.display.height as u32,
        };
        self.state.input = InputState {
            pressed: Vec::new(),
        };
        self.state.suspend = (
            SuspendState::Active,
            if request.power.suspend == SuspendMode::Allowed {
                SuspendResult::None
            } else {
                SuspendResult::Failed
            },
        );
        self.state.audio.active = true;
        self.state.leds = LedState {
            on: false,
            brightness_percent: 0,
        };
        self.state.rumble.active = request.input.rumble;
        if request.input.layout == InputLayout::Arcade {
            self.state.input.pressed.push(Button::Primary);
        }
    }

    pub fn restore(&mut self, snapshot: &PlatformState) -> bool {
        self.state = snapshot.clone();
        self.state == *snapshot
    }

    pub fn safe_default(&mut self) {
        self.state.display = DisplayState {
            logical_width: 640,
            logical_height: 480,
        };
        self.state.input = InputState {
            pressed: Vec::new(),
        };
        self.state.hall_calibration = HallCalibrationState { calibrated: false };
        self.state.power = PowerState {
            external_power: false,
        };
        self.state.battery = BatteryState {
            percent: 0,
            charging: false,
        };
        self.state.suspend = (SuspendState::Active, SuspendResult::None);
        self.state.radios = RadiosState {
            wifi: RadioState {
                enabled: false,
                connected: false,
            },
            bluetooth: RadioState {
                enabled: false,
                connected: false,
            },
        };
        self.state.audio = AudioState {
            enabled: true,
            volume_percent: 50,
            active: false,
        };
        self.state.leds = LedState {
            on: false,
            brightness_percent: 0,
        };
        self.state.rumble = RumbleState { active: false };
        self.state.usb = UsbState {
            connected: false,
            role: UsbRole::None,
        };
        self.state.storage = StorageMode::Available;
    }
}

impl Default for LogicalPlatform {
    fn default() -> Self {
        Self::new()
    }
}
