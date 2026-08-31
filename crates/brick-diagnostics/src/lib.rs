use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
use std::{ffi::CString, os::unix::ffi::OsStrExt};

use audio_routing::{diagnostics_from_state, AudioDiagnostics};
use bootstrap_probe::probe_simulation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const TARGET_SKU: &str = "TG4040";
const MAX_JSON: usize = 16 * 1024;
const MAX_TEXT: usize = 128;
const CRASH_PATH: &str = ".brickpro/data/diagnostics/last-crash.json";
const CRASH_INPUT: &str = ".brickpro/data/diagnostics/crash-input.json";
const BUNDLE_DIR: &str = "trimui-support-bundle-v1";
const ARCHIVE_NAME: &str = "trimui-support-bundle-v1.tar";
const CHECKSUM_NAME: &str = "trimui-support-bundle-v1.tar.sha256";
const AUDIO_STATE_PATH: &str = ".brickpro/data/audio-routing/state.json";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportReport {
    pub schema: &'static str,
    pub status: &'static str,
    pub mode: &'static str,
    pub activating: bool,
    pub target_sku: TargetSku,
    pub firmware: Firmware,
    pub ram: Ram,
    pub battery: Battery,
    pub temperature: Temperature,
    pub storage: Storage,
    pub slots: Slots,
    pub active_core: ActiveCore,
    pub last_crash: LastCrash,
    pub audio: AudioDiagnostics,
    pub policy: SafeModePolicy,
    pub health_checks: Vec<HealthCheck>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSku {
    pub status: &'static str,
    pub value: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Firmware {
    Available { version: String, build: String },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Ram {
    Available {
        total_bytes: u64,
        available_bytes: u64,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Battery {
    Available { percent: u8, charging: bool },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Temperature {
    Available { celsius: i16 },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Storage {
    Available {
        capacity_bytes: u64,
        free_bytes: u64,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Slots {
    Available { active: String, previous: String },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ActiveCore {
    Available { id: String, version: String },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LastCrash {
    Available { record: CrashRecord },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashRecord {
    pub id: String,
    pub at_ms: u64,
    pub component: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeModePolicy {
    pub theme: &'static str,
    pub display: &'static str,
    pub input: &'static str,
    pub network: &'static str,
    pub third_party_themes: &'static str,
    pub background_indexing: &'static str,
    pub automatic_game_launch: &'static str,
    pub third_party_modules: &'static str,
    pub network_auto_start: &'static str,
    pub auto_resume: &'static str,
    pub saves: &'static str,
    pub diagnostics: &'static str,
    pub firmware_mutation: &'static str,
    pub rom_mutation: &'static str,
    pub save_mutation: &'static str,
    pub updater_record_mutation: &'static str,
    pub raw_storage_mutation: &'static str,
    pub emmc_mutation: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    pub id: &'static str,
    pub status: String,
    pub detail: String,
    pub next_step: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreview {
    pub schema: &'static str,
    pub status: &'static str,
    pub included_fields: [&'static str; 2],
    pub bytes: usize,
    pub checksum: String,
}

impl SupportReport {
    pub fn unavailable() -> Self {
        Self {
            schema: "brickpro-support-report/v1",
            status: "recovery",
            mode: "safe-mode",
            activating: false,
            target_sku: TargetSku {
                status: "verified",
                value: TARGET_SKU,
            },
            firmware: Firmware::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            ram: Ram::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            battery: Battery::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            temperature: Temperature::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            storage: Storage::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            slots: Slots::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            active_core: ActiveCore::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            last_crash: LastCrash::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            audio: AudioDiagnostics::Unavailable {
                reason: "not-supplied-by-fixture".into(),
            },
            health_checks: unavailable_health_checks(),
            policy: SafeModePolicy::default(),
        }
    }
}

impl Default for SafeModePolicy {
    fn default() -> Self {
        Self {
            theme: "built-in",
            display: "conservative",
            input: "conservative",
            network: "disabled",
            third_party_themes: "disabled",
            background_indexing: "disabled",
            automatic_game_launch: "disabled",
            third_party_modules: "disabled",
            network_auto_start: "disabled",
            auto_resume: "disabled",
            saves: "read-only",
            diagnostics: "read-only",
            firmware_mutation: "not-permitted",
            rom_mutation: "not-permitted",
            save_mutation: "not-permitted",
            updater_record_mutation: "not-permitted",
            raw_storage_mutation: "not-permitted",
            emmc_mutation: "not-permitted",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticsFixture {
    schema: String,
    firmware: RawFirmware,
    ram: RawRam,
    battery: RawBattery,
    temperature: RawTemperature,
    storage: RawStorage,
    slots: RawSlots,
    #[serde(rename = "activeCore")]
    active_core: RawCore,
    #[serde(rename = "lastCrash")]
    last_crash: RawCrashDatum,
    #[serde(rename = "healthChecks")]
    health_checks: Vec<RawHealthCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawFirmware {
    status: String,
    version: Option<String>,
    build: Option<String>,
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawRam {
    status: String,
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawBattery {
    status: String,
    percent: Option<u8>,
    charging: Option<bool>,
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawTemperature {
    status: String,
    celsius: Option<i16>,
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawStorage {
    status: String,
    capacity_bytes: Option<u64>,
    free_bytes: Option<u64>,
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawSlots {
    status: String,
    active: Option<String>,
    previous: Option<String>,
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawCore {
    status: String,
    id: Option<String>,
    version: Option<String>,
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawCrashDatum {
    status: String,
    record: Option<RawCrash>,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawHealthCheck {
    id: String,
    status: String,
    detail: String,
    next_step: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawCrash {
    schema: String,
    id: String,
    #[serde(rename = "atMs")]
    at_ms: u64,
    component: String,
    reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleMetadata {
    schema: &'static str,
    bundle_version: &'static str,
    source: &'static str,
    target_sku: &'static str,
    redactions: [&'static str; 3],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub schema: &'static str,
    pub status: &'static str,
    pub bundle: &'static str,
    pub archive: &'static str,
    pub checksum: String,
}

pub fn safe_mode_report(root: &Path) -> Result<SupportReport, String> {
    validate_fixture_boundary(root, false)?;
    let probe = probe_simulation(root);
    if probe.reason == "simulation-interface-rejected"
        || probe.reason == "model-identity-missing"
        || probe.reason == "target-sku-mismatch"
        || probe.reason == "real-fingerprint-not-approved"
    {
        return Err("fixture-denied".into());
    }
    Ok(report_from_fixture(root, false).unwrap_or_else(|_| SupportReport::unavailable()))
}

pub fn preview_bundle(root: &Path) -> Result<BundlePreview, String> {
    let (archive, checksum) = bundle_payload(root)?;
    Ok(BundlePreview {
        schema: "brickpro-support-bundle-preview/v1",
        status: "ready",
        included_fields: ["support-report.json", "metadata.json"],
        bytes: archive.len(),
        checksum,
    })
}

pub fn export_bundle(
    root: &Path,
    destination: &Path,
    preview_checksum: &str,
) -> Result<ExportResult, String> {
    validate_destination(destination)?;
    let (archive, checksum) = bundle_payload(root)?;
    if checksum != preview_checksum {
        return Err("preview-confirmation-required".into());
    }
    let checksum_file = format!("{}  {}\n", checksum, ARCHIVE_NAME);
    atomic_export(destination, &archive, checksum_file.as_bytes())?;
    Ok(ExportResult {
        schema: "brickpro-diagnostics-result/v1",
        status: "exported",
        bundle: BUNDLE_DIR,
        archive: ARCHIVE_NAME,
        checksum,
    })
}

fn bundle_payload(root: &Path) -> Result<(Vec<u8>, String), String> {
    validate_fixture_boundary(root, true)?;
    if probe_simulation(root).status != "compatible" {
        return Err("fixture-not-compatible".into());
    }
    let report = report_from_fixture(root, true)?;
    let report_json = json_bytes(&report)?;
    let metadata = BundleMetadata {
        schema: "brickpro-support-bundle-metadata/v1",
        bundle_version: "1",
        source: "synthetic-fixture",
        target_sku: TARGET_SKU,
        redactions: ["secrets", "environment", "private-content"],
    };
    let metadata_json = json_bytes(&metadata)?;
    let archive = tar_bytes(&[
        ("support-report.json", &report_json),
        ("metadata.json", &metadata_json),
    ])?;
    if archive.len() > 64 * 1024 {
        return Err("bundle-too-large".into());
    }
    let checksum = hex_digest(&Sha256::digest(&archive));
    Ok((archive, checksum))
}

pub fn persist_crash(root: &Path) -> Result<(), String> {
    validate_fixture_boundary(root, false)?;
    let input = root.join(CRASH_INPUT);
    let bytes = read_regular(&input, 4096).map_err(|_| "crash-input-invalid".to_string())?;
    let raw: RawCrash =
        serde_json::from_slice(&bytes).map_err(|_| "crash-input-invalid".to_string())?;
    validate_crash(raw.clone()).map_err(|_| "crash-input-invalid".to_string())?;
    let encoded = json_bytes(&raw)?;
    let parent = ensure_parent(root, CRASH_PATH)?;
    let temporary = parent.join("last-crash.json.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "crash-persist-failed".to_string())?;
    file.write_all(&encoded)
        .map_err(|_| "crash-persist-failed".to_string())?;
    file.sync_all()
        .map_err(|_| "crash-persist-failed".to_string())?;
    drop(file);
    fs::rename(&temporary, parent.join("last-crash.json"))
        .map_err(|_| "crash-persist-failed".to_string())?;
    sync_directory(&parent).map_err(|_| "crash-persist-failed".to_string())?;
    Ok(())
}

fn report_from_fixture(root: &Path, strict: bool) -> Result<SupportReport, String> {
    let mut report = SupportReport::unavailable();
    let diagnostics_path = root.join("diagnostics.json");
    let bytes = match read_regular(&diagnostics_path, MAX_JSON) {
        Ok(bytes) => bytes,
        Err(_) if !strict => return Ok(report),
        Err(_) => return Err("diagnostics-input-invalid".into()),
    };
    let fixture: DiagnosticsFixture =
        serde_json::from_slice(&bytes).map_err(|_| "diagnostics-input-invalid".to_string())?;
    if fixture.schema != "brickpro-synthetic-diagnostics-fixture/v1" {
        return Err("diagnostics-input-invalid".into());
    }
    report.firmware =
        firmware(&fixture.firmware).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.ram = ram(&fixture.ram).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.battery =
        battery(&fixture.battery).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.temperature =
        temperature(&fixture.temperature).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.storage =
        storage(&fixture.storage).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.slots = slots(&fixture.slots).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.active_core =
        core(&fixture.active_core).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.last_crash =
        crash_datum(&fixture.last_crash).map_err(|_| "diagnostics-input-invalid".to_string())?;
    report.audio = diagnostics_from_state(&root.join(AUDIO_STATE_PATH));
    report.health_checks = health_checks(&fixture.health_checks)?;
    if let Some(check) = report
        .health_checks
        .iter_mut()
        .find(|check| check.id == "audio")
    {
        match &report.audio {
            AudioDiagnostics::Available { .. } => {
                check.status = "pass".into();
                check.detail = "Audio route available".into();
                check.next_step = "Continue recovery".into();
            }
            AudioDiagnostics::Unavailable { .. } => {
                check.status = "unavailable".into();
                check.detail = "Audio capability unavailable".into();
                check.next_step = "Check audio route".into();
            }
        }
    }
    match load_persisted_crash(root) {
        Ok(Some(value)) => report.last_crash = value,
        Ok(None) => {}
        Err(()) => {
            report.last_crash = LastCrash::Unavailable {
                reason: "crash-record-invalid".into(),
            }
        }
    }
    Ok(report)
}

fn firmware(raw: &RawFirmware) -> Result<Firmware, ()> {
    match raw.status.as_str() {
        "available" => Ok(Firmware::Available {
            version: text(raw.version.as_deref())?,
            build: text(raw.build.as_deref())?,
        }),
        "unavailable" => Ok(Firmware::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}
fn ram(raw: &RawRam) -> Result<Ram, ()> {
    match raw.status.as_str() {
        "available" => {
            let total = raw.total_bytes.ok_or(())?;
            let available = raw.available_bytes.ok_or(())?;
            if available > total {
                return Err(());
            }
            Ok(Ram::Available {
                total_bytes: total,
                available_bytes: available,
            })
        }
        "unavailable" => Ok(Ram::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}
fn battery(raw: &RawBattery) -> Result<Battery, ()> {
    match raw.status.as_str() {
        "available" => Ok(Battery::Available {
            percent: raw.percent.ok_or(())?,
            charging: raw.charging.ok_or(())?,
        }),
        "unavailable" => Ok(Battery::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}
fn temperature(raw: &RawTemperature) -> Result<Temperature, ()> {
    match raw.status.as_str() {
        "available" => {
            let value = raw.celsius.ok_or(())?;
            if !(-100..=150).contains(&value) {
                return Err(());
            }
            Ok(Temperature::Available { celsius: value })
        }
        "unavailable" => Ok(Temperature::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}
fn storage(raw: &RawStorage) -> Result<Storage, ()> {
    match raw.status.as_str() {
        "available" => {
            let capacity = raw.capacity_bytes.ok_or(())?;
            let free = raw.free_bytes.ok_or(())?;
            if free > capacity {
                return Err(());
            }
            Ok(Storage::Available {
                capacity_bytes: capacity,
                free_bytes: free,
            })
        }
        "unavailable" => Ok(Storage::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}
fn slots(raw: &RawSlots) -> Result<Slots, ()> {
    match raw.status.as_str() {
        "available" => {
            let active = text(raw.active.as_deref())?;
            let previous = text(raw.previous.as_deref())?;
            if !["A", "B"].contains(&active.as_str())
                || !["A", "B"].contains(&previous.as_str())
                || active == previous
            {
                return Err(());
            }
            Ok(Slots::Available { active, previous })
        }
        "unavailable" => Ok(Slots::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}
fn core(raw: &RawCore) -> Result<ActiveCore, ()> {
    match raw.status.as_str() {
        "available" => Ok(ActiveCore::Available {
            id: text(raw.id.as_deref())?,
            version: text(raw.version.as_deref())?,
        }),
        "unavailable" => Ok(ActiveCore::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}
fn crash_datum(raw: &RawCrashDatum) -> Result<LastCrash, ()> {
    match raw.status.as_str() {
        "available" => Ok(LastCrash::Available {
            record: validate_crash(raw.record.as_ref().ok_or(())?.clone()).map_err(|_| ())?,
        }),
        "unavailable" => Ok(LastCrash::Unavailable {
            reason: text(raw.reason.as_deref())?,
        }),
        _ => Err(()),
    }
}

const HEALTH_CHECK_IDS: [&str; 8] = [
    "build-sku",
    "storage",
    "battery-power",
    "input",
    "display",
    "audio",
    "wifi",
    "last-failed-stage",
];

fn unavailable_health_checks() -> Vec<HealthCheck> {
    HEALTH_CHECK_IDS
        .iter()
        .map(|id| HealthCheck {
            id,
            status: "unavailable".into(),
            detail: "not-supplied-by-fixture".into(),
            next_step: "Use supported diagnostics".into(),
        })
        .collect()
}

fn health_checks(raw: &[RawHealthCheck]) -> Result<Vec<HealthCheck>, String> {
    if raw.len() != HEALTH_CHECK_IDS.len() {
        return Err("diagnostics-input-invalid".into());
    }
    raw.iter()
        .zip(HEALTH_CHECK_IDS)
        .map(|(check, id)| {
            if check.id != id
                || !matches!(
                    check.status.as_str(),
                    "pass" | "warn" | "fail" | "unavailable"
                )
            {
                return Err("diagnostics-input-invalid".into());
            }
            Ok(HealthCheck {
                id,
                status: check.status.clone(),
                detail: health_text(&check.detail).map_err(|_| "diagnostics-input-invalid")?,
                next_step: health_text(&check.next_step)
                    .map_err(|_| "diagnostics-input-invalid")?,
            })
        })
        .collect()
}

fn health_text(value: &str) -> Result<String, ()> {
    if value.is_empty()
        || value.len() > MAX_TEXT
        || value.chars().any(|c| c.is_control())
        || value.contains('/')
        || value.contains('\\')
        || value.contains("..")
    {
        return Err(());
    }
    let lower = value.to_ascii_lowercase();
    if [
        "password",
        "secret",
        "token",
        "credential",
        "authorization",
        "bearer",
        "rom",
        "bios",
        "private",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(());
    }
    Ok(value.into())
}

fn validate_crash(raw: RawCrash) -> Result<CrashRecord, ()> {
    if raw.schema != "brickpro-synthetic-crash/v1" {
        return Err(());
    }
    Ok(CrashRecord {
        id: text(Some(&raw.id))?,
        at_ms: raw.at_ms,
        component: text(Some(&raw.component))?,
        reason: text(Some(&raw.reason))?,
    })
}

fn text(value: Option<&str>) -> Result<String, ()> {
    let value = value.ok_or(())?;
    if value.is_empty()
        || value.len() > MAX_TEXT
        || value.chars().any(|c| c.is_control())
        || value.contains('/')
        || value.contains('\\')
        || value.contains("..")
        || (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(());
    }
    let lower = value.to_ascii_lowercase();
    for marker in [
        "password",
        "secret",
        "token",
        "credential",
        "authorization",
        "bearer",
        "rom",
        "bios",
        "private",
        "network",
        "radio",
        "command",
        "path",
        "file",
        "log",
        "emmc",
        "updater",
    ] {
        if lower.contains(marker) {
            return Err(());
        }
    }
    Ok(value.to_string())
}

fn validate_fixture_boundary(root: &Path, require_diagnostics: bool) -> Result<(), String> {
    if !root.is_absolute() || !regular_dir(root) {
        return Err("fixture-denied".into());
    }
    for relative in ["fixture.json", "profile.json"] {
        if !regular_file(&root.join(relative)) {
            return Err("fixture-denied".into());
        }
    }
    if require_diagnostics && !regular_file(&root.join("diagnostics.json")) {
        return Err("diagnostics-input-invalid".into());
    }
    Ok(())
}

fn validate_destination(destination: &Path) -> Result<(), String> {
    if !destination.is_absolute()
        || destination.iter().any(|part| part == "." || part == "..")
        || !regular_dir(destination)
        || destination.is_symlink()
        || fs::canonicalize(destination).map_err(|_| "destination-denied")? != destination
    {
        return Err("destination-denied".into());
    }
    let bundle = destination.join(BUNDLE_DIR);
    if bundle.exists() || bundle.is_symlink() {
        return Err("destination-not-empty".into());
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(value)
        .map(|mut bytes| {
            bytes.push(b'\n');
            bytes
        })
        .map_err(|_| "serialization-failed".into())
}

fn read_regular(path: &Path, max: usize) -> Result<Vec<u8>, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if !metadata.file_type().is_file() || metadata.len() as usize > max {
        return Err(());
    }
    fs::read(path).map_err(|_| ())
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_file())
        .unwrap_or(false)
}
fn regular_dir(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false)
}

fn ensure_parent(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = root.join(relative);
    let parent = path
        .parent()
        .ok_or_else(|| "crash-persist-failed".to_string())?;
    let mut current = root.to_path_buf();
    for component in parent
        .strip_prefix(root)
        .map_err(|_| "crash-persist-failed")?
        .components()
    {
        current.push(component);
        if current.exists() {
            if !regular_dir(&current) {
                return Err("crash-persist-failed".into());
            }
        } else {
            fs::create_dir(&current).map_err(|_| "crash-persist-failed".to_string())?;
        }
    }
    Ok(parent.to_path_buf())
}

fn load_persisted_crash(root: &Path) -> Result<Option<LastCrash>, ()> {
    let path = root.join(CRASH_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = read_regular(&path, 4096)?;
    let raw: RawCrash = serde_json::from_slice(&bytes).map_err(|_| ())?;
    Ok(Some(LastCrash::Available {
        record: validate_crash(raw)?,
    }))
}

fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    fs::File::open(path)?.sync_all()
}

fn atomic_export(destination: &Path, archive: &[u8], checksum: &[u8]) -> Result<(), String> {
    let stage = destination.join(format!(".support-bundle-v1-stage-{}", std::process::id()));
    let bundle = destination.join(BUNDLE_DIR);
    fs::create_dir(&stage).map_err(|_| "export-failed".to_string())?;
    let result = (|| {
        write_synced(&stage.join(ARCHIVE_NAME), archive)?;
        write_synced(&stage.join(CHECKSUM_NAME), checksum)?;
        sync_directory(&stage).map_err(|_| "export-failed".to_string())?;
        publish_stage(&stage, &bundle)?;
        sync_directory(destination).map_err(|_| "export-failed".to_string())?;
        Ok(())
    })();
    if stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[cfg(target_os = "linux")]
fn publish_stage(stage: &Path, bundle: &Path) -> Result<(), String> {
    let stage =
        CString::new(stage.as_os_str().as_bytes()).map_err(|_| "export-failed".to_string())?;
    let bundle =
        CString::new(bundle.as_os_str().as_bytes()).map_err(|_| "export-failed".to_string())?;
    // SAFETY: both CStrings remain alive while renameat2 reads their NUL-terminated paths.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2 as libc::c_long,
            libc::AT_FDCWD,
            stage.as_ptr(),
            libc::AT_FDCWD,
            bundle.as_ptr(),
            1u32,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!("export-failed:{}", std::io::Error::last_os_error()))
    }
}

#[cfg(not(target_os = "linux"))]
fn publish_stage(_stage: &Path, _bundle: &Path) -> Result<(), String> {
    Err("export-failed".into())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "export-failed".to_string())?;
    file.write_all(bytes)
        .map_err(|_| "export-failed".to_string())?;
    file.sync_all().map_err(|_| "export-failed".to_string())
}

fn tar_bytes(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    for (name, data) in entries {
        if name.len() > 100 || name.contains('/') && name.split('/').any(|part| part == "..") {
            return Err("archive-path-invalid".into());
        }
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        octal(&mut header[100..108], 0o644);
        octal(&mut header[108..116], 0);
        octal(&mut header[116..124], 0);
        octal(&mut header[124..136], data.len() as u64);
        octal(&mut header[136..148], 0);
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u32 = header.iter().map(|byte| *byte as u32).sum();
        octal_checksum(&mut header[148..156], sum);
        output.extend_from_slice(&header);
        output.extend_from_slice(data);
        while output.len() % 512 != 0 {
            output.push(0);
        }
    }
    output.extend_from_slice(&[0u8; 1024]);
    Ok(output)
}

fn octal(output: &mut [u8], value: u64) {
    let text = format!("{:0width$o}\0", value, width = output.len() - 1);
    output.copy_from_slice(text.as_bytes());
}
fn octal_checksum(output: &mut [u8], value: u32) {
    output.copy_from_slice(format!("{:06o}\0 ", value).as_bytes());
}
