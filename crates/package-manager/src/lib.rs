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
    pub developer: DeveloperPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    pub tier: TrustTier,
    pub immutable_root: String,
    pub runtime_root: String,
    pub cache_root: String,
    pub preserve: Vec<String>,
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
    if !identifier(&manifest.id) || !version(&manifest.version) {
        bail!("invalid package id or version")
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        bail!("package file count is outside bounds")
    }
    if !manifest.entrypoints.is_empty() {
        bail!("executable entrypoints are not supported")
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
    let (manifest, manifest_bytes) = load_manifest(manifest_path)?;
    if target.path != format!("packages/{}/manifest.json", manifest.id) {
        bail!("signed target does not match package id")
    }
    verify_target(target, &manifest_bytes)?;
    verify_tier(&manifest, context)?;
    let payload_root = payload_root
        .canonicalize()
        .context("resolve package payload root")?;
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
            )?;
            let destination = staging.join(&file.path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(destination, bytes)?;
            copied += 1;
            if options.interrupt_after_files == Some(copied) {
                bail!("simulated interrupted install")
            }
        }
        fs::create_dir_all(version_root.parent().unwrap())?;
        fs::rename(&staging, &version_root).context("promote complete package")?;
        let record = ActivationRecord {
            format: "brickpro-package-activation".to_string(),
            schema_version: 1,
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            tier: manifest.tier.clone(),
            immutable_root: format!(
                "{PACKAGE_ROOT}/{}/{}/immutable",
                manifest.id, manifest.version
            ),
            runtime_root: format!(
                "{PACKAGE_ROOT}/{}/{}/runtime",
                manifest.id, manifest.version
            ),
            cache_root: format!("{PACKAGE_ROOT}/{}/{}/cache", manifest.id, manifest.version),
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
    if record.format != "brickpro-package-activation"
        || record.schema_version != 1
        || record.id != id
        || !version(&record.version)
        || record.preserve != PRESERVED
        || record.immutable_root != format!("{PACKAGE_ROOT}/{}/{}/immutable", id, record.version)
        || record.runtime_root != format!("{PACKAGE_ROOT}/{}/{}/runtime", id, record.version)
        || record.cache_root != format!("{PACKAGE_ROOT}/{}/{}/cache", id, record.version)
    {
        bail!("invalid activation record; no package data removed")
    }
    let package_root = root.join(PACKAGE_ROOT).join(id);
    let version_root = package_root.join(&record.version);
    if let Ok(metadata) = fs::symlink_metadata(&version_root) {
        if metadata.file_type().is_symlink() {
            bail!("package version root is a symlink")
        }
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

fn verify_target(target: &VerifiedTarget, bytes: &[u8]) -> Result<()> {
    let mut path = target.path.split('/');
    if target.delegated_role != "packages"
        || path.next() != Some("packages")
        || path.next().is_none()
        || path.next() != Some("manifest.json")
        || path.next().is_some()
        || bytes.len() as u64 != target.length
    {
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

fn reject_executable(path: &str, bytes: &[u8], theme: bool) -> Result<()> {
    let lower = path.to_ascii_lowercase();
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
