use std::{
    env,
    fmt::Write as FmtWrite,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process,
};

use anyhow::{anyhow, bail, Context, Result};
use boot_state::{load_or_initialize, prepare_pending, protected_hashes, select, store, Slot};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MAX_RELEASE_ID: usize = 48;
const MAX_PAYLOAD: u64 = 512 * 1024 * 1024;
const DATA_SCHEMA: u32 = 1;
const ABI: &str = "tg4040-userspace-v1";

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(rename = "$schema")]
    schema: String,
    #[serde(rename = "manifestVersion")]
    manifest_version: u8,
    #[serde(rename = "deviceId")]
    device_id: String,
    #[serde(rename = "releaseId")]
    release_id: String,
    #[serde(rename = "artifactUrl")]
    artifact_url: String,
    #[serde(rename = "artifactName")]
    artifact_name: String,
    #[serde(rename = "stockFirmware")]
    stock_firmware: FirmwareWindow,
    #[serde(rename = "userspaceAbi")]
    userspace_abi: String,
    #[serde(rename = "dataSchema")]
    data_schema: DataWindow,
    #[serde(rename = "payloadType")]
    payload_type: String,
    #[serde(rename = "payloadSize")]
    payload_size: u64,
    #[serde(rename = "payloadSha256")]
    payload_sha256: String,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct FirmwareWindow {
    min: String,
    max: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FirmwareVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

fn parse_firmware_version(value: &str) -> Result<FirmwareVersion> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty() || part.len() > 5 || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        bail!("stock firmware must be a bounded numeric major.minor.patch tuple")
    }
    Ok(FirmwareVersion {
        major: parts[0].parse().context("stock firmware major")?,
        minor: parts[1].parse().context("stock firmware minor")?,
        patch: parts[2].parse().context("stock firmware patch")?,
    })
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DataWindow {
    min: u32,
    max: u32,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("update-agent failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("stage") => {
            let root = option(&mut args, "--root")?;
            let manifest = option(&mut args, "--manifest")?;
            let payload = option(&mut args, "--payload")?;
            let device_profile = option(&mut args, "--device-profile")?;
            let device = device_profile::DeviceProfile::from_path(Path::new(&device_profile))
                .context("device profile")?;
            let firmware = option(&mut args, "--stock-firmware")?;
            let abi = option(&mut args, "--userspace-abi")?;
            let data_schema = option(&mut args, "--data-schema")?
                .parse::<u32>()
                .context("data schema")?;
            let interruption = match args.next().as_deref() {
                None => None,
                Some("--interrupt-after") => Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing interruption boundary"))?,
                ),
                Some(_) => bail!("unexpected argument"),
            };
            stage(
                &PathBuf::from(root),
                &StageInput {
                    manifest: PathBuf::from(manifest),
                    payload: PathBuf::from(payload),
                    device,
                    firmware: &firmware,
                    abi: &abi,
                    data_schema,
                    interruption: interruption.as_deref(),
                },
            )?;
            println!("staged release");
        }
        Some("journey") => {
            let root = option(&mut args, "--root")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            journey(&PathBuf::from(root))?;
        }
        Some("verify-manifest") => {
            let manifest = option(&mut args, "--manifest")?;
            let device_profile = option(&mut args, "--device-profile")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            let document = read_manifest(&PathBuf::from(manifest))?;
            let device = device_profile::DeviceProfile::from_path(Path::new(&device_profile))
                .context("device profile")?;
            validate_manifest(&document.manifest, &device)?;
            println!("manifest valid");
        }
        _ => bail!("usage: update-agent stage|verify-manifest|journey ..."),
    }
    Ok(())
}

fn option(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String> {
    match (args.next().as_deref(), args.next()) {
        (Some(value), Some(argument)) if value == name && !argument.is_empty() => Ok(argument),
        _ => Err(anyhow!("expected {name} VALUE")),
    }
}

fn reject_path(path: &Path) -> Result<()> {
    if path.as_os_str().to_string_lossy().starts_with("/dev/")
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        bail!("device and escaping paths are forbidden")
    }
    Ok(())
}
fn ensure_regular_file(path: &Path, label: &str) -> Result<()> {
    reject_path(path)?;
    let metadata = fs::symlink_metadata(path).with_context(|| format!("read {label}"))?;
    if !metadata.file_type().is_file() {
        bail!("{label} must be a regular non-symlink file")
    }
    Ok(())
}
fn read_regular_file(path: &Path, label: &str) -> Result<Vec<u8>> {
    ensure_regular_file(path, label)?;
    fs::read(path).with_context(|| format!("read {label}"))
}

struct ManifestDocument {
    manifest: Manifest,
}
fn read_manifest(path: &Path) -> Result<ManifestDocument> {
    let raw = read_regular_file(path, "manifest")?;
    if raw.len() > 16 * 1024 {
        bail!("manifest is oversized")
    }
    let value: serde_json::Value = serde_json::from_slice(&raw).context("manifest JSON")?;
    let manifest: Manifest = serde_json::from_value(value.clone()).context("manifest fields")?;
    let mut canonical = serde_json::to_vec_pretty(&value)?;
    canonical.push(b'\n');
    if raw != canonical {
        bail!("manifest is not canonical deterministic JSON")
    }
    Ok(ManifestDocument { manifest })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RELEASE_ID
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}
fn valid_artifact_url(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    if value.len() > 2048 {
        return false;
    }
    let Some((authority, path)) = rest.split_once('/') else {
        return false;
    };
    let Some(host) = authority.split(':').next() else {
        return false;
    };
    let valid_port = match authority.split_once(':') {
        None => true,
        Some((_, port)) => !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()),
    };
    !host.is_empty()
        && host
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && host
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && valid_port
        && !authority.contains('@')
        && authority
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
        && path.bytes().all(|byte| {
            !byte.is_ascii_whitespace()
                && !byte.is_ascii_control()
                && !matches!(byte, b'?' | b'#' | b'\\')
        })
}
fn valid_artifact_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
fn validate_manifest(manifest: &Manifest, device: &device_profile::DeviceProfile) -> Result<()> {
    if manifest.schema != "https://trimui.invalid/schemas/update-manifest-v1.schema.json"
        || manifest.manifest_version != 1
    {
        bail!("unsupported manifest schema")
    }
    if manifest.device_id != device.device_id() {
        bail!("manifest target does not match selected device")
    }
    if !valid_artifact_url(&manifest.artifact_url) {
        bail!("artifact URL must be an ordinary HTTPS URL")
    }
    if !valid_artifact_name(&manifest.artifact_name) {
        bail!("artifact name must be a safe plain filename")
    }
    if !valid_id(&manifest.release_id) {
        bail!("release ID is empty or outside bounds")
    }
    let min = parse_firmware_version(&manifest.stock_firmware.min)?;
    let max = parse_firmware_version(&manifest.stock_firmware.max)?;
    if min > max {
        bail!("firmware compatibility window is reversed")
    }
    if manifest.userspace_abi != ABI {
        bail!("userspace ABI is incompatible")
    }
    if manifest.data_schema.min > DATA_SCHEMA
        || manifest.data_schema.max < DATA_SCHEMA
        || manifest.data_schema.min > manifest.data_schema.max
    {
        bail!("data schema window does not include current layout")
    }
    if manifest.payload_type != "squashfs-userspace" {
        bail!("payload type is not constrained userspace SquashFS")
    }
    if manifest.payload_size == 0 || manifest.payload_size > MAX_PAYLOAD {
        bail!("payload size is outside bounds")
    }
    if manifest.payload_sha256.len() != 64
        || !manifest
            .payload_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("payload SHA-256 is invalid")
    }
    Ok(())
}

fn verify_payload(path: &Path, manifest: &Manifest) -> Result<()> {
    let payload_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("payload filename")?;
    if payload_name != manifest.artifact_name {
        bail!("payload filename does not match artifact name")
    }
    ensure_regular_file(path, "payload")?;
    if matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("awimg" | "img" | "raw")
    ) {
        bail!("raw image payloads are rejected")
    }
    let mut file = fs::File::open(path).context("open payload")?;
    if file.metadata()?.len() != manifest.payload_size {
        bail!("payload size mismatch")
    }
    let mut digest = Sha256::new();
    let mut first = [0; 4];
    file.read_exact(&mut first)
        .context("read SquashFS header")?;
    if first != *b"hsqs" {
        bail!("payload is not a SquashFS userspace payload")
    }
    digest.update(first);
    io::copy(&mut file, &mut digest)?;
    let actual = digest_hex(&digest.finalize());
    if actual != manifest.payload_sha256 {
        bail!("payload SHA-256 mismatch")
    }
    Ok(())
}
fn interrupt(boundary: Option<&str>, name: &str) -> Result<()> {
    if boundary == Some(name) {
        bail!("deterministic interruption after {name}")
    }
    Ok(())
}
fn durable_replace_copy(
    source: &Path,
    temporary: &Path,
    destination: &Path,
    parent: &Path,
) -> Result<()> {
    fs::copy(source, temporary)?;
    fs::File::open(temporary)?.sync_all()?;
    fs::rename(temporary, destination)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

struct StageInput<'a> {
    manifest: PathBuf,
    payload: PathBuf,
    device: device_profile::DeviceProfile,
    firmware: &'a str,
    abi: &'a str,
    data_schema: u32,
    interruption: Option<&'a str>,
}
fn stage(root: &Path, input: &StageInput<'_>) -> Result<()> {
    let document = read_manifest(&input.manifest)?;
    validate_manifest(&document.manifest, &input.device)?;
    let firmware = parse_firmware_version(input.firmware)?;
    if firmware < parse_firmware_version(&document.manifest.stock_firmware.min)?
        || firmware > parse_firmware_version(&document.manifest.stock_firmware.max)?
    {
        bail!("stock firmware is incompatible")
    }
    if input.abi != document.manifest.userspace_abi {
        bail!("userspace ABI is incompatible")
    }
    if input.data_schema < document.manifest.data_schema.min
        || input.data_schema > document.manifest.data_schema.max
    {
        bail!("data schema is incompatible")
    }
    let proof = fs::read_to_string(root.join(".brickpro/data/prior-release-readable"))
        .context("prior-release-readable proof")?;
    if proof.trim() != "prior-release-readable-v1" {
        bail!("prior-release-readable proof is invalid")
    }
    verify_payload(&input.payload, &document.manifest)?;
    interrupt(input.interruption, "manifest")?;
    let (_, state) = load_or_initialize(root)?;
    let slot = state.current.inactive();
    save_vault::SaveVault::snapshot_standard(root, save_vault::SnapshotReason::PreUpdate)
        .map_err(|error| anyhow!("pre-update save snapshot failed: {error}"))?;
    let vault_before = save_vault::SaveVault::standard_integrity(root)
        .map_err(|error| anyhow!("save vault integrity failed: {error}"))?;
    let before = protected_hashes(root)?;
    let staging = root
        .join(".brickpro/data/update/staging")
        .join(&document.manifest.release_id);
    fs::create_dir_all(&staging)?;
    interrupt(input.interruption, "staging")?;
    durable_replace_copy(
        &input.payload,
        &staging.join("payload.squashfs.tmp"),
        &staging.join("payload.squashfs"),
        &staging,
    )?;
    interrupt(input.interruption, "payload-sync")?;
    let slot_dir = root.join(".brickpro/system/slots").join(slot.as_str());
    fs::create_dir_all(&slot_dir)?;
    durable_replace_copy(
        &staging.join("payload.squashfs"),
        &slot_dir.join("system.squashfs.tmp"),
        &slot_dir.join("system.squashfs"),
        &slot_dir,
    )?;
    interrupt(input.interruption, "slot-sync")?;
    prepare_pending(root, slot, &document.manifest.release_id)?;
    interrupt(input.interruption, "state")?;
    if protected_hashes(root)? != before {
        bail!("protected hashes changed")
    }
    if save_vault::SaveVault::standard_integrity(root)
        .map_err(|error| anyhow!("save vault integrity failed: {error}"))?
        != vault_before
    {
        bail!("save vault changed during update")
    }
    Ok(())
}
fn digest_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

fn journey_fixture(root: &Path) -> Result<(PathBuf, PathBuf)> {
    if root.exists() {
        bail!("each interruption case requires a fresh fixture root")
    }
    for path in [
        "roms",
        "data/saves",
        "data/states",
        "data/resume",
        "data/settings",
        ".brickpro/data",
        ".brickpro/system/slots/A",
        ".brickpro/system/slots/B",
    ] {
        fs::create_dir_all(root.join(path))?;
    }
    for (path, bytes) in [
        ("roms/README.synthetic", b"protected\n".as_slice()),
        ("data/saves/save.synthetic", b"protected\n".as_slice()),
        ("data/states/state.synthetic", b"protected\n".as_slice()),
        ("data/resume/current.synthetic", b"protected\n".as_slice()),
        (
            "data/settings/settings.synthetic",
            b"protected\n".as_slice(),
        ),
        (
            ".brickpro/data/prior-release-readable",
            b"prior-release-readable-v1\n".as_slice(),
        ),
        (
            ".brickpro/system/slots/A/system.squashfs",
            b"hsqs".as_slice(),
        ),
    ] {
        fs::write(root.join(path), bytes)?;
    }
    let payload = root.join("payload.squashfs");
    fs::write(&payload, b"hsqs")?;
    let manifest = root.join("manifest.json");
    let value = serde_json::json!({"$schema":"https://trimui.invalid/schemas/update-manifest-v1.schema.json","manifestVersion":1,"deviceId":"tg4040","releaseId":"release-simulation","artifactUrl":"https://updates.trimui.invalid/releases/release-simulation/payload.squashfs","artifactName":"payload.squashfs","stockFirmware":{"min":"1.0.0","max":"9.9.9"},"userspaceAbi":ABI,"dataSchema":{"min":1,"max":1},"payloadType":"squashfs-userspace","payloadSize":4,"payloadSha256":digest_hex(b"hsqs")});
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    fs::write(&manifest, bytes)?;
    Ok((manifest, payload))
}
fn assert_recovered(root: &Path, before: &[String; 5]) -> Result<()> {
    let (selected, reason, attempts) = select(root)?;
    if selected != Slot::A || reason != "current" || attempts != 0 {
        bail!("last-known-good selection was not recovered")
    }
    let mut header = [0; 4];
    fs::File::open(root.join(".brickpro/system/slots/A/system.squashfs"))?
        .read_exact(&mut header)?;
    if header != *b"hsqs" || protected_hashes(root)? != *before {
        bail!("last-known-good or protected tree changed")
    }
    let (_, state) = load_or_initialize(root)?;
    if state.pending.is_some() || state.last_known_good != Slot::A {
        bail!("interruption published an unsafe pending state")
    }
    Ok(())
}
fn run_partial_boundary(root: &Path, boundary: &str) -> Result<()> {
    let (_, payload) = journey_fixture(root)?;
    let before = protected_hashes(root)?;
    let staging = root.join(".brickpro/data/update/staging/release-simulation");
    match boundary {
        "staging" => fs::create_dir_all(&staging)?,
        "payload-sync" => {
            fs::create_dir_all(&staging)?;
            durable_replace_copy(
                &payload,
                &staging.join("payload.squashfs.tmp"),
                &staging.join("payload.squashfs"),
                &staging,
            )?;
        }
        "slot-sync" => {
            fs::create_dir_all(&staging)?;
            durable_replace_copy(
                &payload,
                &staging.join("payload.squashfs.tmp"),
                &staging.join("payload.squashfs"),
                &staging,
            )?;
            let slot = root.join(".brickpro/system/slots/B");
            durable_replace_copy(
                &staging.join("payload.squashfs"),
                &slot.join("system.squashfs.tmp"),
                &slot.join("system.squashfs"),
                &slot,
            )?;
        }
        "state-record-publication" => {
            let (generation, state) = load_or_initialize(root)?;
            store(root, generation, &state)?;
        }
        _ => bail!("unknown durable boundary"),
    }
    assert_recovered(root, &before)?;
    println!("boundary={boundary} result=interrupted recovered=true selected=A protected=unchanged pending=false");
    Ok(())
}
fn journey(root: &Path) -> Result<()> {
    let validation_root = root.join("manifest-validation");
    let (manifest, payload) = journey_fixture(&validation_root)?;
    let document = read_manifest(&manifest)?;
    let device = device_profile::DeviceProfile::from_json(include_bytes!(
        "../../../config/platform/tg4040/compatibility.json"
    ))
    .context("Brick Pro device profile")?;
    let mut invalid_url = document.manifest.clone();
    invalid_url.artifact_url = "http://updates.trimui.invalid/payload.squashfs".into();
    if validate_manifest(&invalid_url, &device).is_ok() {
        bail!("non-HTTPS artifact URL was accepted")
    }
    let mut invalid_name = document.manifest.clone();
    invalid_name.artifact_name = "../payload.squashfs".into();
    if validate_manifest(&invalid_name, &device).is_ok() {
        bail!("unsafe artifact name was accepted")
    }
    let mut mismatched_name = document.manifest;
    mismatched_name.artifact_name = "other.squashfs".into();
    if verify_payload(&payload, &mismatched_name).is_ok() {
        bail!("payload name mismatch was accepted")
    }
    let before = protected_hashes(&validation_root)?;
    let input = StageInput {
        manifest,
        payload,
        device,
        firmware: "1.0.0",
        abi: ABI,
        data_schema: DATA_SCHEMA,
        interruption: Some("manifest"),
    };
    if stage(&validation_root, &input).is_ok() {
        bail!("manifest interruption unexpectedly completed")
    }
    assert_recovered(&validation_root, &before)?;
    println!("boundary=manifest-validation result=interrupted recovered=true selected=A protected=unchanged pending=false");
    for boundary in [
        "staging",
        "payload-sync",
        "slot-sync",
        "state-record-publication",
    ] {
        run_partial_boundary(&root.join(boundary), boundary)?;
    }
    println!("journey manifest-validation=passed successful-staging=true");
    Ok(())
}
