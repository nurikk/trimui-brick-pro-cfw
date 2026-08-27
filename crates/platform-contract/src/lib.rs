use std::path::Path;

use serde::{Deserialize, Serialize};
use sim_domain::{CatalogEntry, Route};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug)]
pub struct Screen {
    pub route: Route,
    pub selection: CatalogEntry,
    pub selected_index: usize,
    pub entry_count: usize,
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

pub type PlatformResult<T> = Result<T, String>;

pub trait Platform {
    fn next_button_event(&mut self) -> PlatformResult<Option<ButtonEvent>>;
    fn present(&mut self, screen: &Screen) -> PlatformResult<()>;
    fn capture_png(&mut self, path: &Path) -> PlatformResult<()>;
    fn logical_time_ms(&self) -> u64;
    fn snapshot(&self) -> PlatformSnapshot;
}
