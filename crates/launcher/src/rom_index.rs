use std::{
    cmp::Ordering,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use sim_domain::{Catalog, CatalogEntry};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_ENTRIES: usize = 4096;
pub const MAX_TITLE_BYTES: usize = 256;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_VISIBLE_ROWS: usize = 12;
pub const MAX_SEARCH_RESULTS: usize = 64;
pub const MAX_QUEUE_DEPTH: usize = 32;
const INDEX_SCHEMA: &str = "launcher-rom-index/v1";
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Entry {
    pub content_id: String,
    pub title: String,
    pub path: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema: String,
    pub status: String,
    pub entry_count: usize,
    pub visible_rows: usize,
    pub search_results: usize,
    pub queue_depth: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupBoundary {
    pub group: String,
    pub first_index: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupIndex {
    pub boundaries: Vec<GroupBoundary>,
}

impl GroupIndex {
    pub fn from_entries(entries: &[Entry]) -> Self {
        let mut boundaries = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let group = title_group(&entry.title);
            if boundaries
                .last()
                .is_none_or(|last: &GroupBoundary| last.group != group)
            {
                boundaries.push(GroupBoundary {
                    group,
                    first_index: index,
                });
            }
        }
        Self { boundaries }
    }

    pub fn from_catalog(catalog: &Catalog) -> Self {
        let mut boundaries = Vec::new();
        for (index, entry) in catalog.entries.iter().enumerate() {
            let group = title_group(&entry.title);
            if boundaries
                .last()
                .is_none_or(|last: &GroupBoundary| last.group != group)
            {
                boundaries.push(GroupBoundary {
                    group,
                    first_index: index,
                });
            }
        }
        Self { boundaries }
    }

    pub fn jump_index(&self, selected_index: usize, next: bool) -> Option<&GroupBoundary> {
        if self.boundaries.len() < 2 {
            return None;
        }
        let current = self
            .boundaries
            .partition_point(|boundary| boundary.first_index <= selected_index);
        let current = current.saturating_sub(1);
        let target = if next {
            current + 1
        } else {
            current.checked_sub(1)?
        };
        self.boundaries.get(target)
    }
}

#[derive(Clone, Debug)]
pub struct Result {
    pub report: Report,
}

pub fn normalized_title_sort_key(title: &str) -> String {
    title.trim().nfd().filter_map(latin_base).collect()
}

pub fn title_group(title: &str) -> String {
    let normalized = normalized_title_sort_key(title);
    let Some(grapheme) = normalized.graphemes(true).next() else {
        return "…".into();
    };
    let Some(character) = grapheme.chars().next() else {
        return "…".into();
    };
    if character.is_ascii_digit() {
        "#".into()
    } else if character.is_ascii_alphabetic() {
        character.to_ascii_uppercase().to_string()
    } else if character.is_alphanumeric() {
        grapheme.to_string()
    } else {
        "…".into()
    }
}

pub fn sort_catalog(catalog: &mut Catalog) {
    catalog.entries.sort_by(compare_titles);
}

fn compare_titles(left: &CatalogEntry, right: &CatalogEntry) -> Ordering {
    normalized_title_sort_key(&left.title)
        .cmp(&normalized_title_sort_key(&right.title))
        .then_with(|| left.id.cmp(&right.id))
}

fn latin_base(character: char) -> Option<char> {
    if matches!(character, '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
    {
        return None;
    }
    if character.is_ascii() {
        return Some(character.to_ascii_uppercase());
    }
    let base = match character {
        'À'..='Å' | 'à'..='å' => 'A',
        'Ç' | 'ç' => 'C',
        'È'..='Ë' | 'è'..='ë' => 'E',
        'Ì'..='Ï' | 'ì'..='ï' => 'I',
        'Ñ' | 'ñ' => 'N',
        'Ò'..='Ö' | 'ò'..='ö' => 'O',
        'Ù'..='Ü' | 'ù'..='ü' => 'U',
        'Ý' | 'ý' | 'ÿ' => 'Y',
        'Æ' | 'æ' => 'A',
        'Œ' | 'œ' => 'O',
        'Ð' | 'ð' => 'Ð',
        'Þ' | 'þ' => 'Þ',
        'Ł' | 'ł' => 'Ł',
        _ => character,
    };
    Some(base.to_uppercase().next().unwrap_or(base))
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
    let mut entries = fs::read(catalog_path)
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
    entries.sort_by(|left, right| {
        normalized_title_sort_key(&left.title)
            .cmp(&normalized_title_sort_key(&right.title))
            .then_with(|| left.content_id.cmp(&right.content_id))
    });
    entries.dedup_by(|right, left| {
        normalized_title_sort_key(&left.title) == normalized_title_sort_key(&right.title)
            && left.path.eq_ignore_ascii_case(&right.path)
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
    let mut entries: Vec<Entry> = serde_json::from_value(entries).ok()?;
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
    entries.sort_by(|left, right| {
        normalized_title_sort_key(&left.title)
            .cmp(&normalized_title_sort_key(&right.title))
            .then_with(|| left.content_id.cmp(&right.content_id))
    });
    Some(entries)
}

fn write_index(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(".rom-index.json.tmp");
    let _ = fs::remove_file(&temporary);
    let groups = GroupIndex::from_entries(entries);
    let value = serde_json::json!({ "schema": INDEX_SCHEMA, "entries": entries, "groups": groups });
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
    fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}
