use std::{env, fs, path::PathBuf, process};

use bootstrap_probe::probe_simulation;
use serde::Serialize;

const CHOICES: [&str; 3] = [
    "previous-userspace-release",
    "safe-mode",
    "stock-passthrough",
];

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
}

fn main() {
    let outcome = match parse_args() {
        Ok(Args::RealDeviceDenied) => recovery("real-fingerprint-not-approved", None, None),
        Ok(Args::Simulation { root, selection }) => simulation(&root, selection),
        Err(_) => recovery("simulation-interface-rejected", None, None),
    };
    let selected = outcome.selected.is_some();
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
    Simulation {
        root: PathBuf,
        selection: Option<&'static str>,
    },
}

fn parse_args() -> Result<Args, ()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--real-device-denied") if args.next().is_none() => Ok(Args::RealDeviceDenied),
        Some("--simulation-fixture-root") => {
            let root = args.next().filter(|value| !value.is_empty()).ok_or(())?;
            let mut selection = None;
            while let Some(argument) = args.next() {
                if argument != "--select" {
                    return Err(());
                }
                let value = args.next().ok_or(())?;
                let value = choice(value.as_str()).ok_or(())?;
                if selection.replace(value).is_some() {
                    return Err(());
                }
            }
            Ok(Args::Simulation {
                root: PathBuf::from(root),
                selection,
            })
        }
        _ => Err(()),
    }
}

fn simulation(root: &std::path::Path, explicit: Option<&'static str>) -> RecoveryResult {
    let probe = probe_simulation(root);
    if probe.status == "recovery" {
        if matches!(
            probe.reason,
            "simulation-interface-rejected"
                | "fixture-invalid"
                | "model-identity-missing"
                | "target-sku-mismatch"
        ) {
            return recovery(probe.reason, None, None);
        }
        let (selected, source) = match explicit {
            Some(choice) => (Some(choice), Some("command-line")),
            None => match read_marker(root, ".brickpro/data/recovery-next-boot") {
                Some(Ok(choice)) => (Some(choice), Some("next-boot-marker")),
                Some(Err(())) => return recovery("recovery-marker-invalid", None, None),
                None => match read_marker(root, ".brickpro/data/recovery-button-chord") {
                    Some(Ok(choice)) => (Some(choice), Some("button-chord-marker")),
                    Some(Err(())) => return recovery("recovery-marker-invalid", None, None),
                    None => (None, None),
                },
            },
        };
        return recovery(probe.reason, selected, source);
    }
    recovery("recovery-not-required", explicit, Some("command-line"))
}

fn recovery(
    reason: &'static str,
    selected: Option<&'static str>,
    selection_source: Option<&'static str>,
) -> RecoveryResult {
    RecoveryResult {
        schema: "brickpro-recovery/v1",
        status: "recovery",
        reason,
        choices: CHOICES,
        selected,
        selection_source,
        activating: false,
    }
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
