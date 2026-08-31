use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::{BufWriter, Read},
    path::{Component as PathComponent, Path, PathBuf},
};

use png::{BitDepth, ColorType, Encoder};
use serde::{
    de::{DeserializeSeed, Error as DeError},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{de::Deserializer as JsonDeserializer, Value};
pub mod catalog;
pub mod importer;
pub use catalog::{
    CatalogTransport, DirectCatalogTransport, ThemesCatalog, ThemesCatalogEntry,
    MAX_THEME_DOWNLOAD_BYTES, THEMES_CATALOG_FORMAT,
};
pub use importer::{
    import_emulationstation_theme, import_es_theme_dir, CompatibilityReport, ImportedTheme,
};

#[cfg(test)]
mod tests;

pub const MAX_CANVAS_PIXELS: u64 = 4 * 1024 * 1024;
pub const MAX_JSON_BYTES: usize = 128 * 1024;
pub const MAX_FILES: usize = 32;
pub const MAX_THEME_DEPTH: usize = 8;
pub const MAX_RESOURCE_BYTES: u64 = 64 * 1024;
pub const MAX_RENDER_BYTES: u64 = MAX_CANVAS_PIXELS * 4;
const MAX_ART_BOOK_NEXT_FILES: usize = 2048;
const MAX_ART_BOOK_NEXT_XML_BYTES: u64 = MAX_JSON_BYTES as u64;
const MAX_ART_BOOK_NEXT_ASSET_BYTES: u64 = MAX_RESOURCE_BYTES * 64;
pub const SCHEMA: &str = "theme-v1";
pub const SCHEMA_URI: &str = "urn:project:theme-v1";
pub const MIN_LABEL_CONTRAST: f64 = 4.5;

pub fn select_theme_layout<'a>(
    device: &device_profile::DeviceProfile,
    available_layouts: &'a [&str],
) -> Result<&'a str, ThemeError> {
    let selected = device.theme_layout_file();
    available_layouts
        .iter()
        .copied()
        .find(|layout| *layout == selected)
        .ok_or_else(|| {
            ThemeError::new(
                Reason::InvalidLayout,
                format!(
                    "theme has no layout for device aspect {}",
                    device.theme_aspect()
                ),
            )
        })
}

pub fn validate_for_device(
    theme: ValidatedTheme,
    device: &device_profile::DeviceProfile,
) -> Result<ValidatedTheme, ThemeError> {
    let (width, height) = device.logical_size();
    if theme.theme.canvas.width != width
        || theme.theme.canvas.height != height
        || theme.theme.canvas.aspect != device.theme_aspect()
    {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            "theme canvas does not match selected device profile",
        ));
    }
    Ok(theme)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    MissingTheme,
    Io,
    MalformedJson,
    DuplicateJsonKey,
    UnknownField,
    InvalidSchema,
    InvalidPath,
    Symlink,
    UnsupportedFile,
    BudgetJsonSize,
    BudgetFileCount,
    BudgetResourceBytes,
    BudgetText,
    InvalidColor,
    InvalidSetting,
    InvalidLayout,
    UnsupportedResource,
    InvalidXml,
    UnsupportedXml,
    InvalidAsset,
    BudgetAsset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemeError {
    pub reason: Reason,
    pub message: String,
}

impl ThemeError {
    fn new(reason: Reason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }

    pub fn json(&self) -> Value {
        serde_json::json!({"ok": false, "reason": self.reason, "message": self.message})
    }
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {}",
            serde_json::to_string(&self.reason).unwrap_or_default(),
            self.message
        )
    }
}

impl std::error::Error for ThemeError {}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub format: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub metadata: Metadata,
    pub canvas: Canvas,
    pub colors: Colors,
    pub resources: Resources,
    pub layout: Layout,
    pub settings: Settings,
    pub fallback: Fallback,
    #[serde(default)]
    pub typography: Option<Typography>,
    #[serde(default)]
    pub assets: Option<Assets>,
    #[serde(default)]
    pub components: Option<Vec<Component>>,
    #[serde(rename = "upstreamContract", default)]
    pub upstream_contract: Option<UpstreamContract>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    pub author: String,
    pub license: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Typography {
    pub family: String,
    #[serde(rename = "titleSize")]
    pub title_size: u16,
    #[serde(rename = "bodySize")]
    pub body_size: u16,
    #[serde(rename = "smallSize")]
    pub small_size: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSpec {
    pub path: String,
    #[serde(rename = "maxBytes")]
    pub max_bytes: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    pub background: Option<AssetSpec>,
    #[serde(rename = "systemArt")]
    pub system_art: Option<AssetSpec>,
    #[serde(rename = "boxArt")]
    pub box_art: Option<AssetSpec>,
    pub screenshot: Option<AssetSpec>,
    pub controller: Option<AssetSpec>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    Image,
    Text,
    Textlist,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaBinding {
    SystemArtwork,
    SystemLogo,
    GameImage,
    GameVideo,
    GameMarquee,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: String,
    pub kind: ComponentKind,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub path: Option<String>,
    pub text: Option<String>,
    pub color: Option<String>,
    #[serde(rename = "fontSize")]
    pub font_size: Option<u16>,
    #[serde(rename = "mediaBinding", default)]
    pub media_binding: Option<MediaBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamContract {
    pub source: String,
    pub revision: String,
    pub variant: String,
    #[serde(rename = "systemArtworkPath")]
    pub system_artwork_path: String,
    #[serde(rename = "systemLogoPath")]
    pub system_logo_path: String,
    pub fonts: Vec<UpstreamAsset>,
    #[serde(rename = "systemArtwork")]
    pub system_artwork: Vec<UpstreamAsset>,
    #[serde(rename = "systemLogos")]
    pub system_logos: Vec<UpstreamAsset>,
    #[serde(rename = "menuAssets")]
    pub menu_assets: Vec<UpstreamAsset>,
    pub options: UpstreamOptions,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamAsset {
    pub path: String,
    pub kind: UpstreamAssetKind,
    #[serde(rename = "maxBytes")]
    pub max_bytes: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpstreamAssetKind {
    Font,
    Image,
    Svg,
    Sound,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamOptions {
    #[serde(rename = "systemArtwork")]
    pub system_artwork: String,
    #[serde(rename = "systemLogos")]
    pub system_logos: String,
    #[serde(rename = "gameArtwork")]
    pub game_artwork: String,
    #[serde(rename = "gameMetadata")]
    pub game_metadata: String,
    #[serde(rename = "fontSize")]
    pub font_size: String,
    #[serde(rename = "colorScheme")]
    pub color_scheme: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: u16,
    pub height: u16,
    pub aspect: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    pub background: String,
    pub surface: String,
    pub accent: String,
    pub text: String,
    pub muted: String,
    pub highlight: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    pub font: Resource,
    pub icon: Resource,
    pub background: Resource,
    pub sound: Resource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub kind: ResourceKind,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(rename = "budgetBytes")]
    pub budget_bytes: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    Generated,
    Builtin,
    ThemeAsset,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub preset: LayoutPreset,
    pub regions: Vec<Region>,
    #[serde(rename = "maxVisibleGames")]
    pub max_visible_games: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutPreset {
    Artbook,
    Contrast,
    Minimal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub id: String,
    pub kind: RegionKind,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegionKind {
    SystemArt,
    GameList,
    BoxArtPlaceholder,
    ScreenshotPlaceholder,
    Metadata,
    Menu,
    HelpStrip,
    Clock,
    Battery,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(rename = "artworkMode")]
    pub artwork_mode: ArtworkMode,
    #[serde(rename = "metadataVisibility")]
    pub metadata_visibility: MetadataVisibility,
    #[serde(rename = "fontScale")]
    pub font_scale: u8,
    #[serde(rename = "colorScheme")]
    pub color_scheme: ColorScheme,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtworkMode {
    SystemArt,
    BoxArt,
    Screenshot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetadataVisibility {
    Full,
    Compact,
    Hidden,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorScheme {
    Dark,
    HighContrast,
    Minimal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fallback {
    pub splash: Splash,
    #[serde(rename = "onInvalid")]
    pub on_invalid: OnInvalid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Splash {
    GeneratedNeutral,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnInvalid {
    SafeArtbook,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidatedTheme {
    theme: Theme,
    #[serde(skip)]
    assets: Vec<LoadedAsset>,
    #[serde(skip)]
    source_assets: Vec<LoadedSourceAsset>,
}

#[derive(Clone, Debug)]
struct LoadedAsset {
    path: String,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

#[derive(Clone, Debug)]
struct LoadedSourceAsset {
    path: String,
    kind: UpstreamAssetKind,
    bytes: Vec<u8>,
}

impl ValidatedTheme {
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn name(&self) -> &str {
        &self.theme.metadata.name
    }

    pub fn asset(&self, path: &str) -> Option<(&[u8], u32, u32)> {
        self.assets
            .iter()
            .find(|asset| asset.path == path)
            .map(|asset| (asset.pixels.as_slice(), asset.width, asset.height))
    }

    pub fn source_asset(&self, path: &str) -> Option<&[u8]> {
        self.source_assets
            .iter()
            .find(|asset| asset.path == path)
            .map(|asset| asset.bytes.as_slice())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub theme: ValidatedTheme,
    pub fallback_reason: Option<Reason>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Scene {
    pub schema: &'static str,
    pub theme: String,
    pub canvas: Canvas,
    pub settings: Settings,
    pub regions: Vec<SceneRegion>,
    pub components: Vec<SceneComponent>,
    pub assets: Vec<SceneAsset>,
    #[serde(rename = "upstreamContract")]
    pub upstream_contract: Option<UpstreamContract>,
    #[serde(rename = "sourceAssets")]
    pub source_assets: Vec<SceneSourceAsset>,
    pub synthetic: SyntheticMetadata,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneAsset {
    pub path: String,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing)]
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneSourceAsset {
    pub path: String,
    pub kind: UpstreamAssetKind,
    pub bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneComponent {
    pub id: String,
    pub kind: String,
    pub bounds: Bounds,
    pub path: Option<String>,
    pub text: Option<String>,
    pub color: Option<String>,
    #[serde(rename = "fontSize")]
    pub font_size: Option<u16>,
    #[serde(rename = "mediaBinding")]
    pub media_binding: Option<MediaBinding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneRegion {
    pub id: String,
    pub kind: RegionKind,
    pub bounds: Bounds,
    pub placeholder: bool,
    pub semantic: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Bounds {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct SyntheticMetadata {
    pub system: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub rating: f32,
    #[serde(rename = "releaseDate")]
    pub release_date: &'static str,
}

struct DuplicateKeys;

impl<'de> DeserializeSeed<'de> for DuplicateKeys {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for DuplicateKeys {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_string<E>(self, _: String) -> Result<(), E> {
        Ok(())
    }

    fn visit_bytes<E>(self, _: &[u8]) -> Result<(), E> {
        Ok(())
    }

    fn visit_byte_buf<E>(self, _: Vec<u8>) -> Result<(), E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        DuplicateKeys.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut access: A) -> Result<(), A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        while access.next_element_seed(DuplicateKeys)?.is_some() {}
        Ok(())
    }

    fn visit_map<A>(self, mut access: A) -> Result<(), A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = access.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(A::Error::custom(format!("duplicate JSON key: {key}")));
            }
            access.next_value_seed(DuplicateKeys)?;
        }
        Ok(())
    }
}

fn parse_json(input: &[u8]) -> Result<ValidatedTheme, ThemeError> {
    if input.len() > MAX_JSON_BYTES {
        return Err(ThemeError::new(
            Reason::BudgetJsonSize,
            "theme JSON exceeds 131072 bytes",
        ));
    }
    if input.contains(&0) {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "theme JSON contains NUL",
        ));
    }
    let text = std::str::from_utf8(input)
        .map_err(|_| ThemeError::new(Reason::MalformedJson, "theme JSON is not UTF-8"))?;
    let mut checker = JsonDeserializer::from_str(text);
    checker.deserialize_any(DuplicateKeys).map_err(|error| {
        let message = error.to_string();
        if message.starts_with("duplicate JSON key:") {
            ThemeError::new(Reason::DuplicateJsonKey, message)
        } else {
            ThemeError::new(Reason::MalformedJson, message)
        }
    })?;
    checker
        .end()
        .map_err(|error| ThemeError::new(Reason::MalformedJson, error.to_string()))?;
    let theme: Theme = serde_json::from_str(text).map_err(|error| {
        let message = error.to_string();
        let reason = if message.contains("unknown field") {
            Reason::UnknownField
        } else if message.contains("settings") {
            Reason::InvalidSetting
        } else if message.contains("layout") {
            Reason::InvalidLayout
        } else {
            Reason::InvalidSchema
        };
        ThemeError::new(reason, message)
    })?;
    validate_theme(theme)
}

fn safe_text(value: &str, label: &str, max: usize) -> Result<(), ThemeError> {
    if value.is_empty() || value.len() > max || value.contains('\0') || value.contains("//") {
        return Err(ThemeError::new(
            Reason::BudgetText,
            format!("{label} is empty, oversized, or unsafe"),
        ));
    }
    if value.contains("://") || value.starts_with('/') || value.contains('\\') {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            format!("{label} contains an external or absolute path"),
        ));
    }
    Ok(())
}

fn hex_color(value: &str, label: &str) -> Result<[u8; 4], ThemeError> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err(ThemeError::new(
            Reason::InvalidColor,
            format!("{label} must be #RRGGBB"),
        ));
    }
    let mut rgb = [0; 3];
    for (index, slot) in rgb.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[1 + index * 2..3 + index * 2], 16).map_err(|_| {
            ThemeError::new(Reason::InvalidColor, format!("{label} must be #RRGGBB"))
        })?;
    }
    Ok([rgb[0], rgb[1], rgb[2], 255])
}

fn contrast(left: [u8; 4], right: [u8; 4]) -> f64 {
    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    let luminance = |color: [u8; 4]| {
        0.2126 * channel(color[0]) + 0.7152 * channel(color[1]) + 0.0722 * channel(color[2])
    };
    let (left, right) = (luminance(left), luminance(right));
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

fn validate_asset_path(value: &str) -> Result<(), ThemeError> {
    if value.len() > 128 || value.ends_with('/') || value.contains("//") {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "asset path is not normalized",
        ));
    }
    validate_relative_path(Path::new(value))
}

fn declared_assets(theme: &Theme) -> Vec<AssetSpec> {
    theme
        .assets
        .as_ref()
        .map(|assets| {
            [
                assets.background.clone(),
                assets.system_art.clone(),
                assets.box_art.clone(),
                assets.screenshot.clone(),
                assets.controller.clone(),
            ]
            .into_iter()
            .flatten()
            .collect()
        })
        .unwrap_or_default()
}

fn decode_asset(path: &str, bytes: &[u8]) -> Result<LoadedAsset, ThemeError> {
    if !path.to_ascii_lowercase().ends_with(".png") {
        return Err(ThemeError::new(
            Reason::UnsupportedResource,
            "only PNG theme assets are supported",
        ));
    }
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes),
        png::Limits {
            bytes: (MAX_RESOURCE_BYTES * 64) as usize,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|_| ThemeError::new(Reason::InvalidAsset, format!("asset {path} is not a PNG")))?;
    let info = reader.info();
    let width = info.width;
    let height = info.height;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4 * 1024 * 1024 {
        return Err(ThemeError::new(
            Reason::BudgetAsset,
            format!("asset {path} dimensions exceed limits"),
        ));
    }
    let mut raw = vec![0; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut raw)
        .map_err(|_| ThemeError::new(Reason::InvalidAsset, format!("asset {path} is corrupt")))?;
    let pixels = match reader.output_color_type() {
        (png::ColorType::Rgb, png::BitDepth::Eight) => raw[..frame.buffer_size()]
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        (png::ColorType::Rgba, png::BitDepth::Eight) => raw[..frame.buffer_size()].to_vec(),
        (png::ColorType::Grayscale, png::BitDepth::Eight) => raw[..frame.buffer_size()]
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => raw[..frame.buffer_size()]
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        _ => {
            return Err(ThemeError::new(
                Reason::InvalidAsset,
                format!("asset {path} has unsupported color format"),
            ))
        }
    };
    if pixels.len() > (MAX_RESOURCE_BYTES * 64) as usize {
        return Err(ThemeError::new(
            Reason::BudgetAsset,
            format!("asset {path} decoded data exceeds limits"),
        ));
    }
    Ok(LoadedAsset {
        path: path.into(),
        width,
        height,
        pixels,
    })
}

pub fn validate_preview_image(bytes: &[u8]) -> Result<(), ThemeError> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(ThemeError::new(
            Reason::BudgetAsset,
            "catalog preview exceeds 4 MiB",
        ));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return decode_asset("preview.png", bytes).map(|_| ());
    }
    if bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9]) {
        return Ok(());
    }
    Err(ThemeError::new(
        Reason::InvalidAsset,
        "catalog preview is not a bounded PNG or JPEG",
    ))
}

pub fn normalize_theme_png(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<Vec<u8>, ThemeError> {
    if bytes.len() > 1024 * 1024 {
        return Err(ThemeError::new(
            Reason::BudgetAsset,
            "upstream PNG exceeds 1 MiB",
        ));
    }
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes),
        png::Limits {
            bytes: 64 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|_| ThemeError::new(Reason::InvalidAsset, "upstream asset is not a PNG"))?;
    let source_width = reader.info().width;
    let source_height = reader.info().height;
    if source_width == 0
        || source_height == 0
        || u64::from(source_width) * u64::from(source_height) > 16 * 1024 * 1024
    {
        return Err(ThemeError::new(
            Reason::BudgetAsset,
            "upstream PNG dimensions exceed limits",
        ));
    }
    let mut raw = vec![0; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut raw)
        .map_err(|_| ThemeError::new(Reason::InvalidAsset, "upstream PNG is corrupt"))?;
    let source_pixels: Vec<u8> = match reader.output_color_type() {
        (png::ColorType::Rgb, png::BitDepth::Eight) => raw[..frame.buffer_size()]
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        (png::ColorType::Rgba, png::BitDepth::Eight) => raw[..frame.buffer_size()].to_vec(),
        _ => {
            return Err(ThemeError::new(
                Reason::InvalidAsset,
                "upstream PNG color format is unsupported",
            ))
        }
    };
    let divisor = source_width
        .div_ceil(max_width)
        .max(source_height.div_ceil(max_height))
        .max(1);
    let width = (source_width / divisor).max(1);
    let height = (source_height / divisor).max(1);
    let mut pixels = vec![0_u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let source_x = x * source_width / width;
            let source_y = y * source_height / height;
            let source = ((source_y * source_width + source_x) * 4) as usize;
            let target = ((y * width + x) * 4) as usize;
            pixels[target..target + 4].copy_from_slice(&source_pixels[source..source + 4]);
        }
    }
    let mut output = Vec::new();
    {
        let mut encoder = Encoder::new(&mut output, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        writer
            .write_image_data(&pixels)
            .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    }
    Ok(output)
}

fn reduced_aspect(mut width: u32, mut height: u32) -> String {
    let mut divisor = width;
    let mut remainder = height;
    while remainder != 0 {
        let next = divisor % remainder;
        divisor = remainder;
        remainder = next;
    }
    width /= divisor;
    height /= divisor;
    format!("{width}:{height}")
}

fn validate_theme(theme: Theme) -> Result<ValidatedTheme, ThemeError> {
    let v1 = theme.schema == SCHEMA_URI && theme.format == SCHEMA && theme.schema_version == 1;
    let v2 = theme.schema == "urn:project:theme-v2"
        && theme.format == "theme-v2"
        && theme.schema_version == 2;
    if !v1 && !v2 {
        return Err(ThemeError::new(
            Reason::InvalidSchema,
            "unsupported theme schema",
        ));
    }
    if v2 {
        let Some(typography) = &theme.typography else {
            return Err(ThemeError::new(
                Reason::InvalidSchema,
                "v2 typography is required",
            ));
        };
        if typography.family != "project-sans"
            || !(12..=96).contains(&typography.title_size)
            || !(8..=64).contains(&typography.body_size)
            || !(8..=48).contains(&typography.small_size)
        {
            return Err(ThemeError::new(
                Reason::InvalidSetting,
                "unsupported v2 typography",
            ));
        }
        if theme.components.as_ref().is_none_or(Vec::is_empty) {
            return Err(ThemeError::new(
                Reason::InvalidLayout,
                "v2 components are required",
            ));
        }
    }
    safe_text(&theme.metadata.name, "metadata.name", 32)?;
    safe_text(&theme.metadata.author, "metadata.author", 64)?;
    if !matches!(
        theme.metadata.license.as_str(),
        "MIT" | "CC-BY-NC-SA" | "not-stated"
    ) {
        return Err(ThemeError::new(
            Reason::InvalidSchema,
            "theme license is unsupported",
        ));
    }
    let canvas_width = u32::from(theme.canvas.width);
    let canvas_height = u32::from(theme.canvas.height);
    if canvas_width == 0
        || canvas_height == 0
        || u64::from(canvas_width) * u64::from(canvas_height) > MAX_CANVAS_PIXELS
        || theme.canvas.aspect != reduced_aspect(canvas_width, canvas_height)
    {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            "canvas dimensions or aspect are invalid",
        ));
    }
    for (label, value) in [
        ("background", &theme.colors.background),
        ("surface", &theme.colors.surface),
        ("accent", &theme.colors.accent),
        ("text", &theme.colors.text),
        ("muted", &theme.colors.muted),
        ("highlight", &theme.colors.highlight),
    ] {
        hex_color(value, label)?;
    }
    let background = hex_color(&theme.colors.background, "background")?;
    for (label, value) in [
        ("text", &theme.colors.text),
        ("highlight", &theme.colors.highlight),
    ] {
        if contrast(hex_color(value, label)?, background) < MIN_LABEL_CONTRAST {
            return Err(ThemeError::new(
                Reason::InvalidColor,
                format!("{label} contrast is below {MIN_LABEL_CONTRAST}:1 against background"),
            ));
        }
    }
    for (label, resource) in [
        ("font", &theme.resources.font),
        ("icon", &theme.resources.icon),
        ("background", &theme.resources.background),
        ("sound", &theme.resources.sound),
    ] {
        if resource.budget_bytes as u64 > MAX_RESOURCE_BYTES {
            return Err(ThemeError::new(
                Reason::BudgetResourceBytes,
                format!("resources.{label} exceeds byte budget"),
            ));
        }
        let allowed = match (label, resource.kind) {
            ("font", ResourceKind::Builtin) => ["generated-sans"].as_slice(),
            ("icon", ResourceKind::Generated) => ["controller-mark"].as_slice(),
            ("background", ResourceKind::Generated) => ["grid-gradient", "flat-field"].as_slice(),
            ("sound", ResourceKind::Builtin) => ["silent"].as_slice(),
            _ => &[] as &[&str],
        };
        if resource.reference.starts_with('/')
            || resource.reference.contains('/')
            || resource.reference.contains('\\')
            || resource.reference == "."
            || resource.reference == ".."
        {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                format!("resources.{label}.ref is not a simple reference"),
            ));
        }
        if !allowed.contains(&resource.reference.as_str()) {
            return Err(ThemeError::new(
                Reason::UnsupportedResource,
                format!("resources.{label} is not a built-in/generated placeholder"),
            ));
        }
        safe_text(&resource.reference, &format!("resources.{label}.ref"), 32)?;
    }
    let total_resource_bytes: u64 = [
        theme.resources.font.budget_bytes,
        theme.resources.icon.budget_bytes,
        theme.resources.background.budget_bytes,
        theme.resources.sound.budget_bytes,
    ]
    .into_iter()
    .map(u64::from)
    .sum();
    if total_resource_bytes > MAX_RESOURCE_BYTES {
        return Err(ThemeError::new(
            Reason::BudgetResourceBytes,
            "theme resource budget exceeded",
        ));
    }
    if !(1..=12).contains(&theme.layout.max_visible_games) || theme.layout.regions.len() > 16 {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            "layout list or region budget exceeded",
        ));
    }
    let mut ids = HashSet::new();
    let mut kinds = HashSet::new();
    for region in &theme.layout.regions {
        safe_text(&region.id, "layout region id", 32)?;
        if !ids.insert(region.id.clone()) {
            return Err(ThemeError::new(
                Reason::InvalidLayout,
                "layout region ids must be unique",
            ));
        }
        if region.width == 0
            || region.height == 0
            || u32::from(region.x) + u32::from(region.width) > canvas_width
            || u32::from(region.y) + u32::from(region.height) > canvas_height
        {
            return Err(ThemeError::new(
                Reason::InvalidLayout,
                "layout region is outside the logical canvas",
            ));
        }
        kinds.insert(region.kind);
    }
    let required = [
        RegionKind::SystemArt,
        RegionKind::GameList,
        RegionKind::BoxArtPlaceholder,
        RegionKind::ScreenshotPlaceholder,
        RegionKind::Metadata,
        RegionKind::Menu,
        RegionKind::HelpStrip,
        RegionKind::Clock,
        RegionKind::Battery,
    ];
    if required.iter().any(|kind| !kinds.contains(kind)) {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            "layout omits a required Artbook region",
        ));
    }
    let clock = theme
        .layout
        .regions
        .iter()
        .find(|region| region.kind == RegionKind::Clock)
        .expect("required clock region");
    let battery = theme
        .layout
        .regions
        .iter()
        .find(|region| region.kind == RegionKind::Battery)
        .expect("required battery region");
    if clock.y != battery.y
        || clock.height != battery.height
        || u32::from(clock.x) + u32::from(clock.width) + 12 > u32::from(battery.x)
    {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            "clock, Wi-Fi, and battery need ordered non-overlapping status space",
        ));
    }
    if !(80..=160).contains(&theme.settings.font_scale) {
        return Err(ThemeError::new(
            Reason::InvalidSetting,
            "fontScale must be between 80 and 160",
        ));
    }
    if theme.fallback.splash != Splash::GeneratedNeutral
        || theme.fallback.on_invalid != OnInvalid::SafeArtbook
    {
        return Err(ThemeError::new(
            Reason::InvalidSetting,
            "only the safe generated fallback is supported",
        ));
    }
    if let Some(components) = &theme.components {
        if components.len() > 64 {
            return Err(ThemeError::new(
                Reason::BudgetAsset,
                "component count exceeds 64",
            ));
        }
        let mut component_ids = HashSet::new();
        for component in components {
            safe_text(&component.id, "component id", 48)?;
            if !component_ids.insert(component.id.clone()) {
                return Err(ThemeError::new(
                    Reason::InvalidLayout,
                    "component ids must be unique",
                ));
            }
            if component.width == 0
                || component.height == 0
                || u32::from(component.x) + u32::from(component.width) > canvas_width
                || u32::from(component.y) + u32::from(component.height) > canvas_height
            {
                return Err(ThemeError::new(
                    Reason::InvalidLayout,
                    "component is outside canvas",
                ));
            }
            if component.kind == ComponentKind::Image
                && component.path.is_none()
                && component.media_binding.is_none()
            {
                return Err(ThemeError::new(
                    Reason::InvalidAsset,
                    "image component has no path or media binding",
                ));
            }
            if let Some(path) = &component.path {
                validate_asset_path(path)?;
            }
            if let Some(color) = &component.color {
                hex_color(color, "component color")?;
            }
            if component.text.as_ref().is_some_and(|text| text.len() > 256) {
                return Err(ThemeError::new(
                    Reason::BudgetText,
                    "component text exceeds 256 bytes",
                ));
            }
            if component
                .font_size
                .is_some_and(|size| !(8..=96).contains(&size))
            {
                return Err(ThemeError::new(
                    Reason::InvalidSetting,
                    "component font size is out of bounds",
                ));
            }
        }
    }
    let declared_paths: BTreeSet<_> = declared_assets(&theme)
        .into_iter()
        .map(|asset| asset.path)
        .collect();
    if let Some(components) = &theme.components {
        for component in components {
            if component.kind == ComponentKind::Image
                && component
                    .path
                    .as_ref()
                    .is_some_and(|path| !declared_paths.contains(path))
            {
                return Err(ThemeError::new(
                    Reason::InvalidAsset,
                    "image component is not backed by a declared asset",
                ));
            }
        }
    }
    if let Some(assets) = &theme.assets {
        let mut declared_bytes = 0_u64;
        let mut seen_asset_paths = BTreeSet::new();
        for asset in [
            assets.background.as_ref(),
            assets.system_art.as_ref(),
            assets.box_art.as_ref(),
            assets.screenshot.as_ref(),
            assets.controller.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_asset_path(&asset.path)?;
            if asset.max_bytes == 0 || u64::from(asset.max_bytes) > MAX_RESOURCE_BYTES * 64 {
                return Err(ThemeError::new(
                    Reason::BudgetAsset,
                    "invalid asset declaration",
                ));
            }
            if seen_asset_paths.insert(&asset.path) {
                declared_bytes = declared_bytes.saturating_add(u64::from(asset.max_bytes));
            }
        }
        if declared_bytes > MAX_RESOURCE_BYTES * 64 {
            return Err(ThemeError::new(
                Reason::BudgetAsset,
                "declared asset budget exceeded",
            ));
        }
    }
    Ok(ValidatedTheme {
        theme,
        assets: Vec::new(),
        source_assets: Vec::new(),
    })
}

fn upstream_xml_value(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    xml[start..]
        .find(&close)
        .map(|end| xml[start..start + end].trim().to_string())
}

fn art_book_include_paths(xml: &str) -> Result<Vec<String>, ThemeError> {
    const ALLOWED: &[&str] = &[
        "./_inc/lang/default_en.xml",
        "./_inc/lang/${lang}.xml",
        "./fonts.xml",
        "${themeCustomizationsPath}fonts.xml",
        "./colors.xml",
        "${themeCustomizationsPath}colors.xml",
        "./aspect-ratio-16-9.xml",
        "./aspect-ratio-4-3.xml",
        "./aspect-ratio-16-10.xml",
        "./aspect-ratio-1-1.xml",
        "./aspect-ratio-3-2.xml",
    ];
    let mut paths = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<include") {
        rest = &rest[start..];
        let end = rest.find('>').ok_or_else(|| {
            ThemeError::new(
                Reason::InvalidXml,
                "Art Book Next include tag is not closed",
            )
        })?;
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        if tag.ends_with("/>") {
            continue;
        }
        let close = rest.find("</include>").ok_or_else(|| {
            ThemeError::new(
                Reason::InvalidXml,
                "Art Book Next include body is not closed",
            )
        })?;
        let include = rest[..close].trim();
        if include.is_empty() || include.contains('<') || !ALLOWED.contains(&include) {
            return Err(ThemeError::new(
                Reason::UnsupportedXml,
                format!("unsupported Art Book Next include {include:?}"),
            ));
        }
        paths.push(include.to_string());
        rest = &rest[close + "</include>".len()..];
    }
    Ok(paths)
}

fn read_bounded_art_book_file(
    root: &Path,
    relative: &Path,
    max_bytes: u64,
    missing_reason: Reason,
) -> Result<Vec<u8>, ThemeError> {
    validate_relative_path(relative)?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| ThemeError::new(missing_reason, error.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err(ThemeError::new(
            Reason::Symlink,
            format!("{} is a symlink", relative.display()),
        ));
    }
    if !metadata.file_type().is_file() {
        return Err(ThemeError::new(
            Reason::UnsupportedFile,
            format!("{} is not a regular file", relative.display()),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(ThemeError::new(
            Reason::BudgetResourceBytes,
            format!("{} exceeds its byte budget", relative.display()),
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|error| ThemeError::new(missing_reason, error.to_string()))?
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(ThemeError::new(
            Reason::BudgetResourceBytes,
            format!("{} exceeds its byte budget", relative.display()),
        ));
    }
    Ok(bytes)
}

fn read_bounded_art_book_xml(path: &Path, relative: &Path) -> Result<String, ThemeError> {
    let bytes =
        read_bounded_art_book_file(path, relative, MAX_ART_BOOK_NEXT_XML_BYTES, Reason::Io)?;
    String::from_utf8(bytes).map_err(|error| ThemeError::new(Reason::InvalidXml, error.to_string()))
}

fn validate_art_book_next_tree(path: &Path) -> Result<(), ThemeError> {
    let mut files = Vec::new();
    let mut entries_seen = 0;
    collect_theme_files_with_limit(
        path,
        path,
        &mut files,
        0,
        &mut entries_seen,
        MAX_ART_BOOK_NEXT_FILES,
    )?;
    for (relative, file) in files {
        let metadata = fs::symlink_metadata(&file)
            .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        if metadata.len() > MAX_ART_BOOK_NEXT_ASSET_BYTES {
            return Err(ThemeError::new(
                Reason::BudgetResourceBytes,
                format!("theme file {relative} exceeds its byte budget"),
            ));
        }
    }
    Ok(())
}

fn load_art_book_next(path: &Path, aspect: &str) -> Result<ValidatedTheme, ThemeError> {
    validate_art_book_next_tree(path)?;
    if aspect != "4-3" {
        return Err(ThemeError::new(
            Reason::UnsupportedXml,
            format!("Art Book Next supports only its inspected 4:3 variant, not {aspect}"),
        ));
    }
    let theme_xml = read_bounded_art_book_xml(path, Path::new("theme.xml"))?;
    let includes = art_book_include_paths(&theme_xml)?;
    for required in ["./fonts.xml", "./colors.xml", "./aspect-ratio-4-3.xml"] {
        if !includes.iter().any(|include| include == required) {
            return Err(ThemeError::new(
                Reason::UnsupportedXml,
                format!("Art Book Next required include {required} is missing"),
            ));
        }
    }
    let colors_xml = read_bounded_art_book_xml(path, Path::new("colors.xml"))?;
    let fonts_xml = read_bounded_art_book_xml(path, Path::new("fonts.xml"))?;
    let aspect_xml = read_bounded_art_book_xml(path, Path::new("aspect-ratio-4-3.xml"))?;
    for (name, xml, required) in [
        (
            "theme.xml",
            theme_xml.as_str(),
            &[
                "Art Book Next (Batocera ES Edition)",
                "Anthony Caccese",
                "creative commons CC-BY-NC-SA",
                "<include>./fonts.xml</include>",
                "<include>./colors.xml</include>",
                "<include ifSubset=\"aspect-ratio:4-3|4-3-auto\">./aspect-ratio-4-3.xml</include>",
            ][..],
        ),
        (
            "fonts.xml",
            fonts_xml.as_str(),
            &[
                "Roboto-Bold.ttf",
                "Roboto-Regular.ttf",
                "Roboto-Light.ttf",
                "ChangaOne-Italic.ttf",
            ][..],
        ),
        (
            "colors.xml",
            colors_xml.as_str(),
            &[
                "<systemBackgroundColor>000000</systemBackgroundColor>",
                "<gamelistListBackgroundColor>222222</gamelistListBackgroundColor>",
                "<menuSelectorColor>444444</menuSelectorColor>",
            ][..],
        ),
        (
            "aspect-ratio-4-3.xml",
            aspect_xml.as_str(),
            &[
                "<formatVersion>7</formatVersion>",
                "<maxLogoCount>4</maxLogoCount>",
                "<w>0.390625</w>",
                "<pos>0.75 0.2916666666666667</pos>",
            ][..],
        ),
    ] {
        if !xml.contains("<theme") || required.iter().any(|value| !xml.contains(value)) {
            return Err(ThemeError::new(
                Reason::UnsupportedXml,
                format!("Art Book Next include {name} is malformed or unsupported"),
            ));
        }
    }

    let source = "https://github.com/anthonycaccese/art-book-next-es".to_string();
    let revision = fs::read_to_string(path.join("SOURCE-COMMIT.txt"))
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("Pinned commit: ").map(str::to_owned))
        })
        .filter(|revision| {
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| {
            ThemeError::new(
                Reason::UnsupportedXml,
                "Art Book Next provenance pin is missing or malformed",
            )
        })?;
    let asset = |path: &str, kind| UpstreamAsset {
        path: path.into(),
        kind,
        max_bytes: MAX_ART_BOOK_NEXT_ASSET_BYTES as u32,
    };
    let contract = UpstreamContract {
        source,
        revision,
        variant: "4:3".into(),
        system_artwork_path: "./_inc/systems/artwork-default/${system.theme}.png".into(),
        system_logo_path: "./_inc/systems/logos/${system.theme}.svg".into(),
        fonts: vec![
            asset("_inc/fonts/Roboto-Bold.ttf", UpstreamAssetKind::Font),
            asset("_inc/fonts/Roboto-Regular.ttf", UpstreamAssetKind::Font),
            asset("_inc/fonts/Roboto-Light.ttf", UpstreamAssetKind::Font),
            asset("_inc/fonts/ChangaOne-Italic.ttf", UpstreamAssetKind::Font),
        ],
        system_artwork: vec![
            asset(
                "_inc/systems/artwork-default/_default.png",
                UpstreamAssetKind::Image,
            ),
            asset(
                "_inc/systems/artwork-default/genesis.png",
                UpstreamAssetKind::Image,
            ),
            asset(
                "_inc/systems/artwork-default/nes.png",
                UpstreamAssetKind::Image,
            ),
            asset(
                "_inc/systems/artwork-default/snes.png",
                UpstreamAssetKind::Image,
            ),
            asset(
                "_inc/systems/artwork-default/psx.png",
                UpstreamAssetKind::Image,
            ),
        ],
        system_logos: vec![
            asset("_inc/systems/logos/genesis.svg", UpstreamAssetKind::Svg),
            asset("_inc/systems/logos/nes.svg", UpstreamAssetKind::Svg),
            asset("_inc/systems/logos/snes.svg", UpstreamAssetKind::Svg),
            asset("_inc/systems/logos/psx.svg", UpstreamAssetKind::Svg),
        ],
        menu_assets: vec![
            asset("_inc/images/space.png", UpstreamAssetKind::Image),
            asset("_inc/images/menu-textinput.png", UpstreamAssetKind::Image),
            asset("_inc/images/menu-icon-system.svg", UpstreamAssetKind::Svg),
            asset(
                "_inc/images/metadata-icon-releasedate.svg",
                UpstreamAssetKind::Svg,
            ),
            asset("_inc/images/help-button-east.svg", UpstreamAssetKind::Svg),
            asset("_inc/sounds/scroll.wav", UpstreamAssetKind::Sound),
        ],
        options: UpstreamOptions {
            system_artwork: "default".into(),
            system_logos: "default".into(),
            game_artwork: "image".into(),
            game_metadata: "on".into(),
            font_size: "default".into(),
            color_scheme: "default".into(),
        },
    };
    let mut assets = Vec::new();
    let mut source_assets = Vec::new();
    for spec in contract
        .fonts
        .iter()
        .chain(&contract.system_artwork)
        .chain(&contract.system_logos)
        .chain(&contract.menu_assets)
    {
        let bytes = read_bounded_art_book_file(
            path,
            Path::new(&spec.path),
            u64::from(spec.max_bytes),
            Reason::MissingTheme,
        )
        .map_err(|error| {
            if error.reason == Reason::Io {
                ThemeError::new(
                    Reason::MissingTheme,
                    format!("Art Book Next resource {} is missing", spec.path),
                )
            } else {
                error
            }
        })?;
        if spec.kind == UpstreamAssetKind::Image {
            assets.push(decode_asset(&format!("./{}", spec.path), &bytes)?);
        }
        source_assets.push(LoadedSourceAsset {
            path: format!("./{}", spec.path),
            kind: spec.kind,
            bytes,
        });
    }
    let color = |name| {
        upstream_xml_value(&colors_xml, name)
            .map(|value| format!("#{}", &value[..6]))
            .ok_or_else(|| {
                ThemeError::new(
                    Reason::UnsupportedXml,
                    format!("Art Book Next color {name} is missing"),
                )
            })
    };
    let components = vec![
        Component {
            id: "system-artwork".into(),
            kind: ComponentKind::Image,
            x: 0,
            y: 0,
            width: 1024,
            height: 768,
            path: None,
            text: None,
            color: Some("#FFFFFF".into()),
            font_size: None,
            media_binding: Some(MediaBinding::SystemArtwork),
        },
        Component {
            id: "system-logo".into(),
            kind: ComponentKind::Image,
            x: 205,
            y: 230,
            width: 614,
            height: 307,
            path: None,
            text: None,
            color: Some("#FFFFFF".into()),
            font_size: None,
            media_binding: Some(MediaBinding::SystemLogo),
        },
        Component {
            id: "gamelist".into(),
            kind: ComponentKind::Textlist,
            x: 48,
            y: 196,
            width: 400,
            height: 438,
            path: None,
            text: None,
            color: Some("#FFFFFF".into()),
            font_size: Some(29),
            media_binding: None,
        },
        Component {
            id: "game-artwork".into(),
            kind: ComponentKind::Image,
            x: 560,
            y: 48,
            width: 416,
            height: 352,
            path: None,
            text: None,
            color: None,
            font_size: None,
            media_binding: Some(MediaBinding::GameImage),
        },
        Component {
            id: "game-description".into(),
            kind: ComponentKind::Text,
            x: 564,
            y: 446,
            width: 408,
            height: 192,
            path: None,
            text: Some("{game:desc}".into()),
            color: Some("#FFFFFF".into()),
            font_size: Some(32),
            media_binding: None,
        },
        Component {
            id: "menu-textinput".into(),
            kind: ComponentKind::Image,
            x: 208,
            y: 120,
            width: 608,
            height: 528,
            path: Some("./_inc/images/menu-textinput.png".into()),
            text: None,
            color: None,
            font_size: None,
            media_binding: None,
        },
    ];
    let theme = Theme {
        schema: "urn:project:theme-v2".into(),
        format: "theme-v2".into(),
        schema_version: 2,
        metadata: Metadata {
            name: "Art Book Next (Batocera ES Edition)".into(),
            author: "Anthony Caccese".into(),
            license: "CC-BY-NC-SA".into(),
        },
        canvas: Canvas {
            width: 1024,
            height: 768,
            aspect: "4:3".into(),
        },
        colors: Colors {
            background: color("systemBackgroundColor")?,
            surface: color("gamelistListBackgroundColor")?,
            accent: color("menuSelectorColor")?,
            text: color("gamelistListDescriptionColor")?,
            muted: color("gamelistListTextlistUnselectedColor")?,
            highlight: color("gamelistListTextlistSelectedColor")?,
        },
        resources: Resources {
            font: Resource {
                kind: ResourceKind::ThemeAsset,
                reference: "_inc/fonts/Roboto-Regular.ttf".into(),
                budget_bytes: 0,
            },
            icon: Resource {
                kind: ResourceKind::ThemeAsset,
                reference: "_inc/images/menu-icon-system.svg".into(),
                budget_bytes: 0,
            },
            background: Resource {
                kind: ResourceKind::ThemeAsset,
                reference: "_inc/images/space.png".into(),
                budget_bytes: 0,
            },
            sound: Resource {
                kind: ResourceKind::ThemeAsset,
                reference: "_inc/sounds/scroll.wav".into(),
                budget_bytes: 0,
            },
        },
        layout: Layout {
            preset: LayoutPreset::Artbook,
            max_visible_games: 7,
            regions: vec![
                ("system-art", RegionKind::SystemArt, 0, 0, 1024, 768),
                ("game-list", RegionKind::GameList, 0, 0, 512, 768),
                ("box-art", RegionKind::BoxArtPlaceholder, 560, 48, 416, 352),
                (
                    "screenshot",
                    RegionKind::ScreenshotPlaceholder,
                    560,
                    192,
                    464,
                    416,
                ),
                ("metadata", RegionKind::Metadata, 564, 446, 408, 192),
                ("menu", RegionKind::Menu, 208, 120, 608, 528),
                ("help", RegionKind::HelpStrip, 0, 680, 1024, 88),
                ("clock", RegionKind::Clock, 760, 32, 104, 32),
                ("battery", RegionKind::Battery, 876, 32, 116, 32),
            ]
            .into_iter()
            .map(|(id, kind, x, y, width, height)| Region {
                id: id.into(),
                kind,
                x,
                y,
                width,
                height,
                visible: true,
            })
            .collect(),
        },
        settings: Settings {
            artwork_mode: ArtworkMode::Screenshot,
            metadata_visibility: MetadataVisibility::Full,
            font_scale: 100,
            color_scheme: ColorScheme::Dark,
        },
        fallback: Fallback {
            splash: Splash::GeneratedNeutral,
            on_invalid: OnInvalid::SafeArtbook,
        },
        typography: Some(Typography {
            family: "Roboto".into(),
            title_size: 48,
            body_size: 32,
            small_size: 24,
        }),
        assets: None,
        components: Some(components),
        upstream_contract: Some(contract),
    };
    Ok(ValidatedTheme {
        theme,
        assets,
        source_assets,
    })
}

pub fn load_theme_dir(path: &Path) -> Result<ValidatedTheme, ThemeError> {
    let root_meta = fs::symlink_metadata(path).map_err(|error| {
        let reason = if error.kind() == std::io::ErrorKind::NotFound {
            Reason::MissingTheme
        } else {
            Reason::Io
        };
        ThemeError::new(reason, error.to_string())
    })?;
    if root_meta.file_type().is_symlink() {
        return Err(ThemeError::new(
            Reason::Symlink,
            "theme directory is a symlink",
        ));
    }
    if !root_meta.is_dir() {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "theme selection must be a directory",
        ));
    }
    let marker_path = path.join("theme.xml");
    if fs::symlink_metadata(&marker_path)
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || metadata.file_type().is_file())
    {
        validate_art_book_next_tree(path)?;
        let marker = read_bounded_art_book_xml(path, Path::new("theme.xml"))?;
        if marker.contains("Art Book Next (Batocera ES Edition)") {
            return load_art_book_next(path, "4-3");
        }
    }
    let mut files = Vec::new();
    let mut entries_seen = 0;
    collect_theme_files(path, path, &mut files, 0, &mut entries_seen)?;
    if files.len() > MAX_FILES {
        return Err(ThemeError::new(
            Reason::BudgetFileCount,
            "theme file count exceeds 32",
        ));
    }
    let theme_file = files
        .iter()
        .find(|(relative, _)| relative == "theme.json")
        .map(|(_, path)| path);
    let xml_file = files
        .iter()
        .find(|(relative, _)| relative == "theme.xml")
        .map(|(_, path)| path);
    let Some(theme_file) = theme_file.or(xml_file) else {
        return Err(ThemeError::new(
            Reason::MissingTheme,
            "theme.json or theme.xml is missing",
        ));
    };
    if theme_file.extension().and_then(|ext| ext.to_str()) == Some("xml") {
        return import_es_theme_dir(path).map(|result| result.theme);
    }
    let size = fs::metadata(theme_file)
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?
        .len();
    if size > MAX_JSON_BYTES as u64 {
        return Err(ThemeError::new(
            Reason::BudgetJsonSize,
            "theme JSON exceeds 131072 bytes",
        ));
    }
    let bytes =
        fs::read(theme_file).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    let mut validated = parse_json(&bytes)?;
    let specs = declared_assets(&validated.theme);
    let allowed: BTreeSet<String> = std::iter::once("theme.json".to_string())
        .chain(std::iter::once("compatibility-report.json".to_string()))
        .chain(specs.iter().map(|spec| spec.path.clone()))
        .collect();
    for (relative, file) in &files {
        if !allowed.contains(relative) {
            return Err(ThemeError::new(
                Reason::UnsupportedFile,
                format!("unsupported theme file {relative}"),
            ));
        }
        let metadata =
            fs::metadata(file).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        if relative == "compatibility-report.json" {
            if metadata.len() > MAX_JSON_BYTES as u64 {
                return Err(ThemeError::new(
                    Reason::BudgetJsonSize,
                    "compatibility report exceeds 131072 bytes",
                ));
            }
            let report =
                fs::read(file).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
            serde_json::from_slice::<CompatibilityReport>(&report)
                .map_err(|error| ThemeError::new(Reason::MalformedJson, error.to_string()))?;
        } else if relative != "theme.json" && metadata.len() > MAX_RESOURCE_BYTES * 64 {
            return Err(ThemeError::new(
                Reason::BudgetAsset,
                format!("asset {relative} exceeds byte budget"),
            ));
        }
    }
    if validated.theme.schema_version == 2 && specs.is_empty() {
        return Err(ThemeError::new(
            Reason::InvalidAsset,
            "v2 theme declares no assets",
        ));
    }
    for spec in specs {
        let file = files
            .iter()
            .find(|(relative, _)| relative == &spec.path)
            .map(|(_, path)| path);
        let Some(file) = file else {
            return Err(ThemeError::new(
                Reason::MissingTheme,
                format!("declared asset {} is missing", spec.path),
            ));
        };
        let bytes =
            fs::read(file).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        if bytes.len() > spec.max_bytes as usize {
            return Err(ThemeError::new(
                Reason::InvalidAsset,
                format!("asset {} failed validation", spec.path),
            ));
        }
        validated.assets.push(decode_asset(&spec.path, &bytes)?);
    }
    Ok(validated)
}

fn collect_theme_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
    depth: usize,
    entries_seen: &mut usize,
) -> Result<(), ThemeError> {
    collect_theme_files_with_limit(root, directory, files, depth, entries_seen, MAX_FILES)
}

fn collect_theme_files_with_limit(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
    depth: usize,
    entries_seen: &mut usize,
    max_entries: usize,
) -> Result<(), ThemeError> {
    if depth > MAX_THEME_DEPTH {
        return Err(ThemeError::new(
            Reason::BudgetFileCount,
            "theme directory depth exceeds 8",
        ));
    }
    let entries =
        fs::read_dir(directory).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    for entry in entries {
        *entries_seen += 1;
        if *entries_seen > max_entries {
            return Err(ThemeError::new(
                Reason::BudgetFileCount,
                "theme directory entry budget exceeded",
            ));
        }
        let entry = entry.map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| ThemeError::new(Reason::InvalidPath, "theme path escaped root"))?;
        validate_relative_path(relative)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(ThemeError::new(
                Reason::Symlink,
                format!("{} is a symlink", relative.display()),
            ));
        }
        if metadata.is_dir() {
            collect_theme_files_with_limit(
                root,
                &path,
                files,
                depth + 1,
                entries_seen,
                max_entries,
            )?;
        } else if metadata.is_file() {
            files.push((relative.to_string_lossy().into_owned(), path));
        } else {
            return Err(ThemeError::new(
                Reason::UnsupportedFile,
                format!("unsupported theme file {}", relative.display()),
            ));
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<(), ThemeError> {
    if path.is_absolute() || path.as_os_str().is_empty() {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "theme path must be relative",
        ));
    }
    for component in path.components() {
        if matches!(
            component,
            PathComponent::CurDir
                | PathComponent::ParentDir
                | PathComponent::RootDir
                | PathComponent::Prefix(_)
        ) {
            return Err(ThemeError::new(
                Reason::InvalidPath,
                "theme path contains ., .., or an absolute component",
            ));
        }
    }
    if path.to_string_lossy().contains('\0') || path.to_string_lossy().contains('\\') {
        return Err(ThemeError::new(
            Reason::InvalidPath,
            "theme path is not a normalized POSIX path",
        ));
    }
    Ok(())
}

pub fn load_bundled_theme(path: &Path, aspect: &str) -> Result<ValidatedTheme, ThemeError> {
    load_art_book_next(path, aspect)
}

pub fn safe_artbook() -> Result<ValidatedTheme, ThemeError> {
    project_artbook_fallback()
}

fn project_artbook_fallback() -> Result<ValidatedTheme, ThemeError> {
    let mut theme = parse_json(include_bytes!("../../../themes/default/theme.json"))?;
    for spec in declared_assets(theme.theme()) {
        let bytes = match spec.path.as_str() {
            "assets/art.png" => include_bytes!("../../../themes/default/assets/art.png").as_slice(),
            "assets/box-art.png" => {
                include_bytes!("../../../themes/default/assets/box-art.png").as_slice()
            }
            "assets/screenshot.png" => {
                include_bytes!("../../../themes/default/assets/screenshot.png").as_slice()
            }
            _ => continue,
        };
        theme.assets.push(decode_asset(&spec.path, bytes)?);
    }
    Ok(theme)
}

fn load_imported_theme(id: &str) -> Result<ValidatedTheme, ThemeError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../themes/imported")
        .join(id);
    load_theme_dir(&path)
}

pub fn simplelife() -> Result<ValidatedTheme, ThemeError> {
    load_imported_theme("simplelife")
}

pub fn techdweeb() -> Result<ValidatedTheme, ThemeError> {
    load_imported_theme("techdweeb")
}

pub fn luma_station() -> Result<ValidatedTheme, ThemeError> {
    let mut theme = project_artbook_fallback()?;
    theme.theme.metadata.name = "Luma Station".into();
    theme.theme.colors.background = "#24112B".into();
    theme.theme.colors.surface = "#4A1F55".into();
    theme.theme.colors.accent = "#52D6B5".into();
    theme.theme.colors.text = "#FFF0D0".into();
    theme.theme.colors.muted = "#B77BBE".into();
    theme.theme.colors.highlight = "#FF9B70".into();
    theme.theme.layout.preset = LayoutPreset::Contrast;
    theme
        .theme
        .layout
        .regions
        .retain(|region| region.id != "system-art");
    theme.theme.layout.regions.insert(
        0,
        Region {
            id: "system-art".into(),
            kind: RegionKind::SystemArt,
            x: 520,
            y: 82,
            width: 440,
            height: 220,
            visible: true,
        },
    );
    if let Some(components) = &mut theme.theme.components {
        for component in components {
            if component.id == "hero" {
                component.x = 32;
                component.y = 96;
                component.width = 430;
                component.height = 188;
            } else if component.id == "title" {
                component.x = 48;
                component.y = 56;
                component.width = 430;
            } else if component.id == "games" {
                component.x = 48;
                component.y = 318;
                component.width = 430;
            }
        }
    }
    Ok(theme)
}

pub fn preview_or_fallback(path: Option<&Path>) -> Result<Preview, ThemeError> {
    match path {
        None => Ok(Preview {
            theme: safe_artbook()?,
            fallback_reason: Some(Reason::MissingTheme),
        }),
        Some(path) => preview_path_or_fallback(path),
    }
}

pub fn preview_path_or_fallback(path: &Path) -> Result<Preview, ThemeError> {
    match load_theme_dir(path) {
        Ok(theme) => Ok(Preview {
            theme,
            fallback_reason: None,
        }),
        Err(error) => Ok(Preview {
            theme: safe_artbook()?,
            fallback_reason: Some(error.reason),
        }),
    }
}

pub fn scene(theme: &ValidatedTheme) -> Scene {
    Scene {
        schema: "theme-scene/v1",
        theme: theme.theme.metadata.name.clone(),
        canvas: theme.theme.canvas.clone(),
        settings: theme.theme.settings.clone(),
        regions: theme
            .theme
            .layout
            .regions
            .iter()
            .map(|region| SceneRegion {
                id: region.id.clone(),
                kind: region.kind,
                bounds: Bounds {
                    x: region.x,
                    y: region.y,
                    width: region.width,
                    height: region.height,
                },
                placeholder: true,
                semantic: semantic(region.kind),
            })
            .collect(),
        components: theme
            .theme
            .components
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|component| SceneComponent {
                id: component.id.clone(),
                kind: component_kind_name(component.kind),
                bounds: Bounds {
                    x: component.x,
                    y: component.y,
                    width: component.width,
                    height: component.height,
                },
                path: component.path.clone(),
                text: component.text.clone(),
                color: component.color.clone(),
                font_size: component.font_size,
                media_binding: component.media_binding,
            })
            .collect(),
        assets: theme
            .assets
            .iter()
            .map(|asset| SceneAsset {
                path: asset.path.clone(),
                width: asset.width,
                height: asset.height,
                pixels: asset.pixels.clone(),
            })
            .collect(),
        upstream_contract: theme.theme.upstream_contract.clone(),
        source_assets: theme
            .source_assets
            .iter()
            .map(|asset| SceneSourceAsset {
                path: asset.path.clone(),
                kind: asset.kind,
                bytes: asset.bytes.len(),
            })
            .collect(),
        synthetic: SyntheticMetadata {
            system: "NOVA/8 HANDHELD",
            title: "Nebula Notes",
            description: "Chart a quiet starship through forgotten constellations.",
            rating: 4.2,
            release_date: "1993-09-14",
        },
    }
}

fn component_kind_name(kind: ComponentKind) -> String {
    match kind {
        ComponentKind::Image => "image",
        ComponentKind::Text => "text",
        ComponentKind::Textlist => "textlist",
    }
    .into()
}

fn semantic(kind: RegionKind) -> String {
    match kind {
        RegionKind::SystemArt => "large generated system artwork browsing".into(),
        RegionKind::GameList => "dense generated game list".into(),
        RegionKind::BoxArtPlaceholder => "neutral box-art placeholder".into(),
        RegionKind::ScreenshotPlaceholder => "neutral screenshot placeholder".into(),
        RegionKind::Metadata => "description rating and release-date metadata".into(),
        RegionKind::Menu => "full-screen console-style menu".into(),
        RegionKind::HelpStrip => "controller-first help strip".into(),
        RegionKind::Clock => "clock affordance".into(),
        RegionKind::Battery => "battery affordance".into(),
    }
}

pub fn write_scene(theme: &ValidatedTheme, path: &Path) -> Result<(), ThemeError> {
    let file =
        fs::File::create(path).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), &scene(theme))
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))
}

pub fn render_png(theme: &ValidatedTheme, path: &Path) -> Result<(), ThemeError> {
    let file =
        fs::File::create(path).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    let writer = BufWriter::new(file);
    let width = u32::from(theme.theme.canvas.width);
    let height = u32::from(theme.theme.canvas.height);
    let mut encoder = Encoder::new(writer, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut output = encoder
        .write_header()
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    let colors = &theme.theme.colors;
    let background = hex_color(&colors.background, "background").unwrap_or([0, 0, 0, 255]);
    let surface = hex_color(&colors.surface, "surface").unwrap_or(background);
    let accent = hex_color(&colors.accent, "accent").unwrap_or(background);
    let text = hex_color(&colors.text, "text").unwrap_or(background);
    let muted = hex_color(&colors.muted, "muted").unwrap_or(background);
    let highlight = hex_color(&colors.highlight, "highlight").unwrap_or(accent);
    let mut data = vec![0_u8; (u64::from(width) * u64::from(height) * 4) as usize];
    for y in 0..height as u16 {
        for x in 0..width as u16 {
            let mut color = background;
            for region in &theme.theme.layout.regions {
                if !region.visible
                    || x < region.x
                    || y < region.y
                    || x >= region.x + region.width
                    || y >= region.y + region.height
                {
                    continue;
                }
                let fill = match region.kind {
                    RegionKind::SystemArt => accent,
                    RegionKind::GameList | RegionKind::Menu => surface,
                    RegionKind::Metadata | RegionKind::HelpStrip => muted,
                    RegionKind::Clock | RegionKind::Battery => highlight,
                    RegionKind::BoxArtPlaceholder | RegionKind::ScreenshotPlaceholder => text,
                };
                let stripe = ((u32::from(x) + u32::from(y)) / 12) % 2 == 0;
                color = if stripe {
                    fill
                } else {
                    blend(fill, background)
                };
                if x == region.x
                    || y == region.y
                    || x + 1 == region.x + region.width
                    || y + 1 == region.y + region.height
                {
                    color = highlight;
                }
            }
            let offset = (y as usize * width as usize + x as usize) * 4;
            data[offset..offset + 4].copy_from_slice(&color);
        }
    }
    for component in theme.theme.components.as_deref().unwrap_or_default() {
        if component.kind != ComponentKind::Image
            || !matches!(component.id.as_str(), "system-artwork" | "game-artwork")
        {
            continue;
        }
        let fallback_path = (component.media_binding == Some(MediaBinding::SystemArtwork))
            .then(|| {
                theme.theme.upstream_contract.as_ref().map(|contract| {
                    contract
                        .system_artwork_path
                        .replace("${system.theme}", "_default")
                })
            })
            .flatten();
        let Some(path) = component.path.as_deref().or(fallback_path.as_deref()) else {
            continue;
        };
        let Some(asset) = theme.assets.iter().find(|asset| asset.path == path) else {
            continue;
        };
        for y in 0..u32::from(component.height) {
            for x in 0..u32::from(component.width) {
                let sx = x * asset.width / u32::from(component.width);
                let sy = y * asset.height / u32::from(component.height);
                let source = ((sy * asset.width + sx) * 4) as usize;
                let target = (((u32::from(component.y) + y) * width + u32::from(component.x) + x)
                    * 4) as usize;
                data[target..target + 4].copy_from_slice(&asset.pixels[source..source + 4]);
            }
        }
    }
    output
        .write_image_data(&data)
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    Ok(())
}

fn blend(first: [u8; 4], second: [u8; 4]) -> [u8; 4] {
    [
        ((u16::from(first[0]) + u16::from(second[0])) / 2) as u8,
        ((u16::from(first[1]) + u16::from(second[1])) / 2) as u8,
        ((u16::from(first[2]) + u16::from(second[2])) / 2) as u8,
        255,
    ]
}

pub fn serialize_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ThemeError> {
    serde_json::to_vec_pretty(value).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))
}

pub fn parse_theme_bytes(input: &[u8]) -> Result<ValidatedTheme, ThemeError> {
    parse_json(input)
}
