use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_FILES: usize = 64;
const MAX_EXPANDED_BYTES: u64 = 1_048_576;
const PACKAGE_STATE: &str = ".brickpro/package-state";
const PACKAGE_ROOT: &str = ".brickpro/packages";
const PRESERVED: [&str; 6] = [
    "/roms",
    "/data/saves",
    "/data/states",
    "/data/resume",
    "/data/settings",
    "/.brickpro/save-vault",
];
const LEGACY_PRESERVED: [&str; 3] = ["/roms", "/data/saves", "/data/states"];

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
    #[serde(rename = "targetAbi", default = "default_target_abi")]
    pub target_abi: String,
    #[serde(rename = "type")]
    pub package_type: PackageType,
    pub files: Vec<ManifestFile>,
    pub entrypoints: Vec<Entrypoint>,
    #[serde(rename = "builtinEntrypoint", default)]
    pub builtin_entrypoint: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub storage: Storage,
    pub runtime: Runtime,
    pub capabilities: Capabilities,
    pub uninstall: UninstallRules,
    #[serde(rename = "corePack", default)]
    pub core_pack: Option<BlockedCorePack>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    Package,
    Theme,
    Recipe,
    Core,
    Portmaster,
    Module,
    Application,
    Utility,
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    #[serde(rename = "requiredBytes")]
    pub required_bytes: u64,
    #[serde(rename = "userData")]
    pub user_data: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DeviceProfile {
    pub sku: String,
    pub abi: String,
    pub libraries: BTreeMap<String, String>,
    pub free_bytes: u64,
}

impl DeviceProfile {
    pub fn brick_pro() -> Self {
        Self {
            sku: "TG4040".into(),
            abi: default_target_abi(),
            libraries: BTreeMap::new(),
            free_bytes: u64::MAX,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preflight {
    pub missing_dependencies: Vec<String>,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub compatibility_error: Option<String>,
}

impl Preflight {
    pub fn ready(&self) -> bool {
        self.compatibility_error.is_none()
            && self.missing_dependencies.is_empty()
            && self.available_bytes >= self.required_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageStatus {
    Available,
    Installed,
    UpdateAvailable,
    Incompatible,
    Broken,
}

pub fn bounded_log(lines: impl IntoIterator<Item = String>) -> Vec<String> {
    lines
        .into_iter()
        .take(32)
        .map(|line| line.chars().take(160).collect())
        .collect()
}

fn default_target_abi() -> String {
    "aarch64-unknown-linux-musl".into()
}

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub network: Vec<NetworkCapability>,
    pub input: Vec<InputCapability>,
    pub audio: Vec<AudioCapability>,
    pub display: Vec<DisplayCapability>,
    pub save: Vec<SaveCapability>,
    #[serde(default)]
    pub filesystem: Vec<FilesystemCapability>,
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
#[serde(rename_all = "kebab-case")]
pub enum FilesystemCapability {
    PackageData,
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
    #[serde(default = "default_target_abi")]
    pub target_abi: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub package_type: PackageType,
    #[serde(rename = "manifestLength")]
    pub manifest_length: u64,
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
        || !sku(&manifest.target_sku)
        || manifest.target_abi != default_target_abi()
    {
        bail!("unsupported package schema, SKU, or ABI")
    }
    reject_core_pack(manifest)?;
    if !identifier(&manifest.id) || !version(&manifest.version) {
        bail!("invalid package id or version")
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        bail!("package file count is outside bounds")
    }
    let mut dependencies = BTreeSet::new();
    if manifest.dependencies.len() > 32
        || manifest.dependencies.iter().any(|dependency| {
            !identifier(&dependency.id)
                || !version(&dependency.version)
                || !dependencies.insert(&dependency.id)
        })
    {
        bail!("package dependency is invalid")
    }
    let mut user_data = BTreeSet::new();
    if manifest.storage.required_bytes > MAX_EXPANDED_BYTES
        || manifest.storage.user_data.len() > MAX_FILES
        || manifest.storage.user_data.iter().any(|path| {
            validate_relative_path(path).is_err()
                || !path.starts_with("writable/")
                || !user_data.insert(path)
                || !manifest
                    .files
                    .iter()
                    .any(|file| file.file_class == FileClass::Writable && file.path == *path)
        })
    {
        bail!("package storage declaration is invalid")
    }
    if manifest.package_type == PackageType::Portmaster {
        validate_portmaster_layout(manifest)?;
    } else if !manifest.entrypoints.is_empty() {
        bail!("executable entrypoints are only supported for PortMaster")
    } else if matches!(
        manifest.package_type,
        PackageType::Module | PackageType::Application | PackageType::Utility
    ) && manifest
        .builtin_entrypoint
        .as_deref()
        .is_none_or(|entrypoint| !identifier(entrypoint))
    {
        bail!("optional package requires a fixed builtin entrypoint")
    } else if manifest.builtin_entrypoint.is_some() {
        bail!("builtin entrypoint is only valid for an optional package")
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
    if !valid_preserve(&manifest.uninstall.preserve)
        || manifest.uninstall.remove != ["immutable", "runtime", "cache", "staging"]
    {
        bail!("uninstall rules do not preserve the protected storage boundary")
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

pub fn preflight(manifest: &PackageManifest, device: &DeviceProfile) -> Preflight {
    let required_bytes = manifest
        .files
        .iter()
        .map(|file| file.length)
        .sum::<u64>()
        .max(manifest.storage.required_bytes);
    let compatibility_error = if manifest.target_sku != device.sku {
        Some("wrong SKU".into())
    } else if manifest.target_abi != device.abi {
        Some("wrong ABI".into())
    } else {
        None
    };
    let missing_dependencies = manifest
        .dependencies
        .iter()
        .filter(|dependency| device.libraries.get(&dependency.id) != Some(&dependency.version))
        .map(|dependency| format!("{} {}", dependency.id, dependency.version))
        .collect();
    Preflight {
        missing_dependencies,
        required_bytes,
        available_bytes: device.free_bytes,
        compatibility_error,
    }
}

fn require_preflight(manifest: &PackageManifest, device: &DeviceProfile) -> Result<()> {
    let preflight = preflight(manifest, device);
    if let Some(error) = preflight.compatibility_error {
        bail!("package preflight failed: {error}")
    }
    if !preflight.missing_dependencies.is_empty() {
        bail!(
            "package preflight missing dependency: {}",
            preflight.missing_dependencies.join(", ")
        )
    }
    if preflight.available_bytes < preflight.required_bytes {
        bail!(
            "package preflight insufficient space: need {}, have {}",
            preflight.required_bytes,
            preflight.available_bytes
        )
    }
    Ok(())
}

pub fn package_status(
    root: &Path,
    manifest: &PackageManifest,
    device: &DeviceProfile,
) -> PackageStatus {
    if !preflight(manifest, device).ready() {
        return PackageStatus::Incompatible;
    }
    let state_path = root
        .join(PACKAGE_STATE)
        .join(format!("{}.json", manifest.id));
    let Ok(bytes) = fs::read(state_path) else {
        return PackageStatus::Available;
    };
    let Ok(record) = serde_json::from_slice::<ActivationRecord>(&bytes) else {
        return PackageStatus::Broken;
    };
    if validate_activation_record(&record, &manifest.id).is_err() {
        return PackageStatus::Broken;
    }
    if !root
        .join(PACKAGE_ROOT)
        .join(&record.id)
        .join(&record.version)
        .is_dir()
    {
        PackageStatus::Broken
    } else if record.version == manifest.version {
        PackageStatus::Installed
    } else {
        PackageStatus::UpdateAvailable
    }
}

pub fn set_enabled(root: &Path, id: &str, enabled: bool) -> Result<()> {
    if !identifier(id) {
        bail!("invalid package id")
    }
    let root = root.canonicalize().context("resolve package root")?;
    let state_path = root.join(PACKAGE_STATE).join(format!("{id}.json"));
    reject_symlink_if_present(&state_path, "activation record")?;
    let mut record: ActivationRecord = serde_json::from_slice(&fs::read(&state_path)?)?;
    validate_activation_record(&record, id)?;
    record.enabled = enabled;
    write_atomic(&state_path, &serde_json::to_vec_pretty(&record)?)
}

pub fn simple_launcher_visible(root: &Path, id: &str) -> bool {
    let state_path = root.join(PACKAGE_STATE).join(format!("{id}.json"));
    let Ok(bytes) = fs::read(state_path) else {
        return false;
    };
    let Ok(record) = serde_json::from_slice::<ActivationRecord>(&bytes) else {
        return false;
    };
    validate_activation_record(&record, id).is_ok() && record.enabled
}

pub fn install(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    options: TransactionOptions,
) -> Result<ActivationRecord> {
    install_with_validation(root, manifest_path, payload_root, options, |_, _| Ok(()))
}

pub fn install_for_device(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    device: &DeviceProfile,
    options: TransactionOptions,
) -> Result<ActivationRecord> {
    install_with_validation_for_device(
        root,
        manifest_path,
        payload_root,
        device,
        options,
        |_, _| Ok(()),
    )
}

pub fn install_with_validation<F>(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    options: TransactionOptions,
    validate_staging: F,
) -> Result<ActivationRecord>
where
    F: FnOnce(&PackageManifest, &Path) -> Result<()>,
{
    install_with_validation_for_device(
        root,
        manifest_path,
        payload_root,
        &DeviceProfile::brick_pro(),
        options,
        validate_staging,
    )
}

pub fn install_with_validation_for_device<F>(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    device: &DeviceProfile,
    options: TransactionOptions,
    validate_staging: F,
) -> Result<ActivationRecord>
where
    F: FnOnce(&PackageManifest, &Path) -> Result<()>,
{
    let (manifest, manifest_bytes) = load_manifest(manifest_path)?;
    require_preflight(&manifest, device)?;
    let root = root.canonicalize().context("resolve package root")?;
    reject_symlink_if_present(&root.join(".brickpro"), "package control root")?;
    save_vault::SaveVault::snapshot_standard(&root, save_vault::SnapshotReason::PrePackage)
        .map_err(|error| anyhow::anyhow!("pre-package save snapshot failed: {error}"))?;
    let vault_before = save_vault::SaveVault::standard_integrity(&root)
        .map_err(|error| anyhow::anyhow!("save vault integrity failed: {error}"))?;
    let payload_root = payload_root
        .canonicalize()
        .context("resolve package payload root")?;
    reject_symlink_path(&payload_root, "package payload root")?;
    let state_root = root.join(PACKAGE_STATE);
    let private_root = root.join(PACKAGE_ROOT);
    reject_symlink_if_present(&state_root, "package state root")?;
    reject_symlink_if_present(&private_root, "package private root")?;
    let state_path = state_root.join(format!("{}.json", manifest.id));
    reject_symlink_if_present(&state_path, "current activation record")?;
    let current = if state_path.exists() {
        let current: ActivationRecord =
            serde_json::from_slice(&fs::read(&state_path)?).context("read current activation")?;
        validate_activation_record(&current, &manifest.id)?;
        if current.package_type != manifest.package_type {
            bail!("package type cannot change during update")
        }
        Some(current)
    } else {
        None
    };
    let version_root = private_root.join(&manifest.id).join(&manifest.version);
    reject_symlink_if_present(&private_root.join(&manifest.id), "package identity root")?;
    reject_symlink_if_present(&version_root, "package version root")?;
    if version_root.exists() {
        bail!("package version is already installed")
    }
    let staging_root = private_root.join(".staging");
    reject_symlink_if_present(&staging_root, "package staging root")?;
    fs::create_dir_all(&staging_root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = staging_root.join(format!("{}-{}-{stamp}", manifest.id, manifest.version));
    fs::create_dir(&staging)?;
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
            fs::create_dir_all(destination.parent().context("package path has no parent")?)?;
            fs::write(&destination, &bytes)?;
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
        carry_user_data(&root, current.as_ref(), &manifest, &staging)?;
        validate_staging(&manifest, &staging)?;
        fs::create_dir_all(
            version_root
                .parent()
                .context("package version has no parent")?,
        )?;
        fs::rename(&staging, &version_root).context("promote complete package")?;
        fs::write(version_root.join("manifest.json"), &manifest_bytes)?;
        let entrypoint = manifest.entrypoints.first();
        let record = ActivationRecord {
            format: "brickpro-package-activation".into(),
            schema_version: 1,
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            target_sku: manifest.target_sku.clone(),
            target_abi: manifest.target_abi.clone(),
            enabled: current.as_ref().is_none_or(|record| record.enabled),
            package_type: manifest.package_type.clone(),
            manifest_length: manifest_bytes.len() as u64,
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
            preserve: PRESERVED.iter().map(|path| (*path).into()).collect(),
        };
        fs::create_dir_all(&state_root)?;
        write_atomic(&state_path, &serde_json::to_vec_pretty(&record)?)?;
        Ok(record)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&version_root);
    }
    if result.is_ok()
        && save_vault::SaveVault::standard_integrity(&root)
            .map_err(|error| anyhow::anyhow!("save vault integrity failed: {error}"))?
            != vault_before
    {
        return Err(anyhow::anyhow!("package transaction changed save vault"));
    }
    result
}

pub fn upgrade(
    root: &Path,
    manifest_path: &Path,
    payload_root: &Path,
    options: TransactionOptions,
) -> Result<ActivationRecord> {
    let root = root.canonicalize().context("resolve package root")?;
    let manifest = load_manifest(manifest_path)?.0;
    let state_path = root
        .join(PACKAGE_STATE)
        .join(format!("{}.json", manifest.id));
    reject_symlink_if_present(&root.join(".brickpro"), "package control root")?;
    let previous = if state_path.is_file() {
        let record: ActivationRecord = serde_json::from_slice(&fs::read(&state_path)?)?;
        validate_activation_record(&record, &manifest.id)?;
        Some(record)
    } else {
        None
    };
    let installed = install(root.as_path(), manifest_path, payload_root, options)?;
    let Some(previous) = previous else {
        return Ok(installed);
    };
    if previous.version == installed.version {
        return Ok(installed);
    }
    let previous_root = root
        .join(PACKAGE_ROOT)
        .join(&previous.id)
        .join(&previous.version);
    reject_symlink_path(&previous_root, "previous package version root")?;
    fs::remove_dir_all(&previous_root).context("remove previous package version")?;
    Ok(installed)
}

pub fn uninstall(root: &Path, id: &str, options: TransactionOptions) -> Result<()> {
    if !identifier(id) {
        bail!("invalid package id")
    }
    let root = root.canonicalize().context("resolve package root")?;
    let state_root = root.join(PACKAGE_STATE);
    let private_root = root.join(PACKAGE_ROOT);
    let package_root = private_root.join(id);
    let staging_root = private_root.join(".staging");
    let state_path = state_root.join(format!("{id}.json"));
    reject_symlink_if_present(&root.join(".brickpro"), "package control root")?;
    reject_symlink_if_present(&state_root, "package state root")?;
    reject_symlink_if_present(&private_root, "package private root")?;
    reject_symlink_if_present(&package_root, "package root")?;
    reject_symlink_if_present(&staging_root, "package staging root")?;
    reject_symlink_if_present(&state_path, "activation record")?;
    if !state_path.is_file() {
        return Ok(());
    }
    let record: ActivationRecord = serde_json::from_slice(&fs::read(&state_path)?)?;
    validate_activation_record(&record, id)?;
    save_vault::SaveVault::snapshot_standard(&root, save_vault::SnapshotReason::PrePackage)
        .map_err(|error| anyhow::anyhow!("pre-package save snapshot failed: {error}"))?;
    let vault_before = save_vault::SaveVault::standard_integrity(&root)
        .map_err(|error| anyhow::anyhow!("save vault integrity failed: {error}"))?;
    let version_root = package_root.join(&record.version);
    reject_symlink_path(&version_root, "package version root")?;
    let canonical_root = root.canonicalize()?;
    if !version_root.canonicalize()?.starts_with(&canonical_root) {
        bail!("package version root escapes private root")
    }
    retain_user_data(&root, id, &version_root)?;
    if let Ok(entries) = fs::read_dir(&staging_root) {
        for entry in entries {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{id}-"))
            {
                let path = entry.path();
                reject_symlink_path(&path, "package staging entry")?;
                fs::remove_dir_all(path)?;
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
    if save_vault::SaveVault::standard_integrity(&root)
        .map_err(|error| anyhow::anyhow!("save vault integrity failed: {error}"))?
        != vault_before
    {
        bail!("package uninstall changed save vault")
    }
    Ok(())
}

fn valid_preserve(values: &[String]) -> bool {
    values
        .iter()
        .map(String::as_str)
        .eq(PRESERVED.iter().copied())
        || values
            .iter()
            .map(String::as_str)
            .eq(LEGACY_PRESERVED.iter().copied())
}

fn reject_symlink_if_present(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!("{label} is a symlink"),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_file_name(format!(
        ".{}.tmp-{stamp}",
        path.file_name()
            .context("activation path has no file name")?
            .to_string_lossy()
    ));
    reject_symlink_if_present(&temporary, "activation temporary")?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn validate_activation_record(record: &ActivationRecord, id: &str) -> Result<()> {
    if record.format != "brickpro-package-activation"
        || record.schema_version != 1
        || record.id != id
        || !version(&record.version)
        || !sku(&record.target_sku)
        || record.target_abi != default_target_abi()
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

fn verify_file(file: &ManifestFile, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != file.length {
        bail!("payload length differs from manifest")
    }
    if hex::encode(Sha256::digest(bytes)) != file.sha256 {
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

fn carry_user_data(
    root: &Path,
    current: Option<&ActivationRecord>,
    manifest: &PackageManifest,
    staging: &Path,
) -> Result<()> {
    let Some(current) = current else {
        return Ok(());
    };
    let previous_root = root
        .join(PACKAGE_ROOT)
        .join(&current.id)
        .join(&current.version);
    for relative in &manifest.storage.user_data {
        let source = previous_root.join(relative);
        if !source.exists() {
            continue;
        }
        let source = private_path(&previous_root, relative)?;
        let destination = staging.join(relative);
        fs::create_dir_all(
            destination
                .parent()
                .context("user data path has no parent")?,
        )?;
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn retain_user_data(root: &Path, id: &str, version_root: &Path) -> Result<()> {
    let (manifest, _) = load_manifest(&version_root.join("manifest.json"))?;
    for relative in &manifest.storage.user_data {
        let source = private_path(version_root, relative)?;
        let relative = relative
            .strip_prefix("writable/")
            .context("user data is outside writable storage")?;
        let destination = root.join("data/packages").join(id).join(relative);
        fs::create_dir_all(
            destination
                .parent()
                .context("retained data has no parent")?,
        )?;
        fs::copy(source, destination)?;
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
    if manifest.entrypoints.len() != 1 {
        bail!("PortMaster requires a single entrypoint")
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
        .context("PortMaster entrypoint is not declared")?;
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
    let root = root.canonicalize().context("resolve package root")?;
    reject_symlink_if_present(&root.join(".brickpro"), "package control root")?;
    let state_path = root.join(PACKAGE_STATE).join(format!("{id}.json"));
    let record: ActivationRecord = serde_json::from_slice(&fs::read(&state_path)?)?;
    if record.format != "brickpro-package-activation"
        || record.schema_version != 1
        || record.id != id
        || record.version != package_version
        || record.target_sku != "TG4040"
        || record.target_abi != default_target_abi()
        || record.package_type != PackageType::Portmaster
        || record.immutable_root != format!("{PACKAGE_ROOT}/{id}/{package_version}/immutable")
        || record.runtime_root != format!("{PACKAGE_ROOT}/{id}/{package_version}/runtime")
        || record.cache_root != format!("{PACKAGE_ROOT}/{id}/{package_version}/cache")
        || record.runtime_path != "runtime"
        || record.entrypoint_name != "launch"
        || record.entrypoint_path != "immutable/port/launch.sh"
        || record.entrypoint_mode != "0755"
        || record.preserve != PRESERVED
    {
        bail!("PortMaster activation identity is invalid")
    }
    let version_root = root.join(PACKAGE_ROOT).join(id).join(package_version);
    let manifest_path = version_root.join("manifest.json");
    let (manifest, manifest_bytes) = load_manifest(&manifest_path)?;
    if manifest.id != id
        || manifest.version != package_version
        || manifest.package_type != PackageType::Portmaster
        || hex::encode(Sha256::digest(&manifest_bytes)) != record.manifest_sha256
        || manifest_bytes.len() as u64 != record.manifest_length
    {
        bail!("PortMaster manifest does not match activation")
    }
    for file in &manifest.files {
        let path = private_path(&version_root, &file.path)?;
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
        id: id.into(),
        version: package_version.into(),
        package_root: version_root,
        runtime_root,
        library_root,
        entrypoint,
    })
}

fn private_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path(relative)?;
    let mut current = root.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        reject_symlink_path(&current, "private package path component")?;
    }
    let canonical_root = root.canonicalize()?;
    let canonical = root.join(relative).canonicalize()?;
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

fn sku(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
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

#[cfg(test)]
mod tests {
    use super::*;

    const PORTMASTER_MANIFEST: &[u8] = include_bytes!(
        "../../../fixtures/session-broker/generated-v1/portmaster-payload/manifest.json"
    );
    const PORTMASTER_PAYLOAD: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/session-broker/generated-v1/portmaster-payload"
    );

    fn temporary_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "package-manager-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("data/saves")).unwrap();
        fs::create_dir_all(root.join("data/states")).unwrap();
        root
    }

    fn manifest_without_hash(path: &str) -> Vec<u8> {
        let mut manifest: Value = serde_json::from_slice(PORTMASTER_MANIFEST).unwrap();
        manifest["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|file| file["path"] == path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("sha256");
        serde_json::to_vec(&manifest).unwrap()
    }

    #[test]
    fn schema_and_runtime_require_each_file_hash() {
        let schema: Value =
            serde_json::from_slice(include_bytes!("../../../schemas/package-v1.schema.json"))
                .unwrap();
        assert!(schema["properties"]["files"]["items"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "sha256"));

        for path in ["immutable/port/launch.sh", "immutable/port/metadata.json"] {
            let root = temporary_root("missing-hash");
            let manifest_path = root.join("manifest.json");
            fs::write(&manifest_path, manifest_without_hash(path)).unwrap();
            assert!(install(
                &root,
                &manifest_path,
                Path::new(PORTMASTER_PAYLOAD),
                TransactionOptions::default(),
            )
            .is_err());
            assert!(!root.join(PACKAGE_STATE).exists());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn shipped_portmaster_manifests_validate() {
        for bytes in [
            include_bytes!("../../../fixtures/demo-content/orbit-garden/payload/manifest.json")
                as &[u8],
            include_bytes!("../../../fixtures/demo-content/signal-workshop/payload/manifest.json"),
            PORTMASTER_MANIFEST,
            include_bytes!(
                "../../../fixtures/session-broker/generated-v1/portmaster-success-payload/manifest.json"
            ),
        ] {
            validate_manifest(&serde_json::from_slice(bytes).unwrap()).unwrap();
        }
    }

    #[test]
    fn rejects_mismatch_and_post_install_tampering() {
        let manifest: PackageManifest = serde_json::from_slice(PORTMASTER_MANIFEST).unwrap();
        let file = manifest
            .files
            .iter()
            .find(|file| file.path == "immutable/port/launch.sh")
            .unwrap();
        let mut bytes = fs::read(Path::new(PORTMASTER_PAYLOAD).join(&file.path)).unwrap();
        bytes[0] ^= 1;
        assert!(verify_file(file, &bytes).is_err());

        let root = temporary_root("tamper");
        install(
            &root,
            Path::new(PORTMASTER_PAYLOAD)
                .join("manifest.json")
                .as_path(),
            Path::new(PORTMASTER_PAYLOAD),
            TransactionOptions::default(),
        )
        .unwrap();
        resolve_portmaster(&root, "generated-portmaster", "1.0.0").unwrap();
        fs::write(
            root.join(".brickpro/packages/generated-portmaster/1.0.0/immutable/port/metadata.json"),
            b"tampered",
        )
        .unwrap();
        assert!(resolve_portmaster(&root, "generated-portmaster", "1.0.0").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
