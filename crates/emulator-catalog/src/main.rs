use emulator_catalog::{
    core_pack_journey, fixture_journey, json, load_fixture, schema_validation_journey, Catalog,
    ChannelName,
};
use std::{env, path::PathBuf, process};

fn main() {
    let command = env::args().nth(1);
    let result = match command.as_deref() {
        Some("validate") => validate(),
        Some("resolve") => resolve(),
        Some("audit" | "bios-audit") => audit(),
        Some("journey") => fixture_journey(),
        Some("schema-validation-journey") => schema_validation_journey(),
        Some("core-pack-journey") => core_pack_journey(),
        _ => Err(emulator_catalog::CatalogError::new("usage", "usage: emulator-catalog validate|resolve|bios-audit|schema-validation-journey|journey|core-pack-journey ...")),
    };
    match result {
        Ok(output) => {
            println!("{output}");
            if matches!(command.as_deref(), Some("audit" | "bios-audit"))
                && (output.contains("\"status\": \"missing\"")
                    || output.contains("\"status\": \"mismatch\""))
            {
                process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}

fn has_flag(flag: &str) -> bool {
    env::args().any(|arg| arg == flag)
}

fn value(flag: &str) -> Result<PathBuf, emulator_catalog::CatalogError> {
    let args: Vec<String> = env::args().collect();
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or_else(|| emulator_catalog::CatalogError::new("usage", format!("missing {flag}")))
}
fn channel() -> Result<ChannelName, emulator_catalog::CatalogError> {
    let args: Vec<String> = env::args().collect();
    let value = args
        .windows(2)
        .find(|pair| pair[0] == "--channel")
        .map(|pair| pair[1].as_str())
        .unwrap_or("stable");
    match value {
        "stable" => Ok(ChannelName::Stable),
        "experimental" => Ok(ChannelName::Experimental),
        _ => Err(emulator_catalog::CatalogError::new(
            "invalid_channel",
            "channel must be stable or experimental",
        )),
    }
}
fn validate() -> emulator_catalog::Result<String> {
    let catalog = Catalog::load(value("--root")?)?;
    let selected = catalog.channel(channel()?)?;
    Ok(format!(
        "validated {} channel: {} systems, {} runners, {} cores, {} profiles",
        selected.id.as_str(),
        selected.systems.len(),
        selected.runners.len(),
        selected.cores.len(),
        selected.profiles.len()
    ))
}
fn resolve() -> emulator_catalog::Result<String> {
    let catalog = Catalog::load(value("--root")?)?;
    let result = catalog.resolve(&load_fixture(value("--case")?)?)?;
    json(&result)
}
fn audit() -> emulator_catalog::Result<String> {
    let catalog_root = if has_flag("--catalog") {
        value("--catalog")?
    } else {
        value("--root")?
    };
    let bios_root = value("--bios-root")?;
    let report = Catalog::load(catalog_root)?.audit(bios_root, channel()?)?;
    json(&report)
}
