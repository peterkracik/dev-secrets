//! Application configuration and store-file location handling.
//!
//! The app keeps a tiny config file that points at where the actual data
//! store lives. By default everything sits under the user's config dir
//! (`~/.config/dev-secrets`), but `devsecrets setup` lets the user assign a
//! different folder for the metadata store.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const APP_DIR: &str = "dev-secrets";
const CONFIG_FILE: &str = "config.json";
const DEFAULT_STORE_FILE: &str = "store.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Absolute path to the data store JSON file.
    pub store_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            store_path: default_store_path(),
        }
    }
}

/// `~/.config/dev-secrets` (falls back to `./.dev-secrets` if no config dir).
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join(APP_DIR))
        .unwrap_or_else(|| PathBuf::from(".dev-secrets"))
}

pub fn config_file() -> PathBuf {
    config_dir().join(CONFIG_FILE)
}

pub fn default_store_path() -> PathBuf {
    config_dir().join(DEFAULT_STORE_FILE)
}

/// Whether the app has been initialised (config file exists).
pub fn is_initialised() -> bool {
    config_file().exists()
}

/// Load the config, falling back to defaults when it does not exist yet.
pub fn load() -> Result<Config> {
    let path = config_file();
    if !path.exists() {
        return Ok(Config::default());
    }
    let data =
        fs::read_to_string(&path).with_context(|| format!("reading config {}", path.display()))?;
    let cfg: Config = serde_json::from_str(&data)
        .with_context(|| format!("parsing config {}", path.display()))?;
    Ok(cfg)
}

/// Persist the config, creating the config directory if needed.
pub fn save(cfg: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating config dir {}", dir.display()))?;
    let path = config_file();
    let data = serde_json::to_string_pretty(cfg)?;
    fs::write(&path, data).with_context(|| format!("writing config {}", path.display()))?;
    Ok(())
}

/// Resolve the store directory for a chosen folder. If the user passes a
/// directory we put `store.json` inside it; if they pass a `.json` file we use
/// it directly.
pub fn store_path_for_folder(folder: &Path) -> PathBuf {
    if folder.extension().map(|e| e == "json").unwrap_or(false) {
        folder.to_path_buf()
    } else {
        folder.join(DEFAULT_STORE_FILE)
    }
}
