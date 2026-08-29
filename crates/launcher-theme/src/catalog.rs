use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Reason, ThemeError};

pub const THEMES_CATALOG_FORMAT: &str = "themes-catalog-v1";
pub const MAX_THEME_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;

pub trait CatalogTransport {
    fn fetch(&self, locator: &str, max_bytes: usize) -> Result<Vec<u8>, ThemeError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectCatalogTransport;

impl CatalogTransport for DirectCatalogTransport {
    fn fetch(&self, locator: &str, max_bytes: usize) -> Result<Vec<u8>, ThemeError> {
        if !safe_locator(locator) || locator.starts_with("fixture:") {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "catalog URL is not safe",
            ));
        }
        let output = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--tlsv1.2",
                "--connect-timeout",
                "10",
                "--max-time",
                "30",
                "--max-filesize",
                &max_bytes.to_string(),
                locator,
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| ThemeError::new(Reason::Io, format!("curl unavailable: {error}")))?;
        if !output.status.success() {
            return Err(ThemeError::new(
                Reason::Io,
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        if output.stdout.len() > max_bytes {
            return Err(ThemeError::new(
                Reason::BudgetAsset,
                "download exceeds configured byte budget",
            ));
        }
        Ok(output.stdout)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemesCatalog {
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub themes: Vec<ThemesCatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemesCatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub locator: String,
    pub author: String,
    #[serde(default)]
    pub screenshot: Option<String>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(default, rename = "sizeMb")]
    pub size_mb: Option<u64>,
    #[serde(default, rename = "upstreamStatus")]
    pub upstream_status: Option<u8>,
    #[serde(default, rename = "aspectRatios")]
    pub aspect_ratios: Vec<String>,
    #[serde(default, rename = "knulliCompatible")]
    pub knulli_compatible: Option<bool>,
}

impl ThemesCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, ThemeError> {
        if bytes.len() > super::MAX_JSON_BYTES {
            return Err(ThemeError::new(
                Reason::BudgetJsonSize,
                "themes.json exceeds 131072 bytes",
            ));
        }
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| ThemeError::new(Reason::MalformedJson, error.to_string()))?;
        let catalog = if value.get("data").is_some() {
            let feed: BatoceraFeed = serde_json::from_value(value)
                .map_err(|error| ThemeError::new(Reason::UnknownField, error.to_string()))?;
            Self {
                format: THEMES_CATALOG_FORMAT.into(),
                schema_version: 1,
                themes: feed
                    .data
                    .into_iter()
                    .map(ThemesCatalogEntry::from_feed)
                    .collect::<Result<Vec<_>, _>>()?,
            }
        } else {
            serde_json::from_value(value)
                .map_err(|error| ThemeError::new(Reason::UnknownField, error.to_string()))?
        };
        if catalog.format != THEMES_CATALOG_FORMAT
            || catalog.schema_version != 1
            || catalog.themes.is_empty()
            || catalog.themes.len() > 512
        {
            return Err(ThemeError::new(
                Reason::InvalidSchema,
                "unsupported themes.json catalog",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for entry in &catalog.themes {
            validate_entry(entry)?;
            if !ids.insert(&entry.id) {
                return Err(ThemeError::new(
                    Reason::InvalidSchema,
                    "themes.json contains duplicate ids",
                ));
            }
        }
        Ok(catalog)
    }

    pub fn select(&self, id: &str, version: &str) -> Result<&ThemesCatalogEntry, ThemeError> {
        self.themes
            .iter()
            .find(|entry| entry.id == id && entry.version == version)
            .ok_or_else(|| {
                ThemeError::new(Reason::InvalidSchema, "theme selection is not catalogued")
            })
    }

    pub fn fetch<T: CatalogTransport>(
        &self,
        id: &str,
        version: &str,
        transport: &T,
    ) -> Result<Vec<u8>, ThemeError> {
        let entry = self.select(id, version)?;
        if entry.locator.starts_with("fixture:") {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "fixture locators are not downloadable",
            ));
        }
        let bytes = transport.fetch(&entry.locator, MAX_THEME_DOWNLOAD_BYTES)?;
        if bytes.len() > MAX_THEME_DOWNLOAD_BYTES {
            return Err(ThemeError::new(
                Reason::BudgetAsset,
                "theme download exceeds 32 MiB",
            ));
        }
        Ok(bytes)
    }

    pub fn load_theme<T: CatalogTransport>(
        &self,
        id: &str,
        version: &str,
        transport: &T,
    ) -> Result<super::ValidatedTheme, ThemeError> {
        let entry = self.select(id, version)?;
        if entry.locator.starts_with("fixture:") {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "fixture locators require a local fixture root",
            ));
        }
        let theme_url = package_file_url(&entry.locator, "theme.json")?;
        let mut theme = super::parse_json(&transport.fetch(&theme_url, super::MAX_JSON_BYTES)?)?;
        for spec in super::declared_assets(theme.theme()) {
            let url = package_file_url(&entry.locator, &spec.path)?;
            let bytes = transport.fetch(&url, spec.max_bytes as usize)?;
            if bytes.len() > spec.max_bytes as usize {
                return Err(ThemeError::new(
                    Reason::BudgetAsset,
                    format!("asset {} exceeds declared limit", spec.path),
                ));
            }
            theme.assets.push(super::decode_asset(&spec.path, &bytes)?);
        }
        Ok(theme)
    }
}

fn package_file_url(base: &str, path: &str) -> Result<String, ThemeError> {
    super::validate_asset_path(path)?;
    if let Some(repository) = base.strip_prefix("https://github.com/") {
        let mut parts = repository.trim_end_matches('/').split('/');
        let owner = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if !identifier(owner) || !identifier(name) || parts.next().is_some() {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "unsupported GitHub locator",
            ));
        }
        return Ok(format!(
            "https://raw.githubusercontent.com/{owner}/{name}/main/{path}"
        ));
    }
    if let Some(parent) = base.strip_suffix("theme.json") {
        return Ok(format!("{parent}{path}"));
    }
    Ok(format!("{}/{}", base.trim_end_matches('/'), path))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatoceraFeed {
    data: Vec<BatoceraEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatoceraEntry {
    theme: String,
    author: String,
    theme_url: String,
    #[serde(rename = "last_update")]
    last_update: String,
    #[serde(rename = "up_to_date")]
    up_to_date: String,
    size: String,
    screenshot: String,
}

impl ThemesCatalogEntry {
    fn from_feed(entry: BatoceraEntry) -> Result<Self, ThemeError> {
        if !calendar_date(&entry.last_update) {
            return Err(ThemeError::new(
                Reason::InvalidSchema,
                "Batocera last_update must be YYYY-MM-DD",
            ));
        }
        let size_mb = entry
            .size
            .parse::<u64>()
            .map_err(|_| ThemeError::new(Reason::InvalidSchema, "Batocera size must be numeric"))?;
        let upstream_status = entry.up_to_date.parse::<u8>().map_err(|_| {
            ThemeError::new(Reason::InvalidSchema, "Batocera up_to_date must be numeric")
        })?;
        let screenshot = batocera_screenshot_url(&entry.screenshot)?;
        Ok(Self {
            id: entry.theme.clone(),
            name: entry.theme,
            version: "1.0.0".into(),
            locator: entry.theme_url,
            author: entry.author,
            screenshot: Some(screenshot),
            updated_at: Some(entry.last_update),
            size_mb: Some(size_mb),
            upstream_status: Some(upstream_status),
            aspect_ratios: Vec::new(),
            knulli_compatible: None,
        })
    }
}

fn validate_entry(entry: &ThemesCatalogEntry) -> Result<(), ThemeError> {
    if !identifier(&entry.id)
        || entry.name.is_empty()
        || entry.name.len() > 64
        || !version(&entry.version)
        || entry.author.is_empty()
        || entry.author.len() > 64
        || !safe_locator(&entry.locator)
    {
        return Err(ThemeError::new(
            Reason::InvalidSchema,
            format!("invalid themes.json entry {}", entry.id),
        ));
    }
    if entry.screenshot.as_ref().is_some_and(|path| {
        if path.starts_with("https://") {
            !safe_locator(path)
        } else {
            !safe_catalog_path(path)
        }
    }) {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "unsafe catalog screenshot path",
        ));
    }
    Ok(())
}

fn batocera_screenshot_url(path: &str) -> Result<String, ThemeError> {
    if !safe_catalog_path(path) || !path.starts_with("themes/") {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "unsafe Batocera screenshot path",
        ));
    }
    Ok(format!("https://batocera.org/upgrades/{path}"))
}

fn calendar_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
        && value[5..7]
            .parse::<u8>()
            .is_ok_and(|month| (1..=12).contains(&month))
        && value[8..10]
            .parse::<u8>()
            .is_ok_and(|day| (1..=31).contains(&day))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn version(value: &str) -> bool {
    value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn safe_catalog_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':'])
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn safe_locator(value: &str) -> bool {
    if let Some(id) = value.strip_prefix("fixture:") {
        return identifier(id);
    }
    if !value.starts_with("https://") || !value.is_ascii() || value.contains(['\\', '@', '?', '#'])
    {
        return false;
    }
    let authority = value[8..].split('/').next().unwrap_or_default();
    !authority.is_empty()
        && authority.len() <= 253
        && authority.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}
