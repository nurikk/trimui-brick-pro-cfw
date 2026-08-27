use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use package_trust::VerifiedTarget;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_FILES: usize = 64;
const MAX_EXPANDED_BYTES: u64 = 1_048_576;
const PACKAGE_STATE: &str = ".brickpro/package-state";
const PACKAGE_ROOT: &str = ".brickpro/packages";
const PRESERVED: [&str; 3] = ["/roms", "/data/saves", "/data/states"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    #[serde(rename = "$schema")]
    pub schema_url: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub id: String,
    pub version: String,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    pub tier: TrustTier,
    #[serde(rename = "type")]
    pub package_type: PackageType,
    pub files: Vec<ManifestFile>,
    pub entrypoints: Vec<Entrypoint>,
    pub runtime: Runtime,
    pub capabilities: Capabilities,
    pub uninstall: UninstallRules,
    pub license: License,
    pub provenance: Provenance,
    #[serde(rename = "corePack", default)]
    pub core_pack: Option<BlockedCorePack>,
    pub developer: DeveloperPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TrustTier {
    Builtin,
    Verified,
    Community,
    Developer,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    Package,
    Theme,
    Recipe,
    Core,
    Portmaster,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub length: u64,
    #[serde(rename = "class")]
    pub file_class: FileClass,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entrypoint {
    pub name: String,
    pub path: String,
    pub mode: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    pub path: String,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub network: Vec<NetworkCapability>,
    pub input: Vec<InputCapability>,
    pub audio: Vec<AudioCapability>,
    pub display: Vec<DisplayCapability>,
    pub save: Vec<SaveCapability>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkCapability {
    Https,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InputCapability {
    Buttons,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioCapability {
    Playback,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DisplayCapability {
    Read,
    Overlay,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SaveCapability {
    Read,
    Write,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileClass {
    Immutable,
    Writable,
    Runtime,
    Cache,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UninstallRules {
    pub preserve: Vec<String>,
    pub remove: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct License {
    pub spdx: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub sbom: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockedCorePack {
    pub status: CorePackStatus,
    #[serde(rename = "blockedReason")]
    pub blocked_reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CorePackStatus {
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeveloperPolicy {
    pub enabled: bool,
    #[serde(rename = "localKeyTrusted")]
    pub local_key_trusted: bool,
    #[serde(rename = "nonRoot")]
    pub non_root: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TrustContext {
    pub signed: bool,
    pub developer_enabled: bool,
    pub local_key_trusted: bool,
    pub running_as_root: bool,
}

impl TrustContext {
    pub const fn community_signed() -> Self {
        Self {
            signed: true,
            developer_enabled: false,
            local_key_trusted: false,
            running_as_root: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TransactionOptions {
    pub interrupt_after_files: Option<usize>,
    pub interrupt_after_removals: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRecord {
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub id: String,
    pub version: String,
    pub target_sku: String,
    pub package_type: PackageType,
    pub tier: TrustTier,
    pub target_path: String,
    pub target_length: u64,
    pub target_sha256: String,
    pub manifest_sha256: String,
    pub immutable_root: String,
    pub runtime_root: String,
    pub cache_root: String,
    pub runtime_path: String,
    pub entrypoint_name: String,
    pub entrypoint_path: String,
    pub entrypoint_mode: String,
    pub preserve: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PortMasterActivation {
    pub id: String,
    pub version: String,
    pub package_root: PathBuf,
    pub runtime_root: PathBuf,
    pub library_root: PathBuf,
    pub entrypoint: PathBuf,
}

pub fn load_manifest(path: &Path) -> Result<(PackageManifest, Vec<u8>)> {
    let bytes = fs::read(path).with_context(|| format!("read manifest {}", path.display()))?;
    let manifest: PackageManifest =
        serde_json::from_slice(&bytes).context("parse package manifest")?;
    validate_manifest(&manifest)?;
    Ok((manifest, bytes))
}

pub fn validate_manifest(manifest: &PackageManifest) -> Result<()> {
    if manifest.schema_url != "https://example.invalid/trimui-brick-package-v1.schema.json"
        || manifest.format != "brickpro-package"
        || manifest.schema_version != 1
        || manifest.target_sku != "TG4040"
    {
        bail!("unsupported package schema or target")
    }
    if manifest.package_type == PackageType::Core {
        reject_core_pack(manifest)?;
    } else if manifest.core_pack.is_some() {
        bail!("core-pack metadata is only valid for core packages")
    }
    if !identifier(&manifest.id) || !version(&manifest.version) {
        bail!("invalid package id or version")
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        bail!("package file count is outside bounds")
    }
    if manifest.package_type == PackageType::Portmaster {
        validate_portmaster_layout(manifest)?;
    } else if !manifest.entrypoints.is_empty() {
        bail!("executable entrypoints are only supported for PortMaster")
    }
    if manifest.runtime.path != "runtime"
        || manifest.runtime.dependencies.len() > 32
        || manifest
            .runtime
            .dependencies
            .iter()
            .any(|dependency| !identifier(dependency))
    {
        bail!("runtime namespace is invalid")
    }
    if manifest.uninstall.preserve != PRESERVED
        || manifest.uninstall.remove != ["immutable", "runtime", "cache", "staging"]
    {
        bail!("uninstall rules do not preserve the protected storage boundary")
    }
    if manifest.provenance.sbom != "provenance/brickpro-cfw.spdx.json"
        || manifest.license.source.is_empty()
        || manifest.provenance.source.is_empty()
    {
        bail!("license or SBOM provenance is incomplete")
    }
    if manifest.tier.is_developer() {
        if !manifest.developer.enabled
            || !manifest.developer.local_key_trusted
            || !manifest.developer.non_root
        {
            bail!("developer policy is not explicit")
        }
    } else if manifest.developer.enabled || manifest.developer.local_key_trusted {
        bail!("developer enablement is only valid for developer tier")
    }
    let mut total = 0u64;
    let mut paths = Vec::new();
    let mut casefolded_paths = Vec::new();
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        if file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            bail!("file hash is not lowercase SHA-256")
        }
        if file.length > MAX_EXPANDED_BYTES
            || total
                .checked_add(file.length)
                .filter(|bytes| *bytes <= MAX_EXPANDED_BYTES)
                .is_none()
        {
            bail!("expanded package size exceeds bound")
        }
        total += file.length;
        let prefix = match file.file_class {
            FileClass::Immutable => "immutable/",
            FileClass::Writable => "writable/",
            FileClass::Runtime => "runtime/",
            FileClass::Cache => "cache/",
        };
        if !file.path.starts_with(prefix) {
            bail!("file class does not match its private namespace")
        }
        if paths.iter().any(|path: &&String| *path == &file.path) {
            bail!("duplicate package path")
        }
        let casefolded = file.path.to_ascii_lowercase();
        if casefolded_paths.iter().any(|path| path == &casefolded) {
            bail!("case-insensitive package path collision")
        }
        paths.push(&file.path);
        casefolded_paths.push(casefolded);
    }
    if manifest.package_type == PackageType::Theme
        && manifest
            .files
            .iter()
            .any(|file| !file.path.ends_with(".json"))
    {
        bail!("themes may contain JSON data only")
    }
    Ok(())
}

pub fn install(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    target: &VerifiedTarget,
    context: TrustContext,
    options: TransactionOptions,
) -> Result<ActivationRecord> {
    install_with_validation(
        root,
        manifest_path,
        payload_root,
        target,
        context,
        options,
        |_, _| Ok(()),
    )
}

pub fn install_with_validation<F>(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    target: &VerifiedTarget,
    context: TrustContext,
    options: TransactionOptions,
    validate_staging: F,
) -> Result<ActivationRecord>
where
    F: FnOnce(&PackageManifest, &Path) -> Result<()>,
{
    let (manifest, manifest_bytes) = load_manifest(manifest_path)?;
    reject_core_pack(&manifest)?;
    let expected_target = if target.delegated_role == "themes" {
        if manifest.version == "1.0.0" {
            format!("themes/{}/manifest.json", manifest.id)
        } else {
            format!("themes/{}/manifest-{}.json", manifest.id, manifest.version)
        }
    } else {
        format!("packages/{}/manifest.json", manifest.id)
    };
    if target.path != expected_target {
        bail!("signed target does not match package identity or version")
    }
    verify_target(target, &manifest_bytes, &manifest)?;
    verify_tier(&manifest, context)?;
    let payload_root = payload_root
        .canonicalize()
        .context("resolve package payload root")?;
    let state_path = root
        .join(PACKAGE_STATE)
        .join(format!("{}.json", manifest.id));
    if fs::symlink_metadata(&state_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("current activation record is a symlink")
    }
    if state_path.exists() {
        let current: ActivationRecord =
            serde_json::from_slice(&fs::read(&state_path)?).context("read current activation")?;
        validate_activation_record(&current, &manifest.id)?;
    }
    let private_root = root.join(PACKAGE_ROOT);
    let version_root = private_root.join(&manifest.id).join(&manifest.version);
    if version_root.exists() {
        bail!("package version is already installed")
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = private_root
        .join(".staging")
        .join(format!("{}-{}-{stamp}", manifest.id, manifest.version));
    fs::create_dir_all(&staging)?;
    let result = (|| {
        let mut copied = 0usize;
        for file in &manifest.files {
            let source = safe_source(&payload_root, &file.path)?;
            let metadata = fs::symlink_metadata(&source)?;
            if !metadata.file_type().is_file() {
                bail!("package payload contains a non-regular file")
            }
            let bytes = fs::read(&source)?;
            verify_file(file, &bytes)?;
            reject_executable(
                &file.path,
                &bytes,
                manifest.package_type == PackageType::Theme,
                manifest
                    .entrypoints
                    .iter()
                    .any(|entrypoint| entrypoint.path == file.path),
            )?;
            let destination = staging.join(&file.path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&destination, bytes)?;
            if let Some(entrypoint) = manifest
                .entrypoints
                .iter()
                .find(|entrypoint| entrypoint.path == file.path)
            {
                #[cfg(unix)]
                fs::set_permissions(
                    &destination,
                    std::os::unix::fs::PermissionsExt::from_mode(
                        u32::from_str_radix(&entrypoint.mode, 8).unwrap_or(0),
                    ),
                )?;
            }
            copied += 1;
            if options.interrupt_after_files == Some(copied) {
                bail!("simulated interrupted install")
            }
        }
        validate_staging(&manifest, &staging)?;
        fs::create_dir_all(
            version_root
                .parent()
                .context("package version has no parent")?,
        )?;
        fs::rename(&staging, &version_root).context("promote complete package")?;
        let installed_manifest = version_root.join("manifest.json");
        fs::write(&installed_manifest, &manifest_bytes)?;
        let entrypoint = manifest.entrypoints.first();
        let record = ActivationRecord {
            format: "brickpro-package-activation".to_string(),
            schema_version: 1,
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            target_sku: manifest.target_sku.clone(),
            package_type: manifest.package_type.clone(),
            tier: manifest.tier.clone(),
            target_path: target.path.clone(),
            target_length: target.length,
            target_sha256: target.sha256.clone(),
            manifest_sha256: hex::encode(Sha256::digest(&manifest_bytes)),
            immutable_root: format!(
                "{PACKAGE_ROOT}/{}/{}/immutable",
                manifest.id, manifest.version
            ),
            runtime_root: format!(
                "{PACKAGE_ROOT}/{}/{}/runtime",
                manifest.id, manifest.version
            ),
            cache_root: format!("{PACKAGE_ROOT}/{}/{}/cache", manifest.id, manifest.version),
            runtime_path: manifest.runtime.path.clone(),
            entrypoint_name: entrypoint.map_or_else(String::new, |value| value.name.clone()),
            entrypoint_path: entrypoint.map_or_else(String::new, |value| value.path.clone()),
            entrypoint_mode: entrypoint.map_or_else(String::new, |value| value.mode.clone()),
            preserve: PRESERVED.iter().map(|path| (*path).to_string()).collect(),
        };
        let state_root = root.join(PACKAGE_STATE);
        fs::create_dir_all(&state_root)?;
        let state_path = state_root.join(format!("{}.json", manifest.id));
        let temporary = state_path.with_extension("tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&record)?)?;
        fs::rename(temporary, state_path)?;
        Ok(record)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&version_root);
    }
    result
}

pub fn upgrade(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    target: &VerifiedTarget,
    context: TrustContext,
    options: TransactionOptions,
) -> Result<ActivationRecord> {
    install(root, manifest_path, payload_root, target, context, options)
}

pub fn uninstall(root: &Path, id: &str, options: TransactionOptions) -> Result<()> {
    if !identifier(id) {
        bail!("invalid package id")
    }
    let state_path = root.join(PACKAGE_STATE).join(format!("{id}.json"));
    if !state_path.is_file() {
        return Ok(());
    }
    let record: ActivationRecord =
        serde_json::from_slice(&fs::read(&state_path)?).context("read activation record")?;
    validate_activation_record(&record, id)?;
    let canonical_root = root.canonicalize()?;
    let package_root = root.join(PACKAGE_ROOT).join(id);
    reject_symlink_path(&package_root, "package root")?;
    let version_root = package_root.join(&record.version);
    reject_symlink_path(&version_root, "package version root")?;
    if !version_root.canonicalize()?.starts_with(&canonical_root) {
        bail!("package version root escapes private root")
    }
    let staging_root = root.join(PACKAGE_ROOT).join(".staging");
    if let Ok(entries) = fs::read_dir(&staging_root) {
        for entry in entries {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{id}-"))
            {
                fs::remove_dir_all(entry.path())?;
            }
        }
    }
    if version_root.exists() {
        if options.interrupt_after_removals == Some(0) {
            bail!("simulated interrupted uninstall")
        }
        fs::remove_dir_all(&version_root)?;
    }
    if package_root.is_dir() && package_root.read_dir()?.next().is_none() {
        fs::remove_dir(package_root)?;
    }
    fs::remove_file(state_path)?;
    Ok(())
}

fn validate_activation_record(record: &ActivationRecord, id: &str) -> Result<()> {
    if record.format != "brickpro-package-activation"
        || record.schema_version != 1
        || record.id != id
        || !version(&record.version)
        || record.target_sku != "TG4040"
        || record.preserve != PRESERVED
        || record.immutable_root != format!("{PACKAGE_ROOT}/{}/{}/immutable", id, record.version)
        || record.runtime_root != format!("{PACKAGE_ROOT}/{}/{}/runtime", id, record.version)
        || record.cache_root != format!("{PACKAGE_ROOT}/{}/{}/cache", id, record.version)
    {
        bail!("invalid activation record; no package data removed")
    }
    Ok(())
}

fn reject_core_pack(manifest: &PackageManifest) -> Result<()> {
    if manifest.package_type != PackageType::Core {
        if manifest.core_pack.is_some() {
            bail!("core-pack metadata is only valid for core packages")
        }
        return Ok(());
    }
    match &manifest.core_pack {
        Some(core_pack)
            if core_pack.status == CorePackStatus::Blocked
                && !core_pack.blocked_reason.is_empty() =>
        {
            bail!("blocked core-pack cannot be installed or activated")
        }
        _ => bail!("core package must have an explicit blocked state"),
    }
}

fn verify_target(target: &VerifiedTarget, bytes: &[u8], manifest: &PackageManifest) -> Result<()> {
    let mut path = target.path.split('/');
    let role = path.next();
    let target_name = path.next();
    let filename = path.next();
    let expected_theme_filename = if manifest.version == "1.0.0" {
        "manifest".to_string()
    } else {
        format!("manifest-{}", manifest.version)
    };
    let valid_theme_target = target.delegated_role == "themes"
        && role == Some("themes")
        && target_name == Some(manifest.id.as_str())
        && filename == Some(format!("{expected_theme_filename}.json").as_str())
        && path.next().is_none();
    let valid_package_target = target.delegated_role == "packages"
        && role == Some("packages")
        && target_name == Some(manifest.id.as_str())
        && filename == Some("manifest.json")
        && path.next().is_none();
    if (!valid_theme_target && !valid_package_target) || bytes.len() as u64 != target.length {
        bail!("manifest is not the signed package target")
    }
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != target.sha256 {
        bail!("manifest hash differs from signed target")
    }
    Ok(())
}

fn verify_tier(manifest: &PackageManifest, context: TrustContext) -> Result<()> {
    if !context.signed {
        bail!("unsigned package rejected")
    }
    if manifest.tier.is_developer()
        && (!context.developer_enabled || !context.local_key_trusted || context.running_as_root)
    {
        bail!("developer tier requires local non-root enablement")
    }
    Ok(())
}

fn verify_file(file: &ManifestFile, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != file.length {
        bail!("payload length differs from manifest")
    }
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != file.sha256 {
        bail!("payload hash differs from manifest")
    }
    Ok(())
}

fn reject_executable(path: &str, bytes: &[u8], theme: bool, entrypoint: bool) -> Result<()> {
    let lower = path.to_ascii_lowercase();
    if entrypoint {
        if !lower.ends_with(".sh") || !bytes.starts_with(b"#!/bin/sh\n") {
            bail!("PortMaster entrypoint must use the fixed shell interpreter")
        }
        return Ok(());
    }
    if !theme
        && (lower.ends_with(".sh")
            || lower.ends_with(".py")
            || lower.ends_with(".lua")
            || lower.ends_with(".so")
            || lower.ends_with(".bin"))
    {
        bail!("runnable payload is prohibited")
    }
    if bytes.starts_with(b"#!") || bytes.starts_with(b"\x7fELF") || bytes.contains(&0) {
        bail!("executable or binary payload is prohibited")
    }
    if theme {
        let value: Value = serde_json::from_slice(bytes).context("theme payload must be JSON")?;
        reject_theme_fields(&value)?;
    }
    Ok(())
}

fn reject_theme_fields(value: &Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if ["script", "command", "exec", "argv", "shell", "entrypoint"]
                    .contains(&key.as_str())
                {
                    bail!("theme contains executable field")
                }
                reject_theme_fields(child)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_theme_fields(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn safe_source(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path(relative)?;
    let source = root.join(relative);
    let parent = source
        .parent()
        .context("payload path has no parent")?
        .canonicalize()?;
    let source = parent.join(
        source
            .file_name()
            .context("payload path has no file name")?,
    );
    if !source.starts_with(root) {
        bail!("payload path escapes payload root")
    }
    Ok(source)
}

fn validate_portmaster_layout(manifest: &PackageManifest) -> Result<()> {
    if !matches!(manifest.tier, TrustTier::Builtin | TrustTier::Verified)
        || manifest.entrypoints.len() != 1
    {
        bail!("PortMaster requires a verified single entrypoint")
    }
    let entrypoint = &manifest.entrypoints[0];
    if entrypoint.name != "launch"
        || entrypoint.path != "immutable/port/launch.sh"
        || entrypoint.mode != "0755"
    {
        bail!("PortMaster entrypoint layout is invalid")
    }
    let file = manifest
        .files
        .iter()
        .find(|file| file.path == entrypoint.path)
        .ok_or_else(|| anyhow::anyhow!("PortMaster entrypoint is not declared"))?;
    if file.file_class != FileClass::Immutable {
        bail!("PortMaster entrypoint is not immutable")
    }
    if !manifest.files.iter().any(|file| {
        matches!(file.file_class, FileClass::Runtime) && file.path.starts_with("runtime/")
    }) || !manifest.runtime.dependencies.is_empty()
        || !manifest.capabilities.network.is_empty()
        || !manifest
            .capabilities
            .input
            .contains(&InputCapability::Buttons)
        || !manifest
            .capabilities
            .display
            .contains(&DisplayCapability::Read)
        || !manifest.capabilities.save.contains(&SaveCapability::Read)
        || !manifest.capabilities.save.contains(&SaveCapability::Write)
    {
        bail!("PortMaster runtime or capability projection is invalid")
    }
    Ok(())
}

pub fn resolve_portmaster(
    root: &Path,
    id: &str,
    package_version: &str,
) -> Result<PortMasterActivation> {
    if !identifier(id) || !version(package_version) {
        bail!("invalid PortMaster package identity")
    }
    let state_path = root.join(PACKAGE_STATE).join(format!("{id}.json"));
    if fs::symlink_metadata(&state_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("PortMaster activation is a symlink")
    }
    let record: ActivationRecord = serde_json::from_slice(&fs::read(&state_path)?)?;
    if record.format != "brickpro-package-activation"
        || record.schema_version != 1
        || record.id != id
        || record.version != package_version
        || record.target_sku != "TG4040"
        || record.package_type != PackageType::Portmaster
        || !matches!(record.tier, TrustTier::Builtin | TrustTier::Verified)
        || record.target_path != format!("packages/{id}/manifest.json")
        || record.target_length == 0
        || record.target_sha256.len() != 64
        || record
            .target_sha256
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
        || record.immutable_root != format!("{PACKAGE_ROOT}/{id}/{package_version}/immutable")
        || record.runtime_root != format!("{PACKAGE_ROOT}/{id}/{package_version}/runtime")
        || record.cache_root != format!("{PACKAGE_ROOT}/{id}/{package_version}/cache")
        || record.runtime_path != "runtime"
        || record.entrypoint_name != "launch"
        || record.entrypoint_path != "immutable/port/launch.sh"
        || record.entrypoint_mode != "0755"
        || record.preserve != PRESERVED
    {
        bail!("PortMaster activation identity or provenance is invalid")
    }
    let canonical_root = root.canonicalize()?;
    let state_parent = root.join(PACKAGE_STATE).canonicalize()?;
    if !state_parent.starts_with(&canonical_root) {
        bail!("PortMaster activation state escapes private root")
    }
    let packages = root.join(PACKAGE_ROOT);
    let version_root = packages.join(id).join(package_version);
    reject_symlink_path(&version_root, "PortMaster package root")?;
    let canonical_version_root = version_root.canonicalize()?;
    if !canonical_version_root.starts_with(&canonical_root) {
        bail!("PortMaster package root escapes private root")
    }
    let manifest_path = version_root.join("manifest.json");
    reject_symlink_path(&manifest_path, "PortMaster manifest")?;
    let (manifest, manifest_bytes) = load_manifest(&manifest_path)?;
    if manifest.id != id
        || manifest.version != package_version
        || manifest.package_type != PackageType::Portmaster
        || manifest.target_sku != "TG4040"
        || hex::encode(Sha256::digest(&manifest_bytes)) != record.manifest_sha256
        || hex::encode(Sha256::digest(&manifest_bytes)) != record.target_sha256
        || manifest_bytes.len() as u64 != record.target_length
        || manifest.runtime.path != record.runtime_path
        || manifest.entrypoints[0].name != record.entrypoint_name
        || manifest.entrypoints[0].path != record.entrypoint_path
        || manifest.entrypoints[0].mode != record.entrypoint_mode
    {
        bail!("PortMaster manifest provenance does not match activation")
    }
    for file in &manifest.files {
        let path = private_path(&version_root, &file.path)?;
        reject_symlink_path(&path, "PortMaster package file")?;
        let bytes = fs::read(&path)?;
        verify_file(file, &bytes)?;
    }
    let runtime_root = private_path(&version_root, "runtime")?;
    let library_root = private_path(&version_root, "runtime/lib")?;
    let entrypoint = private_path(&version_root, &record.entrypoint_path)?;
    if !runtime_root.is_dir() || !library_root.is_dir() || !entrypoint.is_file() {
        bail!("PortMaster private runtime or entrypoint is unavailable")
    }
    #[cfg(unix)]
    if std::os::unix::fs::MetadataExt::mode(&fs::metadata(&entrypoint)?) & 0o777 != 0o755 {
        bail!("PortMaster entrypoint mode differs from activation")
    }
    Ok(PortMasterActivation {
        id: id.to_string(),
        version: package_version.to_string(),
        package_root: version_root,
        runtime_root,
        library_root,
        entrypoint,
    })
}

fn private_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path(relative)?;
    reject_symlink_path(root, "private package root")?;
    let mut current = root.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        reject_symlink_path(&current, "private package path component")?;
    }
    let candidate = root.join(relative);
    let canonical_root = root.canonicalize()?;
    let canonical = candidate.canonicalize()?;
    if !canonical.starts_with(&canonical_root) {
        bail!("private package path escapes package root")
    }
    Ok(canonical)
}

fn reject_symlink_path(path: &Path, label: &str) -> Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("{label} is a symlink")
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("path is not normalized relative POSIX")
    }
    Ok(())
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && value.as_bytes()[0].is_ascii_lowercase()
}
fn version(value: &str) -> bool {
    value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

impl TrustTier {
    fn is_developer(&self) -> bool {
        matches!(self, Self::Developer)
    }
}
