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
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MAX_RELEASE_ID: usize = 48;
const MAX_PAYLOAD: u64 = 512 * 1024 * 1024;
const DATA_SCHEMA: u32 = 1;
const ABI: &str = "tg4040-userspace-v1";
const PRODUCTION_PUBLIC_KEY: &str = include_str!("../../../keys/update.pub");

#[derive(Debug, Deserialize, serde::Serialize)]
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
    #[serde(rename = "releaseSequence")]
    release_sequence: u64,
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
    #[serde(rename = "trustedComment")]
    trusted_comment: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
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

#[derive(Debug, Deserialize, serde::Serialize)]
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
            let signature = option(&mut args, "--signature")?;
            let firmware = option(&mut args, "--stock-firmware")?;
            let abi = option(&mut args, "--userspace-abi")?;
            let schema = option(&mut args, "--data-schema")?
                .parse::<u32>()
                .context("data schema")?;
            let interrupt = match args.next().as_deref() {
                None => None,
                Some("--interrupt-after") => Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing interruption boundary"))?,
                ),
                Some(_) => bail!("unexpected argument"),
            };
            let input = StageInput {
                manifest: PathBuf::from(manifest),
                payload: PathBuf::from(payload),
                signature: PathBuf::from(signature),
                firmware: &firmware,
                abi: &abi,
                data_schema: schema,
                interruption: interrupt.as_deref(),
            };
            stage(&PathBuf::from(root), &input)?;
            println!("staged release")
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
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            let document = read_manifest(&PathBuf::from(manifest))?;
            validate_manifest(&document.manifest)?;
            println!("manifest valid; detached Minisign verification required before activation")
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
            .any(|c| c == std::path::Component::ParentDir)
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
    raw: Vec<u8>,
    digest: String,
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
    let mut identity = value;
    if let serde_json::Value::Object(fields) = &mut identity {
        fields.remove("trustedComment");
    }
    let mut identity_bytes = serde_json::to_vec_pretty(&identity)?;
    identity_bytes.push(b'\n');
    Ok(ManifestDocument {
        manifest,
        raw,
        digest: digest_hex(&identity_bytes),
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RELEASE_ID
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.schema != "https://trimui.invalid/schemas/update-manifest-v1.schema.json"
        || manifest.manifest_version != 1
    {
        bail!("unsupported manifest schema")
    }
    if manifest.device_id != "TG4040" {
        bail!("manifest target must be exact TG4040")
    }
    if !valid_id(&manifest.release_id) {
        bail!("release ID is empty or outside bounds")
    }
    if manifest.release_sequence == 0 {
        bail!("release sequence must be positive")
    }
    let firmware_min = parse_firmware_version(&manifest.stock_firmware.min)?;
    let firmware_max = parse_firmware_version(&manifest.stock_firmware.max)?;
    if firmware_min > firmware_max {
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
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        bail!("payload SHA-256 is invalid")
    }
    if manifest.trusted_comment.len() > 256 {
        bail!("trusted comment is oversized")
    }
    Ok(())
}

fn verify_payload(path: &Path, manifest: &Manifest) -> Result<()> {
    ensure_regular_file(path, "payload")?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(extension.as_deref(), Some("awimg" | "img" | "raw")) {
        bail!("fatal: .awimg/.img/.raw/raw image payloads are rejected")
    }
    let mut file = fs::File::open(path).context("open payload")?;
    let metadata = file.metadata()?;
    if metadata.len() != manifest.payload_size {
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
    let mut actual = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut actual, "{byte:02x}").expect("writing to String cannot fail");
    }
    if actual != manifest.payload_sha256 {
        bail!("payload SHA-256 mismatch")
    }
    Ok(())
}

fn expected_trusted_comment(manifest: &Manifest, manifest_digest: &str) -> String {
    format!(
        "project=trimui-brick-pro-cfw; target=tg4040; release={}; sequence={}; payload-sha256={}; manifest-sha256={}",
        manifest.release_id,
        manifest.release_sequence,
        manifest.payload_sha256,
        manifest_digest,
    )
}

fn verify_signature(document: &ManifestDocument, signature_path: &Path) -> Result<()> {
    let signature_text =
        String::from_utf8(read_regular_file(signature_path, "detached signature")?)
            .context("detached signature is not UTF-8")?;
    let signature = Signature::decode(&signature_text)
        .map_err(|error| anyhow!("decode detached Minisign signature: {error}"))?;
    let public_key = PublicKey::decode(PRODUCTION_PUBLIC_KEY)
        .map_err(|error| anyhow!("decode pinned update public key: {error}"))?;
    public_key
        .verify(&document.raw, &signature, false)
        .map_err(|error| anyhow!("detached Minisign verification failed: {error}"))?;
    let expected = expected_trusted_comment(&document.manifest, &document.digest);
    if document.manifest.trusted_comment != expected || signature.trusted_comment() != expected {
        bail!("signed trusted comment is unbound or legacy")
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
    signature: PathBuf,
    firmware: &'a str,
    abi: &'a str,
    data_schema: u32,
    interruption: Option<&'a str>,
}

fn stage(root: &Path, input: &StageInput<'_>) -> Result<()> {
    let document = read_manifest(&input.manifest)?;
    validate_manifest(&document.manifest)?;
    let firmware = parse_firmware_version(input.firmware)?;
    let firmware_min = parse_firmware_version(&document.manifest.stock_firmware.min)?;
    let firmware_max = parse_firmware_version(&document.manifest.stock_firmware.max)?;
    if firmware < firmware_min || firmware > firmware_max {
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
    let data = root.join(".brickpro/data");
    let proof = fs::read_to_string(data.join("prior-release-readable"))
        .context("prior-release-readable proof")?;
    if proof.trim() != "prior-release-readable-v1" {
        bail!("prior-release-readable proof is invalid")
    }
    verify_payload(&input.payload, &document.manifest)?;
    interrupt(input.interruption, "manifest")?;
    let (_, state) = load_or_initialize(root)?;
    if document.manifest.release_sequence <= state.current_release_sequence {
        bail!("release is a downgrade or equal sequence")
    }
    let slot = state.current.inactive();
    verify_signature(&document, &input.signature)?;
    save_vault::SaveVault::snapshot_standard(root, save_vault::SnapshotReason::PreUpdate)
        .map_err(|error| anyhow!("pre-update save snapshot failed: {error}"))?;
    let vault_before = save_vault::SaveVault::standard_integrity(root)
        .map_err(|error| anyhow!("save vault integrity failed: {error}"))?;
    let before = protected_hashes(root)?;
    let update = root.join(".brickpro/data/update");
    let staging = update.join("staging").join(&document.manifest.release_id);
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
    prepare_pending(
        root,
        slot,
        &document.manifest.release_id,
        document.manifest.release_sequence,
    )?;
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
    let mut value = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

fn journey_fixture(root: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    if root.exists() {
        bail!("each interruption case requires a fresh fixture root")
    }
    fs::create_dir_all(root.join("roms"))?;
    fs::create_dir_all(root.join("data/saves"))?;
    fs::create_dir_all(root.join("data/states"))?;
    fs::create_dir_all(root.join("data/resume"))?;
    fs::create_dir_all(root.join("data/settings"))?;
    fs::create_dir_all(root.join(".brickpro/data"))?;
    fs::create_dir_all(root.join(".brickpro/system/slots/A"))?;
    fs::create_dir_all(root.join(".brickpro/system/slots/B"))?;
    fs::write(root.join("roms/README.synthetic"), b"protected\n")?;
    fs::write(root.join("data/saves/save.synthetic"), b"protected\n")?;
    fs::write(root.join("data/states/state.synthetic"), b"protected\n")?;
    fs::write(root.join("data/resume/current.synthetic"), b"protected\n")?;
    fs::write(
        root.join("data/settings/settings.synthetic"),
        b"protected\n",
    )?;
    fs::write(
        root.join(".brickpro/data/prior-release-readable"),
        b"prior-release-readable-v1\n",
    )?;
    fs::write(
        root.join(".brickpro/system/slots/A/system.squashfs"),
        b"hsqs",
    )?;
    let payload = root.join("input.squashfs");
    fs::write(&payload, b"hsqs")?;
    let release = "release-simulation";
    let manifest = root.join("manifest.json");
    let mut value = serde_json::json!({
        "$schema": "https://trimui.invalid/schemas/update-manifest-v1.schema.json",
        "manifestVersion": 1,
        "deviceId": "TG4040",
        "releaseId": release,
        "releaseSequence": 1,
        "stockFirmware": {"min": "1.0.0", "max": "9.9.9"},
        "userspaceAbi": ABI,
        "dataSchema": {"min": 1, "max": 1},
        "payloadType": "squashfs-userspace",
        "payloadSize": 4,
        "payloadSha256": digest_hex(b"hsqs"),
        "trustedComment": ""
    });
    let mut identity = value.clone();
    identity
        .as_object_mut()
        .expect("manifest object")
        .remove("trustedComment");
    let mut identity_bytes = serde_json::to_vec_pretty(&identity)?;
    identity_bytes.push(b'\n');
    let trusted = format!(
        "project=trimui-brick-pro-cfw; target=tg4040; release={release}; sequence=1; payload-sha256={}; manifest-sha256={}",
        digest_hex(b"hsqs"),
        digest_hex(&identity_bytes),
    );
    value["trustedComment"] = serde_json::Value::String(trusted);
    let mut manifest_bytes = serde_json::to_vec_pretty(&value)?;
    manifest_bytes.push(b'\n');
    fs::write(&manifest, manifest_bytes)?;
    let signature = root.join("manifest.minisig");
    fs::write(
        &signature,
        format!(
            "untrusted comment: synthetic\nAA==\ntrusted comment: {}\nAA==\n",
            value["trustedComment"].as_str().expect("trusted comment")
        ),
    )?;
    Ok((manifest, payload, signature))
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
    let (_, payload, _) = journey_fixture(root)?;
    let before = protected_hashes(root)?;
    let update = root.join(".brickpro/data/update");
    let staging = update.join("staging/release-simulation");
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
    let manifest_root = root.join("manifest-validation");
    let (manifest, payload, signature) = journey_fixture(&manifest_root)?;
    let before = protected_hashes(&manifest_root)?;
    let input = StageInput {
        manifest,
        payload,
        signature,
        firmware: "1.0.0",
        abi: ABI,
        data_schema: DATA_SCHEMA,
        interruption: Some("manifest"),
    };
    if stage(&manifest_root, &input).is_ok() {
        bail!("manifest interruption unexpectedly completed")
    }
    assert_recovered(&manifest_root, &before)?;
    println!("boundary=manifest-validation result=interrupted recovered=true selected=A protected=unchanged pending=false");

    let signature_root = root.join("signature-validation");
    let (manifest, payload, signature) = journey_fixture(&signature_root)?;
    let before = protected_hashes(&signature_root)?;
    let input = StageInput {
        manifest,
        payload,
        signature,
        firmware: "1.0.0",
        abi: ABI,
        data_schema: DATA_SCHEMA,
        interruption: None,
    };
    let error = stage(&signature_root, &input).expect_err("signature validation must fail closed");
    assert_recovered(&signature_root, &before)?;
    println!("boundary=signature-validation result=fail-closed recovered=true selected=A protected=unchanged pending=false reason={error}");

    for boundary in [
        "staging",
        "payload-sync",
        "slot-sync",
        "state-record-publication",
    ] {
        run_partial_boundary(&root.join(boundary), boundary)?;
    }
    println!("journey cryptographic-verification=blocked successful-verified-staging=false");
    Ok(())
}
