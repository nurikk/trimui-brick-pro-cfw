//! Controller-facing, allow-listed storage maintenance. No shell, host paths, or system roots.

use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Write},
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use flate2::read::DeflateDecoder;
use serde::Serialize;
pub use storage_layout::UserRoot as Root;
use storage_layout::{resolve_user_path, validate_logical_path};

pub const PAGE_SIZE: usize = 128;
pub const DIRECTORY_ENTRY_BUDGET: usize = 10_000;
pub const MAX_ARCHIVE_FILES: usize = 1_000;
pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ARCHIVE_EXPANSION: u64 = 100;
const INTERNAL: &str = ".brickpro-file-manager";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Conflict {
    Skip,
    Replace,
    Rename,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    pub directory: bool,
    pub bytes: u64,
    pub hidden: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub root: Root,
    pub path: String,
    pub entries: Vec<Entry>,
    pub next_offset: Option<usize>,
    pub scanned_entries: usize,
    pub expert_mode: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub count: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub completed_files: u64,
    pub total_files: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub action: String,
    pub source: Option<String>,
    pub destination: String,
    pub summary: Summary,
    pub replaced: Option<TrashReceipt>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashReceipt {
    pub root: Root,
    pub original_path: String,
    pub trash_path: String,
    pub summary: Summary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationPreview {
    pub root: Root,
    pub path: String,
    pub summary: Summary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportHandoff {
    pub transport: &'static str,
    pub root: Root,
    pub message: &'static str,
}

/// UI adapter state: input is controller actions, never a filesystem command string.
pub struct Controller {
    manager: FileManager,
    root: Root,
    expert_mode: bool,
}

impl Controller {
    pub fn new(manager: FileManager) -> Self {
        Self {
            manager,
            root: Root::Roms,
            expert_mode: false,
        }
    }

    pub fn select_root(&mut self, root: Root) {
        self.root = root;
    }

    pub fn set_expert_mode(&mut self, enabled: bool) {
        self.expert_mode = enabled;
    }

    pub fn import_handoffs(&self) -> [ImportHandoff; 2] {
        self.manager.import_handoffs()
    }

    pub fn browse(&self, path: &str, offset: usize) -> Result<Listing> {
        self.guard_path(path, true)?;
        self.manager
            .browse(self.root, path, offset, self.expert_mode)
    }

    pub fn preview(&self, root: Root, path: &str) -> Result<OperationPreview> {
        self.guard_path(path, false)?;
        Ok(OperationPreview {
            root,
            path: path.into(),
            summary: inspect_tree(&self.manager.resolve(root, path, false)?)?,
        })
    }

    pub fn copy<F>(
        &self,
        source_root: Root,
        source: &str,
        target_root: Root,
        target: &str,
        conflict: Conflict,
        progress: F,
    ) -> Result<Receipt>
    where
        F: FnMut(Progress) -> bool,
    {
        self.guard_path(source, false)?;
        self.guard_path(target, false)?;
        self.manager
            .copy(source_root, source, target_root, target, conflict, progress)
    }

    pub fn move_path<F>(
        &self,
        source_root: Root,
        source: &str,
        target_root: Root,
        target: &str,
        conflict: Conflict,
        progress: F,
    ) -> Result<Receipt>
    where
        F: FnMut(Progress) -> bool,
    {
        self.guard_path(source, false)?;
        self.guard_path(target, false)?;
        self.manager
            .move_path(source_root, source, target_root, target, conflict, progress)
    }

    pub fn rename(
        &self,
        root: Root,
        source: &str,
        target: &str,
        conflict: Conflict,
    ) -> Result<Receipt> {
        self.guard_path(source, false)?;
        self.guard_path(target, false)?;
        self.manager.rename(root, source, target, conflict)
    }

    pub fn new_folder(&self, root: Root, path: &str) -> Result<Receipt> {
        self.guard_path(path, false)?;
        self.manager.new_folder(root, path)
    }

    pub fn delete(&self, root: Root, path: &str) -> Result<TrashReceipt> {
        self.guard_path(path, false)?;
        self.manager.delete(root, path)
    }

    pub fn restore(&self, receipt: &TrashReceipt, conflict: Conflict) -> Result<Receipt> {
        self.guard_path(&receipt.original_path, false)?;
        self.manager.restore(receipt, conflict)
    }

    pub fn extract_zip<F>(
        &self,
        source_root: Root,
        source: &str,
        target_root: Root,
        target: &str,
        conflict: Conflict,
        progress: F,
    ) -> Result<Receipt>
    where
        F: FnMut(Progress) -> bool,
    {
        self.guard_path(source, false)?;
        self.guard_path(target, false)?;
        self.manager
            .extract_zip(source_root, source, target_root, target, conflict, progress)
    }

    fn guard_path(&self, path: &str, allow_empty: bool) -> Result<()> {
        if path.is_empty() && allow_empty {
            return Ok(());
        }
        validate_logical_path(path)?;
        if !self.expert_mode
            && Path::new(path)
                .components()
                .any(|component| component.as_os_str().as_bytes().starts_with(b"."))
        {
            bail!("hidden paths require expert mode");
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct FileManager {
    storage_root: PathBuf,
    available_bytes_override: Option<u64>,
}

impl FileManager {
    pub fn new(storage_root: impl Into<PathBuf>) -> Result<Self> {
        let storage_root = storage_root.into();
        let metadata = fs::symlink_metadata(&storage_root).context("read storage root")?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("storage root must be a non-symlink directory");
        }
        Ok(Self {
            storage_root,
            available_bytes_override: None,
        })
    }

    /// Simulator-only capacity control; production uses statvfs at the chosen logical root.
    pub fn with_available_bytes(mut self, bytes: u64) -> Self {
        self.available_bytes_override = Some(bytes);
        self
    }

    pub fn import_handoffs(&self) -> [ImportHandoff; 2] {
        [
            ImportHandoff {
                transport: "usb",
                root: Root::UsbImport,
                message: "Open USB import handoff",
            },
            ImportHandoff {
                transport: "network",
                root: Root::NetworkImport,
                message: "Open network import handoff",
            },
        ]
    }

    pub fn browse(
        &self,
        root: Root,
        relative: &str,
        offset: usize,
        expert_mode: bool,
    ) -> Result<Listing> {
        let path = self.resolve(root, relative, true)?;
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() {
            bail!("browse target is not a directory");
        }
        let mut entries = Vec::new();
        let mut scanned_entries = 0;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.as_bytes() == INTERNAL.as_bytes() {
                continue;
            }
            scanned_entries += 1;
            if scanned_entries > DIRECTORY_ENTRY_BUDGET {
                bail!("directory exceeds 10000-entry budget");
            }
            let hidden = name.as_bytes().starts_with(b".");
            if hidden && !expert_mode {
                continue;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!("directory contains a symlink");
            }
            if !metadata.is_file() && !metadata.is_dir() {
                bail!("directory contains an unsupported entry");
            }
            entries.push(Entry {
                path: name.to_string_lossy().into_owned(),
                directory: metadata.is_dir(),
                bytes: metadata.len(),
                hidden,
            });
        }
        entries.sort_by(|left, right| {
            left.path
                .to_ascii_lowercase()
                .cmp(&right.path.to_ascii_lowercase())
        });
        if offset > entries.len() {
            bail!("listing offset is outside the directory");
        }
        let next_offset = (offset + PAGE_SIZE < entries.len()).then_some(offset + PAGE_SIZE);
        Ok(Listing {
            root,
            path: relative.into(),
            entries: entries.into_iter().skip(offset).take(PAGE_SIZE).collect(),
            next_offset,
            scanned_entries,
            expert_mode,
        })
    }

    pub fn copy<F>(
        &self,
        source_root: Root,
        source: &str,
        target_root: Root,
        target: &str,
        conflict: Conflict,
        mut progress: F,
    ) -> Result<Receipt>
    where
        F: FnMut(Progress) -> bool,
    {
        let source_path = self.resolve(source_root, source, false)?;
        let summary = inspect_tree(&source_path)?;
        let target_path = self.resolve(target_root, target, false)?;
        self.preflight(&target_path, summary.bytes)?;
        if paths_overlap(&source_path, &target_path) {
            bail!("copy source and target overlap");
        }
        self.transfer(
            "copy",
            &source_path,
            source,
            target_root,
            target,
            target_path,
            summary,
            conflict,
            &mut progress,
            false,
        )
    }

    pub fn move_path<F>(
        &self,
        source_root: Root,
        source: &str,
        target_root: Root,
        target: &str,
        conflict: Conflict,
        mut progress: F,
    ) -> Result<Receipt>
    where
        F: FnMut(Progress) -> bool,
    {
        let source_path = self.resolve(source_root, source, false)?;
        let summary = inspect_tree(&source_path)?;
        let target_path = self.resolve(target_root, target, false)?;
        self.preflight(&target_path, summary.bytes)?;
        if paths_overlap(&source_path, &target_path) {
            bail!("move source and target overlap");
        }
        self.transfer(
            "move",
            &source_path,
            source,
            target_root,
            target,
            target_path,
            summary,
            conflict,
            &mut progress,
            true,
        )
    }

    pub fn rename(
        &self,
        root: Root,
        source: &str,
        target: &str,
        conflict: Conflict,
    ) -> Result<Receipt> {
        let source_path = self.resolve(root, source, false)?;
        let summary = inspect_tree(&source_path)?;
        let target_path = self.resolve(root, target, false)?;
        if paths_overlap(&source_path, &target_path) {
            bail!("rename source and target overlap");
        }
        let (target_path, replaced) = self.resolve_conflict(root, target, target_path, conflict)?;
        if matches!(conflict, Conflict::Skip) && target_path.exists() {
            return Ok(Receipt {
                action: "rename-skipped".into(),
                source: Some(source.into()),
                destination: target.into(),
                summary,
                replaced: None,
            });
        }
        self.publish(root, &source_path, &target_path, replaced.as_ref())?;
        Ok(Receipt {
            action: "rename".into(),
            source: Some(source.into()),
            destination: self.logical_name(root, &target_path)?,
            summary,
            replaced,
        })
    }

    pub fn new_folder(&self, root: Root, relative: &str) -> Result<Receipt> {
        let path = self.resolve(root, relative, false)?;
        if path.exists() {
            bail!("folder already exists");
        }
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("folder has no parent"))?;
        self.resolve_parent(root, relative)?;
        fs::create_dir(&path)?;
        sync_dir(parent)?;
        Ok(Receipt {
            action: "new-folder".into(),
            source: None,
            destination: relative.into(),
            summary: Summary { count: 1, bytes: 0 },
            replaced: None,
        })
    }

    pub fn delete(&self, root: Root, relative: &str) -> Result<TrashReceipt> {
        let source = self.resolve(root, relative, false)?;
        let summary = inspect_tree(&source)?;
        let trash = self.trash_path(root, relative)?;
        self.publish(root, &source, &trash, None)?;
        Ok(TrashReceipt {
            root,
            original_path: relative.into(),
            trash_path: self.logical_name(root, &trash)?,
            summary,
        })
    }

    pub fn restore(&self, receipt: &TrashReceipt, conflict: Conflict) -> Result<Receipt> {
        let trash = self.resolve_trash(receipt.root, &receipt.trash_path)?;
        if !trash.exists() {
            bail!("trash entry is no longer available");
        }
        let target = self.resolve(receipt.root, &receipt.original_path, false)?;
        let (target, replaced) =
            self.resolve_conflict(receipt.root, &receipt.original_path, target, conflict)?;
        if matches!(conflict, Conflict::Skip) && target.exists() {
            return Ok(Receipt {
                action: "restore-skipped".into(),
                source: Some(receipt.trash_path.clone()),
                destination: receipt.original_path.clone(),
                summary: receipt.summary,
                replaced: None,
            });
        }
        self.publish(receipt.root, &trash, &target, replaced.as_ref())?;
        Ok(Receipt {
            action: "restore".into(),
            source: Some(receipt.trash_path.clone()),
            destination: self.logical_name(receipt.root, &target)?,
            summary: receipt.summary,
            replaced,
        })
    }

    pub fn extract_zip<F>(
        &self,
        source_root: Root,
        source: &str,
        target_root: Root,
        target: &str,
        conflict: Conflict,
        mut progress: F,
    ) -> Result<Receipt>
    where
        F: FnMut(Progress) -> bool,
    {
        let source_path = self.resolve(source_root, source, false)?;
        let archive = open_regular(&source_path)?;
        let metadata = archive.metadata()?;
        if metadata.len() > MAX_ARCHIVE_BYTES {
            bail!("ZIP archive exceeds input byte limit");
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        archive
            .take(MAX_ARCHIVE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
            bail!("ZIP archive exceeds input byte limit");
        }
        let entries = parse_zip(&bytes)?;
        let summary = Summary {
            count: entries.len() as u64,
            bytes: entries.iter().map(|entry| entry.uncompressed).sum(),
        };
        let target_path = self.resolve(target_root, target, false)?;
        self.preflight(&target_path, summary.bytes)?;
        let stage = self.stage_dir(target_root)?;
        let payload = stage.join("payload");
        let result = (|| {
            fs::create_dir(&payload)?;
            let mut state = Progress {
                completed_bytes: 0,
                total_bytes: summary.bytes,
                completed_files: 0,
                total_files: summary.count,
            };
            for entry in entries {
                let output = payload.join(&entry.name);
                let parent = output
                    .parent()
                    .ok_or_else(|| anyhow!("archive output has no parent"))?;
                fs::create_dir_all(parent)?;
                let data = &bytes[entry.data_offset..entry.data_offset + entry.compressed as usize];
                let mut input: Box<dyn Read> = match entry.method {
                    0 => Box::new(Cursor::new(data)),
                    8 => Box::new(DeflateDecoder::new(Cursor::new(data))),
                    _ => bail!("archive compression method is unsupported"),
                };
                let mut output_file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&output)?;
                copy_limited(
                    &mut input,
                    &mut output_file,
                    entry.uncompressed,
                    &mut state,
                    &mut progress,
                )?;
                output_file.sync_all()?;
                state.completed_files += 1;
                if !progress(state.clone()) {
                    bail!("operation cancelled");
                }
            }
            let (target_path, replaced) =
                self.resolve_conflict(target_root, target, target_path, conflict)?;
            if matches!(conflict, Conflict::Skip) && target_path.exists() {
                return Ok(Receipt {
                    action: "extract-skipped".into(),
                    source: Some(source.into()),
                    destination: target.into(),
                    summary,
                    replaced: None,
                });
            }
            self.publish(target_root, &payload, &target_path, replaced.as_ref())?;
            Ok(Receipt {
                action: "extract".into(),
                source: Some(source.into()),
                destination: self.logical_name(target_root, &target_path)?,
                summary,
                replaced,
            })
        })();
        let _ = fs::remove_dir_all(&stage);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn transfer<F>(
        &self,
        action: &str,
        source_path: &Path,
        source: &str,
        target_root: Root,
        target: &str,
        target_path: PathBuf,
        summary: Summary,
        conflict: Conflict,
        progress: &mut F,
        remove_source: bool,
    ) -> Result<Receipt>
    where
        F: FnMut(Progress) -> bool,
    {
        let stage = self.stage_dir(target_root)?;
        let payload = stage.join("payload");
        let result = (|| {
            let mut state = Progress {
                total_bytes: summary.bytes,
                total_files: summary.count,
                completed_bytes: 0,
                completed_files: 0,
            };
            copy_tree(source_path, &payload, &mut state, progress)?;
            let (target_path, replaced) =
                self.resolve_conflict(target_root, target, target_path, conflict)?;
            if matches!(conflict, Conflict::Skip) && target_path.exists() {
                return Ok(Receipt {
                    action: format!("{action}-skipped"),
                    source: Some(source.into()),
                    destination: target.into(),
                    summary,
                    replaced: None,
                });
            }
            self.publish(target_root, &payload, &target_path, replaced.as_ref())?;
            if remove_source {
                remove_tree(source_path)?;
                sync_dir(
                    source_path
                        .parent()
                        .ok_or_else(|| anyhow!("source has no parent"))?,
                )?;
            }
            Ok(Receipt {
                action: action.into(),
                source: Some(source.into()),
                destination: self.logical_name(target_root, &target_path)?,
                summary,
                replaced,
            })
        })();
        let _ = fs::remove_dir_all(&stage);
        result
    }

    fn resolve(&self, root: Root, relative: &str, allow_empty: bool) -> Result<PathBuf> {
        let base = self.root_path(root, true)?;
        if relative.is_empty() && allow_empty {
            return Ok(base);
        }
        validate_logical_path(relative)?;
        if Path::new(relative).starts_with(INTERNAL) {
            bail!("file-manager recovery data is not user-addressable");
        }
        resolve_user_path(&base, relative)
    }

    fn resolve_trash(&self, root: Root, relative: &str) -> Result<PathBuf> {
        validate_logical_path(relative)?;
        let prefix = format!("{INTERNAL}/trash/");
        let Some(name) = relative.strip_prefix(&prefix) else {
            bail!("trash receipt is invalid");
        };
        if Path::new(name).components().count() != 1 {
            bail!("trash receipt is invalid");
        }
        resolve_user_path(&self.root_path(root, false)?, relative)
    }

    fn resolve_parent(&self, root: Root, relative: &str) -> Result<()> {
        validate_logical_path(relative)?;
        let parent = Path::new(relative)
            .parent()
            .and_then(Path::to_str)
            .unwrap_or("");
        let _ = self.resolve(root, parent, true)?;
        Ok(())
    }

    fn root_path(&self, root: Root, create: bool) -> Result<PathBuf> {
        let path = resolve_user_path(&self.storage_root, root.relative_path())?;
        if create && !path.exists() {
            fs::create_dir_all(&path)?;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("approved root is not a directory");
        }
        Ok(path)
    }

    fn logical_name(&self, root: Root, absolute: &Path) -> Result<String> {
        absolute
            .strip_prefix(self.root_path(root, false)?)
            .map_err(Into::into)
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn preflight(&self, target: &Path, required: u64) -> Result<()> {
        let mut base = target
            .parent()
            .ok_or_else(|| anyhow!("target has no parent"))?;
        while !base.exists() {
            base = base
                .parent()
                .ok_or_else(|| anyhow!("target has no existing ancestor"))?;
        }
        let available = self
            .available_bytes_override
            .unwrap_or(available_bytes(base)?);
        if available < required {
            bail!("insufficient free space: need {required} bytes, have {available}");
        }
        Ok(())
    }

    fn stage_dir(&self, root: Root) -> Result<PathBuf> {
        let base = self.root_path(root, true)?;
        let internal = resolve_user_path(&base, INTERNAL)?;
        fs::create_dir(&internal).or_else(|error| {
            (error.kind() == io::ErrorKind::AlreadyExists)
                .then_some(())
                .ok_or(error)
        })?;
        let staging = resolve_user_path(&internal, "staging")?;
        fs::create_dir_all(&staging)?;
        let stage = staging.join(unique_token());
        fs::create_dir(&stage)?;
        Ok(stage)
    }

    fn trash_path(&self, root: Root, original: &str) -> Result<PathBuf> {
        let base = self.root_path(root, true)?;
        let internal = resolve_user_path(&base, INTERNAL)?;
        fs::create_dir(&internal).or_else(|error| {
            (error.kind() == io::ErrorKind::AlreadyExists)
                .then_some(())
                .ok_or(error)
        })?;
        let trash = resolve_user_path(&internal, "trash")?;
        fs::create_dir_all(&trash)?;
        let name = Path::new(original)
            .file_name()
            .ok_or_else(|| anyhow!("delete path has no name"))?;
        Ok(trash.join(format!("{}-{}", unique_token(), name.to_string_lossy())))
    }

    fn publish(
        &self,
        root: Root,
        source: &Path,
        target: &Path,
        replaced: Option<&TrashReceipt>,
    ) -> Result<()> {
        let target_parent = target
            .parent()
            .ok_or_else(|| anyhow!("target has no parent"))?;
        fs::create_dir_all(target_parent)?;
        if let Err(error) = fs::rename(source, target) {
            if let Some(replaced) = replaced {
                let trash = self.resolve_trash(root, &replaced.trash_path)?;
                fs::rename(&trash, target)
                    .context("publish failed and replaced destination could not be restored")?;
                sync_dir(target_parent)?;
                sync_dir(
                    trash
                        .parent()
                        .ok_or_else(|| anyhow!("trash path has no parent"))?,
                )?;
            }
            return Err(error).context("publish operation");
        }
        sync_dir(target_parent)?;
        sync_dir(
            source
                .parent()
                .ok_or_else(|| anyhow!("staged source has no parent"))?,
        )?;
        Ok(())
    }

    fn resolve_conflict(
        &self,
        root: Root,
        requested: &str,
        target: PathBuf,
        conflict: Conflict,
    ) -> Result<(PathBuf, Option<TrashReceipt>)> {
        if !target.exists() {
            return Ok((target, None));
        }
        match conflict {
            Conflict::Skip => Ok((target, None)),
            Conflict::Rename => {
                let parent = target
                    .parent()
                    .ok_or_else(|| anyhow!("target has no parent"))?;
                let stem = target
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| anyhow!("target name is invalid"))?;
                let extension = target
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| format!(".{value}"))
                    .unwrap_or_default();
                for number in 1..=999 {
                    let candidate = parent.join(format!("{stem} ({number}){extension}"));
                    if !candidate.exists() {
                        return Ok((candidate, None));
                    }
                }
                bail!("cannot choose a conflict-renamed target");
            }
            Conflict::Replace => {
                let summary = inspect_tree(&target)?;
                let trash = self.trash_path(root, requested)?;
                self.publish(root, &target, &trash, None)?;
                Ok((
                    target,
                    Some(TrashReceipt {
                        root,
                        original_path: requested.into(),
                        trash_path: self.logical_name(root, &trash)?,
                        summary,
                    }),
                ))
            }
        }
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn inspect_tree(path: &Path) -> Result<Summary> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("operation path contains a symlink");
    }
    if metadata.is_file() {
        return Ok(Summary {
            count: 1,
            bytes: metadata.len(),
        });
    }
    if !metadata.is_dir() {
        bail!("operation path has an unsupported type");
    }
    let mut summary = Summary { count: 1, bytes: 0 };
    for entry in fs::read_dir(path)? {
        let child = inspect_tree(&entry?.path())?;
        summary.count = summary
            .count
            .checked_add(child.count)
            .ok_or_else(|| anyhow!("file count overflow"))?;
        summary.bytes = summary
            .bytes
            .checked_add(child.bytes)
            .ok_or_else(|| anyhow!("byte count overflow"))?;
    }
    Ok(summary)
}

fn copy_tree<F>(source: &Path, target: &Path, state: &mut Progress, progress: &mut F) -> Result<()>
where
    F: FnMut(Progress) -> bool,
{
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!("operation path contains a symlink");
    }
    if metadata.is_file() {
        let mut input = open_regular(source)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(target)?;
        copy_limited(&mut input, &mut output, metadata.len(), state, progress)?;
        output.sync_all()?;
        state.completed_files += 1;
        if !progress(state.clone()) {
            bail!("operation cancelled");
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!("operation path has an unsupported type");
    }
    fs::create_dir(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_tree(
            &entry.path(),
            &target.join(entry.file_name()),
            state,
            progress,
        )?;
    }
    state.completed_files += 1;
    if !progress(state.clone()) {
        bail!("operation cancelled");
    }
    Ok(())
}

fn copy_limited(
    input: &mut dyn Read,
    output: &mut dyn Write,
    expected: u64,
    state: &mut Progress,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<()> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| anyhow!("copy size overflow"))?;
        if copied > expected {
            bail!("archive output exceeds declared size");
        }
        output.write_all(&buffer[..read])?;
        state.completed_bytes = state
            .completed_bytes
            .checked_add(read as u64)
            .ok_or_else(|| anyhow!("copy size overflow"))?;
        if !progress(state.clone()) {
            bail!("operation cancelled");
        }
    }
    if copied != expected {
        bail!("copy size does not match preflight");
    }
    Ok(())
}

fn remove_tree(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("operation path contains a symlink");
    }
    if metadata.is_file() {
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        bail!("operation path has an unsupported type");
    }
    Ok(())
}

fn open_regular(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if !file.metadata()?.is_file() {
        bail!("operation source is not a regular file");
    }
    Ok(file)
}

fn available_bytes(path: &Path) -> Result<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).context("storage path contains NUL")?;
    let mut status = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: statvfs writes the supplied structure and the CString is NUL-terminated.
    if unsafe { libc::statvfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: statvfs succeeded.
    let status = unsafe { status.assume_init() };
    status
        .f_bavail
        .checked_mul(status.f_frsize)
        .ok_or_else(|| anyhow!("available space overflow"))
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all().map_err(Into::into)
}

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{}", nanos, std::process::id())
}

#[derive(Clone, Debug)]
struct ZipEntry {
    name: String,
    method: u16,
    compressed: u64,
    uncompressed: u64,
    data_offset: usize,
}

fn parse_zip(bytes: &[u8]) -> Result<Vec<ZipEntry>> {
    let start = bytes.len().saturating_sub(65_557);
    let eocd = (start..bytes.len().saturating_sub(3))
        .rev()
        .find(|&index| bytes[index..].starts_with(b"PK\x05\x06"))
        .ok_or_else(|| anyhow!("ZIP end record is missing"))?;
    if eocd + 22 > bytes.len() {
        bail!("ZIP end record is truncated");
    }
    let count = le16(bytes, eocd + 10)? as usize;
    let size = le32(bytes, eocd + 12)? as usize;
    let offset = le32(bytes, eocd + 16)? as usize;
    let comment = le16(bytes, eocd + 20)? as usize;
    let Some(central_end) = offset.checked_add(size) else {
        bail!("ZIP archive exceeds bounds");
    };
    if le16(bytes, eocd + 4)? != 0
        || le16(bytes, eocd + 6)? != 0
        || le16(bytes, eocd + 8)? as usize != count
        || eocd.checked_add(22 + comment) != Some(bytes.len())
        || count > MAX_ARCHIVE_FILES
        || central_end != eocd
    {
        bail!("ZIP archive exceeds bounds");
    }
    let mut entries = Vec::new();
    let mut cursor = offset;
    let mut total = 0_u64;
    for _ in 0..count {
        if cursor + 46 > bytes.len() || &bytes[cursor..cursor + 4] != b"PK\x01\x02" {
            bail!("ZIP central directory is invalid");
        }
        let flags = le16(bytes, cursor + 8)?;
        let method = le16(bytes, cursor + 10)?;
        let compressed = le32(bytes, cursor + 20)? as u64;
        let uncompressed = le32(bytes, cursor + 24)? as u64;
        let name_len = le16(bytes, cursor + 28)? as usize;
        let extra_len = le16(bytes, cursor + 30)? as usize;
        let comment_len = le16(bytes, cursor + 32)? as usize;
        let external = le32(bytes, cursor + 38)?;
        let local_offset = le32(bytes, cursor + 42)? as usize;
        let end = cursor
            .checked_add(46 + name_len + extra_len + comment_len)
            .ok_or_else(|| anyhow!("ZIP entry overflow"))?;
        if end > central_end || flags & 0x41 != 0 || !matches!(method, 0 | 8) {
            bail!("ZIP entry is unsupported");
        }
        let name = std::str::from_utf8(&bytes[cursor + 46..cursor + 46 + name_len])
            .context("ZIP entry name is not UTF-8")?;
        let file_type = (external >> 16) & 0o170000;
        if file_type == 0o120000 {
            bail!("ZIP symlink entries are forbidden");
        }
        validate_logical_path(name)?;

        if name.ends_with('/') {
            if !matches!(file_type, 0 | 0o040000) {
                bail!("ZIP directory entry type is unsupported");
            }
            cursor = end;
            continue;
        }
        if !matches!(file_type, 0 | 0o100000) {
            bail!("ZIP entry type is unsupported");
        }
        if uncompressed > 0 && compressed.saturating_mul(MAX_ARCHIVE_EXPANSION) < uncompressed {
            bail!("ZIP expansion ratio exceeds limit");
        }
        total = total
            .checked_add(uncompressed)
            .ok_or_else(|| anyhow!("ZIP size overflow"))?;
        if total > MAX_ARCHIVE_BYTES {
            bail!("ZIP expansion exceeds byte limit");
        }
        if local_offset + 30 > bytes.len()
            || &bytes[local_offset..local_offset + 4] != b"PK\x03\x04"
            || le16(bytes, local_offset + 6)? != flags
            || le16(bytes, local_offset + 8)? != method
        {
            bail!("ZIP local header is invalid");
        }
        let local_name = le16(bytes, local_offset + 26)? as usize;
        let local_extra = le16(bytes, local_offset + 28)? as usize;
        let data_offset = local_offset
            .checked_add(30 + local_name + local_extra)
            .ok_or_else(|| anyhow!("ZIP data offset overflow"))?;
        if bytes.get(local_offset + 30..local_offset + 30 + local_name) != Some(name.as_bytes())
            || data_offset
                .checked_add(compressed as usize)
                .is_none_or(|end| end > offset)
        {
            bail!("ZIP entry data is truncated or inconsistent");
        }
        if entries.iter().any(|entry: &ZipEntry| entry.name == name) {
            bail!("ZIP contains duplicate output paths");
        }

        entries.push(ZipEntry {
            name: name.into(),
            method,
            compressed,
            uncompressed,
            data_offset,
        });
        cursor = end;
    }
    if cursor != central_end || entries.len() > MAX_ARCHIVE_FILES {
        bail!("ZIP archive is malformed or exceeds file-count limit");
    }
    Ok(entries)
}

fn le16(bytes: &[u8], offset: usize) -> Result<u16> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| anyhow!("ZIP field is truncated"))
}
fn le32(bytes: &[u8], offset: usize) -> Result<u32> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| anyhow!("ZIP field is truncated"))
}
