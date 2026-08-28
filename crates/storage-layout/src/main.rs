use std::{
    collections::HashSet,
    env,
    fmt::Write as FmtWrite,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    process,
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{de, de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

const LAYOUT_SCHEMA: &str = "https://example.invalid/trimui-storage-v1.schema.json";
const MIGRATION_FORMAT: &str = "brickpro-storage-migration";
const MAX_FAT32_FILE: u64 = 4_294_967_295;
const MIGRATION_ID: &str = "storage-v1-to-v2";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    #[serde(rename = "$schema")]
    schema: String,
    format: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(rename = "installationUuid")]
    installation_uuid: String,
    #[serde(rename = "activeDataVersion")]
    active_data_version: u32,
    #[serde(rename = "completedMigrations")]
    completed_migrations: Vec<String>,
    filesystem: Filesystem,
    #[serde(rename = "migrationDescriptor")]
    migration_descriptor: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Filesystem {
    kind: FilesystemKind,
    #[serde(rename = "maxFileBytes")]
    max_file_bytes: u64,
    #[serde(rename = "maxPathBytes")]
    max_path_bytes: u16,
    #[serde(rename = "maxComponentBytes")]
    max_component_bytes: u16,
    #[serde(rename = "caseSensitive")]
    case_sensitive: bool,
    #[serde(rename = "supportsAtomicRename")]
    supports_atomic_rename: bool,
    #[serde(rename = "supportsFileSync")]
    supports_file_sync: bool,
    #[serde(rename = "supportsDirectorySync")]
    supports_directory_sync: bool,
    #[serde(rename = "supportsSymlinks")]
    supports_symlinks: bool,
    #[serde(rename = "supportsPosixOwnership")]
    supports_posix_ownership: bool,
    #[serde(rename = "supportsPosixModes")]
    supports_posix_modes: bool,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum FilesystemKind {
    Fat32,
    Exfat,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Migration {
    format: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    id: String,
    from: Version,
    to: Version,
    steps: Vec<Step>,
    #[serde(rename = "priorReleaseReadable")]
    prior_release_readable: bool,
    activation: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Version {
    #[serde(rename = "dataVersion")]
    data_version: u32,
    release: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Step {
    id: String,
    source: String,
    target: String,
    sha256: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum JournalState {
    Prepared,
    Committed,
    RolledBack,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: String,
    generation: u64,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(rename = "migrationId")]
    migration_id: String,
    #[serde(rename = "fromDataVersion")]
    from_data_version: u32,
    #[serde(rename = "toDataVersion")]
    to_data_version: u32,
    state: JournalState,
    #[serde(rename = "checksumsVerified")]
    checksums_verified: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("storage-layout failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let command = args.next().ok_or_else(|| anyhow!("missing command"))?;
    match command.as_str() {
        "validate" => {
            let layout_path = required_option(&mut args, "--layout")?;
            let root = required_option(&mut args, "--root")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            validate(&PathBuf::from(layout_path), &PathBuf::from(root))?;
            println!("storage-v1 valid");
        }
        "simulate-migrate" => {
            let root = required_option(&mut args, "--root")?;
            let to = required_option(&mut args, "--to")?;
            let interrupt = match args.next().as_deref() {
                None => false,
                Some("--interrupt-after-journal") => {
                    if args.next().is_some() {
                        bail!("unexpected argument")
                    }
                    true
                }
                Some(_) => bail!("unexpected argument"),
            };
            if to != "latest" {
                bail!("--to must be latest")
            }
            simulate_migrate(&PathBuf::from(root), interrupt)?;
        }
        "simulate-rollback" => {
            let root = required_option(&mut args, "--root")?;
            if args.next().is_some() {
                bail!("unexpected argument")
            }
            simulate_rollback(&PathBuf::from(root))?;
        }
        _ => bail!("unknown command"),
    }
    Ok(())
}

fn required_option(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String> {
    match args.next().as_deref() {
        Some(value) if value == name => args.next().ok_or_else(|| anyhow!("missing {name} value")),
        Some(_) => bail!("expected {name}"),
        None => bail!("missing {name}"),
    }
}

fn validate(layout_path: &Path, root: &Path) -> Result<()> {
    let layout = read_json::<Layout>(layout_path).context("read layout")?;
    let descriptor = validate_layout(&layout, root)?;
    let descriptor_path = root.join(&layout.migration_descriptor);
    let _ = descriptor;
    if !descriptor_path.is_file() {
        bail!("migration descriptor is missing")
    }
    Ok(())
}

fn validate_layout(layout: &Layout, root: &Path) -> Result<Migration> {
    if layout.schema != LAYOUT_SCHEMA || layout.format != "brickpro-storage-layout" {
        bail!("layout is not storage-v1")
    }
    if layout.schema_version != 1 {
        bail!("unsupported layout schema version")
    }
    if !valid_uuid(&layout.installation_uuid) {
        bail!("installationUuid is not a bounded UUID")
    }
    if layout.active_data_version == 0
        || layout.active_data_version > 1000
        || layout.completed_migrations.len() > 32
    {
        bail!("layout version or migration bounds exceeded")
    }
    let mut migration_ids = HashSet::new();
    for id in &layout.completed_migrations {
        if !valid_identifier(id, 64) || !migration_ids.insert(id) {
            bail!("completed migration IDs must be unique bounded identifiers")
        }
    }
    validate_filesystem(&layout.filesystem)?;
    validate_relative_data_path(&layout.migration_descriptor)?;
    let descriptor_path = root.join(&layout.migration_descriptor);
    let migration =
        read_json::<Migration>(&descriptor_path).context("read migration descriptor")?;
    validate_migration(&migration)?;
    if layout.active_data_version != migration.from.data_version
        && layout.active_data_version != migration.to.data_version
    {
        bail!("layout data version does not match migration")
    }
    if layout.active_data_version == migration.from.data_version
        && !layout.completed_migrations.is_empty()
    {
        bail!("prior data version cannot have completed migrations")
    }
    if layout.active_data_version == migration.to.data_version
        && (layout.completed_migrations.len() != 1
            || layout.completed_migrations[0] != migration.id)
    {
        bail!("current data version must name its completed migration")
    }
    inspect_tree(root, &layout.filesystem)?;
    verify_migration_sources(root, &migration)?;
    for protected in [
        "roms",
        "data/saves",
        "data/states",
        "data/resume",
        "data/settings",
        ".brickpro/save-vault",
    ] {
        if !root.join(protected).is_dir() {
            bail!("required synthetic storage tree is missing")
        }
    }
    Ok(migration)
}

fn validate_filesystem(filesystem: &Filesystem) -> Result<()> {
    if filesystem.max_file_bytes == 0
        || filesystem.max_path_bytes < 32
        || filesystem.max_component_bytes < 8
    {
        bail!("filesystem limits are outside declared bounds")
    }
    if filesystem.max_path_bytes > 4096 || filesystem.max_component_bytes > 255 {
        bail!("filesystem limits are outside declared bounds")
    }
    if filesystem.kind == FilesystemKind::Fat32 && filesystem.max_file_bytes > MAX_FAT32_FILE {
        bail!("FAT32 file limit exceeds 4 GiB minus one byte")
    }
    if filesystem.max_file_bytes > 17_592_186_044_415 {
        bail!("file limit exceeds the contract bound")
    }
    if filesystem.case_sensitive
        || filesystem.supports_symlinks
        || filesystem.supports_posix_ownership
        || filesystem.supports_posix_modes
    {
        bail!("layout relies on unsupported filesystem semantics")
    }
    if !filesystem.supports_atomic_rename
        || !filesystem.supports_file_sync
        || !filesystem.supports_directory_sync
    {
        bail!("filesystem capabilities cannot satisfy migration safety")
    }
    Ok(())
}

fn validate_migration(migration: &Migration) -> Result<()> {
    if migration.format != MIGRATION_FORMAT || migration.schema_version != 1 {
        bail!("migration descriptor format/version is invalid")
    }
    if migration.id != MIGRATION_ID
        || migration.from.data_version >= migration.to.data_version
        || migration.from.release.is_empty()
        || migration.from.release.len() > 64
        || migration.to.release.is_empty()
        || migration.to.release.len() > 64
        || migration.steps.is_empty()
    {
        bail!("migration descriptor bounds or ordering are invalid")
    }
    if !migration.prior_release_readable
        || migration.activation != "blocked-unless-prior-release-readable"
    {
        bail!("migration activation is blocked without prior-release readability")
    }
    let mut ids = HashSet::new();
    for step in &migration.steps {
        if !valid_identifier(&step.id, 64) || !ids.insert(&step.id) {
            bail!("migration step IDs must be unique bounded identifiers")
        }
        validate_data_path(&step.source)?;
        validate_data_path(&step.target)?;
        if !is_sha256(&step.sha256) {
            bail!("migration checksum is not SHA-256")
        }
    }
    Ok(())
}

fn simulate_migrate(root: &Path, interrupt_after_journal: bool) -> Result<()> {
    let layout_path = root.join("data/meta/layout.json");
    let mut layout = read_json::<Layout>(&layout_path).context("read layout")?;
    let migration = validate_layout(&layout, root)?;
    if layout.active_data_version == migration.to.data_version {
        verify_protected_trees(root)?;
        println!("storage-v1 migration already complete");
        return Ok(());
    }
    let journal_path = root.join(format!(
        "data/meta/migrations/{}.journal.json",
        migration.id
    ));
    let journal = if journal_path.exists() {
        Some(read_json::<Journal>(&journal_path).context("read migration journal")?)
    } else {
        None
    };
    if let Some(existing) = journal {
        if existing.migration_id != migration.id {
            bail!("migration journal ID mismatch")
        }
        match existing.state {
            JournalState::Committed => {
                if !existing.checksums_verified {
                    bail!("committed migration lacks checksum verification")
                }
                layout.active_data_version = migration.to.data_version;
                layout.completed_migrations = vec![migration.id.clone()];
                write_json_atomic(&layout_path, &layout)?;
                verify_protected_trees(root)?;
                println!("storage-v1 migration complete");
                return Ok(());
            }
            JournalState::RolledBack => {}
            JournalState::Prepared => {}
        }
    }

    let before = protected_snapshot(root)?;
    let prepared = Journal {
        format: "brickpro-storage-journal".to_string(),
        generation: u64::from(migration.to.data_version),
        schema_version: 1,
        migration_id: migration.id.clone(),
        from_data_version: migration.from.data_version,
        to_data_version: migration.to.data_version,
        state: JournalState::Prepared,
        checksums_verified: false,
    };
    write_json_atomic(&journal_path, &prepared)?;
    if interrupt_after_journal {
        bail!("deterministic interruption after durable journal")
    }

    let stage = root.join(format!("data/meta/migrations/.{}.stage", migration.id));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage)?;
    for step in &migration.steps {
        let source = root.join(&step.source);
        let target = stage.join(data_relative(&step.target)?);
        if !source.is_file() {
            bail!("migration source is missing")
        }
        if sha256_file(&source)? != step.sha256 {
            bail!("migration source checksum mismatch")
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        copy_synced(&source, &target)?;
        if sha256_file(&target)? != step.sha256 {
            bail!("staged migration checksum mismatch")
        }
    }
    for step in &migration.steps {
        let staged = stage.join(data_relative(&step.target)?);
        let target = root.join(&step.target);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(staged, &target).context("commit copy-on-write migration")?;
        sync_directory(target.parent())?;
        if sha256_file(&target)? != step.sha256 {
            bail!("committed migration checksum mismatch")
        }
    }
    let _ = fs::remove_dir_all(&stage);
    let committed = Journal {
        state: JournalState::Committed,
        checksums_verified: true,
        ..prepared
    };
    write_json_atomic(&journal_path, &committed)?;
    layout.active_data_version = migration.to.data_version;
    layout.completed_migrations = vec![migration.id.clone()];
    write_json_atomic(&layout_path, &layout)?;
    if before != protected_snapshot(root)? {
        bail!("protected storage changed during migration")
    }
    println!("storage-v1 migration complete");
    Ok(())
}

fn simulate_rollback(root: &Path) -> Result<()> {
    let layout_path = root.join("data/meta/layout.json");
    let mut layout = read_json::<Layout>(&layout_path).context("read layout")?;
    let migration = validate_layout(&layout, root)?;
    let before = protected_snapshot(root)?;
    if layout.active_data_version == migration.from.data_version {
        if let Some(mut journal) = read_journal(root, &migration)? {
            match journal.state {
                JournalState::Prepared => {
                    journal.state = JournalState::RolledBack;
                    write_json_atomic(&journal_path(root, &migration), &journal)?;
                }
                JournalState::Committed | JournalState::RolledBack => {}
            }
        }
        verify_protected_trees(root)?;
        println!("storage-v1 rollback already at prior release");
        return Ok(());
    }
    let journal =
        read_journal(root, &migration)?.ok_or_else(|| anyhow!("committed journal is missing"))?;
    if journal.state != JournalState::Committed || !journal.checksums_verified {
        bail!("rollback requires a checksum-verified committed migration")
    }
    layout.active_data_version = migration.from.data_version;
    layout.completed_migrations.clear();
    write_json_atomic(&layout_path, &layout)?;
    let mut rolled_back = journal;
    rolled_back.generation = u64::from(migration.from.data_version);
    rolled_back.state = JournalState::RolledBack;
    write_json_atomic(&journal_path(root, &migration), &rolled_back)?;
    if before != protected_snapshot(root)? {
        bail!("protected storage changed during rollback")
    }
    println!("storage-v1 rollback complete; prior release remains readable");
    Ok(())
}

fn read_journal(root: &Path, migration: &Migration) -> Result<Option<Journal>> {
    let path = journal_path(root, migration);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_json(&path).context("read migration journal")?))
}

fn journal_path(root: &Path, migration: &Migration) -> PathBuf {
    root.join(format!(
        "data/meta/migrations/{}.journal.json",
        migration.id
    ))
}

fn verify_migration_sources(root: &Path, migration: &Migration) -> Result<()> {
    for step in &migration.steps {
        let source = root.join(&step.source);
        if !source.is_file() {
            bail!("migration source is missing")
        }
        if sha256_file(&source)? != step.sha256 {
            bail!("migration source checksum mismatch")
        }
    }
    Ok(())
}

fn verify_protected_trees(root: &Path) -> Result<()> {
    let _ = protected_snapshot(root)?;
    Ok(())
}

fn protected_snapshot(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut snapshot = Vec::new();
    for tree in [
        "roms",
        "data/saves",
        "data/states",
        "data/resume",
        "data/settings",
    ] {
        collect_files(&root.join(tree), Path::new(tree), &mut snapshot)?;
    }
    let vault = root.join(".brickpro/save-vault");
    if vault.exists() {
        collect_files(&vault, Path::new(".brickpro/save-vault"), &mut snapshot)?;
    }
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(snapshot)
}

fn collect_files(path: &Path, relative: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
    for entry in fs::read_dir(path).context("read synthetic storage tree")? {
        let entry = entry?;
        let child = entry.path();
        let child_relative = relative.join(entry.file_name());
        let metadata = fs::symlink_metadata(&child)?;
        if metadata.is_dir() {
            collect_files(&child, &child_relative, files)?;
        } else if metadata.is_file() {
            files.push((child_relative, fs::read(child)?));
        } else {
            bail!("protected storage contains an unsupported filesystem object")
        }
    }
    Ok(())
}

fn inspect_tree(root: &Path, filesystem: &Filesystem) -> Result<()> {
    if !root.is_dir() {
        bail!("fixture root is not a directory")
    }
    let mut paths = HashSet::new();
    inspect_entry(root, Path::new(""), filesystem, &mut paths)
}

fn inspect_entry(
    path: &Path,
    relative: &Path,
    filesystem: &Filesystem,
    paths: &mut HashSet<String>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("symlink reliance is forbidden")
    }
    if !relative.as_os_str().is_empty() {
        let encoded = relative.as_os_str().as_bytes();
        if encoded.len() > filesystem.max_path_bytes as usize {
            bail!("path exceeds declared filesystem limit")
        }
        for component in relative.components() {
            let Component::Normal(name) = component else {
                bail!("path contains traversal or non-normal component")
            };
            if name.as_bytes().len() > filesystem.max_component_bytes as usize {
                bail!("path component exceeds declared filesystem limit")
            }
            if forbidden_windows_name(name.to_string_lossy().as_ref()) {
                bail!("path contains a Windows-forbidden name")
            }
        }
        let key = relative.to_string_lossy().to_lowercase();
        if !paths.insert(key) {
            bail!("case-insensitive path collision")
        }
    }
    if metadata.is_file() {
        if metadata.len() > filesystem.max_file_bytes {
            bail!("file exceeds declared filesystem limit")
        }
    } else if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            inspect_entry(
                &entry.path(),
                &relative.join(entry.file_name()),
                filesystem,
                paths,
            )?;
        }
    } else {
        bail!("unsupported filesystem object")
    }
    Ok(())
}

fn validate_relative_data_path(value: &str) -> Result<()> {
    validate_relative(value)?;
    if value.split('/').next().is_none_or(|part| part != "data") {
        bail!("migration descriptor must be under data")
    }
    Ok(())
}

fn validate_data_path(value: &str) -> Result<()> {
    validate_relative(value)?;
    if value
        .split('/')
        .any(|component| component.eq_ignore_ascii_case("roms"))
    {
        bail!("migration paths may not target ROMs")
    }
    if value.split('/').next().is_none_or(|part| part != "data") {
        bail!("migration paths must stay within data")
    }
    Ok(())
}

fn validate_relative(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value.contains('\\')
        || Path::new(value).is_absolute()
    {
        bail!("path is not a bounded relative path")
    }
    for component in Path::new(value).components() {
        match component {
            Component::Normal(_) => {}
            _ => bail!("path contains traversal or non-normal component"),
        }
    }
    Ok(())
}

fn data_relative(path: &str) -> Result<PathBuf> {
    let mut components = Path::new(path).components();
    if components.next() != Some(Component::Normal("data".as_ref())) {
        bail!("path is not under data")
    }
    Ok(components.collect())
}

fn forbidden_windows_name(name: &str) -> bool {
    let trimmed = name.trim_end_matches([' ', '.']);
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23)
                .then_some(byte == b'-')
                .unwrap_or_else(|| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && byte.is_ascii_lowercase())
                || (index > 0
                    && (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
        })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(&mut output, "{byte:02x}");
    }
    Ok(output)
}

fn copy_synced(source: &Path, target: &Path) -> Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn sync_directory(path: Option<&Path>) -> Result<()> {
    let path = path.ok_or_else(|| anyhow!("required directory sync has no parent"))?;
    File::open(path)?.sync_all()?;
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("JSON path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp",
        path.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_file(&temporary);
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    sync_directory(Some(parent))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)?;
    reject_duplicate_keys(&bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn reject_duplicate_keys(bytes: &[u8]) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer
        .deserialize_any(RejectVisitor)
        .map_err(|error| anyhow!("malformed JSON or duplicate named key: {error}"))?;
    deserializer.end()?;
    Ok(())
}

struct RejectSeed;

impl<'de> de::DeserializeSeed<'de> for RejectSeed {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RejectVisitor)
    }
}

struct RejectVisitor;

impl<'de> de::Visitor<'de> for RejectVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_borrowed_str<E>(self, _: &'de str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_some<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RejectVisitor)
    }
    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<(), A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        while sequence.next_element_seed(RejectSeed)?.is_some() {}
        Ok(())
    }
    fn visit_map<A>(self, mut map: A) -> std::result::Result<(), A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate named key: {key}")));
            }
            map.next_value_seed(RejectSeed)?;
        }
        Ok(())
    }
}
