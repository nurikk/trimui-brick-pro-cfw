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
    if request.runner.id != "generated-port"
        || request.runner.version != "1.0.0"
        || !catalog.runners.iter().any(|runner| {
            runner.id == "generated-port"
                && runner.version == "1.0.0"
                && runner.kinds.contains(&launch_contract::LaunchKind::Port)
        })
    {
        return Err("catalog-owned port projection is unavailable".to_string());
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
            "--runtime".to_string(),
            "generated-portmaster-runtime".to_string(),
            "--package".to_string(),
            "generated-port-package".to_string(),
            "--content".to_string(),
            paths.content.display().to_string(),
        ],
        cwd: fixture_root.to_path_buf(),
        adapter: "port",
        confirms_usable_save: false,
    })
}
