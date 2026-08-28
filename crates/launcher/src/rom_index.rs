use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use serde::{Deserialize, Serialize};
use sim_domain::Catalog;

pub const MAX_ENTRIES: usize = 4096;
pub const MAX_TITLE_BYTES: usize = 256;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_VISIBLE_ROWS: usize = 12;
pub const MAX_SEARCH_RESULTS: usize = 64;
pub const MAX_QUEUE_DEPTH: usize = 32;
const INDEX_SCHEMA: &str = "launcher-rom-index/v1";
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Entry {
    pub content_id: String,
    pub title: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema: String,
    pub status: String,
    pub entry_count: usize,
    pub visible_rows: usize,
    pub search_results: usize,
    pub queue_depth: usize,
}

#[derive(Clone, Debug)]
pub struct Result {
    pub report: Report,
}

pub fn spawn(catalog_path: PathBuf, state_root: PathBuf) -> Receiver<Result> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = build(&catalog_path, &state_root);
        let _ = sender.send(result);
    });
    receiver
}

fn build(catalog_path: &Path, state_root: &Path) -> Result {
    let path = state_root.join("rom-index.json");
    if let Some(entries) = read_index(&path) {
        return result("ready", entries);
    }
    let recovering = path.exists();
    let entries = fs::read(catalog_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Catalog>(&bytes).ok())
        .map(|catalog| {
            catalog
                .entries
                .into_iter()
                .take(MAX_ENTRIES)
                .filter_map(|entry| {
                    if entry.id.is_empty()
                        || entry.title.is_empty()
                        || entry.title.len() > MAX_TITLE_BYTES
                    {
                        return None;
                    }
                    Some(Entry {
                        content_id: entry.id,
                        title: entry.title,
                        path: "generated/content.bin".into(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut entries = entries;
    entries.sort_by(|left, right| {
        left.title
            .to_ascii_lowercase()
            .cmp(&right.title.to_ascii_lowercase())
            .then_with(|| left.content_id.cmp(&right.content_id))
    });
    entries.dedup_by(|right, left| {
        left.title.eq_ignore_ascii_case(&right.title) && left.path.eq_ignore_ascii_case(&right.path)
    });
    let status = if entries.is_empty() {
        "partial"
    } else if recovering {
        "recovered"
    } else {
        "rebuilt"
    };
    let _ = write_index(&path, &entries);
    result(status, entries)
}

fn result(status: &str, entries: Vec<Entry>) -> Result {
    Result {
        report: Report {
            schema: INDEX_SCHEMA.into(),
            status: status.into(),
            entry_count: entries.len(),
            visible_rows: MAX_VISIBLE_ROWS,
            search_results: MAX_SEARCH_RESULTS,
            queue_depth: MAX_QUEUE_DEPTH,
        },
    }
}

fn read_index(path: &Path) -> Option<Vec<Entry>> {
    if fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_INDEX_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if value.get("schema")?.as_str()? != INDEX_SCHEMA {
        return None;
    }
    let entries = value.get("entries")?.clone();
    let entries: Vec<Entry> = serde_json::from_value(entries).ok()?;
    if entries.len() > MAX_ENTRIES
        || entries.iter().any(|entry| {
            entry.content_id.is_empty()
                || entry.content_id.len() > 128
                || entry.title.is_empty()
                || entry.title.len() > MAX_TITLE_BYTES
                || entry.path.is_empty()
                || entry.path.len() > MAX_PATH_BYTES
                || entry.path.starts_with('/')
                || entry.path.contains("..")
        })
    {
        return None;
    }
    Some(entries)
}

fn write_index(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(".rom-index.json.tmp");
    let _ = fs::remove_file(&temporary);
    let value = serde_json::json!({ "schema": INDEX_SCHEMA, "entries": entries });
    let bytes = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    fs::File::open(path.parent().unwrap_or_else(|| std::path::Path::new(".")))?.sync_all()
}
