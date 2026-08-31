use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use serde_json::{json, Map, Value};

const VERSION: &str = "sim-control/v1";
const MAX_FRAME: usize = 8192;
const MAX_RESPONSE_FRAME: usize = MAX_FRAME * 8;
const MAX_NAME: usize = 48;

fn main() {
    let result = run();
    if let Err((code, message)) = result {
        if message != "response already printed" {
            println!(
                "{}",
                json!({"version": VERSION, "id": "", "ok": false, "result": null, "error": {"code": code, "message": message}})
            );
        }
        std::process::exit(match code {
            "usage" => 2,
            "unavailable" => 3,
            "protocol_rejected" => 4,
            _ => 5,
        });
    }
}

fn run() -> Result<(), (&'static str, String)> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--socket") {
        return Err(("usage", "--socket is required".into()));
    }
    let socket = args.next().ok_or(("usage", "missing socket".into()))?;
    let command = args.next().ok_or(("usage", "missing command".into()))?;
    let rest: Vec<String> = args.collect();
    let (command, command_args) = command_args(&command, &rest)?;
    let request =
        json!({"version": VERSION, "id": "cli-1", "command": command, "args": command_args});
    let bytes = serde_json::to_vec(&request).map_err(|error| ("usage", error.to_string()))?;
    if bytes.len() > MAX_FRAME {
        return Err(("usage", "request is too large".into()));
    }
    let mut stream = UnixStream::connect(Path::new(&socket))
        .map_err(|_| ("unavailable", "control socket is unavailable".into()))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| ("transport", "cannot set write timeout".into()))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| ("transport", "cannot set read timeout".into()))?;
    stream
        .write_all(&bytes)
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.shutdown(std::net::Shutdown::Write))
        .map_err(|_| ("transport", "control transport failed".into()))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|_| ("transport", "control response timed out".into()))?;
    if response.len() > MAX_RESPONSE_FRAME {
        return Err(("transport", "control response is too large".into()));
    }
    let value: Value = serde_json::from_slice(&response)
        .map_err(|_| ("transport", "malformed control response".into()))?;
    let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
    println!("{}", value);
    if ok {
        return Ok(());
    }
    let code = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("protocol_rejected");
    Err((
        if code == "protocol_rejected" {
            "protocol_rejected"
        } else {
            "transport"
        },
        "response already printed".into(),
    ))
}

fn command_args<'a>(
    command: &'a str,
    args: &[String],
) -> Result<(&'a str, Value), (&'static str, String)> {
    match command {
        "wait-ready" => Ok((
            "wait-ready",
            json!({"timeoutMs": parse_seconds(args)? * 1000}),
        )),
        "state" => {
            require_empty(args)?;
            Ok(("state", json!({})))
        }
        "power" => Ok(("power", power_args(args)?)),
        "button" => Ok(("button", button_args(args)?)),
        "action" => Ok(("action", semantic_action_args(args)?)),
        "hardware" => {
            if args.first().map(String::as_str) != Some("set") {
                return Err(("usage", "hardware set is required".into()));
            }
            Ok(("hardware.set", hardware_args(&args[1..])?))
        }
        "fault" => {
            let enabled = match args.first().map(String::as_str) {
                Some("set") => true,
                Some("clear") => false,
                _ => return Err(("usage", "fault set|clear NAME is required".into())),
            };
            if args.len() != 2 {
                return Err(("usage", "fault set|clear NAME is required".into()));
            }
            Ok(("fault.set", json!({"name": args[1], "enabled": enabled})))
        }
        "adapter" => Ok(("adapter", adapter_args(args)?)),
        "presentation" => Ok(("presentation", presentation_args(args)?)),
        "screenshot" => Ok(("screenshot", artifact_args(args)?)),
        "checkpoint" => Ok(("checkpoint", artifact_args(args)?)),
        "save-sync.status" => {
            require_empty(args)?;
            Ok(("save-sync.status", json!({})))
        }
        "save-sync.resolve" => Ok(("save-sync.resolve", save_sync_args(args)?)),
        "save-vault.history" => {
            require_empty(args)?;
            Ok(("save-vault.history", json!({})))
        }
        "save-vault.preview" => {
            require_empty(args)?;
            Ok(("save-vault.preview", json!({})))
        }
        "save-vault.restore" => {
            if args.len() != 2
                || args[0] != "--confirmed"
                || !["true", "false"].contains(&args[1].as_str())
            {
                return Err(("usage", "--confirmed true|false is required".into()));
            }
            Ok((
                "save-vault.restore",
                json!({"confirmed": args[1] == "true"}),
            ))
        }
        "autosave" => Ok(("autosave", autosave_args(args)?)),
        "resume" => Ok(("resume", resume_args(args)?)),
        "resume.delete" => Ok(("resume.delete", resume_delete_args(args)?)),
        "clock" => Ok(("clock", clock_args(args)?)),
        "lifecycle" => Ok(("lifecycle", lifecycle_args(args)?)),
        _ => Err(("usage", "unknown command".into())),
    }
}

fn save_sync_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() != 2
        || args[0] != "--action"
        || !["keep-local", "keep-remote", "keep-both"].contains(&args[1].as_str())
    {
        return Err((
            "usage",
            "--action keep-local|keep-remote|keep-both is required".into(),
        ));
    }
    Ok(json!({"action": args[1]}))
}

fn require_empty(args: &[String]) -> Result<(), (&'static str, String)> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(("usage", "unexpected arguments".into()))
    }
}

fn parse_seconds(args: &[String]) -> Result<u64, (&'static str, String)> {
    if args.len() != 2 || args[0] != "--timeout" {
        return Err(("usage", "--timeout SECONDS is required".into()));
    }
    let seconds = args[1]
        .parse::<u64>()
        .map_err(|_| ("usage", "timeout must be an integer".into()))?;
    if seconds == 0 || seconds > 30 {
        return Err(("usage", "timeout must be between 1 and 30 seconds".into()));
    }
    Ok(seconds)
}

fn button_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() != 4 || args[0] != "--button" || args[2] != "--action" {
        return Err((
            "usage",
            "--button BUTTON --action press|release is required".into(),
        ));
    }
    if ![
        "up",
        "down",
        "left",
        "right",
        "primary",
        "secondary",
        "start",
        "select",
        "menu",
        "l1",
        "r1",
    ]
    .contains(&args[1].as_str())
        || !["press", "release"].contains(&args[3].as_str())
    {
        return Err(("usage", "button or action is not allowlisted".into()));
    }
    Ok(json!({"button": args[1], "action": args[3]}))
}

fn semantic_action_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() != 2 && args.len() != 4 {
        return Err((
            "usage",
            "--action ACTION [--phase press|release] is required".into(),
        ));
    }
    if args[0] != "--action"
        || !["jump-next-group", "jump-previous-group"].contains(&args[1].as_str())
    {
        return Err(("usage", "semantic action is not allowlisted".into()));
    }
    let phase = if args.len() == 4 {
        if args[2] != "--phase" || !["press", "release"].contains(&args[3].as_str()) {
            return Err(("usage", "phase must be press or release".into()));
        }
        args[3].as_str()
    } else {
        "press"
    };
    Ok(json!({"action": args[1], "phase": phase}))
}

fn power_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    match args {
        [operation, flag, profile]
            if operation == "override"
                && flag == "--profile"
                && ["eco", "balanced", "performance"].contains(&profile.as_str()) =>
        {
            Ok(json!({"operation": operation, "profile": profile}))
        }
        [operation, flag, temperature] if operation == "temperature" && flag == "--celsius" => {
            let temperature = temperature
                .parse::<i16>()
                .map_err(|_| ("usage", "temperature must be an integer".into()))?;
            if temperature < 0i16.saturating_sub(20) || temperature > 150 {
                return Err(("usage", "temperature must be between -20 and 150".into()));
            }
            Ok(json!({"operation": operation, "temperatureC": temperature}))
        }
        _ => Err((
            "usage",
            "power override --profile eco|balanced|performance or power temperature --celsius N is required".into(),
        )),
    }
}

fn hardware_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.is_empty() {
        return Err((
            "usage",
            "at least one typed hardware assignment is required".into(),
        ));
    }
    let mut root = Map::new();
    for assignment in args {
        let (key, value) = assignment
            .split_once('=')
            .ok_or(("usage", "hardware assignments use key=value".into()))?;
        let (group, field) = key
            .split_once('.')
            .ok_or(("usage", "hardware fields are typed dotted names".into()))?;
        let group_map = root.entry(group.to_string()).or_insert_with(|| json!({}));
        let object = group_map
            .as_object_mut()
            .ok_or(("usage", "invalid hardware group".into()))?;
        let parsed = match (group, field) {
            ("battery", "percent") => json!(value
                .parse::<u8>()
                .map_err(|_| ("usage", "battery.percent must be 0-100".into()))?),
            ("battery", "charging")
            | ("battery", "externalPower")
            | ("radio", "enabled")
            | ("radio", "connected") => {
                json!(parse_bool(value)?)
            }
            ("storage", "mode") => {
                if !["available", "full"].contains(&value) {
                    return Err(("usage", "storage.mode must be available or full".into()));
                }
                json!(value)
            }
            _ => return Err(("usage", "hardware field is not allowlisted".into())),
        };
        if object.insert(field.to_string(), parsed).is_some() {
            return Err(("usage", "duplicate hardware field".into()));
        }
    }
    Ok(Value::Object(root))
}

fn parse_bool(value: &str) -> Result<bool, (&'static str, String)> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err((
            "usage",
            "boolean hardware values must be true or false".into(),
        )),
    }
}

fn adapter_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.is_empty() || !["complete", "fail", "exit", "crash"].contains(&args[0].as_str()) {
        return Err(("usage", "adapter action is not allowlisted".into()));
    }
    let mut status = if args[0] == "complete" { 0 } else { 1 };
    let mut value = 0i32;
    let mut index = 1;
    while index < args.len() {
        if index + 1 >= args.len() || !["--status", "--value"].contains(&args[index].as_str()) {
            return Err(("usage", "adapter options are --status N --value N".into()));
        }
        if args[index] == "--status" {
            status = args[index + 1]
                .parse::<u8>()
                .map_err(|_| ("usage", "status must be 0-255".into()))?;
        } else {
            value = args[index + 1]
                .parse::<i32>()
                .map_err(|_| ("usage", "value must be a signed integer".into()))?;
            if !(-1_000_000..=1_000_000).contains(&value) {
                return Err(("usage", "value must be between -1000000 and 1000000".into()));
            }
        }
        index += 2;
    }
    Ok(json!({"action": args[0], "status": status, "value": value}))
}

fn presentation_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() != 2 || args[0] != "--action" {
        return Err((
            "usage",
            "--action must be a generated presentation action".into(),
        ));
    }
    let actions = [
        "home",
        "systems",
        "games",
        "games-no-metadata",
        "favorites",
        "recent",
        "resume",
        "favorite",
        "media-details",
        "theme-garden",
        "update",
        "unavailable",
        "search",
        "settings",
        "settings-form",
        "recovery",
        "modal",
        "scraper-settings",
        "scraper-game",
        "scraper-queue",
        "scraper-progress",
        "scraper-progress-zero",
        "scraper-progress-2",
        "scraper-progress-4",
        "scraper-fallback",
        "scraper-background",
        "scraper-paused",
        "scraper-resumed",
        "scraper-ambiguity",
        "scraper-complete",
        "scraper-cancel",
        "scraper-confirm-cancel",
        "wifi-scan",
        "wifi-access-points",
        "wifi-password",
        "wifi-hidden",
        "wifi-manual",
        "wifi-progress",
        "wifi-error",
        "save-sync-status",
        "save-sync-keep-local",
        "save-sync-keep-remote",
        "save-sync-keep-both",
        "save-vault-history",
        "save-vault-preview",
        "save-vault-confirm",
        "save-vault-restore",
        "save-vault-cancel",
        "fallback",
    ];
    if !actions.contains(&args[1].as_str()) {
        return Err(("usage", "presentation action is not allowlisted".into()));
    }
    Ok(json!({"action": args[1]}))
}

fn artifact_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() != 2 || args[0] != "--name" || !valid_name(&args[1]) {
        return Err(("usage", "--name must be a safe basename".into()));
    }
    Ok(json!({"name": args[1]}))
}

fn autosave_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() < 2
        || args[0] != "--reason"
        || !["normal-exit", "pre-suspend", "low-battery", "periodic"].contains(&args[1].as_str())
    {
        return Err(("usage", "--reason is required and typed".into()));
    }
    let mut value = json!({"reason": args[1]});
    if args.len() > 2 {
        if args.len() != 4
            || args[2] != "--fault"
            || !["none", "artifact", "metadata", "promotion", "pointer"].contains(&args[3].as_str())
        {
            return Err(("usage", "--fault is not allowlisted".into()));
        }
        value["fault"] = json!(args[3]);
    }
    Ok(value)
}

fn clock_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() != 3
        || !["advance", "jump"].contains(&args[0].as_str())
        || !["--minutes", "--milliseconds"].contains(&args[1].as_str())
    {
        return Err((
            "usage",
            "clock advance|jump --minutes N or --milliseconds N is required".into(),
        ));
    }
    let amount = args[2]
        .parse::<u64>()
        .map_err(|_| ("usage", "clock amount must be an integer".into()))?;
    let milliseconds = if args[1] == "--minutes" {
        amount.saturating_mul(60_000)
    } else {
        amount
    };
    if milliseconds > 3_600_000 {
        return Err((
            "usage",
            "clock advance must be at most 3600000 milliseconds".into(),
        ));
    }
    Ok(json!({"operation": args[0], "monotonicMs": milliseconds, "wallClockMs": milliseconds}))
}

fn lifecycle_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() < 3
        || !["suspend", "resume", "shutdown"].contains(&args[0].as_str())
        || args[1] != "--timeout"
    {
        return Err((
            "usage",
            "lifecycle suspend|resume --timeout SECONDS is required".into(),
        ));
    }
    let seconds = args[2]
        .parse::<u64>()
        .map_err(|_| ("usage", "lifecycle timeout must be an integer".into()))?;
    if seconds == 0 || seconds > 30 {
        return Err((
            "usage",
            "lifecycle timeout must be between 1 and 30 seconds".into(),
        ));
    }
    let mut value = json!({"operation": args[0], "timeoutMs": seconds * 1000});
    let mut index = 3;
    while index < args.len() {
        if index + 1 >= args.len() {
            return Err(("usage", "lifecycle option requires a value".into()));
        }
        match args[index].as_str() {
            "--duration-minutes" => {
                let duration = args[index + 1]
                    .parse::<u16>()
                    .map_err(|_| ("usage", "duration must be 1, 5, 10, 15, 30, or 60".into()))?;
                if ![1, 5, 10, 15, 30, 60].contains(&duration) {
                    return Err(("usage", "duration must be 1, 5, 10, 15, 30, or 60".into()));
                }
                value["durationMinutes"] = json!(duration);
            }
            "--source" => {
                if !["user", "deadline", "stale-alarm"].contains(&args[index + 1].as_str()) {
                    return Err(("usage", "wake source is not allowlisted".into()));
                }
                value["wakeSource"] = json!(args[index + 1]);
            }
            _ => return Err(("usage", "unknown lifecycle option".into())),
        }
        index += 2;
    }
    Ok(value)
}

fn resume_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() < 4
        || args[0] != "--content-id"
        || args[2] != "--decision"
        || ![
            "nebula-nes",
            "mirror-ps1",
            "orbit-garden",
            "signal-workshop",
        ]
        .contains(&args[1].as_str())
        || ![
            "resume",
            "retained-matching-core",
            "cold-start-sram",
            "restore-previous",
            "fresh-start",
            "cancel",
        ]
        .contains(&args[3].as_str())
    {
        return Err(("usage", "resume arguments are not allowlisted".into()));
    }
    let mut value = json!({"contentId": args[1], "decision": args[3]});
    let mut index = 4;
    while index < args.len() {
        if index + 1 >= args.len() {
            return Err(("usage", "resume identity overrides are invalid".into()));
        }
        let field = match args[index].as_str() {
            "--runner-id" if valid_name(&args[index + 1]) => "runnerId",
            "--runner-version" if valid_version(&args[index + 1]) => "runnerVersion",
            "--core-id" if valid_name(&args[index + 1]) => "coreId",
            "--core-version" if valid_version(&args[index + 1]) => "coreVersion",
            _ => return Err(("usage", "resume identity overrides are invalid".into())),
        };
        value[field] = json!(args[index + 1]);
        index += 2;
    }
    Ok(value)
}

fn resume_delete_args(args: &[String]) -> Result<Value, (&'static str, String)> {
    if args.len() != 6
        || args[0] != "--content-id"
        || args[2] != "--generation"
        || args[4] != "--confirmed"
        || ![
            "nebula-nes",
            "mirror-ps1",
            "orbit-garden",
            "signal-workshop",
        ]
        .contains(&args[1].as_str())
        || args[3].parse::<u64>().is_err()
        || !["true", "false"].contains(&args[5].as_str())
    {
        return Err((
            "usage",
            "--content-id ID --generation NUMBER --confirmed true|false is required".into(),
        ));
    }
    Ok(json!({
        "contentId": args[1],
        "generation": args[3].parse::<u64>().expect("validated generation"),
        "confirmed": args[5] == "true",
    }))
}

fn valid_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        && name.as_bytes()[0].is_ascii_alphanumeric()
}
