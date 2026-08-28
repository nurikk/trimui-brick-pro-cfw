use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use launcher_theme::{load_theme_dir, render_png, DirectCatalogTransport, ThemesCatalog};
use package_manager::{install_with_validation, uninstall, TransactionOptions, TrustContext};
use package_trust::{RepositoryMetadata, TrustStore, VerificationTime, VerifiedTarget};
use serde::{Deserialize, Serialize};

pub const TARGET_SKU: &str = "TG4040";
pub const THEME_API: &str = "1.0.0";
pub const CACHE_PATH: &str = "/data/cache/theme-garden";
pub const STAGING_PATH: &str = "/data/staging/themes";
const CATALOG_TARGET: &str = "catalog/theme-catalog.json";
const NOW: &str = "2030-01-01T00:00:00Z";
const ARTBOOK: &str = "artbook";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    #[serde(rename = "catalogVersion")]
    pub catalog_version: String,
    pub expires: String,
    pub themes: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    pub author: String,
    pub license: String,
    #[serde(rename = "provenanceUrl")]
    pub provenance_url: String,
    pub versions: Vec<ThemeVersion>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeVersion {
    pub version: String,
    #[serde(rename = "themeApiCompatibility")]
    pub theme_api_compatibility: String,
    #[serde(rename = "themeApiVersion")]
    pub theme_api_version: String,
    #[serde(rename = "targetSku")]
    pub target_sku: String,
    #[serde(rename = "targetPath")]
    pub target_path: String,
    pub package: PackageDigest,
    pub screenshots: ScreenshotMetadata,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDigest {
    pub sha256: String,
    pub length: u64,
    #[serde(rename = "compressedBytes")]
    pub compressed_bytes: u64,
    #[serde(rename = "expandedBytes")]
    pub expanded_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotMetadata {
    pub available: bool,
    pub count: u8,
    #[serde(rename = "maxBytes")]
    pub max_bytes: u64,
    #[serde(rename = "cacheKey")]
    pub cache_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRecord {
    #[serde(rename = "catalogVersion")]
    pub catalog_version: String,
    pub expires: String,
    #[serde(rename = "targetPath")]
    pub target_path: String,
    #[serde(rename = "targetLength")]
    pub target_length: u64,
    #[serde(rename = "targetSha256")]
    pub target_sha256: String,
    #[serde(rename = "delegatedRole")]
    pub delegated_role: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveTheme {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub id: String,
    pub version: String,
    pub theme_api_compatibility: String,
    pub theme_api_version: String,
    pub target_sku: String,
    pub download_size: u64,
    pub expanded_size: u64,
    pub sha256: String,
    pub license: String,
    pub author: String,
    pub provenance: String,
    pub screenshots_available: bool,
    pub cache_state: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub id: String,
    pub from: String,
    pub to: String,
    pub target_path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowRecord {
    pub controller: String,
    pub entries: usize,
    pub active_theme: String,
    pub cache_state: String,
}

pub struct ThemeGarden {
    root: PathBuf,
    fixtures: PathBuf,
    catalog: Catalog,
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let catalog: Self = serde_json::from_slice(bytes).context("parse theme catalog")?;
        if catalog.schema != "urn:project:theme-catalog-v1"
            || catalog.format != "theme-catalog-v1"
            || catalog.schema_version != 1
            || catalog.target_sku != TARGET_SKU
            || catalog.themes.len() != 3
            || catalog.expires.len() != 20
            || !catalog.expires.ends_with('Z')
            || !version(&catalog.catalog_version)
        {
            bail!("unsupported theme catalog")
        }
        let mut ids: Vec<&str> = Vec::new();
        for entry in &catalog.themes {
            validate_entry(entry)?;
            if ids.iter().any(|id| *id == entry.id) {
                bail!("catalog theme identity is duplicated")
            }
            ids.push(entry.id.as_str());
        }
        Ok(catalog)
    }

    pub fn entry(&self, id: &str) -> Result<&CatalogEntry> {
        self.themes
            .iter()
            .find(|entry| entry.id == id)
            .with_context(|| format!("theme is not in catalog: {id}"))
    }
}

impl ThemeGarden {
    pub fn controller_flow(&self) -> Result<FlowRecord> {
        Ok(FlowRecord {
            controller: "controller-first".into(),
            entries: self.browse().len(),
            active_theme: self.active()?.id,
            cache_state: "verified".into(),
        })
    }

    pub fn authenticate(root: &Path, fixtures: &Path) -> Result<Self> {
        let (catalog, report, target) = verify_catalog_target(root, fixtures, None)?;
        let cache = root.join(CACHE_PATH.trim_start_matches('/'));
        fs::create_dir_all(&cache)?;
        let cache_record = CacheRecord {
            catalog_version: catalog.catalog_version.clone(),
            expires: catalog.expires.clone(),
            target_path: report.target.path.clone(),
            target_length: report.target.length,
            target_sha256: report.target.sha256.clone(),
            delegated_role: report.target.delegated_role.clone(),
        };
        atomic_bytes(&cache.join("catalog.json"), &target)?;
        atomic_json(&cache.join("metadata.json"), &cache_record)?;
        Ok(Self {
            root: root.to_path_buf(),
            fixtures: fixtures.to_path_buf(),
            catalog,
        })
    }

    pub fn from_cache(root: &Path, fixtures: &Path) -> Result<Self> {
        let cache = root.join(CACHE_PATH.trim_start_matches('/'));
        let target = fs::read(cache.join("catalog.json"))?;
        let metadata: CacheRecord =
            serde_json::from_slice(&fs::read(cache.join("metadata.json"))?)?;
        let (catalog, report, _) = verify_catalog_target(root, fixtures, Some(&target))?;
        if metadata.catalog_version != catalog.catalog_version
            || metadata.expires != catalog.expires
            || metadata.target_path != report.target.path
            || metadata.target_length != report.target.length
            || metadata.target_sha256 != report.target.sha256
            || metadata.delegated_role != report.target.delegated_role
        {
            bail!("offline cache metadata is not the authenticated target")
        }
        Ok(Self {
            root: root.to_path_buf(),
            fixtures: fixtures.to_path_buf(),
            catalog,
        })
    }

    pub fn browse(&self) -> Vec<&CatalogEntry> {
        self.catalog.themes.iter().collect()
    }

    pub fn select_themes_json(
        &self,
        bytes: &[u8],
        id: &str,
        version: &str,
        fixture_root: &Path,
    ) -> Result<launcher_theme::ValidatedTheme> {
        let catalog =
            ThemesCatalog::parse(bytes).map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let entry = catalog
            .select(id, version)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if let Some(fixture) = entry.locator.strip_prefix("fixture:") {
            let theme_path = fixture_root.join(fixture);
            return load_theme_dir(&theme_path).map_err(|error| anyhow::anyhow!(error.to_string()));
        }
        ThemesCatalog::load_theme(&catalog, id, version, &DirectCatalogTransport)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    pub fn fetch_themes_json<T: launcher_theme::CatalogTransport>(
        &self,
        bytes: &[u8],
        id: &str,
        version: &str,
        transport: &T,
    ) -> Result<Vec<u8>> {
        let catalog =
            ThemesCatalog::parse(bytes).map_err(|error| anyhow::anyhow!(error.to_string()))?;
        catalog
            .fetch(id, version, transport)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    pub fn details(&self, id: &str) -> Result<Detail> {
        let entry = self.catalog.entry(id)?;
        let record = latest(&entry.versions)?;
        Ok(Detail {
            id: entry.id.clone(),
            version: record.version.clone(),
            theme_api_compatibility: record.theme_api_compatibility.clone(),
            theme_api_version: record.theme_api_version.clone(),
            target_sku: record.target_sku.clone(),
            download_size: record.package.compressed_bytes,
            expanded_size: record.package.expanded_bytes,
            sha256: record.package.sha256.clone(),
            license: entry.license.clone(),
            author: entry.author.clone(),
            provenance: entry.provenance_url.clone(),
            screenshots_available: record.screenshots.available,
            cache_state: "verified".into(),
        })
    }

    pub fn preview(&self, id: &str) -> Result<PathBuf> {
        let entry = self.catalog.entry(id)?;
        let source = self.fixtures.join("themes").join(&entry.id);
        let preview = launcher_theme::preview_path_or_fallback(&source)?;
        if preview.fallback_reason.is_some() {
            bail!("candidate preview fell back")
        }
        let record = latest(&entry.versions)?;
        let output = self
            .root
            .join(CACHE_PATH.trim_start_matches('/'))
            .join("previews")
            .join(format!("{}-{}.png", entry.id, record.version));
        if output.exists() {
            return Ok(output);
        }
        fs::create_dir_all(output.parent().context("preview path has no parent")?)?;
        render_png(&preview.theme, &output).map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(output)
    }

    pub fn installed(&self) -> Result<Vec<ActiveTheme>> {
        let mut installed = vec![ActiveTheme {
            id: ARTBOOK.into(),
            version: "1.0.0".into(),
        }];
        let root = self.root.join(".brickpro/packages");
        if root.is_dir() {
            for theme in fs::read_dir(root)? {
                let theme = theme?;
                if !theme.file_type()?.is_dir() || theme.file_name() == ".staging" {
                    continue;
                }
                for version_dir in fs::read_dir(theme.path())? {
                    let version_dir = version_dir?;
                    if version_dir.file_type()?.is_dir() {
                        installed.push(ActiveTheme {
                            id: theme.file_name().to_string_lossy().into_owned(),
                            version: version_dir.file_name().to_string_lossy().into_owned(),
                        });
                    }
                }
            }
        }
        installed.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then(left.version.cmp(&right.version))
        });
        Ok(installed)
    }

    pub fn updates(&self) -> Result<Vec<Update>> {
        let installed = self.installed()?;
        let mut updates = Vec::new();
        for item in installed {
            let Ok(entry) = self.catalog.entry(&item.id) else {
                continue;
            };
            let Some(record) = entry
                .versions
                .iter()
                .filter(|record| greater_version(&record.version, &item.version))
                .max_by(|left, right| compare_version(&left.version, &right.version))
            else {
                continue;
            };
            updates.push(Update {
                id: item.id,
                from: item.version,
                to: record.version.clone(),
                target_path: record.target_path.clone(),
                sha256: record.package.sha256.clone(),
            });
        }
        Ok(updates)
    }

    pub fn active(&self) -> Result<ActiveTheme> {
        let path = self.active_path();
        if !path.exists() {
            let active = default_theme();
            atomic_json(&path, &active)?;
            return Ok(active);
        }
        let active: ActiveTheme = serde_json::from_slice(&fs::read(path)?)?;
        if active.id == ARTBOOK {
            return Ok(active);
        }
        let compatible = self
            .catalog
            .entry(&active.id)
            .ok()
            .and_then(|entry| {
                entry
                    .versions
                    .iter()
                    .find(|record| record.version == active.version)
            })
            .is_some_and(compatible_version);
        let theme = self.installed_theme_path(&active);
        if !compatible || load_theme_dir(&theme).is_err() {
            let fallback = default_theme();
            atomic_json(&self.active_path(), &fallback)?;
            return Ok(fallback);
        }
        Ok(active)
    }

    pub fn install(
        &self,
        id: &str,
        interrupt_after: Option<usize>,
        fail_preview: bool,
    ) -> Result<ActiveTheme> {
        let entry = self.catalog.entry(id)?;
        let installed = self.installed()?;
        let record = entry
            .versions
            .iter()
            .find(|record| {
                !installed
                    .iter()
                    .any(|item| item.id == id && item.version == record.version)
            })
            .with_context(|| format!("no uninstalled version for {id}"))?;
        self.install_version(id, &record.version, interrupt_after, fail_preview)
    }

    pub fn install_version(
        &self,
        id: &str,
        requested_version: &str,
        interrupt_after: Option<usize>,
        fail_preview: bool,
    ) -> Result<ActiveTheme> {
        if self.cache_record()?.expires.as_str() <= NOW {
            bail!("expired offline catalog denies install")
        }
        let entry = self.catalog.entry(id)?;
        let record = entry
            .versions
            .iter()
            .find(|record| record.version == requested_version)
            .with_context(|| format!("catalog has no {id} version {requested_version}"))?;
        if !compatible_version(record) {
            bail!("theme is incompatible with TG4040")
        }
        let manifest = fs::read(self.manifest_fixture_path(id, record)?)?;
        let manifest_path = self.acquire(id, &record.version, &manifest, interrupt_after)?;
        let target = self.verify_package_target(entry, record, &manifest)?;
        let payload = self.payload_path(id, record);
        let root = self.root.clone();
        let cache_root = root.join(CACHE_PATH.trim_start_matches('/'));
        let id_owned = entry.id.clone();
        let version_owned = record.version.clone();
        let result = install_with_validation(
            &root,
            &manifest_path,
            &payload,
            &target,
            TrustContext::community_signed(),
            TransactionOptions::default(),
            move |manifest, staging| {
                if manifest.id != id_owned || manifest.version != version_owned || fail_preview {
                    bail!("candidate preview validation failed")
                }
                let validated = load_theme_dir(&staging.join("immutable"))
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                let output = cache_root
                    .join("previews")
                    .join(format!("{}-{}.png", manifest.id, manifest.version));
                fs::create_dir_all(output.parent().context("preview path has no parent")?)?;
                render_png(&validated, &output)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                Ok(())
            },
        );
        let _ = fs::remove_file(&manifest_path);
        let _ = fs::remove_file(manifest_path.with_extension("validator"));
        result?;
        let active = ActiveTheme {
            id: entry.id.clone(),
            version: record.version.clone(),
        };
        atomic_json(&self.active_path(), &active)?;
        Ok(active)
    }

    pub fn update(&self, id: &str, fail_preview: bool) -> Result<ActiveTheme> {
        let active = self.active()?;
        if active.id != id {
            bail!("theme is not the active update target")
        }
        let update = self
            .updates()?
            .into_iter()
            .find(|update| update.id == id)
            .with_context(|| format!("no catalog update for {id}"))?;
        self.install_version(id, &update.to, None, fail_preview)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        if id == ARTBOOK {
            bail!("Artbook is built-in and cannot be removed")
        }
        let active = self.active()?;
        if active.id == id {
            atomic_json(&self.active_path(), &default_theme())?;
        }
        uninstall(&self.root, id, TransactionOptions::default()).context("remove theme")?;
        let _ = fs::remove_dir_all(self.root.join(".brickpro/packages").join(id));
        Ok(())
    }

    pub fn expire_cache(&self) -> Result<()> {
        let metadata_path = self
            .root
            .join(CACHE_PATH.trim_start_matches('/'))
            .join("metadata.json");
        let mut metadata: CacheRecord = serde_json::from_slice(&fs::read(&metadata_path)?)?;
        metadata.expires = "2029-01-01T00:00:00Z".into();
        atomic_json(&metadata_path, &metadata)?;
        let catalog_path = self
            .root
            .join(CACHE_PATH.trim_start_matches('/'))
            .join("catalog.json");
        let mut catalog: Catalog = serde_json::from_slice(&fs::read(&catalog_path)?)?;
        catalog.expires = metadata.expires;
        atomic_bytes(&catalog_path, &serde_json::to_vec_pretty(&catalog)?)
    }

    fn cache_record(&self) -> Result<CacheRecord> {
        Ok(serde_json::from_slice(&fs::read(
            self.root
                .join(CACHE_PATH.trim_start_matches('/'))
                .join("metadata.json"),
        )?)?)
    }

    fn verify_package_target(
        &self,
        entry: &CatalogEntry,
        record: &ThemeVersion,
        manifest: &[u8],
    ) -> Result<VerifiedTarget> {
        let target = self.verify_signed_target(&record.target_path, manifest)?;
        if target.length != record.package.length || target.sha256 != record.package.sha256 {
            bail!("catalog package pin differs from delegated target")
        }
        if !record
            .target_path
            .starts_with(&format!("themes/{}/", entry.id))
        {
            bail!("catalog target identity differs from theme id")
        }
        Ok(target)
    }

    fn verify_signed_target(
        &self,
        target_path: &str,
        target_bytes: &[u8],
    ) -> Result<VerifiedTarget> {
        let repository = self.fixtures.join("repository");
        let state = self.root.join(".brickpro/theme-garden/trust-state.json");
        Ok(TrustStore::new(&state)
            .verify_repository(
                RepositoryMetadata {
                    root_bytes: &fs::read(repository.join("root.json"))?,
                    root_updates: &[],
                    timestamp_bytes: &fs::read(repository.join("timestamp.json"))?,
                    snapshot_bytes: &fs::read(repository.join("snapshot.json"))?,
                    targets_bytes: &fs::read(repository.join("targets.json"))?,
                    delegated_role: "themes",
                    delegated_bytes: &fs::read(repository.join("themes.json"))?,
                    target_bytes,
                },
                target_path,
                VerificationTime {
                    now_rfc3339: NOW,
                    uncertainty_seconds: 0,
                },
            )?
            .target)
    }

    fn manifest_fixture_path(&self, id: &str, record: &ThemeVersion) -> Result<PathBuf> {
        let filename = Path::new(&record.target_path)
            .file_name()
            .context("catalog target has no filename")?;
        Ok(self
            .fixtures
            .join("repository")
            .join(format!("{}-{}", id, filename.to_string_lossy())))
    }

    fn payload_path(&self, id: &str, record: &ThemeVersion) -> PathBuf {
        let suffix = if record.version == "1.0.0" {
            String::new()
        } else {
            format!("-{}", record.version)
        };
        self.fixtures.join("packages").join(format!("{id}{suffix}"))
    }

    fn acquire(
        &self,
        id: &str,
        version: &str,
        bytes: &[u8],
        interrupt_after: Option<usize>,
    ) -> Result<PathBuf> {
        let directory = self
            .root
            .join(STAGING_PATH.trim_start_matches('/'))
            .join(id);
        fs::create_dir_all(&directory)?;
        let partial = directory.join(format!("{version}.partial"));
        let validator = partial.with_extension("validator");
        let expected_validator = format!("fixture-validator-{id}-{version}");
        if fs::read_to_string(&validator).ok().as_deref() != Some(&expected_validator) {
            let _ = fs::remove_file(&partial);
            fs::write(&validator, &expected_validator)?;
        }
        let mut existing = fs::read(&partial).unwrap_or_default();
        if existing.len() > bytes.len() || !bytes.starts_with(&existing) {
            existing.clear();
            fs::write(&partial, [])?;
        }
        if let Some(limit) = interrupt_after {
            if existing.len() < bytes.len() {
                let end = (existing.len() + limit).min(bytes.len());
                existing.extend_from_slice(&bytes[existing.len()..end]);
                fs::write(&partial, &existing)?;
                bail!("simulated interrupted download")
            }
        }
        existing.extend_from_slice(&bytes[existing.len()..]);
        fs::write(&partial, &existing)?;
        Ok(partial)
    }

    fn active_path(&self) -> PathBuf {
        self.root.join(".brickpro/theme-garden/active.json")
    }

    fn installed_theme_path(&self, active: &ActiveTheme) -> PathBuf {
        self.root
            .join(".brickpro/packages")
            .join(&active.id)
            .join(&active.version)
            .join("immutable")
    }
}

fn verify_catalog_target(
    root: &Path,
    fixtures: &Path,
    cached_target: Option<&[u8]>,
) -> Result<(Catalog, package_trust::VerificationReport, Vec<u8>)> {
    let repository = fixtures.join("repository");
    let target = cached_target
        .map(ToOwned::to_owned)
        .unwrap_or(fs::read(repository.join("catalog.json"))?);
    let state = root.join(".brickpro/theme-garden/trust-state.json");
    let report = TrustStore::new(&state).verify_repository(
        RepositoryMetadata {
            root_bytes: &fs::read(repository.join("root.json"))?,
            root_updates: &[],
            timestamp_bytes: &fs::read(repository.join("timestamp.json"))?,
            snapshot_bytes: &fs::read(repository.join("snapshot.json"))?,
            targets_bytes: &fs::read(repository.join("targets.json"))?,
            delegated_role: "themes",
            delegated_bytes: &fs::read(repository.join("themes.json"))?,
            target_bytes: &target,
        },
        CATALOG_TARGET,
        VerificationTime {
            now_rfc3339: NOW,
            uncertainty_seconds: 0,
        },
    )?;
    let catalog = Catalog::parse(&target)?;
    Ok((catalog, report, target))
}

fn validate_entry(entry: &CatalogEntry) -> Result<()> {
    if !identifier(&entry.id)
        || entry.author != "Project Authors"
        || entry.license != "MIT"
        || !entry.provenance_url.starts_with("https://example.invalid/")
        || entry.versions.is_empty()
        || entry.versions.len() > 2
    {
        bail!("invalid theme catalog entry")
    }
    let mut versions = Vec::new();
    for record in &entry.versions {
        if !version(&record.version)
            || record.theme_api_version != THEME_API
            || record.theme_api_compatibility != ">=1.0.0 <2.0.0"
            || record.target_sku != TARGET_SKU
            || record.target_path != expected_target(&entry.id, &record.version)
            || record.package.sha256.len() != 64
            || !record
                .package
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || record.package.length == 0
            || record.package.compressed_bytes < record.package.length
            || record.package.expanded_bytes > 1_048_576
            || !record.screenshots.available
            || record.screenshots.count != 1
            || record.screenshots.max_bytes > 65_536
            || record.screenshots.cache_key != format!("{}-{}", entry.id, record.version)
            || versions.iter().any(|item| item == &record.version)
        {
            bail!("invalid theme catalog version")
        }
        versions.push(record.version.clone());
    }
    Ok(())
}

fn expected_target(id: &str, version: &str) -> String {
    if version == "1.0.0" {
        format!("themes/{id}/manifest.json")
    } else {
        format!("themes/{id}/manifest-{version}.json")
    }
}

fn compatible_version(record: &ThemeVersion) -> bool {
    record.target_sku == TARGET_SKU
        && record.theme_api_version == THEME_API
        && record.theme_api_compatibility == ">=1.0.0 <2.0.0"
}

fn latest(versions: &[ThemeVersion]) -> Result<&ThemeVersion> {
    versions
        .iter()
        .max_by(|left, right| compare_version(&left.version, &right.version))
        .context("catalog has no versions")
}

fn compare_version(left: &str, right: &str) -> std::cmp::Ordering {
    version_parts(left).cmp(&version_parts(right))
}

fn greater_version(left: &str, right: &str) -> bool {
    compare_version(left, right).is_gt()
}

fn version_parts(value: &str) -> (u64, u64, u64) {
    let mut parts = value.split('.').map(|part| part.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn version(value: &str) -> bool {
    value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn default_theme() -> ActiveTheme {
    ActiveTheme {
        id: ARTBOOK.into(),
        version: "1.0.0".into(),
    }
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes)?;
    fs::rename(temp, path)?;
    Ok(())
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_bytes(path, &serde_json::to_vec_pretty(value)?)
}
