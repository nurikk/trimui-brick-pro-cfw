use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::SessionResult;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sim_platform_contract::PlatformState;

const JOURNAL_SCHEMA: &str = "trimui-session-broker-journal/v1";
const JOURNAL_OWNER: &str = "session-broker-owner-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalPhase {
    Preparing,
    Running,
    Finalizing,
    Completed,
    Recovered,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JournalRecord {
    pub schema: String,
    pub owner: String,
    pub request_id: String,
    pub phase: JournalPhase,
    pub adapter: String,
    pub marker: String,
    pub pid: Option<u32>,
    pub pgid: Option<i32>,
    #[serde(rename = "startTime")]
    pub start_time: Option<u64>,
    pub released: bool,
    pub snapshot: PlatformState,
    pub checksum: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Rejection {
    #[serde(rename = "type")]
    pub result_type: &'static str,
    pub journey: String,
    pub accepted: bool,
    pub reason: String,
    pub restored: bool,
}

pub fn new_record(
    request_id: &str,
    adapter: &str,
    marker: &str,
    snapshot: PlatformState,
) -> JournalRecord {
    let mut record = JournalRecord {
        schema: JOURNAL_SCHEMA.to_string(),
        owner: JOURNAL_OWNER.to_string(),
        request_id: request_id.to_string(),
        phase: JournalPhase::Preparing,
        adapter: adapter.to_string(),
        marker: marker.to_string(),
        pid: None,
        pgid: None,
        start_time: None,
        released: false,
        snapshot,
        checksum: String::new(),
    };
    seal(&mut record);
    record
}

pub fn create(path: &Path, record: &mut JournalRecord) -> io::Result<()> {
    if path.exists() || fs::symlink_metadata(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "journal target already exists",
        ));
    }
    seal(record);
    let bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    sync_parent(path)
}

pub fn transition(path: &Path, record: &mut JournalRecord) -> io::Result<()> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "journal target is a symlink",
        ));
    }
    seal(record);
    let bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    let temporary = durable_temp(path, &bytes)?;
    match fs::rename(&temporary, path) {
        Ok(()) => sync_parent(path),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

pub fn read_valid(path: &Path) -> Option<JournalRecord> {
    let bytes = fs::read(path).ok()?;
    let record: JournalRecord = serde_json::from_slice(&bytes).ok()?;
    if record.schema != JOURNAL_SCHEMA
        || record.owner != JOURNAL_OWNER
        || record.request_id.is_empty()
        || record.marker.len() != 64
        || !record
            .marker
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    let expected = checksum(&record).ok()?;
    (record.checksum == expected).then_some(record)
}

pub fn append_result(path: &Path, result: &SessionResult) -> io::Result<()> {
    let mut line = serde_json::to_vec(result).map_err(io::Error::other)?;
    line.push(b'\n');
    append_bytes(path, &line)
}

pub fn append_json_line(path: &Path, value: &serde_json::Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    append_bytes(path, &line)
}

fn append_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "activity target is a symlink",
        ));
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn durable_temp(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "journal has no parent"))?;
    for _ in 0..8 {
        let suffix = random_hex(16)?;
        let name = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "journal has no filename")
        })?;
        let temporary = parent.join(format!(".{}.{}.tmp", name.to_string_lossy(), suffix));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
                return Ok(temporary);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "journal temporary name collision",
    ))
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "journal has no parent"))?;
    File::open(parent)?.sync_all()
}

pub fn random_marker() -> io::Result<String> {
    random_hex(32)
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut data = vec![0u8; bytes];
    let mut offset = 0;
    while offset < data.len() {
        let count =
            unsafe { libc::getrandom(data[offset..].as_mut_ptr().cast(), data.len() - offset, 0) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "OS CSPRNG returned no bytes",
            ));
        }
        offset += count as usize;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(data.len() * 2);
    for byte in data {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(result)
}

fn seal(record: &mut JournalRecord) {
    record.checksum = checksum(record).unwrap_or_default();
}

fn checksum(record: &JournalRecord) -> io::Result<String> {
    let mut unsigned = record.clone();
    unsigned.checksum.clear();
    let bytes = serde_json::to_vec(&unsigned).map_err(io::Error::other)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
