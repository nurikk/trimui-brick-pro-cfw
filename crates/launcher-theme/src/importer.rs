use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use super::*;

const MAX_XML_NODES: usize = 128;
const MAX_XML_DEPTH: usize = 16;
const MAX_XML_ATTRIBUTES: usize = 16;
const MAX_XML_TEXT_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReport {
    pub subset: String,
    pub status: String,
    pub accepted: Vec<String>,
    pub unsupported: Vec<String>,
    pub rejected: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ImportedTheme {
    pub theme: ValidatedTheme,
    pub report: CompatibilityReport,
}

impl ImportedTheme {
    pub fn write_native_dir(&self, source_root: &Path, output: &Path) -> Result<(), ThemeError> {
        fs::create_dir_all(output)
            .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        fs::write(
            output.join("theme.json"),
            serialize_json(self.theme.theme())?,
        )
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        for spec in declared_assets(self.theme.theme()) {
            let bytes = fs::read(source_root.join(&spec.path)).map_err(|error| {
                ThemeError::new(
                    Reason::MissingTheme,
                    format!("asset {} is missing: {error}", spec.path),
                )
            })?;
            let destination = output.join(&spec.path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
            }
            fs::write(destination, bytes)
                .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

pub fn import_es_theme_dir(root: &Path) -> Result<ImportedTheme, ThemeError> {
    let meta = fs::symlink_metadata(root)
        .map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(ThemeError::new(
            Reason::Symlink,
            "XML theme root must be a real directory",
        ));
    }
    let mut files = Vec::new();
    let mut entries_seen = 0;
    collect_theme_files(root, root, &mut files, 0, &mut entries_seen)?;
    if files.len() > MAX_FILES {
        return Err(ThemeError::new(
            Reason::BudgetFileCount,
            "theme file count exceeds 32",
        ));
    }
    let xml_path = files
        .iter()
        .find(|(name, _)| name == "theme.xml")
        .map(|(_, path)| path)
        .ok_or_else(|| ThemeError::new(Reason::MissingTheme, "theme.xml is missing"))?;
    let xml = fs::read(xml_path).map_err(|error| ThemeError::new(Reason::Io, error.to_string()))?;
    let (document, mut report) = parse_document(&xml)?;
    let components = components(&document, &mut report)?;
    let mut specs = Vec::new();
    let mut paths = BTreeSet::new();
    for component in &components {
        if let Some(path) = &component.path {
            if !paths.insert(path.clone()) {
                continue;
            }
            let file = path_from_relative(path)?;
            let full = root.join(&file);
            let bytes = fs::read(&full).map_err(|_| {
                ThemeError::new(Reason::MissingTheme, format!("asset {path} is missing"))
            })?;
            let max_bytes = 1024 * 1024_u32;
            if bytes.len() > max_bytes as usize {
                return Err(ThemeError::new(
                    Reason::BudgetAsset,
                    format!("asset {path} exceeds limit"),
                ));
            }
            let spec = AssetSpec {
                path: path.clone(),
                max_bytes,
            };
            decode_asset(path, &bytes)?;
            specs.push(spec);
        }
    }
    for (file, _) in &files {
        if file != "theme.xml" && !paths.contains(file) {
            return Err(ThemeError::new(
                Reason::UnsupportedFile,
                format!("unsupported XML theme file {file}"),
            ));
        }
    }
    let assets = Assets {
        background: select_asset(&specs, &["background", "backdrop"]),
        system_art: select_asset(&specs, &["system", "logo", "hero"]),
        box_art: select_asset(&specs, &["box", "cover", "marquee"]),
        screenshot: select_asset(&specs, &["screenshot", "screen", "snap"]),
        controller: None,
    };
    let theme = Theme {
        schema: "urn:project:theme-v2".into(),
        format: "theme-v2".into(),
        schema_version: 2,
        metadata: Metadata {
            name: root_attr(&document, "name")
                .unwrap_or("Imported Theme")
                .to_string(),
            author: "Project Authors".into(),
            license: "MIT".into(),
        },
        canvas: Canvas {
            width: 1024,
            height: 768,
            aspect: "4:3".into(),
        },
        colors: Colors {
            background: "#10131C".into(),
            surface: "#202638".into(),
            accent: "#4D83D8".into(),
            text: "#EDF2FA".into(),
            muted: "#56627A".into(),
            highlight: "#F0B35B".into(),
        },
        resources: Resources {
            font: Resource {
                kind: ResourceKind::Builtin,
                reference: "generated-sans".into(),
                budget_bytes: 0,
            },
            icon: Resource {
                kind: ResourceKind::Generated,
                reference: "controller-mark".into(),
                budget_bytes: 0,
            },
            background: Resource {
                kind: ResourceKind::Generated,
                reference: "grid-gradient".into(),
                budget_bytes: 0,
            },
            sound: Resource {
                kind: ResourceKind::Builtin,
                reference: "silent".into(),
                budget_bytes: 0,
            },
        },
        layout: imported_layout(&components),
        settings: Settings {
            artwork_mode: ArtworkMode::SystemArt,
            metadata_visibility: MetadataVisibility::Full,
            font_scale: 100,
            color_scheme: ColorScheme::Dark,
        },
        fallback: Fallback {
            splash: Splash::GeneratedNeutral,
            on_invalid: OnInvalid::SafeArtbook,
        },
        typography: Some(Typography {
            family: "project-sans".into(),
            title_size: 42,
            body_size: 22,
            small_size: 16,
        }),
        assets: Some(assets),
        components: Some(components),
    };
    let mut validated = validate_theme(theme)?;
    validated.theme.metadata = Metadata {
        name: root_attr(&document, "name")
            .unwrap_or("Imported Theme")
            .to_string(),
        author: root_attr(&document, "author")
            .unwrap_or("Project Authors")
            .to_string(),
        license: root_attr(&document, "license")
            .unwrap_or("MIT")
            .to_string(),
    };
    for spec in declared_assets(&validated.theme) {
        let bytes = fs::read(root.join(&spec.path)).map_err(|_| {
            ThemeError::new(
                Reason::MissingTheme,
                format!("asset {} is missing", spec.path),
            )
        })?;
        validated.assets.push(decode_asset(&spec.path, &bytes)?);
    }
    report.status = "imported".into();
    Ok(ImportedTheme {
        theme: validated,
        report,
    })
}

pub fn import_emulationstation_theme(root: &Path) -> Result<ImportedTheme, ThemeError> {
    import_es_theme_dir(root)
}

fn path_from_relative(value: &str) -> Result<PathBuf, ThemeError> {
    validate_asset_path(value)?;
    Ok(PathBuf::from(value))
}

fn select_asset(specs: &[AssetSpec], needles: &[&str]) -> Option<AssetSpec> {
    specs
        .iter()
        .find(|spec| {
            let path = spec.path.to_ascii_lowercase();
            needles.iter().any(|needle| path.contains(needle))
        })
        .cloned()
        .or_else(|| specs.first().cloned())
}

fn imported_layout(components: &[Component]) -> Layout {
    let hero = components
        .iter()
        .find(|component| component.kind == ComponentKind::Image);
    let (hero_x, hero_y, hero_width, hero_height) = hero.map_or((32, 72, 360, 250), |component| {
        (component.x, component.y, component.width, component.height)
    });
    Layout {
        preset: LayoutPreset::Artbook,
        max_visible_games: 8,
        regions: vec![
            (
                "system-art",
                RegionKind::SystemArt,
                hero_x,
                hero_y,
                hero_width,
                hero_height,
            ),
            ("game-list", RegionKind::GameList, 420, 72, 572, 250),
            ("box-art", RegionKind::BoxArtPlaceholder, 32, 340, 176, 210),
            (
                "screenshot",
                RegionKind::ScreenshotPlaceholder,
                224,
                340,
                280,
                158,
            ),
            ("metadata", RegionKind::Metadata, 528, 340, 464, 210),
            ("menu", RegionKind::Menu, 32, 580, 960, 86),
            ("help", RegionKind::HelpStrip, 32, 690, 700, 44),
            ("clock", RegionKind::Clock, 760, 12, 104, 36),
            ("battery", RegionKind::Battery, 880, 12, 112, 36),
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
    }
}

fn parse_document(bytes: &[u8]) -> Result<(Node, CompatibilityReport), ThemeError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ThemeError::new(Reason::InvalidXml, "XML must be UTF-8"))?;
    if bytes.len() > MAX_JSON_BYTES
        || text.contains("<!")
        || text.contains("<?")
        || text.contains("&")
    {
        return Err(ThemeError::new(
            Reason::UnsupportedXml,
            "XML declarations, entities, and external content are unsupported",
        ));
    }
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut node_count = 0;
    let mut cursor = 0;
    while let Some(open) = text[cursor..].find('<') {
        let open = cursor + open;
        if !text[cursor..open].trim().is_empty() {
            if let Some(node) = stack.last_mut() {
                let text = text[cursor..open].trim();
                if node.text.len() + text.len() > MAX_XML_TEXT_BYTES {
                    return Err(ThemeError::new(
                        Reason::BudgetText,
                        "XML text exceeds 256 bytes",
                    ));
                }
                node.text.push_str(text);
            }
        }
        let end = find_tag_end(&text[open + 1..])
            .ok_or_else(|| ThemeError::new(Reason::InvalidXml, "unterminated XML tag"))?
            + open
            + 1;
        let tag = text[open + 1..end].trim();
        if tag.len() > 4096 {
            return Err(ThemeError::new(
                Reason::BudgetText,
                "XML tag exceeds 4096 bytes",
            ));
        }
        if let Some(stripped) = tag.strip_prefix('/') {
            let name = stripped.trim();
            let node = stack
                .pop()
                .ok_or_else(|| ThemeError::new(Reason::InvalidXml, "unexpected closing tag"))?;
            if node.name != name {
                return Err(ThemeError::new(
                    Reason::InvalidXml,
                    "XML tags are not balanced",
                ));
            }
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else if root.replace(node).is_some() {
                return Err(ThemeError::new(
                    Reason::InvalidXml,
                    "XML has multiple roots",
                ));
            }
        } else {
            if stack.len() >= MAX_XML_DEPTH {
                return Err(ThemeError::new(
                    Reason::BudgetFileCount,
                    "XML depth exceeds 16",
                ));
            }
            node_count += 1;
            if node_count > MAX_XML_NODES {
                return Err(ThemeError::new(
                    Reason::BudgetFileCount,
                    "XML node count exceeds 128",
                ));
            }
            let self_closing = tag.ends_with('/');
            let body = tag.trim_end_matches('/').trim();
            let (name, attrs) = parse_tag(body)?;
            if attrs.len() > MAX_XML_ATTRIBUTES {
                return Err(ThemeError::new(
                    Reason::BudgetText,
                    "XML attribute count exceeds 16",
                ));
            }
            stack.push(Node {
                name,
                attrs,
                children: Vec::new(),
                text: String::new(),
            });
            if self_closing {
                let node = stack.pop().ok_or_else(|| {
                    ThemeError::new(Reason::InvalidXml, "self-closing XML tag is incomplete")
                })?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else if root.replace(node).is_some() {
                    return Err(ThemeError::new(
                        Reason::InvalidXml,
                        "XML has multiple roots",
                    ));
                }
            }
        }
        cursor = end + 1;
    }
    if !text[cursor..].trim().is_empty() || !stack.is_empty() {
        return Err(ThemeError::new(Reason::InvalidXml, "XML is incomplete"));
    }
    let root = root.ok_or_else(|| ThemeError::new(Reason::InvalidXml, "XML has no root"))?;
    if root.name != "theme" {
        return Err(ThemeError::new(
            Reason::UnsupportedXml,
            "root element must be theme",
        ));
    }
    let mut report = CompatibilityReport {
        subset: "emulationstation-batocera-knulli-data-v1".into(),
        status: "validated".into(),
        accepted: vec!["theme".into()],
        unsupported: Vec::new(),
        rejected: Vec::new(),
    };
    check_attrs(
        &root,
        &["formatVersion", "name", "author", "license", "provenance"],
        &mut report,
    )?;
    if root_attr(&root, "formatVersion") != Some("4") {
        return Err(ThemeError::new(
            Reason::UnsupportedXml,
            "formatVersion must be literal 4",
        ));
    }
    for view in &root.children {
        if view.name != "view" {
            return Err(ThemeError::new(
                Reason::UnsupportedXml,
                format!("element {} is outside the supported subset", view.name),
            ));
        }
        check_attrs(view, &["name"], &mut report)?;
    }
    if root.children.is_empty() {
        return Err(ThemeError::new(
            Reason::InvalidXml,
            "theme must contain a view",
        ));
    }
    Ok((root, report))
}

fn components(root: &Node, report: &mut CompatibilityReport) -> Result<Vec<Component>, ThemeError> {
    let mut output = Vec::new();
    let mut ids = BTreeSet::new();
    for view in &root.children {
        for node in &view.children {
            if !matches!(node.name.as_str(), "image" | "text" | "textlist") {
                return Err(ThemeError::new(
                    Reason::UnsupportedXml,
                    format!("element {} is not in the supported subset", node.name),
                ));
            }
            check_attrs(
                node,
                &["name", "path", "pos", "size", "color", "fontSize", "text"],
                report,
            )?;
            for child in &node.children {
                if !matches!(
                    child.name.as_str(),
                    "path" | "pos" | "size" | "color" | "fontSize" | "text"
                ) || !child.children.is_empty()
                    || !child.attrs.is_empty()
                {
                    return Err(ThemeError::new(
                        Reason::UnsupportedXml,
                        format!("property {} is unsupported", child.name),
                    ));
                }
            }
            let id = attr_or_child(node, "name").map_or_else(
                || {
                    format!(
                        "{}-{}",
                        view_attr(view, "name").unwrap_or("view"),
                        output.len()
                    )
                },
                str::to_string,
            );
            if !ids.insert(id.clone()) {
                return Err(ThemeError::new(
                    Reason::InvalidLayout,
                    format!("duplicate component {id}"),
                ));
            }
            let (x, y) = pair(attr_or_child(node, "pos"), "pos")?;
            let (width, height) = pair(attr_or_child(node, "size"), "size")?;
            let path = attr_or_child(node, "path").map(str::to_string);
            if node.name == "image" && path.is_none() {
                return Err(ThemeError::new(
                    Reason::InvalidAsset,
                    format!("image {id} has no path"),
                ));
            }
            let font_size = attr_or_child(node, "fontSize")
                .map(|value| {
                    value.parse::<u16>().map_err(|_| {
                        ThemeError::new(Reason::InvalidSetting, "fontSize must be an integer")
                    })
                })
                .transpose()?;
            let color = attr_or_child(node, "color").map(str::to_string);
            if let Some(color) = &color {
                hex_color(color, "XML color")?;
            }
            let text = attr_or_child(node, "text").map(str::to_string);
            if text.as_ref().is_some_and(|value| value.len() > 256) {
                return Err(ThemeError::new(
                    Reason::BudgetText,
                    "XML text exceeds 256 bytes",
                ));
            }
            output.push(Component {
                id,
                kind: match node.name.as_str() {
                    "image" => ComponentKind::Image,
                    "text" => ComponentKind::Text,
                    _ => ComponentKind::Textlist,
                },
                x,
                y,
                width,
                height,
                path,
                text,
                color,
                font_size,
            });
            report.accepted.push(node.name.clone());
        }
    }
    if output.is_empty() {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            "XML contains no supported components",
        ));
    }
    Ok(output)
}

fn check_attrs(
    node: &Node,
    allowed: &[&str],
    report: &mut CompatibilityReport,
) -> Result<(), ThemeError> {
    let mut seen = BTreeSet::new();
    for (name, _) in &node.attrs {
        if !allowed.contains(&name.as_str()) {
            report.rejected.push(format!("{}@{}", node.name, name));
            return Err(ThemeError::new(
                Reason::UnsupportedXml,
                format!("property {name} is unsupported"),
            ));
        }
        if !seen.insert(name) {
            return Err(ThemeError::new(
                Reason::InvalidXml,
                format!("duplicate property {name}"),
            ));
        }
    }
    Ok(())
}

fn parse_tag(body: &str) -> Result<(String, Vec<(String, String)>), ThemeError> {
    let mut i = 0;
    let name = token(body, &mut i)?;
    let mut attrs = Vec::new();
    while i < body.len() {
        while body.as_bytes().get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if i == body.len() {
            break;
        }
        let key = token(body, &mut i)?;
        while body.as_bytes().get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if body.as_bytes().get(i) != Some(&b'=') {
            return Err(ThemeError::new(
                Reason::InvalidXml,
                "XML attribute has no value",
            ));
        }
        i += 1;
        while body.as_bytes().get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let quote = *body
            .as_bytes()
            .get(i)
            .ok_or_else(|| ThemeError::new(Reason::InvalidXml, "XML attribute is incomplete"))?;
        if quote != b'"' && quote != b'\'' {
            return Err(ThemeError::new(
                Reason::InvalidXml,
                "XML attributes must be quoted",
            ));
        }
        i += 1;
        let start = i;
        while body.as_bytes().get(i).is_some_and(|byte| *byte != quote) {
            i += 1;
        }
        if i == body.len() {
            return Err(ThemeError::new(
                Reason::InvalidXml,
                "XML attribute is unterminated",
            ));
        }
        attrs.push((key, body[start..i].to_string()));
        i += 1;
    }
    Ok((name, attrs))
}

fn token(body: &str, i: &mut usize) -> Result<String, ThemeError> {
    while body.as_bytes().get(*i).is_some_and(u8::is_ascii_whitespace) {
        *i += 1;
    }
    let start = *i;
    while body
        .as_bytes()
        .get(*i)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
    {
        *i += 1;
    }
    if *i == start {
        return Err(ThemeError::new(Reason::InvalidXml, "XML tag has no name"));
    }
    Ok(body[start..*i].to_string())
}

fn find_tag_end(value: &str) -> Option<usize> {
    let mut quote = None;
    for (index, byte) in value.bytes().enumerate() {
        if quote == Some(byte) {
            quote = None;
        } else if quote.is_none() && (byte == b'"' || byte == b'\'') {
            quote = Some(byte);
        } else if quote.is_none() && byte == b'>' {
            return Some(index);
        }
    }
    None
}

fn root_attr<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    node.attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}
fn view_attr<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    root_attr(node, name)
}
fn attr_or_child<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    root_attr(node, name).or_else(|| {
        node.children
            .iter()
            .find(|child| child.name == name)
            .map(|child| child.text.trim())
    })
}
fn pair(value: Option<&str>, label: &str) -> Result<(u16, u16), ThemeError> {
    let value = value
        .ok_or_else(|| ThemeError::new(Reason::InvalidLayout, format!("{label} is required")))?;
    let parts: Vec<_> = value
        .split(|char: char| char == ',' || char.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 2 {
        return Err(ThemeError::new(
            Reason::InvalidLayout,
            format!("{label} must contain two integers"),
        ));
    }
    let parse = |part: &str| {
        part.parse::<u16>().map_err(|_| {
            ThemeError::new(
                Reason::InvalidLayout,
                format!("{label} must contain integers"),
            )
        })
    };
    Ok((parse(parts[0])?, parse(parts[1])?))
}
