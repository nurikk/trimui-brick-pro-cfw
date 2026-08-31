//! Simulator-only TG4040 power policy state.
//!
//! Values come from the checked-in synthetic fixture. They are not permission
//! to touch real hardware and are not physical TG4040 measurements.

use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Not,
};

const POLICY_BYTES: &[u8] = include_bytes!("../../../config/platform/tg4040/power-policies.json");
const PROFILE_IDS: [&str; 3] = ["eco", "balanced", "performance"];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Catalog {
    schema: String,
    target_sku: String,
    lane: String,
    hardware_verified: bool,
    real_device_operations: String,
    thermal: Thermal,
    global_default: String,
    profiles: Vec<Profile>,
    system_defaults: BTreeMap<String, String>,
    game_overrides: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Thermal {
    limit_c: i16,
    hysteresis_c: i16,
    throttle_profile: String,
    throttling_enabled: bool,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Profile {
    id: String,
    cpu: CpuPolicy,
    gpu: GpuPolicy,
    display: DisplayPolicy,
    tradeoff: Tradeoff,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CpuPolicy {
    min_k_hz: u32,
    max_k_hz: u32,
    policy: String,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GpuPolicy {
    max_k_hz: u32,
    policy: String,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DisplayPolicy {
    width: u16,
    height: u16,
    refresh_hz: u16,
    mode: String,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Tradeoff {
    fps: f32,
    p99_frame_ms: f32,
    temperature_c: f32,
    power_w: f32,
}

#[derive(Clone, Debug)]
pub struct PowerPolicyController {
    catalog: Catalog,
    requested_profile: String,
    requested_source: &'static str,
    context: &'static str,
    temperature_c: i16,
    throttled: bool,
    safe_mode_reset: bool,
}

impl PowerPolicyController {
    pub fn new() -> Result<Self, String> {
        let catalog: Catalog = serde_json::from_slice(POLICY_BYTES)
            .map_err(|error| format!("invalid TG4040 power policy fixture: {error}"))?;
        validate(&catalog)?;
        Ok(Self {
            temperature_c: 35,
            catalog,
            requested_profile: "eco".into(),
            requested_source: "launcher",
            context: "launcher",
            throttled: false,
            safe_mode_reset: false,
        })
    }

    pub fn begin_game(&mut self, system_id: &str, content_id: &str) -> Result<(), String> {
        let (profile, source) = if let Some(profile) = self.catalog.game_overrides.get(content_id) {
            (profile.clone(), "game-override")
        } else if let Some(profile) = self.catalog.system_defaults.get(system_id) {
            (profile.clone(), "system-default")
        } else {
            (self.catalog.global_default.clone(), "global-default")
        };
        self.request(profile, source, "game")
    }

    pub fn set_game_override(&mut self, profile: &str) -> Result<(), String> {
        if self.context != "game" {
            return Err("a temporary power override requires an active game".into());
        }
        self.request(profile.to_string(), "user-override", "game")
    }

    pub fn suspend(&mut self) {
        self.low_power("suspend", "suspend");
    }

    pub fn wake(&mut self) {
        self.low_power("wake", "launcher");
    }

    pub fn game_exit(&mut self) {
        self.low_power("game-exit", "launcher");
    }

    pub fn safe_mode_reset(&mut self) {
        self.low_power("safe-mode-reset", "launcher");
        self.safe_mode_reset = true;
    }

    pub fn set_temperature(&mut self, temperature_c: i16) -> Result<(), String> {
        if temperature_c < 0i16.saturating_sub(20) || temperature_c > 150 {
            return Err("temperatureC must be between -20 and 150".into());
        }
        self.temperature_c = temperature_c;
        if temperature_c >= self.catalog.thermal.limit_c {
            self.throttled = true;
        } else if temperature_c <= self.catalog.thermal.limit_c - self.catalog.thermal.hysteresis_c
        {
            self.throttled = false;
        }
        Ok(())
    }

    pub fn evidence(&self) -> Value {
        let effective_id = if self.throttled {
            self.catalog.thermal.throttle_profile.as_str()
        } else {
            self.requested_profile.as_str()
        };
        let profile = self
            .profile(effective_id)
            .expect("validated power profile reference");
        json!({
            "schema": self.catalog.schema,
            "targetSku": self.catalog.target_sku,
            "lane": self.catalog.lane,
            "hardwareVerified": self.catalog.hardware_verified,
            "realDeviceOperations": self.catalog.real_device_operations,
            "context": self.context,
            "globalDefault": self.catalog.global_default,
            "requestedProfile": self.requested_profile,
            "effectiveProfile": effective_id,
            "requestedSource": self.requested_source,
            "effectiveSource": if self.throttled { "thermal-limit" } else { self.requested_source },
            "temporaryGamePolicy": self.requested_source == "user-override",
            "safeModeReset": self.safe_mode_reset,
            "temperatureC": self.temperature_c,
            "thermalLimitC": self.catalog.thermal.limit_c,
            "thermalHysteresisC": self.catalog.thermal.hysteresis_c,
            "throttlingEnabled": self.catalog.thermal.throttling_enabled,
            "throttled": self.throttled,
            "policy": profile,
        })
    }

    fn low_power(&mut self, source: &'static str, context: &'static str) {
        self.requested_profile = "eco".into();
        self.requested_source = source;
        self.context = context;
        self.safe_mode_reset = false;
    }

    fn request(
        &mut self,
        profile: String,
        source: &'static str,
        context: &'static str,
    ) -> Result<(), String> {
        if self.profile(&profile).is_none() {
            return Err("power profile must be eco, balanced, or performance".into());
        }
        self.requested_profile = profile;
        self.requested_source = source;
        self.context = context;
        self.safe_mode_reset = false;
        Ok(())
    }

    fn profile(&self, id: &str) -> Option<&Profile> {
        self.catalog
            .profiles
            .iter()
            .find(|profile| profile.id == id)
    }
}

fn validate(catalog: &Catalog) -> Result<(), String> {
    let ids = catalog
        .profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect::<BTreeSet<_>>();
    if catalog.schema != "tg4040-power-policies/v1"
        || catalog.target_sku != "TG4040"
        || catalog.lane != "host-native userspace simulator"
        || catalog.hardware_verified
        || catalog.real_device_operations != "denied"
        || catalog.thermal.throttling_enabled.not()
        || catalog.thermal.limit_c > 80
        || catalog.thermal.hysteresis_c < 1
        || catalog.thermal.throttle_profile != "eco"
        || ids != PROFILE_IDS.into_iter().collect()
        || catalog.global_default != "balanced"
    {
        return Err("TG4040 power policy safety boundary is invalid".into());
    }
    for profile in &catalog.profiles {
        if profile.cpu.min_k_hz > profile.cpu.max_k_hz
            || profile.cpu.max_k_hz > 1_800_000
            || profile.gpu.max_k_hz > 500_000
            || profile.display.width != 1024
            || profile.display.height != 768
            || profile.display.refresh_hz != 60
            || profile.display.mode != "1024x768@60"
        {
            return Err(format!("unsafe synthetic power profile: {}", profile.id));
        }
    }
    if catalog
        .system_defaults
        .values()
        .chain(catalog.game_overrides.values())
        .any(|id| !ids.contains(id.as_str()))
    {
        return Err("power policy references an unknown profile".into());
    }
    Ok(())
}
