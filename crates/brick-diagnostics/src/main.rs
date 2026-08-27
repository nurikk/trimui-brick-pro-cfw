use std::{env, path::PathBuf, process};

use brick_diagnostics::{export_bundle, persist_crash, safe_mode_report};

fn main() {
    let result = match parse_args() {
        Ok(Action::Present(root)) => safe_mode_report(&root).and_then(|report| {
            serde_json::to_string(&report).map_err(|_| "serialization-failed".to_string())
        }),
        Ok(Action::Persist(root)) => persist_crash(&root).map(|_| {
            "{\"schema\":\"brickpro-diagnostics-result/v1\",\"status\":\"crash-persisted\"}"
                .to_string()
        }),
        Ok(Action::Export(root, destination)) => {
            export_bundle(&root, &destination).and_then(|result| {
                serde_json::to_string(&result).map_err(|_| "serialization-failed".to_string())
            })
        }
        Err(_) => Err("diagnostics-interface-rejected".to_string()),
    };
    match result {
        Ok(output) => println!("{output}"),
        Err(_) => {
            println!("{{\"schema\":\"brickpro-diagnostics-result/v1\",\"status\":\"denied\",\"reason\":\"diagnostics-interface-rejected\"}}");
            process::exit(1);
        }
    }
}

enum Action {
    Present(PathBuf),
    Persist(PathBuf),
    Export(PathBuf, PathBuf),
}

fn parse_args() -> Result<Action, ()> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("--simulation-fixture-root") {
        return Err(());
    }
    let root = PathBuf::from(args.next().filter(|value| !value.is_empty()).ok_or(())?);
    match args.next().as_deref() {
        Some("--present-safe-mode") if args.next().is_none() => Ok(Action::Present(root)),
        Some("--persist-crash") if args.next().is_none() => Ok(Action::Persist(root)),
        Some("--export-support-bundle") => {
            let destination =
                PathBuf::from(args.next().filter(|value| !value.is_empty()).ok_or(())?);
            if args.next().is_some() {
                return Err(());
            }
            Ok(Action::Export(root, destination))
        }
        _ => Err(()),
    }
}
