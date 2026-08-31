use std::path::{Path, PathBuf};

use launch_contract::{Catalog, LaunchKind, LaunchRequest};

pub mod portmaster;
pub mod retroarch;
pub mod standalone;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    Success,
    Nonzero,
    Signal,
    Timeout,
    Cancel,
    SpawnError,
    Grandchild,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Nonzero => "nonzero",
            Self::Signal => "signal",
            Self::Timeout => "timeout",
            Self::Cancel => "cancel",
            Self::SpawnError => "spawn-error",
            Self::Grandchild => "grandchild",
        }
    }
}

pub struct ResolvedPaths {
    pub content: PathBuf,
    pub save: PathBuf,
    pub state: PathBuf,
}

pub struct LaunchPlan {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub adapter: &'static str,
    pub confirms_usable_save: bool,
    pub log_path: Option<PathBuf>,
}

pub fn input_environment(
    request: &LaunchRequest,
    adapter: &str,
) -> Result<Vec<(String, String)>, String> {
    let mappings = input_profile::ResolvedMappings {
        bindings: request.input.bindings.clone(),
        hotkeys: request.input.hotkeys.clone(),
    };
    let mapping = input_profile::export_adapter_input(adapter, &mappings)
        .map_err(|error| format!("input mapping export rejected: {error}"))?;
    Ok(vec![("TRIMUI_INPUT_MAPPING".into(), mapping)])
}

pub fn build_plan(
    request: &LaunchRequest,
    catalog: &Catalog,
    fixture_root: &Path,
    helper: &Path,
    paths: &ResolvedPaths,
    mode: RunMode,
) -> Result<LaunchPlan, String> {
    if !helper.is_absolute() {
        return Err("helper executable is not absolute".to_string());
    }
    match &request.kind {
        LaunchKind::Libretro => {
            retroarch::plan(request, catalog, fixture_root, helper, paths, mode)
        }
        LaunchKind::Standalone => {
            standalone::plan(request, catalog, fixture_root, helper, paths, mode)
        }
        LaunchKind::Portmaster => {
            portmaster::plan(request, catalog, fixture_root, helper, paths, mode)
        }
    }
}
