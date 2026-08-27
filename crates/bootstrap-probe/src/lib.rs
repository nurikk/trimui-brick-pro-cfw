use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

const FIXTURE_SCHEMA: &str = "brickpro-synthetic-bootstrap-fixture/v1";
const PROFILE_SCHEMA: &str = "brickpro-bootstrap-profile/v1";
const TARGET_SKU: &str = "TG4040";
const REQUIRED_INPUT: &[&str] = &["dpad", "face-primary", "start", "select", "menu"];
const FIXTURE_IDS: &[&str] = &[
    "supported",
    "wrong-model",
    "unsupported-firmware",
    "missing-framebuffer",
    "invalid-framebuffer",
    "missing-input",
    "unsupported-storage",
    "missing-storage",
    "no-real-fingerprint",
    "recovery-chord",
    "recovery-next-boot",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub schema: &'static str,
    pub status: &'static str,
    pub reason: &'static str,
    pub handoff_eligible: bool,
}

impl ProbeResult {
    fn compatible() -> Self {
        Self {
            schema: "brickpro-bootstrap-probe/v1",
            status: "compatible",
            reason: "compatible",
            handoff_eligible: true,
        }
    }

    pub fn recovery(reason: &'static str) -> Self {
        Self {
            schema: "brickpro-bootstrap-probe/v1",
            status: "recovery",
            reason,
            handoff_eligible: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    schema: String,
    #[serde(rename = "fixtureId")]
    fixture_id: String,
    mode: String,
    #[serde(rename = "targetSku")]
    target_sku: String,
    #[serde(rename = "syntheticApproval")]
    synthetic_approval: bool,
    #[serde(rename = "realDeviceActivation")]
    real_device_activation: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    schema: String,
    model: Option<Model>,
    firmware: Option<Firmware>,
    framebuffer: Option<Framebuffer>,
    input: Option<Input>,
    storage: Option<Storage>,
    #[serde(rename = "syntheticProfileApproved")]
    synthetic_profile_approved: bool,
    #[serde(rename = "realDeviceFingerprintApproved")]
    real_device_fingerprint_approved: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    sku: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Firmware {
    #[serde(rename = "contractId")]
    contract_id: String,
    supported: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Framebuffer {
    present: bool,
    width: u32,
    height: u32,
    format: String,
    #[serde(rename = "strideBytes")]
    stride_bytes: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Storage {
    #[serde(rename = "logicalSd")]
    logical_sd: bool,
    #[serde(rename = "systemSlots")]
    system_slots: Vec<String>,
    data: bool,
    roms: bool,
    update: bool,
    #[serde(rename = "systemReadOnly")]
    system_read_only: bool,
}

pub fn probe_simulation(root: &Path) -> ProbeResult {
    let manifest: FixtureManifest = match read_json(root.join("fixture.json")) {
        Some(value) => value,
        None => return ProbeResult::recovery("simulation-interface-rejected"),
    };
    if manifest.schema != FIXTURE_SCHEMA
        || manifest.mode != "synthetic"
        || manifest.target_sku != TARGET_SKU
        || !manifest.synthetic_approval
        || manifest.real_device_activation
        || !FIXTURE_IDS.contains(&manifest.fixture_id.as_str())
    {
        return ProbeResult::recovery("simulation-interface-rejected");
    }

    let profile: Profile = match read_json(root.join("profile.json")) {
        Some(value) => value,
        None => return ProbeResult::recovery("fixture-invalid"),
    };
    if profile.schema != PROFILE_SCHEMA {
        return ProbeResult::recovery("fixture-invalid");
    }
    let model = match profile.model {
        Some(value) => value,
        None => return ProbeResult::recovery("model-identity-missing"),
    };
    if model.sku != TARGET_SKU {
        return ProbeResult::recovery("target-sku-mismatch");
    }

    let firmware = match profile.firmware {
        Some(value) => value,
        None => return ProbeResult::recovery("firmware-missing"),
    };
    if !firmware.supported || firmware.contract_id != "synthetic-tg4040-firmware-v1" {
        return ProbeResult::recovery("firmware-unsupported");
    }

    let framebuffer = match profile.framebuffer {
        Some(value) => value,
        None => return ProbeResult::recovery("framebuffer-missing"),
    };
    if !framebuffer.present {
        return ProbeResult::recovery("framebuffer-missing");
    }
    if framebuffer.width != 1024
        || framebuffer.height != 768
        || framebuffer.format != "rgba8888"
        || framebuffer.stride_bytes != 4096
    {
        return ProbeResult::recovery("framebuffer-invalid");
    }

    let input = match profile.input {
        Some(value) => value,
        None => return ProbeResult::recovery("input-missing"),
    };
    if REQUIRED_INPUT
        .iter()
        .any(|required| !input.capabilities.iter().any(|actual| actual == required))
    {
        return ProbeResult::recovery("input-capability-missing");
    }

    let storage = match profile.storage {
        Some(value) => value,
        None => return ProbeResult::recovery("storage-missing"),
    };
    if !storage.logical_sd
        || storage.system_slots.len() != 2
        || !storage.system_slots.iter().any(|slot| slot == "A")
        || !storage.system_slots.iter().any(|slot| slot == "B")
        || !storage.data
        || !storage.roms
        || !storage.update
        || !storage.system_read_only
    {
        return ProbeResult::recovery("storage-unsupported");
    }
    for relative in [
        ".brickpro/system/slots/A",
        ".brickpro/system/slots/B",
        ".brickpro/data/update",
        "roms",
    ] {
        if !directory_without_symlink(&root.join(relative)) {
            return ProbeResult::recovery("storage-missing");
        }
    }

    if !profile.synthetic_profile_approved
        || profile.real_device_fingerprint_approved != Some(false)
    {
        return ProbeResult::recovery("real-fingerprint-not-approved");
    }
    ProbeResult::compatible()
}

fn directory_without_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir())
        .unwrap_or(false)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() > 16 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub fn print_result(result: &ProbeResult) {
    println!(
        "{}",
        serde_json::to_string(result).expect("probe result is serializable")
    );
}
