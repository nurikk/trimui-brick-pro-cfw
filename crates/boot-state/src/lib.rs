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
    pub current_release_sequence: u64,
    pub previous_release: String,
    pub previous_release_sequence: u64,
    pub pending_release: Option<String>,
    pub pending_release_sequence: Option<u64>,
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
            current_release_sequence: 0,
            previous_release: "empty".into(),
            previous_release_sequence: 0,
            pending_release: None,
            pending_release_sequence: None,
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

fn checksum(generation: u64, state: &State) -> Result<String> {
    let bytes = serde_json::to_vec(&UncheckedRecord { generation, state })?;
    Ok(hex(&Sha256::digest(bytes)))
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
            (pending, "pending")
        }
        Some(_) => {
            state.pending = None;
            state.pending_release = None;
            state.pending_release_sequence = None;
            state.rollback_reason = Some("automatic-rollback".into());
            store(root, generation, &state)?;
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
    state.previous_release_sequence = state.current_release_sequence;
    state.current = pending;
    state.current_release = state
        .pending_release
        .take()
        .ok_or_else(|| anyhow!("pending release id missing"))?;
    state.current_release_sequence = state
        .pending_release_sequence
        .take()
        .ok_or_else(|| anyhow!("pending release sequence missing"))?;
    state.pending = None;
    state.attempts = 0;
    state.last_known_good = pending;
    state.rollback_reason = None;
    store(root, generation, &state)?;
    Ok(state)
}

pub fn prepare_pending(root: &Path, slot: Slot, release: &str, sequence: u64) -> Result<()> {
    let (generation, mut state) = load_or_initialize(root)?;
    if slot == state.current {
        bail!("pending slot must be inactive")
    }
    state.pending = Some(slot);
    state.pending_release = Some(release.to_owned());
    state.pending_release_sequence = Some(sequence);
    state.attempts = 0;
    state.rollback_reason = None;
    store(root, generation, &state)
}

pub fn protected_hashes(root: &Path) -> Result<[String; 3]> {
    Ok([
        tree_hash(&root.join("roms"))?,
        tree_hash(&root.join("data/saves"))?,
        tree_hash(&root.join("data/states"))?,
    ])
}

fn tree_hash(path: &Path) -> Result<String> {
    if !path.is_dir() {
        bail!("protected path is missing: {}", path.display())
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if !child.is_file() {
            bail!("protected tree contains non-file entry")
        }
        entries.push((entry.file_name(), fs::read(child)?));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (name, bytes) in entries {
        digest.update(name.as_encoded_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(hex(&digest.finalize()))
}
