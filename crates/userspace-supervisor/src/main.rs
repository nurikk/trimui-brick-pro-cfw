use std::{
    env, fs,
    io::Write,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    process::{self, Child, Command, Stdio},
};

use anyhow::{anyhow, bail, Result};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    schema: &'static str,
    readiness: bool,
    trace: Vec<String>,
    shutdown_order: Vec<String>,
    launcher_restarts: u8,
    broker_restarts: u8,
    reaped_children: u8,
    session_failure: bool,
    boot_health_failure: bool,
    controlled_next_boot_recovery: bool,
    pending_blessed: bool,
}

struct Service {
    name: &'static str,
    child: Child,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("userspace-supervisor failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() == Some("--simulation-fixture-root") {
        let root = args
            .next()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("expected simulation fixture root"))?;
        if args.next().is_some() {
            bail!("unexpected simulation arguments")
        }
        return simulation_handoff(&root);
    }
    let mut args = env::args().skip(1);
    if args.next().as_deref() == Some("--child") {
        if args.next().is_none() || args.next().is_some() {
            bail!("child mode requires one service name")
        }
        return child_mode();
    }
    let command = env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("missing command"))?;
    if command != "simulate" && command != "journey" {
        bail!("usage: userspace-supervisor simulate --root ROOT --scenario NAME")
    }
    let mut args = env::args().skip(2);
    let root = match (args.next().as_deref(), args.next()) {
        (Some("--root"), Some(value)) if !value.is_empty() => PathBuf::from(value),
        _ => bail!("expected --root ROOT"),
    };
    let scenario = match (args.next().as_deref(), args.next()) {
        (Some("--scenario"), Some(value)) if !value.is_empty() => value,
        _ => bail!("expected --scenario healthy|launcher-restart|broker-restart|session-failure|essential-failure|hal-loss|shutdown"),
    };
    if args.next().is_some() {
        bail!("unexpected argument")
    }
    simulate(&root, &scenario)
}

fn child_mode() -> Result<()> {
    std::thread::sleep(std::time::Duration::from_secs(60));
    Ok(())
}

fn simulation_handoff(root: &Path) -> Result<()> {
    if !root.is_absolute() || root == Path::new("/") || root.to_string_lossy().contains("/dev/") {
        bail!("simulation root must be an absolute fixture root")
    }
    let state = root.join(".brickpro/data/update");
    fs::create_dir_all(&state)?;
    fs::write(
        state.join("supervisor-handoff.json"),
        br#"{"schema":"brickpro-supervisor-handoff/v1","mode":"synthetic","handoff":"accepted","activating":false}"#,
    )?;
    println!("simulation handoff accepted");
    Ok(())
}

fn spawn_service(
    name: &'static str,
    services: &mut Vec<Service>,
    trace: &mut Vec<String>,
) -> Result<()> {
    let executable = env::current_exe()?;
    let mut command = if let Some(qemu) = env::var_os("BRICKPRO_QEMU_USER") {
        let mut command = Command::new(qemu);
        command.arg(&executable);
        command
    } else {
        Command::new(&executable)
    };
    let child = command
        .args(["--child", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    trace.push(format!("{name}-start"));
    services.push(Service { name, child });
    match name {
        "hal" => trace.push("hal-ready".into()),
        "broker" => trace.push("broker-ready".into()),
        "launcher" => trace.push("launcher-first-frame".into()),
        _ => {}
    }
    Ok(())
}

fn reap_service(
    name: &'static str,
    services: &mut Vec<Service>,
    trace: &mut Vec<String>,
) -> Result<()> {
    let position = services
        .iter()
        .position(|service| service.name == name)
        .ok_or_else(|| anyhow!("service {name} is not running"))?;
    let mut service = services.remove(position);
    let _ = service.child.kill();
    service.child.wait()?;
    trace.push(format!("{name}-reaped"));
    Ok(())
}

fn restart_service(
    name: &'static str,
    services: &mut Vec<Service>,
    trace: &mut Vec<String>,
    attempts: &mut u8,
) -> Result<()> {
    if *attempts >= 2 {
        bail!("{name} restart bound exceeded")
    }
    *attempts += 1;
    trace.push(format!("{name}-restart-attempt-{}", *attempts));
    reap_service(name, services, trace)?;
    spawn_service(name, services, trace)
}

fn stop_service(
    name: &'static str,
    services: &mut Vec<Service>,
    trace: &mut Vec<String>,
    shutdown_order: &mut Vec<String>,
) -> Result<()> {
    if services.iter().any(|service| service.name == name) {
        shutdown_order.push(format!("{name}-stop"));
        trace.push(format!("{name}-stop"));
        reap_service(name, services, trace)?;
    }
    Ok(())
}

fn simulate(root: &Path, scenario: &str) -> Result<()> {
    if !matches!(
        scenario,
        "healthy"
            | "launcher-restart"
            | "broker-restart"
            | "session-failure"
            | "essential-failure"
            | "hal-loss"
            | "shutdown"
    ) {
        bail!("unknown supervisor scenario")
    }
    if !root.is_absolute() || root == Path::new("/") || root.to_string_lossy().contains("/dev/") {
        bail!("supervisor root must be an absolute fixture root")
    }
    let socket = root.join(".brickpro/data/update/supervisor.sock");
    fs::create_dir_all(socket.parent().expect("fixed socket parent"))?;
    let _ = fs::remove_file(&socket);
    let listener =
        UnixListener::bind(&socket).map_err(|error| anyhow!("typed readiness socket: {error}"))?;
    let mut status = Status {
        schema: "brickpro-userspace-supervisor/v1",
        readiness: false,
        trace: Vec::new(),
        shutdown_order: Vec::new(),
        launcher_restarts: 0,
        broker_restarts: 0,
        reaped_children: 0,
        session_failure: false,
        boot_health_failure: false,
        controlled_next_boot_recovery: false,
        pending_blessed: false,
    };
    let mut services = Vec::new();
    spawn_service("hal", &mut services, &mut status.trace)?;
    spawn_service("broker", &mut services, &mut status.trace)?;
    spawn_service("launcher", &mut services, &mut status.trace)?;
    status.readiness = true;
    match scenario {
        "healthy" => {}
        "launcher-restart" => restart_service(
            "launcher",
            &mut services,
            &mut status.trace,
            &mut status.launcher_restarts,
        )?,
        "broker-restart" => restart_service(
            "broker",
            &mut services,
            &mut status.trace,
            &mut status.broker_restarts,
        )?,
        "session-failure" => {
            status.session_failure = true;
            status.trace.push("session-result-failure".into());
        }
        "essential-failure" => {
            restart_service(
                "launcher",
                &mut services,
                &mut status.trace,
                &mut status.launcher_restarts,
            )?;
            restart_service(
                "launcher",
                &mut services,
                &mut status.trace,
                &mut status.launcher_restarts,
            )?;
            status.readiness = false;
            status.boot_health_failure = true;
            status.controlled_next_boot_recovery = true;
        }
        "hal-loss" => {
            status.trace.push("hal-loss".into());
            reap_service("hal", &mut services, &mut status.trace)?;
            status.readiness = false;
            status.boot_health_failure = true;
            status.controlled_next_boot_recovery = true;
        }
        "shutdown" => {
            status.readiness = false;
        }
        _ => unreachable!("scenario was validated before lifecycle start"),
    }
    for name in ["launcher", "broker", "hal"] {
        stop_service(
            name,
            &mut services,
            &mut status.trace,
            &mut status.shutdown_order,
        )?;
    }
    status.reaped_children = status
        .trace
        .iter()
        .filter(|event| event.ends_with("-reaped"))
        .count() as u8;
    if !services.is_empty() || status.reaped_children == 0 {
        bail!("synthetic child reaping incomplete")
    }
    status.trace.push("all-children-reaped".into());
    let bytes = serde_json::to_vec_pretty(&status)?;
    let status_path = root.join(".brickpro/data/update/supervisor.status.json");
    let mut file = fs::File::create(status_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(listener);
    fs::remove_file(&socket)?;
    println!("scenario={scenario} readiness={} launcher-restarts={} broker-restarts={} reaped={} session-failure={} boot-health-failure={} pending-blessed={}", status.readiness, status.launcher_restarts, status.broker_restarts, status.reaped_children, status.session_failure, status.boot_health_failure, status.pending_blessed);
    println!("trace={}", status.trace.join(","));
    if scenario == "shutdown" {
        println!("shutdown={}", status.shutdown_order.join(","));
    }
    Ok(())
}
