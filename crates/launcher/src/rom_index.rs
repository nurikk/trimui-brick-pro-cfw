use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc::{self, Receiver},
    },
    thread,
    time::UNIX_EPOCH,
};

use sha2::{Digest, Sha256};
use sim_domain::{Catalog, CatalogEntry};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_ENTRIES: usize = 30_000;
pub const MAX_TITLE_BYTES: usize = 256;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_VISIBLE_ROWS: usize = 12;
pub const MAX_SEARCH_RESULTS: usize = 64;
pub const MAX_QUEUE_DEPTH: usize = 32;
const INDEX_SCHEMA: &str = "launcher-rom-index/v1";
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const ROM_EXTENSIONS: [&str; 19] = [
    "7z", "bin", "chd", "gba", "gb", "gbc", "gen", "iso", "md", "m3u", "n64", "nds", "nes", "pce",
    "sfc", "smc", "v64", "z64", "zip",
];
const SERVICE_NAMES: [&str; 5] = [
    "bios",
    "saves",
    "states",
    "system volume information",
    "lost+found",
];

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Entry {
    pub content_id: String,
    pub title: String,
    pub path: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub friendly_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub system_id: String,
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub source_size: u64,
    #[serde(default)]
    pub source_modified_nanos: u128,
    #[serde(default)]
    pub source_device: u64,
    #[serde(default)]
    pub source_inode: u64,
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
    #[serde(default)]
    pub added: usize,
    #[serde(default)]
    pub removed: usize,
    #[serde(default)]
    pub changed: usize,
    #[serde(default)]
    pub skipped: usize,
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
            .partition_point(|boundary| boundary.first_index <= selected_index)
            .saturating_sub(1);
        self.boundaries.get(if next {
            current + 1
        } else {
            current.checked_sub(1)?
        })
    }
}

#[derive(Clone, Debug)]
pub struct Result {
    pub report: Report,
    pub entries: Vec<Entry>,
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
        let _ = sender.send(build_catalog(&catalog_path, &state_root));
    });
    receiver
}

/// Refresh a ROM root into the same atomic JSON index used by the launcher.
/// Cancellation deliberately leaves the last complete index untouched.
pub fn refresh(rom_root: &Path, state_root: &Path, cancelled: &AtomicBool) -> Result {
    let path = state_root.join("rom-index.json");
    let previous = read_index(&path).unwrap_or_default();
    let (mut entries, skipped) = scan_roms(rom_root, &previous, cancelled);
    if cancelled.load(AtomicOrdering::Acquire) {
        return result("cancelled", previous, 0, 0, 0, skipped);
    }
    entries.sort_by(compare_entries);
    let (added, removed, changed) = delta(&previous, &entries);
    let status = if added + removed + changed == 0 {
        "ready"
    } else {
        "refreshed"
    };
    if write_index(&path, &entries).is_err() {
        return result("write-failed", previous, 0, 0, 0, skipped);
    }
    result(status, entries, added, removed, changed, skipped)
}

fn build_catalog(catalog_path: &Path, state_root: &Path) -> Result {
    let path = state_root.join("rom-index.json");
    let previous = read_index(&path);
    let recovering = path.exists() && previous.is_none();
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
                        content_id: entry.id.clone(),
                        title: entry.title.clone(),
                        path: "generated/content.bin".into(),
                        filename: entry.title.clone(),
                        friendly_name: entry.title.clone(),
                        display_name: entry.title,
                        system_id: entry.system,
                        source_hash: format!("catalog:{}", entry.id),
                        source_size: 0,
                        source_modified_nanos: 0,
                        source_device: 0,
                        source_inode: 0,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(previous) = &previous {
        preserve_display_names(&mut entries, previous);
    }
    entries.sort_by(compare_entries);
    let (added, removed, changed) = delta(previous.as_deref().unwrap_or(&[]), &entries);
    let status = if entries.is_empty() {
        "partial"
    } else if recovering {
        "recovered"
    } else if added + removed + changed == 0 {
        "ready"
    } else {
        "rebuilt"
    };
    if write_index(&path, &entries).is_err() {
        return result("write-failed", previous.unwrap_or_default(), 0, 0, 0, 0);
    }
    result(status, entries, added, removed, changed, 0)
}

fn scan_roms(root: &Path, previous: &[Entry], cancelled: &AtomicBool) -> (Vec<Entry>, usize) {
    let mut files = Vec::new();
    let mut skipped = 0;
    walk(root, root, &mut files, &mut skipped, cancelled);
    files.sort();
    let playlist_members = files
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("m3u")))
        .filter_map(|path| playlist_members(root, path).ok())
        .flatten()
        .collect::<HashSet<_>>();
    let by_path: HashMap<_, _> = previous
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    let mut entries = Vec::new();
    for path in files
        .into_iter()
        .filter(|path| !playlist_members.contains(path))
        .take(MAX_ENTRIES)
    {
        if cancelled.load(AtomicOrdering::Acquire) {
            break;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            skipped += 1;
            continue;
        };
        let Some(relative) = relative.to_str().map(|value| value.replace('\\', "/")) else {
            skipped += 1;
            continue;
        };
        let Ok(metadata) = fs::metadata(&path) else {
            skipped += 1;
            continue;
        };
        let size = metadata.len();
        let modified = modified_nanos(&metadata);
        let (device, inode) = file_identity(&metadata);
        let source_hash = if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("m3u"))
        {
            hash_playlist(root, &path).unwrap_or_default()
        } else if let Some(old) = by_path
            .get(relative.as_str())
            .filter(|old| old.source_size == size && old.source_modified_nanos == modified)
        {
            old.source_hash.clone()
        } else {
            hash_file(&path).unwrap_or_default()
        };
        if source_hash.is_empty() {
            skipped += 1;
            continue;
        }
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned();
        let friendly_name = friendly_name(&filename);
        if filename.is_empty() || friendly_name.is_empty() {
            skipped += 1;
            continue;
        }
        let content_id = stable_content_id(&source_hash, device, inode, previous, &entries);
        entries.push(Entry {
            content_id,
            title: friendly_name.clone(),
            path: relative,
            filename,
            friendly_name: friendly_name.clone(),
            display_name: friendly_name,
            system_id: system_id(root, &path),
            source_hash,
            source_size: size,
            source_modified_nanos: modified,
            source_device: device,
            source_inode: inode,
        });
    }
    preserve_display_names(&mut entries, previous);
    (entries, skipped)
}

fn walk(
    root: &Path,
    current: &Path,
    files: &mut Vec<PathBuf>,
    skipped: &mut usize,
    cancelled: &AtomicBool,
) {
    if cancelled.load(AtomicOrdering::Acquire) {
        return;
    }
    let Ok(read_dir) = fs::read_dir(current) else {
        *skipped += 1;
        return;
    };
    for item in read_dir {
        if cancelled.load(AtomicOrdering::Acquire) {
            return;
        }
        let Ok(item) = item else {
            *skipped += 1;
            continue;
        };
        let path = item.path();
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            *skipped += 1;
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if name.starts_with('.') || SERVICE_NAMES.contains(&lower.as_str()) {
            *skipped += 1;
            continue;
        }
        let Ok(kind) = item.file_type() else {
            *skipped += 1;
            continue;
        };
        if kind.is_symlink() {
            *skipped += 1;
            continue;
        }
        if kind.is_dir() {
            walk(root, &path, files, skipped, cancelled);
            continue;
        }
        if kind.is_file() && is_rom(&path) {
            files.push(path);
        } else {
            *skipped += 1;
        }
    }
}

fn is_rom(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ROM_EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(buffer.get(..count).unwrap_or_default());
    }
    Ok(format!("sha256-{:x}", hasher.finalize()))
}

fn hash_playlist(root: &Path, path: &Path) -> std::io::Result<String> {
    let members = playlist_members(root, path)?;
    if members.is_empty() {
        return Ok(String::new());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"m3u\0");
    for disc in members {
        hasher.update(hash_file(&disc)?.as_bytes());
        hasher.update(b"\0");
    }
    Ok(format!("sha256-{:x}", hasher.finalize()))
}

fn playlist_members(root: &Path, path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let root = root.canonicalize()?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let relative = Path::new(line);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "unsafe playlist member",
                ));
            }
            let candidate = parent.join(relative);
            if !fs::symlink_metadata(&candidate)?.file_type().is_file()
                || !candidate.canonicalize()?.starts_with(&root)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid playlist member",
                ));
            }
            Ok(candidate)
        })
        .collect()
}

fn stable_content_id(
    hash: &str,
    device: u64,
    inode: u64,
    previous: &[Entry],
    pending: &[Entry],
) -> String {
    if let Some(entry) = previous
        .iter()
        .find(|entry| entry.source_device == device && entry.source_inode == inode && device != 0)
    {
        return entry.content_id.clone();
    }
    let matches: Vec<_> = previous
        .iter()
        .filter(|entry| entry.source_hash == hash)
        .collect();
    if matches.len() == 1 {
        return matches[0].content_id.clone();
    }
    if matches.is_empty() && !pending.iter().any(|entry| entry.content_id == hash) {
        return hash.into();
    }
    let mut hasher = Sha256::new();
    hasher.update(hash.as_bytes());
    hasher.update(device.to_le_bytes());
    hasher.update(inode.to_le_bytes());
    format!("sha256-{:x}", hasher.finalize())
}

fn preserve_display_names(entries: &mut [Entry], previous: &[Entry]) {
    let previous: HashMap<_, _> = previous
        .iter()
        .map(|entry| (entry.content_id.as_str(), entry))
        .collect();
    for entry in entries {
        if let Some(old) = previous.get(entry.content_id.as_str()) {
            if !old.display_name.is_empty() && old.display_name != old.friendly_name {
                entry.display_name = old.display_name.clone();
                entry.title = old.display_name.clone();
            }
        }
    }
}

fn friendly_name(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(filename);
    stem.replace(['_', '.'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn system_id(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

fn modified_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos())
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}
#[cfg(not(unix))]
fn file_identity(_: &fs::Metadata) -> (u64, u64) {
    (0, 0)
}

fn delta(previous: &[Entry], entries: &[Entry]) -> (usize, usize, usize) {
    let old: HashMap<_, _> = previous
        .iter()
        .map(|entry| (entry.content_id.as_str(), entry))
        .collect();
    let new: HashMap<_, _> = entries
        .iter()
        .map(|entry| (entry.content_id.as_str(), entry))
        .collect();
    let added = new.keys().filter(|id| !old.contains_key(**id)).count();
    let removed = old.keys().filter(|id| !new.contains_key(**id)).count();
    let changed = new
        .iter()
        .filter(|(id, entry)| {
            old.get(**id).is_some_and(|old| {
                old.path != entry.path
                    || old.source_hash != entry.source_hash
                    || old.title != entry.title
            })
        })
        .count();
    (added, removed, changed)
}

fn compare_entries(left: &Entry, right: &Entry) -> Ordering {
    normalized_title_sort_key(&left.title)
        .cmp(&normalized_title_sort_key(&right.title))
        .then_with(|| left.content_id.cmp(&right.content_id))
        .then_with(|| left.path.cmp(&right.path))
}

fn result(
    status: &str,
    entries: Vec<Entry>,
    added: usize,
    removed: usize,
    changed: usize,
    skipped: usize,
) -> Result {
    Result {
        report: Report {
            schema: INDEX_SCHEMA.into(),
            status: status.into(),
            entry_count: entries.len(),
            visible_rows: MAX_VISIBLE_ROWS,
            search_results: MAX_SEARCH_RESULTS,
            queue_depth: MAX_QUEUE_DEPTH,
            added,
            removed,
            changed,
            skipped,
        },
        entries,
    }
}

fn read_index(path: &Path) -> Option<Vec<Entry>> {
    if fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    if fs::metadata(path).ok()?.len() > MAX_INDEX_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if value.get("schema")?.as_str()? != INDEX_SCHEMA {
        return None;
    }
    let mut entries: Vec<Entry> = serde_json::from_value(value.get("entries")?.clone()).ok()?;
    if entries.len() > MAX_ENTRIES || entries.iter().any(invalid_entry) {
        return None;
    }
    entries.sort_by(compare_entries);
    Some(entries)
}

fn invalid_entry(entry: &Entry) -> bool {
    entry.content_id.is_empty()
        || entry.content_id.len() > 128
        || entry.title.is_empty()
        || entry.title.len() > MAX_TITLE_BYTES
        || entry.path.is_empty()
        || entry.path.len() > MAX_PATH_BYTES
        || Path::new(&entry.path).is_absolute()
        || entry.path.split('/').any(|part| part == "..")
}

fn write_index(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(".rom-index.json.tmp");
    let _ = fs::remove_file(&temporary);
    let value = serde_json::json!({ "schema": INDEX_SCHEMA, "entries": entries, "groups": GroupIndex::from_entries(entries) });
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
