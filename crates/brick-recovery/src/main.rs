use std::{env, fs, io::Write, path::PathBuf, process};

use bootstrap_probe::probe_simulation;
use brick_diagnostics::{safe_mode_report, SupportReport};
use serde::Serialize;

const CHOICES: [&str; 3] = [
    "previous-userspace-release",
    "safe-mode",
    "stock-passthrough",
];
const ACTIONS: [&str; 5] = [
    "reset-ui-profile",
    "disable-last-module-or-theme",
    "choose-internal-storage",
    "retry-index",
    "previous-update-slot",
];
const NEXT_BOOT_MARKER: &str = ".brickpro/data/recovery-next-boot";
const RECOVERY_DATA: &str = ".brickpro/data/recovery";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryResult {
    schema: &'static str,
    status: &'static str,
    reason: &'static str,
    choices: [&'static str; 3],
    selected: Option<&'static str>,
    selection_source: Option<&'static str>,
    activating: bool,
    actions: [&'static str; 5],
    #[serde(skip_serializing_if = "Option::is_none")]
    applied_action: Option<&'static str>,
    cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    safe_mode_presentation: Option<SupportReport>,
}

fn main() {
    let outcome = match parse_args() {
        Ok(Args::RealDeviceDenied) => recovery(
            "real-fingerprint-not-approved",
            None,
            None,
            None,
            None,
            false,
        ),
        Ok(Args::Simulation { root, command }) => simulation(&root, command),
        Err(_) => recovery(
            "simulation-interface-rejected",
            None,
            None,
            None,
            None,
            false,
        ),
    };
    let selected = outcome.selected.is_some() || outcome.cancelled;
    println!(
        "{}",
        serde_json::to_string(&outcome).expect("recovery result is serializable")
    );
    if !selected {
        process::exit(1);
    }
}

enum Args {
    RealDeviceDenied,
    Simulation { root: PathBuf, command: Command },
}

enum Command {
    Select(Option<&'static str>),
    ScheduleSafeMode,
    ApplyAction(&'static str),
    Cancel,
}

fn parse_args() -> Result<Args, ()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--real-device-denied") if args.next().is_none() => Ok(Args::RealDeviceDenied),
        Some("--simulation-fixture-root") => {
            let root = args.next().filter(|value| !value.is_empty()).ok_or(())?;
            let command = match args.next().as_deref() {
                None => Command::Select(None),
                Some("--select") => {
                    Command::Select(Some(choice(&args.next().ok_or(())?).ok_or(())?))
                }
                Some("--schedule-safe-mode") => Command::ScheduleSafeMode,
                Some("--apply-action") => {
                    Command::ApplyAction(action(&args.next().ok_or(())?).ok_or(())?)
                }
                Some("--cancel") => Command::Cancel,
                _ => return Err(()),
            };
            if args.next().is_some() {
                return Err(());
            }
            Ok(Args::Simulation {
                root: PathBuf::from(root),
                command,
            })
        }
        _ => Err(()),
    }
}

fn simulation(root: &std::path::Path, command: Command) -> RecoveryResult {
    let probe = probe_simulation(root);
    if matches!(command, Command::Cancel) {
        return recovery(probe.reason, None, None, None, None, true);
    }
    if matches!(command, Command::ScheduleSafeMode) {
        return match schedule_safe_mode(root, &probe) {
            Ok(()) => recovery(
                probe.reason,
                Some("safe-mode"),
                Some("next-boot-request"),
                None,
                None,
                false,
            ),
            Err(()) => recovery("recovery-request-denied", None, None, None, None, false),
        };
    }
    if let Command::ApplyAction(action) = command {
        return match apply_action(root, &probe, action) {
            Ok(()) => recovery(
                probe.reason,
                Some("safe-mode"),
                Some("recovery-action"),
                Some(action),
                presentation(root, Some("safe-mode")),
                false,
            ),
            Err(()) => recovery("recovery-action-denied", None, None, None, None, false),
        };
    }
    let explicit = match command {
        Command::Select(value) => value,
        _ => unreachable!(),
    };
    let next_boot = explicit
        .is_none()
        .then(|| read_marker(root, NEXT_BOOT_MARKER));
    if probe.status == "recovery" || next_boot.as_ref().is_some_and(Option::is_some) {
        if matches!(
            probe.reason,
            "simulation-interface-rejected"
                | "fixture-invalid"
                | "model-identity-missing"
                | "target-sku-mismatch"
        ) {
            return recovery(probe.reason, None, None, None, None, false);
        }
        let (selected, source) = match explicit {
            Some(choice) => (Some(choice), Some("command-line")),
            None => match next_boot.expect("next-boot marker was evaluated") {
                Some(Ok(choice)) => (Some(choice), Some("next-boot-marker")),
                Some(Err(())) => {
                    return recovery("recovery-marker-invalid", None, None, None, None, false)
                }
                None => match read_marker(root, ".brickpro/data/recovery-button-chord") {
                    Some(Ok(choice)) => (Some(choice), Some("button-chord-marker")),
                    Some(Err(())) => {
                        return recovery("recovery-marker-invalid", None, None, None, None, false)
                    }
                    None => (None, None),
                },
            },
        };
        if source == Some("next-boot-marker") {
            let _ = fs::remove_file(root.join(NEXT_BOOT_MARKER));
        }
        let reason = if probe.status == "recovery" {
            probe.reason
        } else {
            "safe-mode-requested"
        };
        return recovery(
            reason,
            selected,
            source,
            None,
            presentation(root, selected),
            false,
        );
    }
    recovery(
        "recovery-not-required",
        explicit,
        Some("command-line"),
        None,
        presentation(root, explicit),
        false,
    )
}

fn recovery(
    reason: &'static str,
    selected: Option<&'static str>,
    selection_source: Option<&'static str>,
    applied_action: Option<&'static str>,
    safe_mode_presentation: Option<SupportReport>,
    cancelled: bool,
) -> RecoveryResult {
    RecoveryResult {
        schema: "brickpro-recovery/v1",
        status: "recovery",
        reason,
        choices: CHOICES,
        selected,
        selection_source,
        activating: false,
        actions: ACTIONS,
        applied_action,
        cancelled,
        safe_mode_presentation,
    }
}

fn presentation(root: &std::path::Path, selected: Option<&'static str>) -> Option<SupportReport> {
    (selected == Some("safe-mode"))
        .then(|| safe_mode_report(root).unwrap_or_else(|_| SupportReport::unavailable()))
}

fn read_marker(root: &std::path::Path, relative: &str) -> Option<Result<&'static str, ()>> {
    let bytes = fs::read(root.join(relative)).ok()?;
    if bytes.len() > 64 {
        return Some(Err(()));
    }
    let value = match std::str::from_utf8(&bytes) {
        Ok(value) => value.trim(),
        Err(_) => return Some(Err(())),
    };
    Some(choice(value).ok_or(()))
}

fn choice(value: &str) -> Option<&'static str> {
    CHOICES.iter().copied().find(|choice| *choice == value)
}

fn action(value: &str) -> Option<&'static str> {
    ACTIONS.iter().copied().find(|action| *action == value)
}

fn schedule_safe_mode(
    root: &std::path::Path,
    probe: &bootstrap_probe::ProbeResult,
) -> Result<(), ()> {
    if probe.status != "compatible" {
        return Err(());
    }
    write_recovery_file(root, NEXT_BOOT_MARKER, b"safe-mode\n")
}

fn apply_action(
    root: &std::path::Path,
    probe: &bootstrap_probe::ProbeResult,
    action: &'static str,
) -> Result<(), ()> {
    if probe.status != "compatible" {
        return Err(());
    }
    if action == "previous-update-slot" {
        return write_recovery_file(root, NEXT_BOOT_MARKER, b"previous-userspace-release\n");
    }
    write_recovery_file(root, &format!("{RECOVERY_DATA}/{action}"), b"requested\n")
}

fn write_recovery_file(root: &std::path::Path, relative: &str, value: &[u8]) -> Result<(), ()> {
    let path = root.join(relative);
    let parent = path.parent().ok_or(())?;
    let mut current = root.to_path_buf();
    for component in parent.strip_prefix(root).map_err(|_| ())?.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Err(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|_| ())?
            }
            Err(_) => return Err(()),
        }
    }
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| ())?;
    file.write_all(value).map_err(|_| ())?;
    file.sync_all().map_err(|_| ())?;
    drop(file);
    fs::rename(temporary, path).map_err(|_| ())
}
