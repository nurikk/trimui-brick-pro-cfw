use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use serde::{Deserialize, Serialize};
use sim_platform_contract::battery::BatteryPolicy;
use ui_model::{ScraperProgress, UiPreferences};

const SCHEMA_VERSION: u16 = 1;
const IDENTITY: &str = "Artbook";
const MAX_BYTES: u64 = 64 * 1024;
const MAX_FAVORITES: usize = 512;
const MAX_RECENT: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RecentItem {
    pub content_id: String,
    pub playtime_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct State {
    pub schema_version: u16,
    pub identity: String,
    pub preferences: UiPreferences,
    pub favorites: Vec<String>,
    pub recent: Vec<RecentItem>,
    #[serde(default)]
    pub battery_policy: BatteryPolicy,
    #[serde(default)]
    pub scraper_progress: Option<ScraperProgress>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            identity: IDENTITY.into(),
            preferences: UiPreferences::default(),
            favorites: Vec::new(),
            recent: Vec::new(),
            battery_policy: BatteryPolicy::default(),
            scraper_progress: None,
        }
    }
}

pub fn load(root: &Path) -> State {
    let path = root.join("launcher-state.json");
    let Ok(file_type) = fs::symlink_metadata(&path) else {
        return State::default();
    };
    if file_type.file_type().is_symlink() {
        return State::default();
    }
    let Ok(metadata) = fs::metadata(&path) else {
        return State::default();
    };
    if metadata.len() > MAX_BYTES {
        return State::default();
    }
    let Ok(bytes) = fs::read(path) else {
        return State::default();
    };
    let Ok(state) = serde_json::from_slice::<State>(&bytes) else {
        return State::default();
    };
    if !valid_state(&state) {
        State::default()
    } else {
        state
    }
}

pub fn save(root: &Path, state: &State) -> std::io::Result<()> {
    if !valid_state(state) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid launcher state",
        ));
    }
    fs::create_dir_all(root)?;
    let path = root.join("launcher-state.json");
    let temporary = root.join(".launcher-state.json.tmp");
    let _ = fs::remove_file(&temporary);
    let bytes = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("launcher state is oversized"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, &path)?;
    fs::File::open(root)?.sync_all()
}

fn valid_state(state: &State) -> bool {
    state.schema_version == SCHEMA_VERSION
        && state.identity == IDENTITY
        && state.favorites.len() <= MAX_FAVORITES
        && state.recent.len() <= MAX_RECENT
        && !state.favorites.windows(2).any(|ids| ids[0] >= ids[1])
        && state.favorites.iter().all(|id| valid_id(id))
        && state.recent.iter().all(|item| valid_id(&item.content_id))
        && state.battery_policy.validate().is_ok()
        && state
            .scraper_progress
            .as_ref()
            .is_none_or(valid_scraper_progress)
}

fn valid_scraper_progress(progress: &ScraperProgress) -> bool {
    let expected_percent = if progress.total == 0 {
        progress.percent == 0 || progress.percent == 100
    } else {
        progress.percent
            == ((u32::from(progress.completed) * 100) / u32::from(progress.total)) as u8
    };
    progress.total <= 256
        && progress.completed <= progress.total
        && expected_percent
        && matches!(progress.configured_slots, 1 | 2 | 4)
        && progress.rows.len() <= progress.configured_slots as usize
        && progress
            .paused_reason
            .as_deref()
            .is_none_or(valid_scraper_reason)
        && progress.rows.iter().all(valid_scraper_row)
}

fn valid_scraper_row(row: &ui_model::ScraperRow) -> bool {
    valid_id(&row.game_id.0)
        && !row.title.is_empty()
        && row.title.len() <= 128
        && !row.title.chars().any(char::is_control)
        && row.provider.as_deref().is_none_or(valid_scraper_id)
        && row
            .fallback_transition
            .as_deref()
            .is_none_or(valid_scraper_text)
}

fn valid_scraper_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".:_-".contains(character))
}

fn valid_scraper_reason(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

fn valid_scraper_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.chars().any(char::is_control)
        && !value.contains('/')
        && !value.contains('\\')
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}
