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
    if request.runner.id != "generated-standalone"
        || request.runner.version != "1.0.0"
        || !catalog.runners.iter().any(|runner| {
            runner.id == "generated-standalone"
                && runner.version == "1.0.0"
                && runner
                    .kinds
                    .contains(&launch_contract::LaunchKind::Standalone)
        })
    {
        return Err("catalog-owned standalone projection is unavailable".to_string());
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
            "--content".to_string(),
            paths.content.display().to_string(),
            "--save".to_string(),
            paths.save.display().to_string(),
            "--state".to_string(),
            paths.state.display().to_string(),
        ],
        cwd: fixture_root.to_path_buf(),
        env: Vec::new(),
        adapter: "standalone",
        confirms_usable_save: true,
        log_path: None,
    })
}
