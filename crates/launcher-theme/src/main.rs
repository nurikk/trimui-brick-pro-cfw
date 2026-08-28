use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use launcher_theme::{preview_path_or_fallback, render_png, serialize_json, write_scene};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("validate") => {
            let theme = required_path(&mut args, "--theme")?;
            reject_extra(&mut args)?;
            match launcher_theme::load_theme_dir(&theme) {
                Ok(_) => println!("{{\"ok\":true,\"theme\":{}}}", json_string(&theme)),
                Err(error) => {
                    let output = serialize_json(&error.json()).map_err(|item| item.to_string())?;
                    eprintln!(
                        "{}",
                        String::from_utf8(output).map_err(|item| item.to_string())?
                    );
                    return Err("theme validation failed".into());
                }
            }
            Ok(())
        }
        Some("preview") => {
            let theme = required_path(&mut args, "--theme")?;
            let output = required_path(&mut args, "--output")?;
            reject_extra(&mut args)?;
            render_preview(Some(&theme), &output)
        }
        Some("catalog") => {
            let catalog = required_path(&mut args, "--catalog")?;
            reject_extra(&mut args)?;
            let parsed = launcher_theme::ThemesCatalog::parse(
                &fs::read(catalog).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            println!(
                "{}",
                String::from_utf8(serialize_json(&parsed).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Some("import") => {
            let theme = required_path(&mut args, "--theme")?;
            let output = required_path(&mut args, "--output")?;
            reject_extra(&mut args)?;
            let imported =
                launcher_theme::import_es_theme_dir(&theme).map_err(|error| error.to_string())?;
            imported
                .write_native_dir(&theme, &output)
                .map_err(|error| error.to_string())?;
            fs::write(
                output.join("compatibility-report.json"),
                serialize_json(&imported.report).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            println!(
                "{}",
                String::from_utf8(
                    serialize_json(&imported.report).map_err(|error| error.to_string())?
                )
                .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Some("demo") => {
            let output = required_path(&mut args, "--output")?;
            let mut themes = Vec::new();
            while let Some(argument) = args.next() {
                if argument != "--theme" {
                    return Err(format!("unexpected argument: {argument}"));
                }
                themes.push(PathBuf::from(args.next().ok_or("missing --theme value")?));
            }
            if themes.is_empty() {
                themes = vec![
                    PathBuf::from("themes/default"),
                    PathBuf::from("fixtures/theme-import/owned-a"),
                    PathBuf::from("fixtures/theme-import/owned-b"),
                ];
            }
            fs::create_dir_all(&output).map_err(|error| error.to_string())?;
            let mut results = Vec::new();
            for theme in themes {
                let preview =
                    preview_path_or_fallback(&theme).map_err(|error| error.to_string())?;
                if preview.fallback_reason.is_some() {
                    return Err(format!("shipped demo theme rejected: {}", theme.display()));
                }
                let name = slug(preview.theme.name());
                let png = output.join(format!("{name}.png"));
                let scene = output.join(format!("{name}.scene.json"));
                render_png(&preview.theme, &png).map_err(|error| error.to_string())?;
                write_scene(&preview.theme, &scene).map_err(|error| error.to_string())?;
                results.push(serde_json::json!({
                    "theme": preview.theme.name(),
                    "png": format!("{name}.png"),
                    "scene": format!("{name}.scene.json")
                }));
            }
            fs::write(
                output.join("summary.json"),
                serialize_json(&results).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let output = serialize_json(&results).map_err(|error| error.to_string())?;
            println!(
                "{}",
                String::from_utf8(output).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Some(command) => Err(format!("unknown command: {command}")),
        None => Err("usage: launcher-theme validate|preview|import|catalog|demo".into()),
    }
}

fn render_preview(theme: Option<&Path>, output: &Path) -> Result<(), String> {
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let preview = match theme {
        Some(path) => preview_path_or_fallback(path).map_err(|error| error.to_string())?,
        None => launcher_theme::preview_or_fallback(None).map_err(|error| error.to_string())?,
    };
    let result = serde_json::json!({
        "ok": true,
        "theme": preview.theme.name(),
        "fallback": preview.fallback_reason.is_some(),
        "reason": preview.fallback_reason,
    });
    render_png(&preview.theme, &output.join("preview.png")).map_err(|error| error.to_string())?;
    write_scene(&preview.theme, &output.join("scene.json")).map_err(|error| error.to_string())?;
    fs::write(
        output.join("result.json"),
        serialize_json(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let output = serialize_json(&result).map_err(|error| error.to_string())?;
    println!(
        "{}",
        String::from_utf8(output).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn required_path(args: &mut impl Iterator<Item = String>, option: &str) -> Result<PathBuf, String> {
    match (args.next().as_deref(), args.next()) {
        (Some(value), Some(path)) if value == option => Ok(PathBuf::from(path)),
        (Some(value), _) => Err(format!("expected {option}, got {value}")),
        (None, _) => Err(format!("missing {option}")),
    }
}

fn reject_extra(args: &mut impl Iterator<Item = String>) -> Result<(), String> {
    args.next().map_or(Ok(()), |argument| {
        Err(format!("unexpected argument: {argument}"))
    })
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn json_string(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap_or_else(|_| "\"theme\"".into())
}
