use std::{env, fs, path::Path};

#[cfg(unix)]
use std::{ffi::CString, os::unix::ffi::OsStrExt};

use launch_contract::{Catalog, LaunchKind, LaunchRequest};
use package_manager::{portmaster_user_paths, resolve_portmaster, PortArchitecture, PortGraphics};

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
    let activation =
        resolve_portmaster(fixture_root, &package.id, &package.version).map_err(|error| {
            let error = error.to_string();
            error.strip_prefix("PortMaster library ").map_or_else(
                || format!("PortMaster activation rejected: {error}"),
                |library| format!("missing-library:{library}"),
            )
        })?;
    let user_paths = portmaster_user_paths(fixture_root)
        .map_err(|error| format!("PortMaster user paths unavailable: {error}"))?;
    preflight(&activation, paths)?;
    let library_root = activation.library_root.clone();
    let writable_root = paths
        .save
        .parent()
        .ok_or_else(|| "writable-path-unavailable".to_string())?;
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
        env: {
            let mut env = super::input_environment(request, "portmaster")?;
            env.extend([
                (
                    "PORTMASTER_RUNTIME_ROOT".to_string(),
                    activation.runtime_root.display().to_string(),
                ),
                (
                    "PORTMASTER_RUNTIME_VERSION".to_string(),
                    activation.runtime.runtime_version,
                ),
                (
                    "PORTMASTER_LIBRARY_PATH".to_string(),
                    library_root.display().to_string(),
                ),
                (
                    "LD_LIBRARY_PATH".to_string(),
                    library_root.display().to_string(),
                ),
                ("HOME".to_string(), writable_root.display().to_string()),
                (
                    "XDG_DATA_HOME".to_string(),
                    writable_root.display().to_string(),
                ),
                (
                    "XDG_CONFIG_HOME".to_string(),
                    paths
                        .state
                        .parent()
                        .unwrap_or(writable_root)
                        .display()
                        .to_string(),
                ),
                (
                    "PORTMASTER_IMPORTS".to_string(),
                    user_paths.imports.display().to_string(),
                ),
            ]);
            env
        },
        adapter: "portmaster",
        confirms_usable_save: true,
        log_path: Some(user_paths.logs.join(format!("{}.log", request.request_id))),
    })
}

fn preflight(
    activation: &package_manager::PortMasterActivation,
    paths: &ResolvedPaths,
) -> Result<(), String> {
    let expected_architecture = match activation.runtime.architecture {
        PortArchitecture::Armv7 => "armv7",
        PortArchitecture::Aarch64 => "aarch64",
    };
    let architecture = env::var("BRICKPRO_PORTMASTER_ARCH").unwrap_or_else(|_| "aarch64".into());
    if architecture != expected_architecture
        && (expected_architecture != "armv7" || architecture != "aarch64")
    {
        return Err(format!("incompatible-architecture:{expected_architecture}"));
    }
    for library in &activation.runtime.libraries {
        if !activation
            .library_root
            .join(format!("{library}.library"))
            .is_file()
        {
            return Err(format!("missing-library:{library}"));
        }
    }
    let capabilities = env::var("BRICKPRO_PORTMASTER_CAPABILITIES")
        .unwrap_or_else(|_| "sdl,opengl,gl4es,weston,audio,input,network".into());
    let capabilities = capabilities.split(',').map(str::trim).collect::<Vec<_>>();
    for graphics in &activation.runtime.graphics {
        let name = match graphics {
            PortGraphics::Sdl => "sdl",
            PortGraphics::Opengl => "opengl",
            PortGraphics::Gl4es => "gl4es",
            PortGraphics::Weston => "weston",
        };
        if !capabilities.contains(&name) {
            return Err(format!("graphics-capability-unavailable:{name}"));
        }
    }
    if !capabilities.contains(&"audio") || !capabilities.contains(&"input") {
        return Err("audio-or-input-unavailable".to_string());
    }
    if activation.runtime.network && !capabilities.contains(&"network") {
        return Err("network-unavailable".to_string());
    }
    for path in [&paths.save, &paths.state] {
        let Some(parent) = path.parent() else {
            return Err("writable-path-unavailable".to_string());
        };
        if !parent.is_dir()
            || fs::metadata(parent)
                .map(|metadata| metadata.permissions().readonly())
                .unwrap_or(true)
        {
            return Err("writable-path-unavailable".to_string());
        }
        if available_bytes(parent)? < activation.runtime.min_free_bytes {
            return Err("insufficient-free-space".to_string());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn available_bytes(path: &Path) -> Result<u64, String> {
    let path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| "writable-path-unavailable")?;
    let mut status = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err("writable-path-unavailable".to_string());
    }
    let status = unsafe { status.assume_init() };
    status
        .f_bavail
        .checked_mul(status.f_frsize)
        .ok_or_else(|| "insufficient-free-space".to_string())
}

#[cfg(not(unix))]
fn available_bytes(_path: &Path) -> Result<u64, String> {
    Ok(u64::MAX)
}
