//! Loading and saving the [`Store`] to disk as JSON.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config;
use crate::model::Store;

/// Handle that ties an in-memory [`Store`] to its backing file.
pub struct StoreHandle {
    pub path: PathBuf,
    pub store: Store,
}

impl StoreHandle {
    /// Open the store referenced by the app config, creating an empty one if
    /// the file does not exist yet.
    pub fn open() -> Result<Self> {
        let cfg = config::load()?;
        Self::open_at(cfg.store_path)
    }

    pub fn open_at(path: PathBuf) -> Result<Self> {
        let store = if path.exists() {
            load(&path)?
        } else {
            Store::default()
        };
        Ok(StoreHandle { path, store })
    }

    pub fn save(&self) -> Result<()> {
        save(&self.path, &self.store)
    }
}

pub fn load(path: &Path) -> Result<Store> {
    let data =
        fs::read_to_string(path).with_context(|| format!("reading store {}", path.display()))?;
    if data.trim().is_empty() {
        return Ok(Store::default());
    }
    let store: Store =
        serde_json::from_str(&data).with_context(|| format!("parsing store {}", path.display()))?;
    Ok(store)
}

pub fn save(path: &Path, store: &Store) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating store dir {}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(store)?;
    fs::write(path, data).with_context(|| format!("writing store {}", path.display()))?;
    Ok(())
}
