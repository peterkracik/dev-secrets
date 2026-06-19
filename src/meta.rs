//! Folder → (project, environment) assignments.
//!
//! When you run `devsecrets setup` inside a project folder, the folder is
//! recorded here together with the project and environment you picked. From
//! then on, plain commands like `devsecrets export` (and the TUI) know which
//! project/environment this folder belongs to without you repeating it.
//!
//! Stored as `meta.json` in the config dir.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config;

const META_FILE: &str = "meta.json";

/// One folder's assignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
    pub project: String,
    pub env: String,
}

/// All folder assignments, keyed by absolute folder path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub assignments: BTreeMap<String, Assignment>,
}

pub fn meta_file() -> PathBuf {
    config::config_dir().join(META_FILE)
}

pub fn load() -> Result<Meta> {
    let path = meta_file();
    if !path.exists() {
        return Ok(Meta::default());
    }
    let data =
        fs::read_to_string(&path).with_context(|| format!("reading meta {}", path.display()))?;
    if data.trim().is_empty() {
        return Ok(Meta::default());
    }
    let meta: Meta =
        serde_json::from_str(&data).with_context(|| format!("parsing meta {}", path.display()))?;
    Ok(meta)
}

pub fn save(meta: &Meta) -> Result<()> {
    let dir = config::config_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating config dir {}", dir.display()))?;
    let path = meta_file();
    let data = serde_json::to_string_pretty(meta)?;
    fs::write(&path, data).with_context(|| format!("writing meta {}", path.display()))?;
    Ok(())
}

impl Meta {
    pub fn get(&self, folder: &str) -> Option<&Assignment> {
        self.assignments.get(folder)
    }

    pub fn set(&mut self, folder: String, project: String, env: String) {
        self.assignments.insert(folder, Assignment { project, env });
    }

    /// Folders assigned to a given project (for display).
    pub fn folders_for_project<'a>(&'a self, project: &'a str) -> Vec<&'a str> {
        self.assignments
            .iter()
            .filter(|(_, a)| a.project == project)
            .map(|(folder, _)| folder.as_str())
            .collect()
    }
}

/// Canonical absolute path string for `p` (file or dir; need not exist).
pub fn canonical_path(p: &Path) -> Result<PathBuf> {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    };
    Ok(fs::canonicalize(&abs).unwrap_or(abs))
}

/// Canonical absolute path string for a directory.
pub fn canonical_dir(dir: &Path) -> Result<String> {
    Ok(canonical_path(dir)?.to_string_lossy().into_owned())
}

/// The canonical path of the current working directory.
pub fn current_dir() -> Result<String> {
    canonical_dir(Path::new("."))
}
