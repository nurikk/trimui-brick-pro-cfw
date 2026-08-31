use std::{fs, io::Write, path::Path};

use anyhow::{bail, Result};
use file_manager::{
    Conflict, Controller, FileManager, Root, DIRECTORY_ENTRY_BUDGET, MAX_ARCHIVE_FILES, PAGE_SIZE,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("file-manager journey failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("journey") if args.next().as_deref() == Some("--root") => {
            let root = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing --root"))?;
            if args.next().is_some() {
                bail!("unexpected argument");
            }
            journey(Path::new(&root))
        }
        _ => bail!("usage: brickpro-file-manager journey --root DIRECTORY"),
    }
}

fn journey(root: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root)?;
    let mut manager = Controller::new(FileManager::new(root)?);
    for approved in Root::ALL {
        manager.select_root(approved);
        manager.browse("", 0)?;
    }
    if Root::from_id(".brickpro/system/slots/A").is_some() {
        bail!("system slot became a selectable root");
    }
    if manager.import_handoffs().map(|handoff| handoff.transport) != ["usb", "network"] {
        bail!("import handoff is incomplete");
    }

    fs::write(root.join("roms/source.bin"), b"original")?;
    manager.copy(
        Root::Roms,
        "source.bin",
        Root::Screenshots,
        "copy.bin",
        Conflict::Skip,
        |_| true,
    )?;
    if fs::read(root.join("data/screenshots/copy.bin"))? != b"original" {
        bail!("copy did not publish bytes");
    }
    fs::write(root.join("data/screenshots/copy.bin"), b"existing")?;
    let skipped = manager.copy(
        Root::Roms,
        "source.bin",
        Root::Screenshots,
        "copy.bin",
        Conflict::Skip,
        |_| true,
    )?;
    if skipped.action != "copy-skipped"
        || fs::read(root.join("data/screenshots/copy.bin"))? != b"existing"
    {
        bail!("skip conflict mutated destination");
    }
    let renamed = manager.copy(
        Root::Roms,
        "source.bin",
        Root::Screenshots,
        "copy.bin",
        Conflict::Rename,
        |_| true,
    )?;
    if renamed.destination != "copy (1).bin" {
        bail!("rename conflict did not choose deterministic name");
    }
    let replaced = manager.copy(
        Root::Roms,
        "source.bin",
        Root::Screenshots,
        "copy.bin",
        Conflict::Replace,
        |_| true,
    )?;
    if replaced.replaced.is_none()
        || fs::read(root.join("data/screenshots/copy.bin"))? != b"original"
    {
        bail!("replace conflict was not recoverable");
    }

    let cancelled = manager.copy(
        Root::Roms,
        "source.bin",
        Root::Themes,
        "cancel.bin",
        Conflict::Skip,
        |progress| progress.completed_bytes == 0,
    );
    if cancelled.is_ok()
        || root.join("data/themes/cancel.bin").exists()
        || fs::read(root.join("roms/source.bin"))? != b"original"
    {
        bail!("cancelled copy published a destination or changed source");
    }
    manager.move_path(
        Root::Screenshots,
        "copy (1).bin",
        Root::Themes,
        "moved.bin",
        Conflict::Skip,
        |_| true,
    )?;
    if root.join("data/screenshots/copy (1).bin").exists()
        || !root.join("data/themes/moved.bin").is_file()
    {
        bail!("move was not committed atomically enough");
    }

    manager.new_folder(Root::Themes, "maintenance")?;
    manager.rename(
        Root::Themes,
        "moved.bin",
        "maintenance/renamed.bin",
        Conflict::Skip,
    )?;
    if !root.join("data/themes/maintenance/renamed.bin").is_file() {
        bail!("controller new-folder/rename journey failed");
    }

    let preview = manager.preview(Root::Screenshots, "copy.bin")?;
    if preview.path != "copy.bin" || preview.summary.count != 1 || preview.summary.bytes != 8 {
        bail!("delete preview did not provide exact path/count/bytes");
    }

    let deleted = manager.delete(Root::Screenshots, "copy.bin")?;
    if root.join("data/screenshots/copy.bin").exists() {
        bail!("delete did not stage recoverably");
    }
    manager.restore(&deleted, Conflict::Skip)?;
    if !root.join("data/screenshots/copy.bin").is_file() {
        bail!("restore did not republish file");
    }

    write_stored_zip(
        &root.join("roms/maintenance.zip"),
        "inside/ok.txt",
        b"archive data",
    )?;
    manager.extract_zip(
        Root::Roms,
        "maintenance.zip",
        Root::PortMaster,
        "imported",
        Conflict::Skip,
        |_| true,
    )?;
    if fs::read(root.join("Ports/imported/inside/ok.txt"))? != b"archive data" {
        bail!("bounded archive extraction failed");
    }
    write_stored_zip(&root.join("roms/traversal.zip"), "../escape", b"no")?;
    if manager
        .extract_zip(
            Root::Roms,
            "traversal.zip",
            Root::PortMaster,
            "bad",
            Conflict::Skip,
            |_| true,
        )
        .is_ok()
        || root.join("Ports/bad").exists()
    {
        bail!("archive traversal was not rejected before mutation");
    }
    write_stored_zip(&root.join("roms/ratio.zip"), "large.bin", b"x")?;
    let ratio_path = root.join("roms/ratio.zip");
    let mut ratio = fs::read(&ratio_path)?;
    let central = ratio
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .ok_or_else(|| anyhow::anyhow!("fixture central directory is missing"))?;
    ratio[central + 24..central + 28].copy_from_slice(&101_u32.to_le_bytes());
    fs::write(&ratio_path, ratio)?;
    if manager
        .extract_zip(
            Root::Roms,
            "ratio.zip",
            Root::PortMaster,
            "ratio-bad",
            Conflict::Skip,
            |_| true,
        )
        .is_ok()
        || root.join("Ports/ratio-bad").exists()
    {
        bail!("archive expansion limit was not enforced before mutation");
    }

    write_stored_zip(&root.join("roms/count.zip"), "one", b"")?;
    let count_path = root.join("roms/count.zip");
    let mut count = fs::read(&count_path)?;
    let end = count.len() - 22;
    let excessive = (MAX_ARCHIVE_FILES as u16 + 1).to_le_bytes();
    count[end + 8..end + 10].copy_from_slice(&excessive);
    count[end + 10..end + 12].copy_from_slice(&excessive);
    fs::write(&count_path, count)?;
    if manager
        .extract_zip(
            Root::Roms,
            "count.zip",
            Root::PortMaster,
            "count-bad",
            Conflict::Skip,
            |_| true,
        )
        .is_ok()
        || root.join("Ports/count-bad").exists()
    {
        bail!("archive file-count limit was not enforced before mutation");
    }

    if manager
        .copy(
            Root::Roms,
            "source.bin",
            Root::Themes,
            "space.bin",
            Conflict::Skip,
            |_| true,
        )
        .is_err()
    {
        bail!("baseline capacity unexpectedly failed");
    }
    let small = Controller::new(FileManager::new(root)?.with_available_bytes(0));
    if small
        .copy(
            Root::Roms,
            "source.bin",
            Root::Themes,
            "no-space.bin",
            Conflict::Skip,
            |_| true,
        )
        .is_ok()
        || root.join("data/themes/no-space.bin").exists()
    {
        bail!("space preflight mutated destination");
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/tmp", root.join("roms/escape"))?;
        manager.select_root(Root::Roms);
        if manager.browse("", 0).is_ok() {
            bail!("symlink escape was not rejected");
        }
        fs::remove_file(root.join("roms/escape"))?;
    }
    fs::write(root.join("roms/BIOS/shared.bin"), b"protected")?;
    if manager
        .copy(
            Root::BiosImports,
            "shared.bin",
            Root::Roms,
            "BIOS/shared.bin",
            Conflict::Replace,
            |_| true,
        )
        .is_ok()
        || fs::read(root.join("roms/BIOS/shared.bin"))? != b"protected"
    {
        bail!("overlapping logical roots were not rejected before mutation");
    }
    manager.set_expert_mode(true);
    if manager
        .delete(Root::Screenshots, ".brickpro-file-manager")
        .is_ok()
    {
        bail!("recovery staging became a user-addressable root");
    }
    manager.set_expert_mode(false);
    fs::write(root.join("data/screenshots/.hidden"), b"hidden")?;
    manager.select_root(Root::Screenshots);
    if manager
        .browse("", 0)?
        .entries
        .iter()
        .any(|entry| entry.hidden)
        || manager.delete(Root::Screenshots, ".hidden").is_ok()
    {
        bail!("normal controller mode exposed a hidden entry");
    }
    manager.set_expert_mode(true);
    if !manager
        .browse("", 0)?
        .entries
        .iter()
        .any(|entry| entry.hidden)
    {
        bail!("expert mode did not expose hidden entries");
    }
    manager.set_expert_mode(false);
    let screenshot_root = root.join("data/screenshots");
    let existing = fs::read_dir(&screenshot_root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().as_encoded_bytes() != b".brickpro-file-manager")
        .count();
    for index in 0..DIRECTORY_ENTRY_BUDGET.saturating_sub(existing) {
        fs::write(screenshot_root.join(format!("list-{index:05}")), b"")?;
    }
    manager.select_root(Root::Screenshots);
    let first = manager.browse("", 0)?;
    if first.entries.len() != PAGE_SIZE || first.next_offset != Some(PAGE_SIZE) {
        bail!("10k-entry directory exceeded navigation page budget");
    }
    println!("file-manager journey: PASS (controller copy/move/rename/folder/conflict/cancel/trash/archive/path/space/hidden/10k bounds)");
    Ok(())
}

fn write_stored_zip(path: &Path, name: &str, body: &[u8]) -> Result<()> {
    let name = name.as_bytes();
    let crc = crc32(body);
    let mut output = Vec::new();
    output.extend_from_slice(b"PK\x03\x04\x14\0\0\0\0\0\0\0\0\0");
    output.extend_from_slice(&crc.to_le_bytes());
    output.extend_from_slice(&(body.len() as u32).to_le_bytes());
    output.extend_from_slice(&(body.len() as u32).to_le_bytes());
    output.extend_from_slice(&(name.len() as u16).to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(name);
    output.extend_from_slice(body);
    let central = output.len();
    output.extend_from_slice(b"PK\x01\x02\x14\0\x14\0\0\0\0\0\0\0\0\0");
    output.extend_from_slice(&crc.to_le_bytes());
    output.extend_from_slice(&(body.len() as u32).to_le_bytes());
    output.extend_from_slice(&(body.len() as u32).to_le_bytes());
    output.extend_from_slice(&(name.len() as u16).to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(name);
    let size = output.len() - central;
    output.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
    output.extend_from_slice(&(size as u32).to_le_bytes());
    output.extend_from_slice(&(central as u32).to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    let mut file = fs::File::create(path)?;
    file.write_all(&output)?;
    file.sync_all()?;
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0xffff_ffff, |crc, byte| {
        (0..8).fold(crc ^ u32::from(*byte), |value, _| {
            (value >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(value & 1))
        })
    }) ^ 0xffff_ffff
}
