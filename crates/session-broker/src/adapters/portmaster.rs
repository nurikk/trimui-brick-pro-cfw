use std::path::Path;

use launch_contract::{Catalog, LaunchKind, LaunchRequest};
use package_manager::resolve_portmaster;

use super::{LaunchPlan, ResolvedPaths, RunMode};

pub fn plan(
    request: &LaunchRequest,
    catalog: &Catalog,
    fixture_root: &Path,
    _helper: &Path,
    paths: &ResolvedPaths,
    mode: RunMode,
) -> Result<LaunchPlan, String> {
    if request.runner.id != "generated-portmaster"
        || request.runner.version != "1.0.0"
        || request.kind != LaunchKind::Portmaster
        || !catalog.runners.iter().any(|runner| {
            runner.id == request.runner.id
                && runner.version == request.runner.version
                && runner.kinds.contains(&LaunchKind::Portmaster)
        })
    {
        return Err("catalog-owned PortMaster projection is unavailable".to_string());
    }
    let package = request
        .package
        .as_ref()
        .ok_or_else(|| "PortMaster package identity is missing".to_string())?;
    let activation = resolve_portmaster(fixture_root, &package.id, &package.version)
        .map_err(|error| format!("PortMaster activation rejected: {error}"))?;
    let library_root = activation.library_root.clone();
    Ok(LaunchPlan {
        executable: activation.entrypoint,
        args: vec![
            "--scenario".to_string(),
            mode.as_str().to_string(),
            "--content".to_string(),
            paths.content.display().to_string(),
            "--save".to_string(),
            paths.save.display().to_string(),
            "--state".to_string(),
            paths.state.display().to_string(),
        ]
        .into_iter()
        .chain(
            (request.resume_mode != launch_contract::ResumeMode::Fresh)
                .then_some("--resume".to_string()),
        )
        .collect(),
        cwd: activation.package_root,
        env: vec![
            (
                "PORTMASTER_RUNTIME_ROOT".to_string(),
                activation.runtime_root.display().to_string(),
            ),
            (
                "PORTMASTER_LIBRARY_PATH".to_string(),
                library_root.display().to_string(),
            ),
        ],
        adapter: "portmaster",
        confirms_usable_save: true,
    })
}
