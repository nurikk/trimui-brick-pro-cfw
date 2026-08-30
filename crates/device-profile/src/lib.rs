use std::{collections::BTreeMap, fmt, path::Path};

use serde::Deserialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceProfileError(String);

impl fmt::Display for DeviceProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DeviceProfileError {}

#[derive(Clone, Debug, Deserialize)]
struct CompatibilityDocument {
    #[serde(rename = "schemaVersion")]
    schema_version: String,
    #[serde(rename = "deviceId")]
    device_id: String,
    #[serde(rename = "targetSku")]
    target_sku: String,
    display: CompatibilityDisplay,
    #[serde(flatten)]
    _other: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompatibilityDisplay {
    expected: CompatibilityFramebuffer,
    orientation: Orientation,
    #[serde(rename = "themeAspect")]
    theme_aspect: ThemeAspect,
    #[serde(default, rename = "defaultTheme")]
    _default_theme: Option<String>,
    #[serde(default, rename = "physicalPanel")]
    physical_panel: Option<PhysicalPanel>,
    #[serde(default, rename = "status")]
    _status: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompatibilityFramebuffer {
    format: String,
    height: u16,
    #[serde(rename = "strideBytes")]
    stride_bytes: u32,
    width: u16,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Orientation {
    Landscape,
    Portrait,
}

#[derive(Clone, Copy, Debug, Deserialize)]
enum ThemeAspect {
    #[serde(rename = "4:3")]
    FourThree,
    #[serde(rename = "16:9")]
    SixteenNine,
    #[serde(rename = "3:2")]
    ThreeTwo,
    #[serde(rename = "1:1")]
    OneOne,
}

impl ThemeAspect {
    const fn dimensions(self) -> (u16, u16) {
        match self {
            Self::FourThree => (4, 3),
            Self::SixteenNine => (16, 9),
            Self::ThreeTwo => (3, 2),
            Self::OneOne => (1, 1),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::FourThree => "4:3",
            Self::SixteenNine => "16:9",
            Self::ThreeTwo => "3:2",
            Self::OneOne => "1:1",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PhysicalPanel {
    #[serde(rename = "activeWidthMm")]
    pub active_width_mm: Option<f32>,
    #[serde(rename = "activeHeightMm")]
    pub active_height_mm: Option<f32>,
    #[serde(rename = "diagonalInches")]
    pub diagonal_inches: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceProfile {
    device_id: String,
    target_sku: String,
    logical_width: u16,
    logical_height: u16,
    theme_aspect: &'static str,
    framebuffer_format: String,
    framebuffer_stride_bytes: u32,
    physical_panel: Option<PhysicalPanel>,
}

impl Eq for DeviceProfile {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualViewport {
    pub target_sku: String,
    pub width: u16,
    pub height: u16,
}

impl DeviceProfile {
    pub fn from_path(path: &Path) -> Result<Self, DeviceProfileError> {
        let bytes = std::fs::read(path)
            .map_err(|error| DeviceProfileError(format!("read device profile: {error}")))?;
        Self::from_json(&bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, DeviceProfileError> {
        let document: CompatibilityDocument = serde_json::from_slice(bytes)
            .map_err(|error| DeviceProfileError(format!("parse device profile: {error}")))?;
        if document.schema_version != "1.0.0" {
            return Err(DeviceProfileError(
                "unsupported device profile schemaVersion".into(),
            ));
        }
        valid_device_id(&document.device_id, "deviceId")?;
        if document.target_sku.is_empty()
            || document.target_sku.len() > 64
            || !document
                .target_sku
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(DeviceProfileError("targetSku is invalid".into()));
        }
        let display = document.display;
        if display.expected.width == 0 {
            return Err(DeviceProfileError("display width must be positive".into()));
        }
        if display.expected.height == 0 {
            return Err(DeviceProfileError("display height must be positive".into()));
        }
        if display.expected.format != "rgba8888" {
            return Err(DeviceProfileError("unsupported framebuffer format".into()));
        }
        if display.expected.stride_bytes < u32::from(display.expected.width) * 4 {
            return Err(DeviceProfileError(
                "framebuffer stride is smaller than one row".into(),
            ));
        }
        match display.orientation {
            Orientation::Landscape if display.expected.width < display.expected.height => {
                return Err(DeviceProfileError(
                    "landscape width must not be smaller than height".into(),
                ));
            }
            Orientation::Portrait if display.expected.height < display.expected.width => {
                return Err(DeviceProfileError(
                    "portrait height must not be smaller than width".into(),
                ));
            }
            _ => {}
        }
        let (aspect_width, aspect_height) = display.theme_aspect.dimensions();
        if u32::from(display.expected.width) * u32::from(aspect_height)
            != u32::from(display.expected.height) * u32::from(aspect_width)
        {
            return Err(DeviceProfileError(
                "theme aspect does not match display dimensions".into(),
            ));
        }
        if let Some(panel) = display.physical_panel {
            if (panel.active_width_mm.is_some() != panel.active_height_mm.is_some()
                || panel.active_width_mm.is_none() && panel.diagonal_inches.is_none())
                || [
                    panel.active_width_mm,
                    panel.active_height_mm,
                    panel.diagonal_inches,
                ]
                .into_iter()
                .flatten()
                .any(|value| !value.is_finite() || value <= 0.0)
            {
                return Err(DeviceProfileError(
                    "physical panel dimensions must be complete and positive".into(),
                ));
            }
        }
        Ok(Self {
            device_id: document.device_id,
            target_sku: document.target_sku,
            logical_width: display.expected.width,
            logical_height: display.expected.height,
            theme_aspect: display.theme_aspect.as_str(),
            framebuffer_format: display.expected.format,
            framebuffer_stride_bytes: display.expected.stride_bytes,
            physical_panel: display.physical_panel,
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }
    pub fn target_sku(&self) -> &str {
        &self.target_sku
    }
    pub const fn logical_size(&self) -> (u16, u16) {
        (self.logical_width, self.logical_height)
    }
    pub const fn theme_aspect(&self) -> &str {
        self.theme_aspect
    }
    pub fn theme_layout_file(&self) -> String {
        format!("aspect-ratio-{}.xml", self.theme_aspect.replace(':', "-"))
    }
    pub fn virtual_viewport(&self) -> VirtualViewport {
        VirtualViewport {
            target_sku: self.target_sku.clone(),
            width: self.logical_width,
            height: self.logical_height,
        }
    }
    pub fn framebuffer_format(&self) -> &str {
        &self.framebuffer_format
    }
    pub const fn framebuffer_stride_bytes(&self) -> u32 {
        self.framebuffer_stride_bytes
    }

    pub const fn physical_panel(&self) -> Option<PhysicalPanel> {
        self.physical_panel
    }

    pub fn automatic_scale_percent(&self) -> u16 {
        let Some(panel) = self.physical_panel else {
            return 120;
        };
        let ppi = if let (Some(width_mm), Some(height_mm)) =
            (panel.active_width_mm, panel.active_height_mm)
        {
            let width_inches = f64::from(width_mm) / 25.4;
            let height_inches = f64::from(height_mm) / 25.4;
            f64::from(self.logical_width).hypot(f64::from(self.logical_height))
                / width_inches.hypot(height_inches)
        } else if let Some(diagonal_inches) = panel.diagonal_inches {
            f64::from(self.logical_width).hypot(f64::from(self.logical_height))
                / f64::from(diagonal_inches)
        } else {
            return 120;
        };
        ((ppi / 160.0 * 100.0).clamp(100.0, 150.0) / 5.0).round() as u16 * 5
    }
}

fn valid_device_id(value: &str, label: &str) -> Result<(), DeviceProfileError> {
    if value.is_empty()
        || value.len() > 64
        || !value.as_bytes()[0].is_ascii_lowercase()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(DeviceProfileError(format!("{label} is invalid")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::DeviceProfile;

    #[test]
    fn automatic_scale_uses_physical_density() {
        for (width, height, diagonal, expected) in [
            (1024, 768, 4.0, 150),
            (1024, 768, 7.0, 115),
            (640, 480, 3.5, 145),
        ] {
            let json = format!(
                r#"{{"schemaVersion":"1.0.0","deviceId":"density-test","targetSku":"DENSITY","display":{{"expected":{{"format":"rgba8888","height":{height},"strideBytes":{stride},"width":{width}}},"orientation":"landscape","themeAspect":"4:3","physicalPanel":{{"diagonalInches":{diagonal}}}}}}}"#,
                stride = width * 4
            );
            let profile = DeviceProfile::from_json(json.as_bytes()).unwrap();
            assert_eq!(profile.automatic_scale_percent(), expected);
        }
    }

    #[test]
    fn automatic_scale_falls_back_and_accepts_active_dimensions() {
        let missing = DeviceProfile::from_json(include_bytes!(
            "../../../config/platform/tg4040/compatibility.json"
        ))
        .unwrap();
        assert_eq!(missing.automatic_scale_percent(), 120);

        let json = br#"{"schemaVersion":"1.0.0","deviceId":"density-mm","targetSku":"DENSITY","display":{"expected":{"format":"rgba8888","height":768,"strideBytes":4096,"width":1024},"orientation":"landscape","themeAspect":"4:3","physicalPanel":{"activeWidthMm":81.28,"activeHeightMm":60.96}}}"#;
        let profile = DeviceProfile::from_json(json).unwrap();
        assert_eq!(profile.automatic_scale_percent(), 150);
        assert!(profile.physical_panel().is_some());
    }

    #[test]
    fn density_fixture_profiles_cover_diagonal_and_active_dimensions() {
        for (source, expected) in [
            (
                include_bytes!("../../../fixtures/platform/ui-density-1024-4/compatibility.json")
                    .as_slice(),
                150,
            ),
            (
                include_bytes!("../../../fixtures/platform/ui-density-1024-7/compatibility.json")
                    .as_slice(),
                115,
            ),
            (
                include_bytes!("../../../fixtures/platform/ui-density-640-3-5/compatibility.json")
                    .as_slice(),
                145,
            ),
            (
                include_bytes!(
                    "../../../fixtures/platform/ui-density-active-mm/compatibility.json"
                )
                .as_slice(),
                150,
            ),
        ] {
            let profile = DeviceProfile::from_json(source).unwrap();
            assert_eq!(profile.automatic_scale_percent(), expected);
            assert!(profile.physical_panel().is_some());
        }
    }

    #[test]
    fn seven_inch_active_dimensions_match_diagonal_density() {
        let profile = DeviceProfile::from_json(
            br#"{
                "deviceId":"ui-density-active-mm-7",
                "display":{
                    "expected":{"format":"rgba8888","height":768,"strideBytes":4096,"width":1024},
                    "orientation":"landscape","themeAspect":"4:3",
                    "physicalPanel":{"activeWidthMm":142.24,"activeHeightMm":106.68}
                },
                "schemaVersion":"1.0.0","targetSku":"UI-DENSITY-ACTIVE-MM-7"
            }"#,
        )
        .unwrap();
        assert_eq!(profile.automatic_scale_percent(), 115);
    }

    #[test]
    fn brick_pro_profile_remains_tg4040_four_three() {
        let profile = DeviceProfile::from_json(include_bytes!(
            "../../../config/platform/tg4040/compatibility.json"
        ))
        .expect("Brick Pro device profile parses");
        assert_eq!(profile.device_id(), "tg4040");
        assert_eq!(profile.target_sku(), "TG4040");
        assert_eq!(profile.logical_size(), (1024, 768));
        assert_eq!(profile.theme_aspect(), "4:3");
        assert_eq!(profile.theme_layout_file(), "aspect-ratio-4-3.xml");
    }

    #[test]
    fn synthetic_wide_device_selects_sixteen_nine() {
        let profile = DeviceProfile::from_json(include_bytes!(
            "../../../fixtures/platform/synthetic-wide/compatibility.json"
        ))
        .expect("synthetic device profile parses");
        assert_eq!(profile.logical_size(), (1280, 720));
        assert_eq!(profile.theme_layout_file(), "aspect-ratio-16-9.xml");
        assert_eq!(
            profile.virtual_viewport(),
            super::VirtualViewport {
                target_sku: "SYNTHETIC-WIDE".into(),
                width: 1280,
                height: 720,
            }
        );
    }

    #[test]
    fn zero_width_is_rejected() {
        let error = DeviceProfile::from_json(br#"{"schemaVersion":"1.0.0","deviceId":"bad","targetSku":"BAD","display":{"expected":{"format":"rgba8888","height":720,"strideBytes":1,"width":0},"orientation":"landscape","themeAspect":"16:9"}}"#)
            .expect_err("zero width must reject");
        assert!(error.to_string().contains("display width"));
    }
}
