use std::path::Path;

use launch_contract::{Catalog, LaunchRequest};

use super::{LaunchPlan, ResolvedPaths, RunMode};

pub fn plan(
    request: &LaunchRequest,
    catalog: &Catalog,
    fixture_root: &Path,
    helper: &Path,
    paths: &ResolvedPaths,
    mode: RunMode,
) -> Result<LaunchPlan, String> {
    if request.runner.id != "generated-libretro"
        || request.runner.version != "1.0.0"
        || request
            .core
            .as_ref()
            .map(|core| (core.id.as_str(), core.version.as_str()))
            != Some(("generated-core", "1.0.0"))
        || !catalog.runners.iter().any(|runner| {
            runner.id == "generated-libretro"
                && runner.version == "1.0.0"
                && runner
                    .kinds
                    .contains(&launch_contract::LaunchKind::Libretro)
        })
    {
        return Err("catalog-owned RetroArch projection is unavailable".to_string());
    }
    let executable = if mode == RunMode::SpawnError {
        fixture_root.join("generated-helper-missing")
    } else {
        helper.to_path_buf()
    };
    Ok(LaunchPlan {
        executable,
        args: vec![
            "--scenario".to_string(),
            mode.as_str().to_string(),
            "--config".to_string(),
            fixture_root
                .join("config/retroarch/generated.cfg")
                .display()
                .to_string(),
            "-L".to_string(),
            fixture_root
                .join("cores/generated-core.so")
                .display()
                .to_string(),
            paths.content.display().to_string(),
        ],
        cwd: fixture_root.to_path_buf(),
        env: Vec::new(),
        adapter: "retroarch",
        confirms_usable_save: true,
    })
}
