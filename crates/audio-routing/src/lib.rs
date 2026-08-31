use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

pub const TARGET_SKU: &str = "TG4040";
pub const DEFAULT_SAMPLE_RATE_HZ: u32 = 48_000;
pub const STATE_SCHEMA: &str = "brickpro-audio-route/v1";
pub const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
pub const MAX_SAMPLE_RATE_HZ: u32 = 192_000;

pub const fn valid_sample_rate_hz(sample_rate_hz: u32) -> bool {
    sample_rate_hz >= MIN_SAMPLE_RATE_HZ && sample_rate_hz <= MAX_SAMPLE_RATE_HZ
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sink {
    Speaker,
    Jack,
}

impl Sink {
    const PRIORITY: [Self; 2] = [Self::Jack, Self::Speaker];
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Runtime {
    RetroArch,
    DraStic,
    Ppsspp,
    Flycast,
    PortMaster,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlsaBuffer {
    pub buffer_frames: u16,
    pub period_frames: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouteSnapshot {
    requested_sink: Sink,
    volumes: [u8; 2],
    stream_active: bool,
    sample_rate_hz: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AudioDiagnostics {
    Available {
        active_sink: Sink,
        sample_rate_hz: u32,
        underrun_count: u64,
        speaker_amp_enabled: bool,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouteManager {
    available: BTreeSet<Sink>,
    current_sink: Sink,
    requested_sink: Sink,
    volumes: BTreeMap<Sink, u8>,
    stream_active: bool,
    sample_rate_hz: u32,
    underrun_count: u64,
    speaker_amp_enabled: bool,
    session_snapshot: Option<RouteSnapshot>,
    system_suspend_snapshot: Option<RouteSnapshot>,
}

impl Default for RouteManager {
    fn default() -> Self {
        let mut volumes = BTreeMap::new();
        volumes.insert(Sink::Speaker, 50);
        volumes.insert(Sink::Jack, 50);
        Self {
            available: [Sink::Speaker].into_iter().collect(),
            current_sink: Sink::Speaker,
            requested_sink: Sink::Speaker,
            volumes,
            stream_active: false,
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            underrun_count: 0,
            speaker_amp_enabled: false,
            session_snapshot: None,
            system_suspend_snapshot: None,
        }
    }
}

impl RouteManager {
    pub fn current_sink(&self) -> Sink {
        self.current_sink
    }

    pub fn volume(&self, sink: Sink) -> u8 {
        self.volumes[&sink]
    }

    pub fn select_sink(&mut self, sink: Sink) -> bool {
        self.requested_sink = sink;
        self.refresh_route()
    }

    pub fn set_volume(&mut self, sink: Sink, volume: u8) {
        self.volumes.insert(sink, volume.min(100));
    }

    pub fn set_sink_present(&mut self, sink: Sink, present: bool) -> bool {
        if present {
            self.available.insert(sink);
        } else {
            self.available.remove(&sink);
        }
        self.refresh_route()
    }

    pub fn begin_stream(&mut self, sample_rate_hz: u32) -> bool {
        if valid_sample_rate_hz(sample_rate_hz) {
            self.sample_rate_hz = sample_rate_hz;
            // The amp is live before the stream opens so its first samples are not clipped.
            self.speaker_amp_enabled = self.current_sink == Sink::Speaker;
            self.stream_active = true;
            true
        } else {
            false
        }
    }

    pub fn end_stream(&mut self) {
        self.stream_active = false;
        self.speaker_amp_enabled = false;
    }

    pub fn record_underrun(&mut self) {
        self.underrun_count = self.underrun_count.saturating_add(1);
    }

    pub fn begin_session(&mut self) -> bool {
        if self.session_snapshot.is_some() || self.system_suspend_snapshot.is_some() {
            return false;
        }
        self.session_snapshot = Some(self.snapshot());
        self.end_stream();
        true
    }

    pub fn end_session(&mut self) -> bool {
        if self.system_suspend_snapshot.is_some() {
            return false;
        }
        let snapshot = self.session_snapshot.take();
        self.restore(snapshot)
    }

    pub fn suspend(&mut self) -> bool {
        if self.system_suspend_snapshot.is_some() {
            return false;
        }
        self.system_suspend_snapshot = Some(self.snapshot());
        self.end_stream();
        true
    }

    pub fn wake(&mut self) -> bool {
        let snapshot = self.system_suspend_snapshot.take();
        self.restore(snapshot)
    }

    pub fn diagnostics(&self) -> AudioDiagnostics {
        AudioDiagnostics::Available {
            active_sink: self.current_sink,
            sample_rate_hz: self.sample_rate_hz,
            underrun_count: self.underrun_count,
            speaker_amp_enabled: self.speaker_amp_enabled,
        }
    }

    fn snapshot(&self) -> RouteSnapshot {
        RouteSnapshot {
            requested_sink: self.requested_sink,
            volumes: [self.volume(Sink::Speaker), self.volume(Sink::Jack)],
            stream_active: self.stream_active,
            sample_rate_hz: self.sample_rate_hz,
        }
    }

    fn restore(&mut self, snapshot: Option<RouteSnapshot>) -> bool {
        let Some(snapshot) = snapshot else {
            return false;
        };
        self.requested_sink = snapshot.requested_sink;
        self.volumes.insert(Sink::Speaker, snapshot.volumes[0]);
        self.volumes.insert(Sink::Jack, snapshot.volumes[1]);
        self.sample_rate_hz = snapshot.sample_rate_hz;
        self.refresh_route();
        if snapshot.stream_active {
            self.begin_stream(snapshot.sample_rate_hz);
        } else {
            self.end_stream();
        }
        true
    }

    fn refresh_route(&mut self) -> bool {
        let next = if self.available.contains(&self.requested_sink) {
            self.requested_sink
        } else {
            Sink::PRIORITY
                .into_iter()
                .find(|sink| self.available.contains(sink))
                .unwrap_or(Sink::Speaker)
        };
        let changed = self.current_sink != next;
        self.current_sink = next;
        self.speaker_amp_enabled = self.stream_active && next == Sink::Speaker;
        changed
    }

    fn valid(&self) -> bool {
        valid_sample_rate_hz(self.sample_rate_hz)
            && self.volumes.contains_key(&Sink::Speaker)
            && self.volumes.contains_key(&Sink::Jack)
            && self.volumes.values().all(|volume| *volume <= 100)
            && self.available.contains(&Sink::Speaker)
            && self.available.contains(&self.current_sink)
            && self.speaker_amp_enabled
                == (self.stream_active && self.current_sink == Sink::Speaker)
            && self.session_snapshot.as_ref().is_none_or(|snapshot| {
                valid_sample_rate_hz(snapshot.sample_rate_hz)
                    && snapshot.volumes.iter().all(|volume| *volume <= 100)
            })
            && self
                .system_suspend_snapshot
                .as_ref()
                .is_none_or(|snapshot| {
                    valid_sample_rate_hz(snapshot.sample_rate_hz)
                        && snapshot.volumes.iter().all(|volume| *volume <= 100)
                })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StoredRoute {
    schema: String,
    route: RouteManager,
}

pub fn load_route(path: &Path) -> Result<RouteManager, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("audio-state-missing".into());
        }
        Err(_) => return Err("audio-state-unavailable".into()),
    };
    let stored: StoredRoute =
        serde_json::from_slice(&bytes).map_err(|_| "audio-state-invalid".to_string())?;
    if stored.schema == STATE_SCHEMA && stored.route.valid() {
        Ok(stored.route)
    } else {
        Err("audio-state-invalid".into())
    }
}

pub fn load_route_or_default(path: &Path) -> Result<RouteManager, String> {
    match load_route(path) {
        Err(error) if error == "audio-state-missing" => Ok(RouteManager::default()),
        result => result,
    }
}

pub fn save_route(path: &Path, route: &RouteManager) -> Result<(), String> {
    route
        .valid()
        .then_some(())
        .ok_or_else(|| "audio-state-invalid".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "audio-state-write-failed".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "audio-state-write-failed".to_string())?;
    let bytes = serde_json::to_vec_pretty(&StoredRoute {
        schema: STATE_SCHEMA.into(),
        route: route.clone(),
    })
    .map_err(|_| "audio-state-write-failed".to_string())?;
    fs::write(path, bytes).map_err(|_| "audio-state-write-failed".to_string())
}

pub fn diagnostics_from_state(path: &Path) -> AudioDiagnostics {
    load_route(path)
        .map(|route| route.diagnostics())
        .unwrap_or_else(|reason| AudioDiagnostics::Unavailable { reason })
}

pub trait AudioAdapter {
    fn present_sinks(&mut self) -> Result<BTreeSet<Sink>, String>;
    fn apply(&mut self, route: &RouteManager) -> Result<(), String>;
}

pub struct LiveRouteManager<A> {
    route: RouteManager,
    adapter: A,
}

impl<A: AudioAdapter> LiveRouteManager<A> {
    pub fn new(route: RouteManager, adapter: A) -> Self {
        Self { route, adapter }
    }

    pub fn route(&self) -> &RouteManager {
        &self.route
    }

    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    pub fn poll_hotplug(&mut self) -> Result<(), String> {
        let present = self.adapter.present_sinks()?;
        self.route.set_sink_present(Sink::Speaker, true);
        self.route
            .set_sink_present(Sink::Jack, present.contains(&Sink::Jack));
        self.adapter.apply(&self.route)
    }

    pub fn mutate(&mut self, action: impl FnOnce(&mut RouteManager)) -> Result<(), String> {
        action(&mut self.route);
        self.adapter.apply(&self.route)
    }

    pub fn transition(
        &mut self,
        action: impl FnOnce(&mut RouteManager) -> bool,
    ) -> Result<bool, String> {
        if action(&mut self.route) {
            self.adapter.apply(&self.route)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn into_route(self) -> RouteManager {
        self.route
    }
}

#[derive(Default)]
pub struct FixtureAudioAdapter {
    present: BTreeSet<Sink>,
    pub apply_count: u32,
}

impl FixtureAudioAdapter {
    pub fn set_sink_present(&mut self, sink: Sink, present: bool) {
        if present {
            self.present.insert(sink);
        } else {
            self.present.remove(&sink);
        }
    }
}

impl AudioAdapter for FixtureAudioAdapter {
    fn present_sinks(&mut self) -> Result<BTreeSet<Sink>, String> {
        Ok(self.present.clone())
    }

    fn apply(&mut self, _route: &RouteManager) -> Result<(), String> {
        self.apply_count = self.apply_count.saturating_add(1);
        Ok(())
    }
}

pub struct SystemAudioAdapter {
    amixer: PathBuf,
    card: u8,
    speaker_control: String,
    jack_control: String,
    speaker_amp: PathBuf,
    jack_present: PathBuf,
}

impl SystemAudioAdapter {
    pub fn new(
        card: u8,
        speaker_control: String,
        jack_control: String,
        speaker_amp: PathBuf,
        jack_present: PathBuf,
    ) -> Self {
        Self {
            amixer: PathBuf::from("/usr/bin/amixer"),
            card,
            speaker_control,
            jack_control,
            speaker_amp,
            jack_present,
        }
    }

    fn amixer(&self, control: &str, value: &str) -> Result<(), String> {
        let status = Command::new(&self.amixer)
            .args(["-q", "-c", &self.card.to_string(), "sset", control, value])
            .status()
            .map_err(|_| "alsa-mixer-unavailable".to_string())?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| "alsa-mixer-failed".into())
    }
}

impl AudioAdapter for SystemAudioAdapter {
    fn present_sinks(&mut self) -> Result<BTreeSet<Sink>, String> {
        let jack = fs::read_to_string(&self.jack_present)
            .map_err(|_| "jack-hotplug-unavailable".to_string())?;
        let mut sinks: BTreeSet<Sink> = [Sink::Speaker].into_iter().collect();
        if jack.trim() == "1" {
            sinks.insert(Sink::Jack);
        } else if jack.trim() != "0" {
            return Err("jack-hotplug-invalid".into());
        }
        Ok(sinks)
    }

    fn apply(&mut self, route: &RouteManager) -> Result<(), String> {
        self.amixer(
            &self.speaker_control,
            &format!("{}%", route.volume(Sink::Speaker)),
        )?;
        self.amixer(
            &self.jack_control,
            &format!("{}%", route.volume(Sink::Jack)),
        )?;
        self.amixer(
            &self.speaker_control,
            if route.current_sink == Sink::Speaker {
                "on"
            } else {
                "off"
            },
        )?;
        self.amixer(
            &self.jack_control,
            if route.current_sink == Sink::Jack {
                "on"
            } else {
                "off"
            },
        )?;
        fs::write(
            &self.speaker_amp,
            if route.speaker_amp_enabled {
                "1\n"
            } else {
                "0\n"
            },
        )
        .map_err(|_| "speaker-amp-failed".to_string())
    }
}

pub const fn alsa_buffer(runtime: Runtime) -> AlsaBuffer {
    match runtime {
        Runtime::RetroArch => AlsaBuffer {
            buffer_frames: 1536,
            period_frames: 384,
        },
        Runtime::DraStic | Runtime::Ppsspp => AlsaBuffer {
            buffer_frames: 2048,
            period_frames: 512,
        },
        Runtime::Flycast => AlsaBuffer {
            buffer_frames: 4096,
            period_frames: 1024,
        },
        Runtime::PortMaster => AlsaBuffer {
            buffer_frames: 1536,
            period_frames: 384,
        },
    }
}

pub const fn default_alsa_buffer() -> AlsaBuffer {
    AlsaBuffer {
        buffer_frames: 1024,
        period_frames: 256,
    }
}
