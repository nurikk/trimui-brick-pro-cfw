#![cfg(feature = "host")]

use std::{
    path::PathBuf,
    process,
    sync::{atomic::AtomicBool, Arc},
};

use anyhow::{anyhow, Result};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    flag,
};
use sim_host_platform::{Backend, HostPlatform};

struct Args {
    profile: PathBuf,
    device_profile: PathBuf,
    catalog: PathBuf,
    evidence: PathBuf,
    backend: Backend,
    keep_alive: bool,
}

fn main() {
    let result = parse_args().and_then(execute);
    if let Err(error) = result {
        eprintln!("simulator startup failed: {error}");
        process::exit(1);
    }
}

fn parse_args() -> Result<Args> {
    let mut profile = None;
    let mut device_profile = None;
    let mut catalog = None;
    let mut evidence = None;
    let mut backend = None;
    let mut keep_alive = false;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--profile" => {
                profile = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow!("missing --profile value"))?,
                ))
            }
            "--device-profile" => {
                device_profile = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow!("missing --device-profile value"))?,
                ))
            }
            "--catalog" => {
                catalog = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow!("missing --catalog value"))?,
                ))
            }
            "--evidence" => {
                evidence = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow!("missing --evidence value"))?,
                ))
            }
            "--backend=dummy" => backend = Some(Backend::Dummy),
            "--backend=x11" => backend = Some(Backend::X11),
            "--keep-alive" => keep_alive = true,
            _ => return Err(anyhow!("unknown launcher argument")),
        }
    }
    Ok(Args {
        profile: profile.ok_or_else(|| anyhow!("missing --profile"))?,
        device_profile: device_profile.ok_or_else(|| anyhow!("missing --device-profile"))?,
        catalog: catalog.ok_or_else(|| anyhow!("missing --catalog"))?,
        evidence: evidence.ok_or_else(|| anyhow!("missing --evidence"))?,
        backend: backend.ok_or_else(|| anyhow!("missing --backend"))?,
        keep_alive,
    })
}

fn execute(args: Args) -> Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    flag::register(SIGTERM, Arc::clone(&stop)).map_err(|error| anyhow!(error.to_string()))?;
    flag::register(SIGINT, Arc::clone(&stop)).map_err(|error| anyhow!(error.to_string()))?;
    sim_launcher::run(
        &args.catalog,
        &args.evidence,
        args.keep_alive,
        &stop,
        || HostPlatform::new(&args.profile, &args.device_profile, args.backend),
    )
}
