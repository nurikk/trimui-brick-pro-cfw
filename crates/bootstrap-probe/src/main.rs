use std::{env, path::PathBuf, process};

use bootstrap_probe::{print_result, probe_simulation, ProbeResult};

fn main() {
    let result = match parse_args() {
        Ok(Some(root)) => probe_simulation(&root),
        Ok(None) => ProbeResult::recovery("real-fingerprint-not-approved"),
        Err(_) => ProbeResult::recovery("simulation-interface-rejected"),
    };
    let failed = !result.handoff_eligible;
    print_result(&result);
    if failed {
        process::exit(1);
    }
}

fn parse_args() -> Result<Option<PathBuf>, ()> {
    let mut args = env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (None, None, None) => Ok(None),
        (Some("--real-device"), None, None) => Ok(None),
        (Some("--simulation-fixture-root"), Some(root), None) if !root.is_empty() => {
            Ok(Some(PathBuf::from(root)))
        }
        _ => Err(()),
    }
}
