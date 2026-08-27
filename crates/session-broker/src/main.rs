mod adapters;
mod helper_logic;
mod journal;
mod state;

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use adapters::{LaunchPlan, ResolvedPaths, RunMode};
use launch_contract::{
    parse_catalog_json, parse_request_json, validate_host_fixture, Catalog, LaunchKind,
    LaunchRequest, LogicalPath, PathRoot,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const JOURNEYS: &[&str] = &[
    "success",
    "standalone",
    "port",
    "nonzero",
    "signal",
    "timeout",
    "cancel",
    "grandchild",
    "spawn-error",
    "busy",
    "invalid-catalog",
    "escape",
    "hash-mismatch",
    "command-shaped",
    "restart",
    "marker-mismatch",
    "start-time-mismatch",
    "publication-failure",
    "symlink-collision",
    "temp-collision",
    "crash-before-publish",
    "crash-after-publish",
    "crash-after-release",
    "result-fsync-failure",
];

fn main() {
    if let Err(error) = execute(std::env::args().skip(1).collect()) {
        eprintln!("session-broker failed: {error}");
        std::process::exit(1);
    }
}

fn execute(arguments: Vec<String>) -> Result<(), String> {
    if arguments
        .first()
        .is_some_and(|argument| argument == "--helper")
    {
        helper_logic::run(arguments.into_iter().skip(1).collect());
        return Ok(());
    }
    let Some(command) = arguments.first() else {
        return Err(
            "usage: session-broker simulate [--fixture-root DIR] [--journey NAME]".to_string(),
        );
    };
    if command != "simulate" {
        return Err("only simulate is available".to_string());
    }
    let mut fixture_root = default_fixture_root();
    let mut journeys = None;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--fixture-root" => {
                index += 1;
                fixture_root = PathBuf::from(
                    arguments
                        .get(index)
                        .ok_or_else(|| "missing fixture root".to_string())?,
                );
            }
            "--journey" => {
                index += 1;
                journeys = Some(vec![arguments
                    .get(index)
                    .ok_or_else(|| "missing journey".to_string())?
                    .clone()]);
            }
            "--journeys" => {
                index += 1;
                journeys = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "missing journeys".to_string())?
                        .split(',')
                        .map(str::to_string)
                        .collect(),
                );
            }
            _ => return Err("unknown simulate argument".to_string()),
        }
        index += 1;
    }
    let journeys =
        journeys.unwrap_or_else(|| JOURNEYS.iter().map(|name| (*name).to_string()).collect());
    for journey in journeys {
        if !JOURNEYS.contains(&journey.as_str()) {
            return Err("unknown journey".to_string());
        }
        let output = run_journey(&fixture_root, &journey)?;
        println!(
            "{}",
            serde_json::to_string(&output).map_err(|_| "result encoding failed")?
        );
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(untagged)]
enum Output {
    Session(journal::SessionResult),
    Rejection(journal::Rejection),
}

fn run_journey(source: &Path, journey: &str) -> Result<Output, String> {
    let root = temp_root(journey);
    let _ = fs::remove_dir_all(&root);
    copy_tree(source, &root)?;
    let catalog_bytes = fs::read(root.join("catalog.synthetic.json"))
        .map_err(|_| "catalog fixture unavailable".to_string())?;
    let mut catalog =
        parse_catalog_json(&catalog_bytes).map_err(|_| "catalog fixture is invalid".to_string())?;
    let request_name = match journey {
        "standalone" => "standalone.synthetic.json",
        "port" => "port.synthetic.json",
        "command-shaped" => "command-shaped.synthetic.json",
        _ => "libretro.synthetic.json",
    };
    let request_bytes = fs::read(root.join("requests").join(request_name))
        .map_err(|_| "request fixture unavailable".to_string())?;
    if journey == "command-shaped" {
        if parse_request_json(&request_bytes).is_ok() {
            return Err("command-shaped fixture was accepted".to_string());
        }
        let output = Output::Rejection(journal::Rejection {
            result_type: "LaunchRejected",
            journey: journey.to_string(),
            accepted: false,
            reason: "request shape is not closed".to_string(),
            restored: true,
        });
        let _ = fs::remove_dir_all(root);
        return Ok(output);
    }
    let mut request =
        parse_request_json(&request_bytes).map_err(|_| "request fixture is invalid".to_string())?;
    match journey {
        "invalid-catalog" => catalog.schema_version = 2,
        "escape" => request.content_path.relative = "../outside.bin".to_string(),
        "hash-mismatch" => request.content_sha256 = "0".repeat(64),
        _ => {}
    }
    let mut broker = Broker::new(root.join("host-root"));
    let output = match journey {
        "restart" | "marker-mismatch" | "start-time-mismatch" => {
            broker.seed_recovery_fixture(journey)?;
            Output::Session(broker.recover(journey))
        }
        "busy" => {
            broker.mark_busy();
            Output::Rejection(broker.reject_busy(journey))
        }
        "symlink-collision" | "temp-collision" => {
            broker.seed_symlink_collision(&request)?;
            Output::Session(broker.launch(request, catalog, journey, RunMode::Success, Fault::None))
        }
        "invalid-catalog" | "escape" | "hash-mismatch" => {
            Output::Session(broker.launch(request, catalog, journey, RunMode::Success, Fault::None))
        }
        "standalone" | "port" | "success" => {
            Output::Session(broker.launch(request, catalog, journey, RunMode::Success, Fault::None))
        }
        "nonzero" => {
            Output::Session(broker.launch(request, catalog, journey, RunMode::Nonzero, Fault::None))
        }
        "signal" => {
            Output::Session(broker.launch(request, catalog, journey, RunMode::Signal, Fault::None))
        }
        "timeout" => {
            Output::Session(broker.launch(request, catalog, journey, RunMode::Timeout, Fault::None))
        }
        "cancel" => {
            Output::Session(broker.launch(request, catalog, journey, RunMode::Cancel, Fault::None))
        }
        "grandchild" => Output::Session(broker.launch(
            request,
            catalog,
            journey,
            RunMode::Grandchild,
            Fault::None,
        )),
        "spawn-error" => Output::Session(broker.launch(
            request,
            catalog,
            journey,
            RunMode::SpawnError,
            Fault::None,
        )),
        "publication-failure" | "crash-before-publish" => Output::Session(broker.launch(
            request,
            catalog,
            journey,
            RunMode::Success,
            if journey == "publication-failure" {
                Fault::PublicationFailure
            } else {
                Fault::CrashBeforePublish
            },
        )),
        "crash-after-publish" => Output::Session(broker.launch(
            request,
            catalog,
            journey,
            RunMode::Success,
            Fault::CrashAfterPublish,
        )),
        "crash-after-release" => Output::Session(broker.launch(
            request,
            catalog,
            journey,
            RunMode::Success,
            Fault::CrashAfterRelease,
        )),
        "result-fsync-failure" => {
            fs::create_dir_all(
                broker
                    .fixture_root
                    .join("data/activity/sessions/results.jsonl"),
            )
            .map_err(|_| "result fixture unavailable".to_string())?;
            Output::Session(broker.launch(request, catalog, journey, RunMode::Success, Fault::None))
        }
        _ => return Err("unsupported journey".to_string()),
    };
    let _ = fs::remove_dir_all(root);
    Ok(output)
}

struct Broker {
    phase: Phase,
    platform: state::LogicalPlatform,
    fixture_root: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Idle,
    Preparing,
    Running,
    Finalizing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    None,
    PublicationFailure,
    CrashBeforePublish,
    CrashAfterPublish,
    CrashAfterRelease,
}

#[derive(Clone)]
enum Outcome {
    Success,
    Nonzero(i32),
    Signal(i32),
    Timeout,
    Cancel,
    SpawnFailure,
    Recovery,
    ValidationFailure,
    AdapterFailure,
    ResultFailure,
    WithDuration(Box<Outcome>, u64),
}

#[derive(Clone)]
struct Identity {
    pid: u32,
    pgid: i32,
    start_time: u64,
    marker: String,
    released: bool,
}

impl Broker {
    fn new(fixture_root: PathBuf) -> Self {
        Self {
            phase: Phase::Idle,
            platform: state::LogicalPlatform::new(),
            fixture_root,
        }
    }

    fn mark_busy(&mut self) {
        self.phase = Phase::Running;
    }

    fn reject_busy(&self, journey: &str) -> journal::Rejection {
        journal::Rejection {
            result_type: "LaunchRejected",
            journey: journey.to_string(),
            accepted: false,
            reason: "busy".to_string(),
            restored: true,
        }
    }

    fn launch(
        &mut self,
        request: LaunchRequest,
        catalog: Catalog,
        journey: &str,
        mode: RunMode,
        fault: Fault,
    ) -> journal::SessionResult {
        if self.phase != Phase::Idle {
            return rejection_as_session(journey, "busy");
        }
        self.phase = Phase::Preparing;
        let runner = Some(request.runner.id.clone());
        let core = request.core.as_ref().map(|value| value.id.clone());
        let _paths = match self.validate_and_resolve(&request, &catalog) {
            Ok(paths) => paths,
            Err(_) => {
                self.phase = Phase::Idle;
                return rejection_as_session(journey, "validation-failure");
            }
        };
        let snapshot = self.platform.snapshot();
        self.platform.apply_profile(&request);
        let adapter = adapter_name(request.kind.clone());
        let marker = match journal::random_marker() {
            Ok(marker) => marker,
            Err(_) => {
                self.platform.safe_default();
                self.phase = Phase::Idle;
                return rejection_as_session(journey, "recovery");
            }
        };
        let sessions = self.fixture_root.join("data/activity/sessions");
        if fs::create_dir_all(&sessions).is_err() {
            self.platform.restore(&snapshot);
            self.phase = Phase::Idle;
            return rejection_as_session(journey, "journal-failure");
        }
        let journal_path = sessions.join(format!("{}.json", request.request_id));
        let mut record =
            journal::new_record(&request.request_id, adapter, &marker, snapshot.clone());
        if journal::create(&journal_path, &mut record).is_err() {
            self.platform.restore(&snapshot);
            self.phase = Phase::Idle;
            return rejection_as_session(journey, "journal-failure");
        }
        let plan = match self.validate_and_plan(&request, &catalog, mode) {
            Ok(plan) => plan,
            Err(_) => {
                return self.finish(
                    &request,
                    journey,
                    runner,
                    core,
                    adapter,
                    snapshot,
                    journal_path,
                    record,
                    false,
                    true,
                    Outcome::AdapterFailure,
                )
            }
        };
        let (mut child, barrier) = match spawn_with_barrier(&plan, &marker) {
            Ok(value) => value,
            Err(_) => {
                return self.finish(
                    &request,
                    journey,
                    runner,
                    core,
                    adapter,
                    snapshot,
                    journal_path,
                    record,
                    false,
                    true,
                    Outcome::SpawnFailure,
                )
            }
        };
        self.phase = Phase::Running;
        let mut identity = match identity_for(child.id(), &marker) {
            Some(identity) => identity,
            None => {
                let _ = stop_direct(&mut child);
                return self.finish(
                    &request,
                    journey,
                    runner,
                    core,
                    adapter,
                    snapshot,
                    journal_path,
                    record,
                    false,
                    false,
                    Outcome::Recovery,
                );
            }
        };
        record.pid = Some(identity.pid);
        record.pgid = Some(identity.pgid);
        record.start_time = Some(identity.start_time);
        record.phase = journal::JournalPhase::Running;
        let publish = if matches!(fault, Fault::PublicationFailure | Fault::CrashBeforePublish) {
            Err(io::Error::other("injected journal publication failure"))
        } else {
            journal::transition(&journal_path, &mut record)
        };
        if publish.is_err() {
            drop(barrier);
            let cleanup = stop_owned_group(&identity);
            let reap = child.wait().is_ok();
            return self.finish(
                &request,
                journey,
                runner,
                core,
                adapter,
                snapshot,
                journal_path,
                record,
                false,
                cleanup && reap,
                Outcome::Recovery,
            );
        }
        if fault == Fault::CrashAfterPublish {
            drop(barrier);
            let cleanup = stop_owned_group(&identity);
            let reap = child.wait().is_ok();
            return self.finish(
                &request,
                journey,
                runner,
                core,
                adapter,
                snapshot,
                journal_path,
                record,
                false,
                cleanup && reap,
                Outcome::Recovery,
            );
        }
        if barrier.release().is_err() {
            let cleanup = stop_owned_group(&identity);
            let reap = child.wait().is_ok();
            return self.finish(
                &request,
                journey,
                runner,
                core,
                adapter,
                snapshot,
                journal_path,
                record,
                false,
                cleanup && reap,
                Outcome::Recovery,
            );
        }
        identity.released = true;
        record.released = true;
        if journal::transition(&journal_path, &mut record).is_err() {
            let cleanup = stop_owned_group(&identity);
            let reap = child.wait().is_ok();
            return self.finish(
                &request,
                journey,
                runner,
                core,
                adapter,
                snapshot,
                journal_path,
                record,
                false,
                cleanup && reap,
                Outcome::Recovery,
            );
        }
        if fault == Fault::CrashAfterRelease {
            let cleanup = stop_owned_group(&identity);
            let reap = child.wait().is_ok();
            return self.finish(
                &request,
                journey,
                runner,
                core,
                adapter,
                snapshot,
                journal_path,
                record,
                plan.confirms_usable_save,
                cleanup && reap,
                Outcome::Recovery,
            );
        }
        let started = Instant::now();
        let (mut outcome, duration_ms) = wait_for_child(&mut child, mode, started);
        let cleanup = stop_owned_group(&identity);
        let reap = child.wait().is_ok();
        if !cleanup || !reap {
            outcome = Outcome::Recovery;
        }
        self.finish(
            &request,
            journey,
            runner,
            core,
            plan.adapter,
            snapshot,
            journal_path,
            record,
            plan.confirms_usable_save,
            cleanup && reap,
            outcome.with_duration(duration_ms),
        )
    }

    fn validate_and_resolve(
        &self,
        request: &LaunchRequest,
        catalog: &Catalog,
    ) -> Result<ResolvedPaths, String> {
        validate_host_fixture(request, catalog, &self.fixture_root)
            .map_err(|_| "request validation failed".to_string())?;
        let content = resolve_path(
            &self.fixture_root,
            &request.content_path,
            PathRoot::Roms,
            true,
        )?;
        let save = resolve_path(
            &self.fixture_root,
            &request.save_path,
            PathRoot::DataSaves,
            false,
        )?;
        let state = resolve_path(
            &self.fixture_root,
            &request.state_path,
            PathRoot::DataStates,
            false,
        )?;
        if sha256_file(&content)? != request.content_sha256 {
            return Err("content integrity check failed".to_string());
        }
        Ok(ResolvedPaths {
            content,
            save,
            state,
        })
    }

    fn validate_and_plan(
        &self,
        request: &LaunchRequest,
        catalog: &Catalog,
        mode: RunMode,
    ) -> Result<LaunchPlan, String> {
        let paths = self.validate_and_resolve(request, catalog)?;
        let current =
            std::env::current_exe().map_err(|_| "helper executable unavailable".to_string())?;
        let sibling = current.with_file_name("session-broker-helper");
        let helper = if sibling.is_file() { sibling } else { current };
        adapters::build_plan(request, catalog, &self.fixture_root, &helper, &paths, mode)
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &mut self,
        request: &LaunchRequest,
        journey: &str,
        runner: Option<String>,
        core: Option<String>,
        adapter: &str,
        snapshot: sim_platform_contract::PlatformState,
        journal_path: PathBuf,
        mut record: journal::JournalRecord,
        confirms_save: bool,
        cleanup_ok: bool,
        outcome: Outcome,
    ) -> journal::SessionResult {
        self.phase = Phase::Finalizing;
        record.phase = journal::JournalPhase::Finalizing;
        record.pid = None;
        record.pgid = None;
        record.start_time = None;
        let finalizing_ok = journal::transition(&journal_path, &mut record).is_ok();
        let mut safe_default = !finalizing_ok || !cleanup_ok;
        let mut restored = if safe_default {
            self.platform.safe_default();
            true
        } else {
            self.platform.restore(&snapshot)
        };
        if !restored {
            self.platform.safe_default();
            restored = true;
            safe_default = true;
        }
        let fields = outcome.finish_fields();
        let save_ok = if fields.reason == "success" && confirms_save {
            write_file_sync(
                &resolve_path_unchecked(&self.fixture_root, &request.save_path),
                b"generated-session-save-v1",
            )
            .is_ok()
        } else {
            false
        };
        let activity = self.fixture_root.join("data/activity");
        let metadata_ok = append_metadata(&activity, request, adapter, fields.duration_ms);
        let mut result = journal::SessionResult {
            result_type: "SessionResult",
            journey: journey.to_string(),
            accepted: true,
            runner,
            core,
            reason: fields.reason,
            duration_ms: fields.duration_ms,
            restored,
            safe_default,
            persistence_status: if finalizing_ok && metadata_ok {
                "durable"
            } else {
                "failed"
            },
            resume_published: false,
            exit_code: fields.exit_code,
            signal: fields.signal,
        };
        let result_path = activity.join("sessions/results.jsonl");
        let result_ok = journal::append_result(&result_path, &result).is_ok();
        let resume_allowed = result_ok
            && metadata_ok
            && save_ok
            && confirms_save
            && request.resume_mode != launch_contract::ResumeMode::Fresh
            && result.reason == "success";
        if resume_allowed {
            let resume = serde_json::json!({
                "requestId": request.request_id.as_str(),
                "adapter": adapter,
                "usable": true
            });
            result.resume_published =
                journal::write_resume(&activity.join("resume.json"), &resume).is_ok();
        }
        let completed_ok = {
            record.phase = journal::JournalPhase::Completed;
            journal::transition(&journal_path, &mut record).is_ok()
        };
        if !result_ok || !metadata_ok || !completed_ok {
            result.persistence_status = "failed";
            result.resume_published = false;
            if result.reason == "success" && !result_ok {
                result.reason = "result-persistence-failure".to_string();
            }
        }
        append_bounded_log(&activity.join("session.log"), "session finalized\n");
        self.phase = Phase::Idle;
        result
    }

    fn seed_symlink_collision(&self, request: &LaunchRequest) -> Result<(), String> {
        let sessions = self.fixture_root.join("data/activity/sessions");
        fs::create_dir_all(&sessions).map_err(|_| "journal fixture unavailable".to_string())?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            sessions.join("outside.synthetic"),
            sessions.join(format!("{}.json", request.request_id)),
        )
        .map_err(|_| "journal fixture unavailable".to_string())?;
        #[cfg(not(unix))]
        return Err("symlink journey requires Unix".to_string());
        Ok(())
    }

    fn seed_recovery_fixture(&self, journey: &str) -> Result<(), String> {
        let sessions = self.fixture_root.join("data/activity/sessions");
        fs::create_dir_all(&sessions).map_err(|_| "recovery fixture unavailable".to_string())?;
        let mut record = journal::new_record(
            "generated-recovery-1",
            "retroarch",
            &journal::random_marker().map_err(|_| "recovery fixture unavailable".to_string())?,
            self.platform.snapshot(),
        );
        let pid = std::process::id();
        record.pid = Some(pid);
        record.pgid = current_pgid();
        record.start_time = current_start_time();
        if journey == "marker-mismatch" {
            record.marker = "0".repeat(64);
        }
        if journey == "start-time-mismatch" {
            record.start_time = record.start_time.map(|value| value.saturating_add(1));
        }
        if journey == "restart" {
            record.pid = None;
            record.pgid = None;
            record.start_time = None;
            record.marker =
                journal::random_marker().map_err(|_| "recovery fixture unavailable".to_string())?;
        }
        journal::create(&sessions.join("unfinished.synthetic.json"), &mut record)
            .map_err(|_| "recovery fixture unavailable".to_string())?;
        if journey == "restart" {
            fs::write(sessions.join("malformed.synthetic.json"), b"not-json")
                .map_err(|_| "recovery fixture unavailable".to_string())?;
        }
        Ok(())
    }

    fn recover(&mut self, journey: &str) -> journal::SessionResult {
        let sessions = self.fixture_root.join("data/activity/sessions");
        let mut needed = false;
        let mut invalid = false;
        if let Ok(entries) = fs::read_dir(&sessions) {
            for entry in entries.flatten().filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            }) {
                let path = entry.path();
                match journal::read_valid(&path) {
                    Some(mut record)
                        if matches!(
                            record.phase,
                            journal::JournalPhase::Preparing
                                | journal::JournalPhase::Running
                                | journal::JournalPhase::Finalizing
                        ) =>
                    {
                        needed = true;
                        let stopped = match (record.pid, record.pgid, record.start_time) {
                            (Some(pid), Some(pgid), Some(start_time)) => {
                                stop_owned_group(&Identity {
                                    pid,
                                    pgid,
                                    start_time,
                                    marker: record.marker.clone(),
                                    released: record.released,
                                })
                            }
                            _ => true,
                        };
                        if !stopped {
                            invalid = true;
                        }
                        self.platform.safe_default();
                        record.phase = journal::JournalPhase::Recovered;
                        record.pid = None;
                        record.pgid = None;
                        record.start_time = None;
                        if journal::transition(&path, &mut record).is_err() {
                            invalid = true;
                        }
                    }
                    Some(_) => {}
                    None => invalid = true,
                }
            }
        }
        let result = journal::SessionResult {
            result_type: "SessionResult",
            journey: journey.to_string(),
            accepted: false,
            runner: None,
            core: None,
            reason: if needed || invalid {
                if invalid {
                    "recovery-invalid-journal"
                } else {
                    "recovery"
                }
            } else {
                "recovery-idle"
            }
            .to_string(),
            duration_ms: 0,
            restored: true,
            safe_default: needed || invalid,
            persistence_status: "durable",
            resume_published: false,
            exit_code: None,
            signal: None,
        };
        let activity = self.fixture_root.join("data/activity");
        let result_ok =
            journal::append_result(&activity.join("sessions/results.jsonl"), &result).is_ok();
        self.phase = Phase::Idle;
        if result_ok {
            result
        } else {
            journal::SessionResult {
                persistence_status: "failed",
                ..result
            }
        }
    }
}

struct Fields {
    reason: String,
    duration_ms: u64,
    exit_code: Option<i32>,
    signal: Option<i32>,
}

impl Outcome {
    fn with_duration(self, duration_ms: u64) -> Self {
        match self {
            Self::Success => Self::WithDuration(Box::new(Self::Success), duration_ms),
            Self::Nonzero(code) => Self::WithDuration(Box::new(Self::Nonzero(code)), duration_ms),
            Self::Signal(signal) => Self::WithDuration(Box::new(Self::Signal(signal)), duration_ms),
            Self::Timeout => Self::WithDuration(Box::new(Self::Timeout), duration_ms),
            Self::Cancel => Self::WithDuration(Box::new(Self::Cancel), duration_ms),
            Self::SpawnFailure => Self::WithDuration(Box::new(Self::SpawnFailure), duration_ms),
            Self::Recovery => Self::WithDuration(Box::new(Self::Recovery), duration_ms),
            Self::ValidationFailure => {
                Self::WithDuration(Box::new(Self::ValidationFailure), duration_ms)
            }
            Self::AdapterFailure => Self::WithDuration(Box::new(Self::AdapterFailure), duration_ms),
            Self::ResultFailure => Self::WithDuration(Box::new(Self::ResultFailure), duration_ms),
            Self::WithDuration(value, _) => Self::WithDuration(value, duration_ms),
        }
    }

    fn finish_fields(self) -> Fields {
        let (outcome, duration_ms) = match self {
            Self::WithDuration(value, duration_ms) => (*value, duration_ms),
            value => (value, 0),
        };
        match outcome {
            Self::Success => Fields::new("success", duration_ms, None, None),
            Self::Nonzero(code) => Fields::new("nonzero-exit", duration_ms, Some(code), None),
            Self::Signal(signal) => Fields::new("signal-death", duration_ms, None, Some(signal)),
            Self::Timeout => Fields::new("timeout", duration_ms, None, None),
            Self::Cancel => Fields::new("cancellation", duration_ms, None, None),
            Self::SpawnFailure => Fields::new("spawn-failure", duration_ms, None, None),
            Self::Recovery => Fields::new("recovery", duration_ms, None, None),
            Self::ValidationFailure => Fields::new("validation-failure", duration_ms, None, None),
            Self::AdapterFailure => Fields::new("adapter-failure", duration_ms, None, None),
            Self::ResultFailure => {
                Fields::new("result-persistence-failure", duration_ms, None, None)
            }
            Self::WithDuration(_, _) => unreachable!(),
        }
    }
}

impl Fields {
    fn new(reason: &str, duration_ms: u64, exit_code: Option<i32>, signal: Option<i32>) -> Self {
        Self {
            reason: reason.to_string(),
            duration_ms,
            exit_code,
            signal,
        }
    }
}

fn rejection_as_session(journey: &str, reason: &str) -> journal::SessionResult {
    journal::SessionResult {
        result_type: "SessionResult",
        journey: journey.to_string(),
        accepted: false,
        runner: None,
        core: None,
        reason: reason.to_string(),
        duration_ms: 0,
        restored: true,
        safe_default: false,
        persistence_status: "not-applicable",
        resume_published: false,
        exit_code: None,
        signal: None,
    }
}

#[cfg(unix)]
struct LaunchBarrier {
    write_fd: libc::c_int,
}

#[cfg(unix)]
impl LaunchBarrier {
    fn release(mut self) -> io::Result<()> {
        let byte = [1u8];
        let result = unsafe { libc::write(self.write_fd, byte.as_ptr().cast(), 1) };
        let error = if result == 1 {
            None
        } else {
            Some(io::Error::last_os_error())
        };
        unsafe { libc::close(self.write_fd) };
        self.write_fd = -1;
        error.map_or(Ok(()), Err)
    }
}

#[cfg(unix)]
impl Drop for LaunchBarrier {
    fn drop(&mut self) {
        if self.write_fd >= 0 {
            unsafe { libc::close(self.write_fd) };
            self.write_fd = -1;
        }
    }
}

#[cfg(unix)]
fn spawn_with_barrier(plan: &LaunchPlan, marker: &str) -> io::Result<(Child, LaunchBarrier)> {
    use std::os::unix::process::CommandExt;
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let read_fd = fds[0];
    let write_fd = fds[1];
    let mut command = Command::new(&plan.executable);
    if plan
        .executable
        .file_name()
        .is_some_and(|name| name == "session-broker")
    {
        command.arg("--helper");
    }
    command
        .args(&plan.args)
        .env_clear()
        .env("BROKER_SESSION_MARKER", marker)
        .env("BROKER_BARRIER_FD", read_fd.to_string())
        .current_dir(&plan.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            libc::close(write_fd);
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    match command.spawn() {
        Ok(child) => {
            unsafe { libc::close(read_fd) };
            Ok((child, LaunchBarrier { write_fd }))
        }
        Err(error) => {
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            Err(error)
        }
    }
}

#[cfg(not(unix))]
fn spawn_with_barrier(_plan: &LaunchPlan, _marker: &str) -> io::Result<(Child, ())> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process barriers require Unix",
    ))
}

fn stop_direct(child: &mut Child) -> bool {
    child.kill().is_ok() && child.wait().is_ok()
}

#[cfg(unix)]
fn identity_for(pid: u32, marker: &str) -> Option<Identity> {
    for _ in 0..20 {
        let pgid = unsafe { libc::getpgid(pid as i32) };
        if pgid >= 0 {
            if let Some(start_time) = proc_start_time(pid) {
                let identity = Identity {
                    pid,
                    pgid,
                    start_time,
                    marker: marker.to_string(),
                    released: false,
                };
                if identity_matches(&identity) {
                    return Some(identity);
                }
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    None
}

#[cfg(not(unix))]
fn identity_for(_pid: u32, _marker: &str) -> Option<Identity> {
    None
}

#[cfg(unix)]
fn identity_matches(identity: &Identity) -> bool {
    (unsafe { libc::getpgid(identity.pid as i32) == identity.pgid })
        && proc_start_time(identity.pid) == Some(identity.start_time)
        && proc_has_marker(identity.pid, &identity.marker)
}

#[cfg(unix)]
fn group_owned(identity: &Identity) -> bool {
    if !group_exists(identity.pgid) {
        return true;
    }
    if identity_matches(identity) {
        return true;
    }
    if !identity.released {
        return false;
    }
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            return false;
        };
        proc_pgid(pid) == Some(identity.pgid) && proc_has_marker(pid, &identity.marker)
    })
}

#[cfg(unix)]
fn stop_owned_group(identity: &Identity) -> bool {
    if identity.pgid <= 1 || !group_owned(identity) {
        return !group_exists(identity.pgid);
    }
    if unsafe { libc::kill(-identity.pgid, libc::SIGTERM) } != 0
        && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return false;
    }
    if wait_group_gone(identity.pgid, Duration::from_millis(100)) {
        return true;
    }
    if unsafe { libc::kill(-identity.pgid, libc::SIGKILL) } != 0
        && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return false;
    }
    wait_group_gone(identity.pgid, Duration::from_millis(100))
}

#[cfg(not(unix))]
fn stop_owned_group(_identity: &Identity) -> bool {
    false
}

#[cfg(unix)]
fn wait_group_gone(pgid: i32, limit: Duration) -> bool {
    let started = Instant::now();
    loop {
        if !group_exists(pgid) || !group_has_live_processes(pgid) {
            return true;
        }
        if started.elapsed() >= limit {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(unix)]
fn group_exists(pgid: i32) -> bool {
    (unsafe { libc::kill(-pgid, 0) == 0 })
        || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn group_has_live_processes(pgid: i32) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return true;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            return false;
        };
        proc_pgid(pid) == Some(pgid) && proc_state(pid) != Some(b'Z')
    })
}

#[cfg(unix)]
fn proc_has_marker(pid: u32, marker: &str) -> bool {
    let expected = format!("BROKER_SESSION_MARKER={marker}");
    fs::read(format!("/proc/{pid}/environ")).is_ok_and(|bytes| {
        bytes
            .split(|byte| *byte == 0)
            .any(|entry| entry == expected.as_bytes())
    })
}

#[cfg(unix)]
fn proc_state(pid: u32) -> Option<u8> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = text.rfind(')')?;
    text.as_bytes().get(close + 2).copied()
}

#[cfg(unix)]
fn proc_pgid(pid: u32) -> Option<i32> {
    let (_, pgid, _) = proc_stat(pid)?;
    Some(pgid)
}

#[cfg(unix)]
fn proc_start_time(pid: u32) -> Option<u64> {
    let (_, _, start_time) = proc_stat(pid)?;
    Some(start_time)
}

#[cfg(unix)]
fn proc_stat(pid: u32) -> Option<(u32, i32, u64)> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = text.rfind(')')?;
    let fields = text
        .get(close + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    Some((
        pid,
        fields.get(2)?.parse().ok()?,
        fields.get(19)?.parse().ok()?,
    ))
}

#[cfg(unix)]
fn current_pgid() -> Option<i32> {
    let pgid = unsafe { libc::getpgid(0) };
    (pgid >= 0).then_some(pgid)
}

#[cfg(not(unix))]
fn current_pgid() -> Option<i32> {
    None
}

fn current_start_time() -> Option<u64> {
    proc_start_time(std::process::id())
}

fn wait_for_child(child: &mut Child, mode: RunMode, started: Instant) -> (Outcome, u64) {
    let deadline = match mode {
        RunMode::Timeout => Some(Duration::from_millis(50)),
        RunMode::Cancel | RunMode::Grandchild => Some(Duration::from_millis(20)),
        _ => None,
    };
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return (status_outcome(status), started.elapsed().as_millis() as u64);
        }
        if let Some(limit) = deadline {
            if started.elapsed() >= limit {
                return (
                    if mode == RunMode::Cancel {
                        Outcome::Cancel
                    } else {
                        Outcome::Timeout
                    },
                    started.elapsed().as_millis() as u64,
                );
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn status_outcome(status: std::process::ExitStatus) -> Outcome {
    if let Some(code) = status.code() {
        if code == 0 {
            Outcome::Success
        } else {
            Outcome::Nonzero(code)
        }
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            Outcome::Signal(status.signal().unwrap_or(-1))
        }
        #[cfg(not(unix))]
        {
            Outcome::Signal(-1)
        }
    }
}

fn append_metadata(
    activity: &Path,
    request: &LaunchRequest,
    adapter: &str,
    duration_ms: u64,
) -> bool {
    let playtime = serde_json::json!({
        "requestId": request.request_id.as_str(),
        "durationMs": duration_ms
    });
    let recent = serde_json::json!({
        "requestId": request.request_id.as_str(),
        "adapter": adapter
    });
    fs::create_dir_all(activity).is_ok()
        && journal::append_json_line(&activity.join("playtime.jsonl"), &playtime).is_ok()
        && journal::append_json_line(&activity.join("recent.jsonl"), &recent).is_ok()
}

fn append_bounded_log(path: &Path, message: &str) {
    if fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        >= 4096
    {
        return;
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let remaining = 4096usize.saturating_sub(
            file.metadata()
                .map(|value| value.len() as usize)
                .unwrap_or(4096),
        );
        let _ = file.write_all(&message.as_bytes()[..remaining.min(message.len())]);
        let _ = file.sync_all();
    }
}

fn write_file_sync(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn adapter_name(kind: LaunchKind) -> &'static str {
    match kind {
        LaunchKind::Libretro => "retroarch",
        LaunchKind::Standalone => "standalone",
        LaunchKind::Port => "port",
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "content cannot be read".to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = std::io::Read::read(&mut file, &mut buffer)
            .map_err(|_| "content cannot be read".to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn resolve_path(
    root: &Path,
    logical: &LogicalPath,
    expected: PathRoot,
    must_exist: bool,
) -> Result<PathBuf, String> {
    let root_name = match expected {
        PathRoot::Roms => "roms",
        PathRoot::DataSaves => "data/saves",
        PathRoot::DataStates => "data/states",
    };
    let base = fs::canonicalize(root.join(root_name))
        .map_err(|_| "fixture root unavailable".to_string())?;
    let candidate = base.join(&logical.relative);
    let resolved = match fs::symlink_metadata(&candidate) {
        Ok(_) => {
            fs::canonicalize(&candidate).map_err(|_| "logical path unavailable".to_string())?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound && !must_exist => {
            let parent = candidate
                .parent()
                .ok_or_else(|| "logical path unavailable".to_string())?;
            fs::canonicalize(parent)
                .map_err(|_| "logical path unavailable".to_string())?
                .join(
                    candidate
                        .file_name()
                        .ok_or_else(|| "logical path unavailable".to_string())?,
                )
        }
        Err(_) => return Err("logical path unavailable".to_string()),
    };
    let canonical_root =
        fs::canonicalize(root).map_err(|_| "fixture root unavailable".to_string())?;
    if !resolved.starts_with(&canonical_root) || (must_exist && !resolved.is_file()) {
        return Err("logical path escapes fixture root".to_string());
    }
    Ok(resolved)
}

fn resolve_path_unchecked(root: &Path, logical: &LogicalPath) -> PathBuf {
    let base = match &logical.root {
        PathRoot::Roms => root.join("roms"),
        PathRoot::DataSaves => root.join("data/saves"),
        PathRoot::DataStates => root.join("data/states"),
    };
    base.join(&logical.relative)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|_| "cannot create fixture".to_string())?;
    for entry in fs::read_dir(source).map_err(|_| "cannot read fixture".to_string())? {
        let entry = entry.map_err(|_| "cannot read fixture".to_string())?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|_| "cannot read fixture".to_string())?
            .is_dir()
        {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|_| "cannot copy fixture".to_string())?;
        }
    }
    Ok(())
}

fn default_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/session-broker/generated-v1")
}

fn temp_root(journey: &str) -> PathBuf {
    std::env::temp_dir().join(format!("session-broker-{}-{}", std::process::id(), journey))
}
