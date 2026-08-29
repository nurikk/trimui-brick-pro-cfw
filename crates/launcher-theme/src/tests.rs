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
