use std::{
    fmt::Write as FmtWrite,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const UPDATE_DIR: &str = ".brickpro/data/update";
const MAX_RECORD_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Slot {
    #[serde(rename = "A")]
    A,
    #[serde(rename = "B")]
    B,
}

impl Slot {
    pub fn inactive(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct State {
    pub current: Slot,
    pub previous: Slot,
    pub pending: Option<Slot>,
    pub attempts: u8,
    pub last_known_good: Slot,
    pub rollback_reason: Option<String>,
    pub current_release: String,
    pub previous_release: String,
    pub pending_release: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateStatus {
    pub schema: String,
    pub current_release: String,
    pub target_release: String,
    pub source: String,
    pub stage: String,
    pub progress_percent: u8,
    pub error: Option<String>,
    pub action: String,
    pub journal: Vec<String>,
}

impl UpdateStatus {
    pub fn new(current_release: &str, target_release: &str, source: &str) -> Self {
        Self {
            schema: "update-status/v1".into(),
            current_release: current_release.into(),
            target_release: target_release.into(),
            source: source.into(),
            stage: "preflight".into(),
            progress_percent: 0,
            error: None,
            action: "Checking compatibility, space, power, and protected data".into(),
            journal: vec!["preflight".into()],
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            current: Slot::A,
            previous: Slot::A,
            pending: None,
            attempts: 0,
            last_known_good: Slot::A,
            rollback_reason: None,
            current_release: "base".into(),
            previous_release: "empty".into(),
            pending_release: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    generation: u64,
    state: State,
    checksum: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UncheckedRecord<'a> {
    generation: u64,
    state: &'a State,
}

fn update_dir(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute() || root == Path::new("/") || root.join("dev").exists() {
        bail!("simulation root must be an absolute, non-device fixture root")
    }
    Ok(root.join(UPDATE_DIR))
}

fn status_path(root: &Path) -> Result<PathBuf> {
    Ok(update_dir(root)?.join("status.json"))
}

pub fn load_update_status(root: &Path) -> Result<Option<UpdateStatus>> {
    let path = status_path(root)?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_RECORD_BYTES {
        bail!("update status is not a bounded regular file")
    }
    let status: UpdateStatus = serde_json::from_slice(&fs::read(path)?)?;
    validate_update_status(&status)?;
    Ok(Some(status))
}

fn valid_stage(stage: &str) -> bool {
    [
        "preflight",
        "download",
        "unpack",
        "apply",
        "first-boot",
        "complete",
        "rollback",
        "error",
    ]
    .contains(&stage)
}

fn validate_update_status(status: &UpdateStatus) -> Result<()> {
    if status.schema == "update-status/v1"
        && matches!(status.source.as_str(), "online" | "sideload")
        && valid_stage(&status.stage)
        && status.progress_percent <= 100
        && (1..=48).contains(&status.current_release.len())
        && (1..=48).contains(&status.target_release.len())
        && (1..=256).contains(&status.action.len())
        && status
            .error
            .as_ref()
            .is_none_or(|error| (1..=256).contains(&error.len()))
        && (1..=16).contains(&status.journal.len())
        && status.journal.iter().all(|stage| valid_stage(stage))
    {
        return Ok(());
    }
    bail!("update status is invalid")
}

pub fn publish_update_status(root: &Path, status: &UpdateStatus) -> Result<()> {
    validate_update_status(status)?;
    let dir = update_dir(root)?;
    fs::create_dir_all(&dir)?;
    let path = status_path(root)?;
    let temporary = dir.join("status.json.tmp");
    let mut bytes = serde_json::to_vec_pretty(status)?;
    bytes.push(b'\n');
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

pub fn advance_update_status(
    root: &Path,
    stage: &str,
    progress: u8,
    error: Option<&str>,
    action: &str,
) -> Result<()> {
    let mut status =
        load_update_status(root)?.ok_or_else(|| anyhow!("update status is missing"))?;
    status.stage = stage.into();
    status.progress_percent = progress;
    status.error = error.map(str::to_owned);
    status.action = action.into();
    if status
        .journal
        .last()
        .is_none_or(|previous| previous != stage)
    {
        if status.journal.len() == 16 {
            status.journal.remove(0);
        }
        status.journal.push(stage.into());
    }
    publish_update_status(root, &status)
}

fn checksum(generation: u64, state: &State) -> Result<String> {
    let bytes = serde_json::to_vec(&UncheckedRecord { generation, state })?;
    Ok(hex(Sha256::digest(bytes).as_ref()))
}

fn hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

fn read_record(path: &Path) -> Result<(u64, State, Vec<u8>)> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_RECORD_BYTES {
        bail!("state record is oversized")
    }
    let raw = fs::read(path)?;
    let record: Record = serde_json::from_slice(&raw)
        .with_context(|| format!("read state record {}", path.display()))?;
    if record.checksum != checksum(record.generation, &record.state)? {
        bail!("state record checksum mismatch")
    }
    Ok((record.generation, record.state, raw))
}

pub fn state_path(root: &Path, index: u8) -> Result<PathBuf> {
    if index > 1 {
        return Err(anyhow!("state record index must be 0 or 1"));
    }
    Ok(update_dir(root)?.join(format!("state.{index}.json")))
}

pub fn load(root: &Path) -> Result<(u64, State)> {
    let mut valid = Vec::new();
    for index in 0..=1 {
        if let Ok(record) = read_record(&state_path(root, index)?) {
            valid.push(record);
        }
    }
    if valid.len() == 2 && valid[0].0 == valid[1].0 && valid[0].2 != valid[1].2 {
        bail!("ambiguous boot-state records at the same generation")
    }
    valid
        .into_iter()
        .max_by_key(|record| record.0)
        .map(|(generation, state, _)| (generation, state))
        .ok_or_else(|| anyhow!("no valid boot-state record"))
}

pub fn load_or_initialize(root: &Path) -> Result<(u64, State)> {
    match load(root) {
        Ok(value) => Ok(value),
        Err(error) => {
            let records_exist = state_path(root, 0)?.exists() || state_path(root, 1)?.exists();
            if records_exist {
                return Err(error);
            }
            let state = State::default();
            store(root, 0, &state)?;
            Ok((0, state))
        }
    }
}

pub fn store(root: &Path, generation: u64, state: &State) -> Result<()> {
    if state.attempts > 3 {
        bail!("pending boot attempts exceed bound")
    }
    let dir = update_dir(root)?;
    fs::create_dir_all(&dir)?;
    let next_generation = generation
        .checked_add(1)
        .ok_or_else(|| anyhow!("generation overflow"))?;
    let index = (next_generation % 2) as u8;
    let record = Record {
        generation: next_generation,
        state: state.clone(),
        checksum: checksum(next_generation, state)?,
    };
    let bytes = serde_json::to_vec_pretty(&record)?;
    let path = state_path(root, index)?;
    let temporary = dir.join(format!("state.{index}.json.tmp"));
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    fs::File::open(&dir)?.sync_all()?;
    Ok(())
}

pub fn select(root: &Path) -> Result<(Slot, &'static str, u8)> {
    let (generation, mut state) = load_or_initialize(root)?;
    let slot = match state.pending {
        Some(pending) if state.attempts < 3 => {
            state.attempts += 1;
            store(root, generation, &state)?;
            let _ = advance_update_status(
                root,
                "first-boot",
                90,
                None,
                "If startup fails three times, the previous release starts automatically",
            );
            (pending, "pending")
        }
        Some(_) => {
            state.pending = None;
            state.pending_release = None;
            state.rollback_reason = Some("automatic-rollback".into());
            state.attempts = 0;
            store(root, generation, &state)?;
            let _ = advance_update_status(
                root,
                "error",
                100,
                Some("The new release did not pass first boot"),
                "The previous release was restored; retry with a complete compatible package",
            );
            (state.previous, "automatic-rollback")
        }
        None => (state.current, "current"),
    };
    Ok((slot.0, slot.1, state.attempts))
}

pub fn mark_healthy(root: &Path, evidence: [bool; 5]) -> Result<State> {
    if evidence.iter().any(|value| !value) {
        bail!("health-gate evidence incomplete")
    }
    let (generation, mut state) = load_or_initialize(root)?;
    let pending = state.pending.ok_or_else(|| anyhow!("no pending release"))?;
    let former_current = state.current;
    state.previous = former_current;
    state.previous_release = state.current_release.clone();
    state.current = pending;
    state.current_release = state
        .pending_release
        .take()
        .ok_or_else(|| anyhow!("pending release id missing"))?;
    state.pending = None;
    state.attempts = 0;
    state.last_known_good = pending;
    state.rollback_reason = None;
    store(root, generation, &state)?;
    let _ = advance_update_status(
        root,
        "complete",
        100,
        None,
        "Update complete; Restore previous remains available from Recovery",
    );

    Ok(state)
}

pub fn rollback(root: &Path) -> Result<State> {
    let (generation, mut state) = load_or_initialize(root)?;
    if state.pending.is_some() {
        state.pending = None;
        state.pending_release = None;
        state.attempts = 0;
        state.rollback_reason = Some("manual-rollback".into());
    } else if state.current != state.previous {
        std::mem::swap(&mut state.current, &mut state.previous);
        std::mem::swap(&mut state.current_release, &mut state.previous_release);
        state.last_known_good = state.current;
        state.attempts = 0;
        state.rollback_reason = Some("manual-rollback".into());
    } else {
        bail!("previous slot is not available")
    }
    store(root, generation, &state)?;
    let _ = advance_update_status(
        root,
        "rollback",
        100,
        None,
        "Previous release restored; ROM library contents were not changed",
    );
    Ok(state)
}

pub fn prepare_pending(root: &Path, slot: Slot, release: &str) -> Result<()> {
    let (generation, mut state) = load_or_initialize(root)?;
    if slot == state.current {
        bail!("pending slot must be inactive")
    }
    state.pending = Some(slot);
    state.pending_release = Some(release.to_owned());
    state.attempts = 0;
    state.rollback_reason = None;
    store(root, generation, &state)
}

pub fn protected_hashes(root: &Path) -> Result<[String; 5]> {
    Ok([
        tree_hash(&root.join("roms"))?,
        tree_hash(&root.join("data/saves"))?,
        tree_hash(&root.join("data/states"))?,
        tree_hash(&root.join("data/resume"))?,
        tree_hash(&root.join("data/settings"))?,
    ])
}

pub fn tree_hash(path: &Path) -> Result<String> {
    if !path.is_dir() {
        bail!("protected path is missing: {}", path.display())
    }
    let mut entries = Vec::new();
    collect_files(path, Path::new(""), &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (name, bytes) in entries {
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    let digest = digest.finalize();
    Ok(hex(digest.as_ref()))
}

fn collect_files(path: &Path, relative: &Path, entries: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)?;
        let child_relative = relative.join(entry.file_name());
        if metadata.is_dir() {
            collect_files(&child, &child_relative, entries)?;
        } else if metadata.is_file() {
            entries.push((
                child_relative.to_string_lossy().into_owned(),
                fs::read(child)?,
            ));
        } else {
            bail!("protected tree contains unsupported filesystem object")
        }
    }
    Ok(())
}
