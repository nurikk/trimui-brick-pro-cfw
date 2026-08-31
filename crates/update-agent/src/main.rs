use std::{
    env,
    ffi::CString,
    fmt::Write as FmtWrite,
    fs,
    io::{self, Read},
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process,
};

use anyhow::{anyhow, bail, Context, Result};
use boot_state::{
    advance_update_status, load, load_or_initialize, load_update_status, mark_healthy,
    prepare_pending, protected_hashes, publish_update_status, rollback, select, state_path,
    tree_hash, Slot, State, UpdateStatus,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MAX_RELEASE_ID: usize = 48;
const MAX_PAYLOAD: u64 = 512 * 1024 * 1024;
const DATA_SCHEMA: u32 = 1;
const ABI: &str = "tg4040-userspace-v1";
const MIN_BATTERY_PERCENT: u8 = 50;
const APPROVED_USER_DATA: [(&str, &str); 6] = [
    ("saves", "data/saves"),
    ("credentials", "data/credentials"),
    ("achievements", "data/achievements"),
    ("mappings", "data/mappings"),
    ("fn-led-settings", "data/settings/fn-led"),
    ("service-settings", "data/settings/services"),
];

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(rename = "$schema")]
    schema: String,
    #[serde(rename = "manifestVersion")]
    manifest_version: u8,
    #[serde(rename = "deviceId")]
    device_id: String,
    #[serde(rename = "targetSku")]
    target_sku: String,
    #[serde(rename = "hardwareRevision")]
    hardware_revision: String,
    #[serde(rename = "sourceRelease")]
    source_release: String,
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
    #[serde(rename = "requiredFreeBytes")]
    required_free_bytes: u64,
    #[serde(rename = "userDataManifest")]
    user_data_manifest: UserDataManifest,
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

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct UserDataManifest {
    format: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    entries: Vec<UserDataEntry>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct UserDataEntry {
    class: String,
    path: String,
    #[serde(rename = "sourceSchema")]
    source_schema: u32,
    #[serde(rename = "targetSchema")]
    target_schema: u32,
    migration: String,
}

#[derive(Clone, Copy)]
struct PowerStatus {
    battery_percent: u8,
    external_power: bool,
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
            let source = option(&mut args, "--source")?;
            let device_profile = option(&mut args, "--device-profile")?;
            let device = device_profile::DeviceProfile::from_path(Path::new(&device_profile))
                .context("device profile")?;
            let firmware = option(&mut args, "--stock-firmware")?;
            let abi = option(&mut args, "--userspace-abi")?;
            let data_schema = option(&mut args, "--data-schema")?
                .parse::<u32>()
                .context("data schema")?;
            let battery_percent = option(&mut args, "--battery-percent")?
                .parse::<u8>()
                .context("battery percent")?;
            let external_power = match option(&mut args, "--external-power")?.as_str() {
                "true" => true,
                "false" => false,
                _ => bail!("external power must be true or false"),
            };
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
                    source: &source,
                    device,
                    firmware: &firmware,
                    abi: &abi,
                    data_schema,
                    power: PowerStatus {
                        battery_percent,
                        external_power,
                    },
                    available_bytes: None,
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
    if manifest.device_id != device.device_id()
        || manifest.target_sku != device.target_sku()
        || Some(manifest.hardware_revision.as_str()) != device.hardware_revision()
    {
        bail!("manifest device, SKU, or hardware revision does not match this device")
    }
    if !valid_id(&manifest.source_release)
        || !valid_id(&manifest.release_id)
        || manifest.source_release == manifest.release_id
    {
        bail!("source or target release is invalid")
    }
    if !valid_artifact_url(&manifest.artifact_url) {
        bail!("artifact URL must be an ordinary HTTPS URL")
    }
    if !valid_artifact_name(&manifest.artifact_name) {
        bail!("artifact name must be a safe plain filename")
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
    let peak = manifest
        .payload_size
        .checked_mul(3)
        .ok_or_else(|| anyhow!("peak update space overflow"))?;
    if manifest.required_free_bytes != peak {
        bail!("requiredFreeBytes must equal the three-copy peak")
    }
    validate_user_data_manifest(manifest)?;
    Ok(())
}

fn validate_user_data_manifest(manifest: &Manifest) -> Result<()> {
    let user_data = &manifest.user_data_manifest;
    if user_data.format != "update-user-data-manifest"
        || user_data.schema_version != 1
        || user_data.entries.len() != APPROVED_USER_DATA.len()
    {
        bail!("approved user-data manifest is invalid")
    }
    for (class, path) in APPROVED_USER_DATA {
        let matches: Vec<_> = user_data
            .entries
            .iter()
            .filter(|entry| entry.class == class && entry.path == path)
            .collect();
        if matches.len() != 1 {
            bail!("approved user-data manifest must contain each class and path exactly once")
        }
        let entry = matches[0];
        if entry.source_schema != DATA_SCHEMA
            || entry.target_schema < entry.source_schema
            || entry.target_schema > manifest.data_schema.max
            || entry.migration != "shared-data-copy-on-write"
        {
            bail!("approved user-data migration is incompatible")
        }
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
    verify_payload_content(path, manifest)
}

fn verify_payload_content(path: &Path, manifest: &Manifest) -> Result<()> {
    ensure_regular_file(path, "payload")?;
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
    let digest = digest.finalize();
    let actual = digest_hex(digest.as_ref());
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
    source: &'a str,
    device: device_profile::DeviceProfile,
    firmware: &'a str,
    abi: &'a str,
    data_schema: u32,
    power: PowerStatus,
    available_bytes: Option<u64>,
    interruption: Option<&'a str>,
}

fn state_without_mutation(root: &Path) -> Result<State> {
    match load(root) {
        Ok((_, state)) => Ok(state),
        Err(_) if !state_path(root, 0)?.exists() && !state_path(root, 1)?.exists() => {
            Ok(State::default())
        }
        Err(error) => Err(error),
    }
}

fn available_bytes(path: &Path) -> Result<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).context("storage path contains NUL")?;
    let mut status = MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: statvfs writes the provided structure and the CString is NUL-terminated.
    if unsafe { libc::statvfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error()).context("read available update space");
    }
    // SAFETY: statvfs returned success, so the structure was initialized.
    let status = unsafe { status.assume_init() };
    status
        .f_bavail
        .checked_mul(status.f_frsize)
        .ok_or_else(|| anyhow!("available update space overflow"))
}

fn approved_user_data_hashes(root: &Path) -> Result<Vec<String>> {
    APPROVED_USER_DATA
        .iter()
        .map(|(_, path)| tree_hash(&root.join(path)))
        .collect()
}

fn stage(root: &Path, input: &StageInput<'_>) -> Result<()> {
    let document = read_manifest(&input.manifest)?;
    if !matches!(input.source, "online" | "sideload") {
        bail!("update source must be online or sideload")
    }
    if !valid_id(&document.manifest.release_id) {
        bail!("target release is invalid")
    }
    let current = state_without_mutation(root)?;
    let mut status = UpdateStatus::new(
        &current.current_release,
        &document.manifest.release_id,
        input.source,
    );
    if let Some(previous) = load_update_status(root)?.filter(|previous| {
        previous.current_release == status.current_release
            && previous.target_release == status.target_release
            && previous.source == status.source
    }) {
        status.journal = previous.journal;
        if status
            .journal
            .last()
            .is_none_or(|stage| stage != "preflight")
        {
            if status.journal.len() == 16 {
                status.journal.remove(0);
            }
            status.journal.push("preflight".into());
        }
    }
    publish_update_status(root, &status)?;
    let result = stage_inner(root, input, &document.manifest, &current);
    if let Err(error) = &result {
        let message: String = error.to_string().chars().take(240).collect();
        let progress = load_update_status(root)
            .ok()
            .flatten()
            .map_or(0, |status| status.progress_percent);
        let _ = advance_update_status(
            root,
            "error",
            progress,
            Some(&message),
            "Fix the reported problem, then retry; the previous release remains bootable",
        );
    }
    result
}

fn stage_inner(
    root: &Path,
    input: &StageInput<'_>,
    manifest: &Manifest,
    current: &State,
) -> Result<()> {
    validate_manifest(manifest, &input.device)?;
    if current.current_release != manifest.source_release {
        bail!("package source release does not match the active release")
    }
    let firmware = parse_firmware_version(input.firmware)?;
    if firmware < parse_firmware_version(&manifest.stock_firmware.min)?
        || firmware > parse_firmware_version(&manifest.stock_firmware.max)?
    {
        bail!("stock firmware is incompatible")
    }
    if input.abi != manifest.userspace_abi {
        bail!("userspace ABI is incompatible")
    }
    if input.data_schema < manifest.data_schema.min
        || input.data_schema > manifest.data_schema.max
        || manifest
            .user_data_manifest
            .entries
            .iter()
            .any(|entry| entry.source_schema != input.data_schema)
    {
        bail!("data schema is incompatible")
    }
    if input.power.battery_percent > 100
        || (!input.power.external_power && input.power.battery_percent < MIN_BATTERY_PERCENT)
    {
        bail!("connect external power or charge the battery to at least 50%")
    }
    let free = input.available_bytes.unwrap_or(available_bytes(root)?);
    if free < manifest.required_free_bytes {
        bail!("insufficient free space for the staged and inactive-slot copies")
    }
    let proof = fs::read_to_string(root.join(".brickpro/data/prior-release-readable"))
        .context("prior-release-readable proof")?;
    if proof.trim() != "prior-release-readable-v1" {
        bail!("prior-release-readable proof is invalid")
    }
    verify_payload(&input.payload, manifest)?;
    let approved_before = approved_user_data_hashes(root)?;
    let protected_before = protected_hashes(root)?;
    interrupt(input.interruption, "preflight")?;

    advance_update_status(
        root,
        "download",
        25,
        None,
        if input.source == "online" {
            "Online payload downloaded and verified"
        } else {
            "Sideload payload found and verified"
        },
    )?;
    interrupt(input.interruption, "download")?;

    let (_, state) = load_or_initialize(root)?;
    if state.current != current.current || state.current_release != current.current_release {
        bail!("active release changed during preflight")
    }
    let slot = state.current.inactive();
    save_vault::SaveVault::snapshot_standard(root, save_vault::SnapshotReason::PreUpdate)
        .map_err(|error| anyhow!("pre-update save snapshot failed: {error}"))?;
    let vault_before = save_vault::SaveVault::standard_integrity(root)
        .map_err(|error| anyhow!("save vault integrity failed: {error}"))?;
    let staging = root
        .join(".brickpro/data/update/staging")
        .join(&manifest.release_id);
    fs::create_dir_all(&staging)?;
    let staged_payload = staging.join(&manifest.artifact_name);
    durable_replace_copy(
        &input.payload,
        &staging.join(format!("{}.tmp", manifest.artifact_name)),
        &staged_payload,
        &staging,
    )?;
    verify_payload_content(&staged_payload, manifest)?;
    advance_update_status(
        root,
        "unpack",
        50,
        None,
        "Package unpacked into update staging",
    )?;
    interrupt(input.interruption, "unpack")?;

    let slot_dir = root.join(".brickpro/system/slots").join(slot.as_str());
    fs::create_dir_all(&slot_dir)?;
    let slot_payload = slot_dir.join("system.squashfs");
    durable_replace_copy(
        &staged_payload,
        &slot_dir.join("system.squashfs.tmp"),
        &slot_payload,
        &slot_dir,
    )?;
    verify_payload_content(&slot_payload, manifest)?;
    advance_update_status(
        root,
        "apply",
        80,
        None,
        "Inactive slot verified; the active slot is unchanged",
    )?;
    interrupt(input.interruption, "apply")?;

    if approved_user_data_hashes(root)? != approved_before
        || protected_hashes(root)? != protected_before
    {
        bail!("protected user data changed")
    }
    if save_vault::SaveVault::standard_integrity(root)
        .map_err(|error| anyhow!("save vault integrity failed: {error}"))?
        != vault_before
    {
        bail!("save vault changed during update")
    }
    prepare_pending(root, slot, &manifest.release_id)?;
    advance_update_status(
        root,
        "first-boot",
        90,
        None,
        "Restart to test the new release; Recovery can restore the previous slot",
    )?;
    interrupt(input.interruption, "first-boot")?;
    Ok(())
}
fn digest_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut value, "{byte:02x}");
    }
    value
}

fn update_manifest_value() -> serde_json::Value {
    let entries: Vec<_> = APPROVED_USER_DATA
        .iter()
        .map(|(class, path)| {
            serde_json::json!({
                "class": class,
                "migration": "shared-data-copy-on-write",
                "path": path,
                "sourceSchema": DATA_SCHEMA,
                "targetSchema": DATA_SCHEMA
            })
        })
        .collect();
    serde_json::json!({
        "$schema": "https://trimui.invalid/schemas/update-manifest-v1.schema.json",
        "artifactName": "payload.squashfs",
        "artifactUrl": "https://updates.trimui.invalid/releases/release-simulation/payload.squashfs",
        "dataSchema": {"max": 1, "min": 1},
        "deviceId": "tg4040",
        "hardwareRevision": "synthetic-v1",
        "manifestVersion": 1,
        "payloadSha256": digest_hex(Sha256::digest(b"hsqs").as_ref()),
        "payloadSize": 4,
        "payloadType": "squashfs-userspace",
        "releaseId": "release-simulation",
        "requiredFreeBytes": 12,
        "sourceRelease": "base",
        "stockFirmware": {"max": "9.9.9", "min": "1.0.0"},
        "targetSku": "TG4040",
        "userDataManifest": {
            "entries": entries,
            "format": "update-user-data-manifest",
            "schemaVersion": 1
        },
        "userspaceAbi": ABI
    })
}

fn write_manifest(path: &Path, value: &serde_json::Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
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
        "data/credentials",
        "data/achievements",
        "data/mappings",
        "data/settings/fn-led",
        "data/settings/services",
        ".brickpro/data",
        ".brickpro/system/slots/A",
        ".brickpro/system/slots/B",
    ] {
        fs::create_dir_all(root.join(path))?;
    }
    for (path, bytes) in [
        ("roms/README.synthetic", b"protected\n".as_slice()),
        (
            "data/saves/save.synthetic",
            b"save-n-minus-one\n".as_slice(),
        ),
        ("data/states/state.synthetic", b"protected\n".as_slice()),
        ("data/resume/current.synthetic", b"protected\n".as_slice()),
        (
            "data/credentials/wifi.synthetic",
            b"credential\n".as_slice(),
        ),
        (
            "data/achievements/history.synthetic",
            b"achievement\n".as_slice(),
        ),
        ("data/mappings/pad.synthetic", b"mapping\n".as_slice()),
        (
            "data/settings/fn-led/options.synthetic",
            b"fn-led\n".as_slice(),
        ),
        (
            "data/settings/services/options.synthetic",
            b"services\n".as_slice(),
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
    write_manifest(&manifest, &update_manifest_value())?;
    Ok((manifest, payload))
}

fn fixture_device() -> Result<device_profile::DeviceProfile> {
    device_profile::DeviceProfile::from_json(include_bytes!(
        "../../../config/platform/tg4040/compatibility.json"
    ))
    .context("Brick Pro device profile")
}

fn run_fixture_stage(
    root: &Path,
    manifest: &Path,
    payload: &Path,
    source: &str,
    available_bytes: u64,
    power: PowerStatus,
    interruption: Option<&str>,
) -> Result<()> {
    stage(
        root,
        &StageInput {
            manifest: manifest.into(),
            payload: payload.into(),
            source,
            device: fixture_device()?,
            firmware: "1.0.0",
            abi: ABI,
            data_schema: DATA_SCHEMA,
            power,
            available_bytes: Some(available_bytes),
            interruption,
        },
    )
}

fn assert_previous_bootable(
    root: &Path,
    protected_before: &[String; 5],
    approved_before: &[String],
) -> Result<()> {
    let mut header = [0; 4];
    fs::File::open(root.join(".brickpro/system/slots/A/system.squashfs"))?
        .read_exact(&mut header)?;
    let state = state_without_mutation(root)?;
    if header != *b"hsqs"
        || state.current != Slot::A
        || state.last_known_good != Slot::A
        || protected_hashes(root)? != *protected_before
        || approved_user_data_hashes(root)? != approved_before
    {
        bail!("previous slot or protected user data is not recoverable")
    }
    Ok(())
}

fn assert_preflight_did_not_mutate_active_state(
    root: &Path,
    protected_before: &[String; 5],
    approved_before: &[String],
) -> Result<()> {
    if state_path(root, 0)?.exists()
        || state_path(root, 1)?.exists()
        || root
            .join(".brickpro/system/slots/B/system.squashfs")
            .exists()
    {
        bail!("rejected preflight mutated slot or boot state")
    }
    assert_previous_bootable(root, protected_before, approved_before)
}

fn journey(root: &Path) -> Result<()> {
    let validation_root = root.join("preflight-and-rollback");
    let (manifest, payload) = journey_fixture(&validation_root)?;
    let protected_before = protected_hashes(&validation_root)?;
    let approved_before = approved_user_data_hashes(&validation_root)?;
    let safe_power = PowerStatus {
        battery_percent: 80,
        external_power: false,
    };
    let mut value = update_manifest_value();

    for (field, replacement, label) in [
        ("targetSku", serde_json::json!("OTHER"), "wrong SKU"),
        (
            "hardwareRevision",
            serde_json::json!("other-revision"),
            "wrong revision",
        ),
        (
            "sourceRelease",
            serde_json::json!("other-release"),
            "wrong source release",
        ),
    ] {
        let original = value[field].clone();
        value[field] = replacement;
        write_manifest(&manifest, &value)?;
        if run_fixture_stage(
            &validation_root,
            &manifest,
            &payload,
            "online",
            12,
            safe_power,
            None,
        )
        .is_ok()
        {
            bail!("{label} payload was accepted")
        }
        assert_preflight_did_not_mutate_active_state(
            &validation_root,
            &protected_before,
            &approved_before,
        )?;
        value[field] = original;
    }
    write_manifest(&manifest, &value)?;

    fs::write(&payload, b"hsq")?;
    if run_fixture_stage(
        &validation_root,
        &manifest,
        &payload,
        "online",
        12,
        safe_power,
        None,
    )
    .is_ok()
    {
        bail!("truncated payload was accepted")
    }
    fs::write(&payload, b"hsqx")?;
    if run_fixture_stage(
        &validation_root,
        &manifest,
        &payload,
        "online",
        12,
        safe_power,
        None,
    )
    .is_ok()
    {
        bail!("corrupt payload was accepted")
    }
    fs::write(&payload, b"hsqs")?;
    if run_fixture_stage(
        &validation_root,
        &manifest,
        &payload,
        "online",
        11,
        safe_power,
        None,
    )
    .is_ok()
    {
        bail!("insufficient-space payload was accepted")
    }
    if run_fixture_stage(
        &validation_root,
        &manifest,
        &payload,
        "online",
        12,
        PowerStatus {
            battery_percent: 49,
            external_power: false,
        },
        None,
    )
    .is_ok()
    {
        bail!("unsafe power state was accepted")
    }
    assert_preflight_did_not_mutate_active_state(
        &validation_root,
        &protected_before,
        &approved_before,
    )?;

    run_fixture_stage(
        &validation_root,
        &manifest,
        &payload,
        "online",
        12,
        safe_power,
        None,
    )?;
    let (selected, reason, _) = select(&validation_root)?;
    if selected != Slot::B || reason != "pending" {
        bail!("staged inactive slot was not selected for first boot")
    }
    let healthy = mark_healthy(&validation_root, [true; 5])?;
    if healthy.current != Slot::B || approved_user_data_hashes(&validation_root)? != approved_before
    {
        bail!("N-1 to N did not preserve the approved user-data manifest")
    }
    let restored = rollback(&validation_root)?;
    if restored.current != Slot::A
        || restored.current_release != "base"
        || protected_hashes(&validation_root)? != protected_before
        || approved_user_data_hashes(&validation_root)? != approved_before
    {
        bail!("tested rollback did not restore the prior slot and data")
    }
    let status = load_update_status(&validation_root)?.context("update status")?;
    for stage in [
        "preflight",
        "download",
        "unpack",
        "apply",
        "first-boot",
        "complete",
        "rollback",
    ] {
        if !status.journal.iter().any(|entry| entry == stage) {
            bail!("update journal is missing {stage}")
        }
    }
    if status.source != "online"
        || status.current_release != "base"
        || status.target_release != "release-simulation"
    {
        bail!("online update status is incomplete")
    }

    for boundary in ["preflight", "download", "unpack", "apply", "first-boot"] {
        let case_root = root.join(format!("interrupt-{boundary}"));
        let (case_manifest, case_payload) = journey_fixture(&case_root)?;
        let case_protected = protected_hashes(&case_root)?;
        let case_approved = approved_user_data_hashes(&case_root)?;
        if run_fixture_stage(
            &case_root,
            &case_manifest,
            &case_payload,
            "sideload",
            12,
            safe_power,
            Some(boundary),
        )
        .is_ok()
        {
            bail!("{boundary} interruption unexpectedly completed")
        }
        assert_previous_bootable(&case_root, &case_protected, &case_approved)?;
        run_fixture_stage(
            &case_root,
            &case_manifest,
            &case_payload,
            "sideload",
            12,
            safe_power,
            None,
        )?;
        let (selected, reason, _) = select(&case_root)?;
        if selected != Slot::B || reason != "pending" {
            bail!("{boundary} interruption did not resume safely")
        }
        let restored = rollback(&case_root)?;
        if restored.current != Slot::A {
            bail!("{boundary} recovery did not restore the prior slot")
        }
        assert_previous_bootable(&case_root, &case_protected, &case_approved)?;
        let status = load_update_status(&case_root)?.context("sideload update status")?;
        if status.source != "sideload" {
            bail!("sideload source was not exposed to the UI")
        }
        println!(
            "boundary={boundary} interrupted=true resumed=true rollback=A protected=unchanged"
        );
    }
    println!(
        "update journey: online+sideload preflight, interruption resume, first boot, user-data preservation, and rollback passed"
    );
    Ok(())
}
