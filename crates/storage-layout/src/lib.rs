//! Shared logical-path guard for user-storage consumers.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, Result};
use serde::Serialize;

pub const MAX_LOGICAL_PATH_BYTES: usize = 256;

/// Controller-visible, writable user-storage roots. System slots are intentionally absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserRoot {
    Roms,
    BiosImports,
    PortMaster,
    Screenshots,
    Themes,
    SavesExport,
    UpdateSideload,
    UsbImport,
    NetworkImport,
}

impl UserRoot {
    pub const ALL: [Self; 9] = [
        Self::Roms,
        Self::BiosImports,
        Self::PortMaster,
        Self::Screenshots,
        Self::Themes,
        Self::SavesExport,
        Self::UpdateSideload,
        Self::UsbImport,
        Self::NetworkImport,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Roms => "roms",
            Self::BiosImports => "bios-imports",
            Self::PortMaster => "portmaster-data",
            Self::Screenshots => "screenshots",
            Self::Themes => "themes",
            Self::SavesExport => "saves-export",
            Self::UpdateSideload => "update-sideload",
            Self::UsbImport => "usb-import",
            Self::NetworkImport => "network-import",
        }
    }

    pub const fn relative_path(self) -> &'static str {
        match self {
            Self::Roms => "roms",
            Self::BiosImports => "roms/BIOS",
            Self::PortMaster => "Ports",
            Self::Screenshots => "data/screenshots",
            Self::Themes => "data/themes",
            Self::SavesExport => "data/saves/export",
            Self::UpdateSideload => "data/update/sideload",
            Self::UsbImport => "data/imports/usb",
            Self::NetworkImport => "data/imports/network",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|root| root.id() == id)
    }
}

/// Rejects every path form that storage-layout itself cannot safely address.
pub fn validate_logical_path(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_LOGICAL_PATH_BYTES
        || value.contains('\\')
        || value.as_bytes().contains(&0)
        || Path::new(value).is_absolute()
    {
        bail!("path is not a bounded relative path");
    }
    for component in Path::new(value).components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("path contains traversal or non-normal component");
        }
    }
    Ok(())
}

/// Resolves a logical child while rejecting symlinks in both the root and every existing parent.
pub fn resolve_user_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_logical_path(relative)?;
    let root_metadata = fs::symlink_metadata(root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        bail!("user storage root must be a non-symlink directory");
    }
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(component) = component else {
            unreachable!()
        };
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => bail!("path contains a symlink"),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}
