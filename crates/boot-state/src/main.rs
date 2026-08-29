use std::{env, path::PathBuf, process};

use anyhow::{anyhow, bail, Result};
use boot_state::{mark_healthy, prepare_pending, select, Slot};

fn main() {
    if let Err(error) = run() {
        eprintln!("boot-state failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("select") => {
            let root = root(&mut args)?;
            reject_extra(&mut args)?;
            let (slot, reason, attempts) = select(&root)?;
            println!(
                "selected={} reason={} attempts={attempts}",
                slot.as_str(),
                reason
            );
        }
        Some("mark-healthy") => {
            let root = root(&mut args)?;
            let mut evidence = [false; 5];
            for flag in args.by_ref() {
                let index = match flag.as_str() {
                    "--hal-self-check" => 0,
                    "--broker-ready" => 1,
                    "--launcher-first-frame" => 2,
                    "--writable-data" => 3,
                    "--readable-roms" => 4,
                    _ => bail!("unknown health evidence flag: {flag}"),
                };
                if evidence[index] {
                    bail!("duplicate health evidence")
                }
                evidence[index] = true;
            }
            let state = mark_healthy(&root, evidence)?;
            println!(
                "current={} previous={} attempts={} last-known-good={}",
                state.current.as_str(),
                state.previous.as_str(),
                state.attempts,
                state.last_known_good.as_str()
            );
        }
        Some("journey") => {
            let root = root(&mut args)?;
            reject_extra(&mut args)?;
            prepare_pending(&root, Slot::B, "release-b")?;
            let _ = select(&root)?;
            let _ = select(&root)?;
            let _ = select(&root)?;
            let (slot, reason, attempts) = select(&root)?;
            println!(
                "selected={} reason={} attempts={attempts}",
                slot.as_str(),
                reason
            );
        }
        Some("healthy-journey") => {
            let root = root(&mut args)?;
            reject_extra(&mut args)?;
            prepare_pending(&root, Slot::B, "release-b")?;
            let state = mark_healthy(&root, [true; 5])?;
            println!(
                "current={} previous={} attempts={} last-known-good={}",
                state.current.as_str(),
                state.previous.as_str(),
                state.attempts,
                state.last_known_good.as_str()
            );
        }
        _ => bail!(
            "usage: boot-state select|mark-healthy|journey|healthy-journey --root FIXTURE_ROOT"
        ),
    }
    Ok(())
}

fn root(args: &mut impl Iterator<Item = String>) -> Result<PathBuf> {
    match (args.next().as_deref(), args.next()) {
        (Some("--root"), Some(root)) if !root.is_empty() => Ok(PathBuf::from(root)),
        _ => Err(anyhow!("expected --root FIXTURE_ROOT")),
    }
}

fn reject_extra(args: &mut impl Iterator<Item = String>) -> Result<()> {
    if args.next().is_some() {
        bail!("unexpected argument")
    }
    Ok(())
}
