use std::{fs, path::PathBuf};

use super::*;

struct MemoryTransport;

impl CatalogTransport for MemoryTransport {
    fn fetch(&self, locator: &str, _: usize) -> Result<Vec<u8>, ThemeError> {
        if locator == "https://raw.githubusercontent.com/project/pocket/main/theme.json" {
            return fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../themes/default/theme.json"),
            )
            .map_err(|error| ThemeError::new(Reason::Io, error.to_string()));
        }
        if locator.starts_with("https://raw.githubusercontent.com/project/pocket/main/assets/") {
            return fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../themes/default/assets/art.png"),
            )
            .map_err(|error| ThemeError::new(Reason::Io, error.to_string()));
        }
        Err(ThemeError::new(
            Reason::Io,
            format!("unexpected URL {locator}"),
        ))
    }
}

#[test]
fn v1_fallback_is_available() -> Result<(), ThemeError> {
    let _theme = safe_artbook()?;
    Ok(())
}

#[test]
fn selected_device_aspect_selects_its_layout_file() -> Result<(), ThemeError> {
    let brick = device_profile::DeviceProfile::from_json(include_bytes!(
        "../../../config/platform/tg4040/compatibility.json"
    ))
    .map_err(|error| ThemeError::new(Reason::InvalidLayout, error.to_string()))?;
    let wide = device_profile::DeviceProfile::from_json(include_bytes!(
        "../../../fixtures/platform/synthetic-wide/compatibility.json"
    ))
    .map_err(|error| ThemeError::new(Reason::InvalidLayout, error.to_string()))?;
    let layouts = ["aspect-ratio-4-3.xml", "aspect-ratio-16-9.xml"];

    assert_eq!(
        select_theme_layout(&brick, &layouts)?,
        "aspect-ratio-4-3.xml"
    );
    assert_eq!(
        select_theme_layout(&wide, &layouts)?,
        "aspect-ratio-16-9.xml"
    );
    Ok(())
}

#[test]
fn selected_device_rejects_a_theme_with_a_different_viewport() -> Result<(), ThemeError> {
    let wide = device_profile::DeviceProfile::from_json(include_bytes!(
        "../../../fixtures/platform/synthetic-wide/compatibility.json"
    ))
    .map_err(|error| ThemeError::new(Reason::InvalidLayout, error.to_string()))?;

    let error = validate_for_device(safe_artbook()?, &wide)
        .expect_err("4:3 theme must not validate for 16:9 device");
    assert_eq!(error.reason, Reason::InvalidLayout);
    Ok(())
}

#[test]
fn external_art_book_marker_is_bounded_before_product_loading(
) -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!(
        "launcher-theme-artbook-external-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let mut marker = String::from(
        "Art Book Next (Batocera ES Edition) Anthony Caccese creative commons CC-BY-NC-SA ./aspect-ratio-4-3.xml",
    );
    marker.push_str(&"x".repeat(MAX_JSON_BYTES));
    fs::write(root.join("theme.xml"), marker)?;

    let error = load_theme_dir(&root).expect_err("oversized Art Book marker must be rejected");
    assert_eq!(error.reason, Reason::BudgetResourceBytes);
    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn brick_pro_compatibility_selects_bundled_art_book_4_3() -> Result<(), Box<dyn std::error::Error>>
{
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let compatibility: serde_json::Value = serde_json::from_slice(&fs::read(
        root.join("config/platform/tg4040/compatibility.json"),
    )?)?;
    let theme_id = compatibility["display"]["defaultTheme"].as_str().unwrap();
    let aspect = compatibility["display"]["themeAspect"]
        .as_str()
        .unwrap()
        .replace(':', "-");
    let theme = load_bundled_theme(&root.join("themes/upstream").join(theme_id), &aspect)?;
    assert_eq!(theme.name(), "Art Book Next (Batocera ES Edition)");
    assert_eq!(theme.theme().metadata.license, "CC-BY-NC-SA");
    assert!(theme
        .asset("./_inc/systems/artwork-default/genesis.png")
        .is_some());
    assert!(theme.asset("./_inc/systems/logos/genesis.png").is_some());
    assert!(theme
        .theme()
        .components
        .as_ref()
        .unwrap()
        .iter()
        .any(|component| component.path.as_deref() == Some("./_inc/systems/logos/genesis.png")));
    assert!(root
        .join("themes/upstream/art-book-next-es/aspect-ratio-4-3.xml")
        .is_file());
    Ok(())
}

#[test]
fn native_v2_assets_are_validated_before_publish() -> Result<(), ThemeError> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../themes/samples/project-v2");
    let theme = load_theme_dir(&path)?;
    assert_eq!(theme.theme().schema_version, 2);
    assert!(theme.asset("assets/art.png").is_some());
    Ok(())
}

#[test]
fn xml_subset_imports_to_v2() -> Result<(), ThemeError> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/theme-import/owned-a");
    let imported = import_es_theme_dir(&path)?;
    assert_eq!(imported.theme.theme().schema_version, 2);
    assert_eq!(imported.report.status, "imported");
    assert_eq!(
        imported.report.subset,
        "emulationstation-batocera-knulli-data-v1"
    );
    Ok(())
}

#[test]
fn imported_output_is_reloadable_native_v2() -> Result<(), Box<dyn std::error::Error>> {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/theme-import/owned-a");
    let output = std::env::temp_dir().join(format!("launcher-theme-import-{}", std::process::id()));
    let _ = fs::remove_dir_all(&output);
    let imported = import_es_theme_dir(&source)?;
    let report = imported.report.clone();
    imported.write_native_dir(&source, &output)?;
    let reloaded = load_theme_dir(&output)?;
    assert_eq!(reloaded.theme().schema_version, 2);
    assert_eq!(reloaded.name(), imported.theme.name());
    assert_eq!(report, imported.report);
    assert!(output.join("assets/art.png").is_file());
    let _ = fs::remove_dir_all(output);
    Ok(())
}

#[test]
fn scene_exposes_validated_components_and_pixels() -> Result<(), ThemeError> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../themes/default");
    let default_scene = scene(&load_theme_dir(&path)?);
    assert_eq!(default_scene.components.len(), 3);
    assert_eq!(default_scene.components[0].kind, "image");
    assert!(!default_scene.assets[0].pixels.is_empty());
    let safe_scene = scene(&safe_artbook()?);
    assert!(!safe_scene.assets[0].pixels.is_empty());
    Ok(())
}

#[test]
fn remote_catalog_loads_native_theme_and_assets() -> Result<(), ThemeError> {
    let bytes = br#"{"format":"themes-catalog-v1","schemaVersion":1,"themes":[{"id":"pocket","name":"Pocket","version":"1.0.0","locator":"https://github.com/project/pocket","author":"Project"}]}"#;
    let catalog = ThemesCatalog::parse(bytes)?;
    let theme = catalog.load_theme("pocket", "1.0.0", &MemoryTransport)?;
    assert!(theme.asset("assets/art.png").is_some());
    Ok(())
}

#[test]
fn traversal_and_xml_work_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("launcher-theme-bounds-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let mut nested = root.clone();
    for _ in 0..(MAX_THEME_DEPTH + 2) {
        nested = nested.join("nested");
    }
    fs::create_dir_all(&nested)?;
    let error = import_es_theme_dir(&root).expect_err("deep directory must be bounded");
    assert_eq!(error.reason, Reason::BudgetFileCount);
    let mut xml = String::from("<theme formatVersion=\"4\"><view>");
    for _ in 0..130 {
        xml.push_str("<text pos=\"0 0\" size=\"1 1\" text=\"x\"/>");
    }
    xml.push_str("</view></theme>");
    fs::write(root.join("theme.xml"), xml)?;
    let error = import_es_theme_dir(&root).expect_err("large XML tree must be bounded");
    assert_eq!(error.reason, Reason::BudgetFileCount);
    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn importer_keeps_named_media_roles_distinct() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("launcher-theme-media-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("assets"))?;
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../themes/default/assets");
    for name in ["art.png", "box-art.png", "screenshot.png"] {
        fs::copy(source.join(name), root.join("assets").join(name))?;
    }
    fs::write(
        root.join("theme.xml"),
        r#"<theme formatVersion="4" name="Media Roles"><view name="system"><image path="assets/art.png" pos="0 0" size="1 1"/><image path="assets/box-art.png" pos="0 0" size="1 1"/><image path="assets/screenshot.png" pos="0 0" size="1 1"/></view></theme>"#,
    )?;
    let imported = import_es_theme_dir(&root)?;
    let assets = imported.theme.theme().assets.as_ref().expect("assets");
    assert_eq!(assets.system_art.as_ref().unwrap().path, "assets/art.png");
    assert_eq!(assets.box_art.as_ref().unwrap().path, "assets/box-art.png");
    assert_eq!(
        assets.screenshot.as_ref().unwrap().path,
        "assets/screenshot.png"
    );
    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn luma_station_switch_changes_composition() -> Result<(), ThemeError> {
    let artbook = safe_artbook()?;
    let luma = crate::luma_station()?;
    assert_ne!(artbook.name(), luma.name());
    assert_ne!(
        artbook.theme().colors.background,
        luma.theme().colors.background
    );
    assert_ne!(
        artbook.theme().layout.regions[0].x,
        luma.theme().layout.regions[0].x
    );
    Ok(())
}

#[test]
fn renderer_emits_1024x768_png_for_valid_theme() -> Result<(), Box<dyn std::error::Error>> {
    let path =
        std::env::temp_dir().join(format!("launcher-theme-render-{}.png", std::process::id()));
    render_png(&safe_artbook()?, &path)?;
    let bytes = fs::read(&path)?;
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        (&bytes[16..20], &bytes[20..24]),
        (&[0, 0, 4, 0][..], &[0, 0, 3, 0][..])
    );
    let _ = fs::remove_file(path);
    Ok(())
}

#[test]
fn real_batocera_theme_slices_import_as_distinct_sources() -> Result<(), ThemeError> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../themes/imported");
    let simple = import_es_theme_dir(&root.join("simplelife"))?;
    let techdweeb = import_es_theme_dir(&root.join("techdweeb"))?;
    assert_eq!(simple.theme.name(), "SimpleLife");
    assert_eq!(techdweeb.theme.name(), "Techdweeb");
    assert_eq!(
        simple.theme.theme().metadata.author,
        "DarrenCarol / Mr. Overlay"
    );
    assert_eq!(
        techdweeb.theme.theme().metadata.author,
        "TechDweeb; XML by Anthony Caccese"
    );
    assert_eq!(simple.theme.theme().metadata.license, "not-stated");
    assert_eq!(techdweeb.theme.theme().metadata.license, "CC-BY-NC-SA");
    assert!(simple.theme.asset("assets/hero.png").is_some());
    assert!(techdweeb.theme.asset("assets/hero.png").is_some());
    assert_ne!(
        simple.theme.asset("assets/hero.png").unwrap().1,
        techdweeb.theme.asset("assets/hero.png").unwrap().1
    );
    assert_eq!(simple.report.status, "imported");
    assert_eq!(techdweeb.report.status, "imported");
    Ok(())
}

#[test]
fn themes_json_catalog_rejects_unsafe_locator() -> Result<(), Box<dyn std::error::Error>> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/theme-import/themes.json");
    let catalog = ThemesCatalog::parse(&fs::read(path)?)?;
    assert!(catalog.select("owned-a", "1.0.0").is_ok());
    let bad = br#"{"format":"themes-catalog-v1","schemaVersion":1,"themes":[{"id":"x","name":"X","version":"1.0.0","locator":"file:///tmp/theme","author":"X"}]}"#;
    assert!(ThemesCatalog::parse(bad).is_err());
    let feed = br#"{"data":[{"theme":"Pocket","author":"Project","theme_url":"https://github.com/project/pocket","last_update":"2024-01-01","up_to_date":"0","size":"10","screenshot":"themes/Pocket.jpg"}]}"#;
    let feed = ThemesCatalog::parse(feed)?;
    assert_eq!(
        feed.select("Pocket", "1.0.0")?.locator,
        "https://github.com/project/pocket"
    );
    Ok(())
}

#[test]
fn xml_scripts_and_traversal_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("launcher-theme-negative-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(
        root.join("theme.xml"),
        "<theme formatVersion=\"4\"><view><script/></view></theme>",
    )?;
    let error = match import_es_theme_dir(&root) {
        Ok(_) => return Err("script must be rejected".into()),
        Err(error) => error,
    };
    assert_eq!(error.reason, Reason::UnsupportedXml);
    fs::write(root.join("theme.xml"), "<theme formatVersion=\"4\"><view><image path=\"../escape.png\" pos=\"0 0\" size=\"1 1\"/></view></theme>")?;
    let error = match import_es_theme_dir(&root) {
        Ok(_) => return Err("traversal must be rejected".into()),
        Err(error) => error,
    };
    assert_eq!(error.reason, Reason::InvalidPath);
    let _ = fs::remove_dir_all(root);
    Ok(())
}
