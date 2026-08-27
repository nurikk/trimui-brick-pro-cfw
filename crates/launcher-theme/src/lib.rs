use std::{
    collections::HashSet,
    fs,
    io::BufWriter,
    path::{Component, Path, PathBuf},
};

use png::{BitDepth, ColorType, Encoder};
use serde::{
    de::{DeserializeSeed, Error as DeError},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{de::Deserializer as JsonDeserializer, Value};

pub const CANVAS_WIDTH: u32 = 1024;
pub const CANVAS_HEIGHT: u32 = 768;
pub const MAX_JSON_BYTES: usize = 128 * 1024;
pub const MAX_FILES: usize = 32;
pub const MAX_RESOURCE_BYTES: u64 = 64 * 1024;
pub const MAX_RENDER_BYTES: u64 = CANVAS_WIDTH as u64 * CANVAS_HEIGHT as u64 * 4;
pub const SCHEMA: &str = "theme-v1";
pub const SCHEMA_URI: &str = "urn:project:theme-v1";

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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    pub author: String,
    pub license: String,
    pub provenance: String,
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
pub struct ValidatedTheme(Theme);

impl ValidatedTheme {
    pub fn theme(&self) -> &Theme {
        &self.0
    }

    pub fn name(&self) -> &str {
        &self.0.metadata.name
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
    pub synthetic: SyntheticMetadata,
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

fn validate_theme(theme: Theme) -> Result<ValidatedTheme, ThemeError> {
    if theme.schema != SCHEMA_URI || theme.format != SCHEMA || theme.schema_version != 1 {
        return Err(ThemeError::new(
            Reason::InvalidSchema,
            "unsupported theme schema",
        ));
    }
    safe_text(&theme.metadata.name, "metadata.name", 32)?;
    safe_text(&theme.metadata.author, "metadata.author", 64)?;
    if theme.metadata.license != "MIT" || theme.metadata.provenance != "project-authored" {
        return Err(ThemeError::new(
            Reason::InvalidSchema,
            "theme must carry project-authored MIT metadata",
        ));
    }
    if theme.canvas.width as u32 != CANVAS_WIDTH
        || theme.canvas.height as u32 != CANVAS_HEIGHT
        || theme.canvas.aspect != "4:3"
    {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            "canvas must be 1024x768 with 4:3 aspect",
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
            || u32::from(region.x) + u32::from(region.width) > CANVAS_WIDTH
            || u32::from(region.y) + u32::from(region.height) > CANVAS_HEIGHT
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
    Ok(ValidatedTheme(theme))
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
    let mut files = Vec::new();
    collect_theme_files(path, path, &mut files)?;
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
    let Some(theme_file) = theme_file else {
        return Err(ThemeError::new(
            Reason::MissingTheme,
            "theme.json is missing",
        ));
    };
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
    parse_json(&bytes)
}

fn collect_theme_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), ThemeError> {
    let entries =
        fs::read_dir(directory).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    for entry in entries {
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
            return Err(ThemeError::new(
                Reason::UnsupportedFile,
                format!(
                    "nested theme directory {} is unsupported",
                    relative.display()
                ),
            ));
        }
        if !metadata.is_file() || relative != Path::new("theme.json") {
            return Err(ThemeError::new(
                Reason::UnsupportedFile,
                format!("unsupported theme file {}", relative.display()),
            ));
        }
        files.push((relative.to_string_lossy().into_owned(), path));
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
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_)
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

pub fn safe_artbook() -> Result<ValidatedTheme, ThemeError> {
    parse_json(include_bytes!("../../../themes/default/theme.json"))
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
        theme: theme.0.metadata.name.clone(),
        canvas: theme.0.canvas.clone(),
        settings: theme.0.settings.clone(),
        regions: theme
            .0
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
        synthetic: SyntheticMetadata {
            system: "Generated System",
            title: "Generated Demo 1",
            description: "Deterministic synthetic metadata for preview only.",
            rating: 4.2,
            release_date: "1993-09-14",
        },
    }
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
    let mut encoder = Encoder::new(writer, CANVAS_WIDTH, CANVAS_HEIGHT);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut output = encoder
        .write_header()
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    let colors = &theme.0.colors;
    let background = hex_color(&colors.background, "background").unwrap_or([0, 0, 0, 255]);
    let surface = hex_color(&colors.surface, "surface").unwrap_or(background);
    let accent = hex_color(&colors.accent, "accent").unwrap_or(background);
    let text = hex_color(&colors.text, "text").unwrap_or(background);
    let muted = hex_color(&colors.muted, "muted").unwrap_or(background);
    let highlight = hex_color(&colors.highlight, "highlight").unwrap_or(accent);
    let mut data = vec![0_u8; MAX_RENDER_BYTES as usize];
    for y in 0..CANVAS_HEIGHT as u16 {
        for x in 0..CANVAS_WIDTH as u16 {
            let mut color = background;
            for region in &theme.0.layout.regions {
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
            let offset = (y as usize * CANVAS_WIDTH as usize + x as usize) * 4;
            data[offset..offset + 4].copy_from_slice(&color);
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
