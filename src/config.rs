//! Application settings and store-file location handling.
//!
//! Everything lives under the user's config dir, `~/.config/devsecrets`:
//!
//! - `settings.json` — points at where the secrets store file lives. By
//!   default that is `store.json` in the same directory, but it can be moved
//!   anywhere (e.g. a synced folder) via `devsecrets settings store <path>`.
//! - `meta.json`     — folder → (project, environment) assignments (see
//!   [`crate::meta`]).
//! - `store.json`    — the secrets themselves (default location).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const APP_DIR: &str = "devsecrets";
const SETTINGS_FILE: &str = "settings.json";
const DEFAULT_STORE_FILE: &str = "store.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Absolute path to the secrets store JSON file.
    pub store_path: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            store_path: default_store_path(),
        }
    }
}

/// Where all config lives. We deliberately use `~/.config/devsecrets` on every
/// Unix (including macOS, where `dirs::config_dir()` would otherwise return
/// `~/Library/Application Support`). `$XDG_CONFIG_HOME` is honoured if set; on
/// Windows we use the standard config dir.
pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join(APP_DIR);
        }
    }
    #[cfg(windows)]
    {
        return dirs::config_dir()
            .map(|d| d.join(APP_DIR))
            .unwrap_or_else(|| PathBuf::from(".devsecrets"));
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".config").join(APP_DIR);
        }
        PathBuf::from(".devsecrets")
    }
}

pub fn settings_file() -> PathBuf {
    config_dir().join(SETTINGS_FILE)
}

pub fn default_store_path() -> PathBuf {
    config_dir().join(DEFAULT_STORE_FILE)
}

/// Whether the app has been initialised (settings file exists).
pub fn is_initialised() -> bool {
    settings_file().exists()
}

/// Load settings, falling back to defaults when they do not exist yet.
pub fn load() -> Result<Settings> {
    let path = settings_file();
    if !path.exists() {
        return Ok(Settings::default());
    }
    let data = fs::read_to_string(&path)
        .with_context(|| format!("reading settings {}", path.display()))?;
    let settings: Settings = serde_json::from_str(&data)
        .with_context(|| format!("parsing settings {}", path.display()))?;
    Ok(settings)
}

/// Persist settings, creating the config directory if needed.
pub fn save(settings: &Settings) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating config dir {}", dir.display()))?;
    let path = settings_file();
    let data = serde_json::to_string_pretty(settings)?;
    fs::write(&path, data).with_context(|| format!("writing settings {}", path.display()))?;
    Ok(())
}

/// Resolve a store file path from a user-supplied location. If they pass a
/// directory we put `store.json` inside it; if they pass a `.json` file we use
/// it directly.
pub fn store_path_for(location: &Path) -> PathBuf {
    if location.extension().map(|e| e == "json").unwrap_or(false) {
        location.to_path_buf()
    } else {
        location.join(DEFAULT_STORE_FILE)
    }
}
