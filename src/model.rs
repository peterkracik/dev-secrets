//! Core data model for the secret store.
//!
//! The store is a simple tree:
//!
//! ```text
//! Store
//! └── projects: { name -> Project }
//!     └── environments: { name -> Environment }
//!         └── values: { key -> value }
//! ```
//!
//! `IndexMap` is used everywhere so that insertion order is preserved, which
//! keeps exported `.env` files stable and readable.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// The root of the on-disk data store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub projects: IndexMap<String, Project>,
}

/// A project groups together one or more environments (instances).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Project {
    /// Name of the environment used when none is given on export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_env: Option<String>,
    /// Optional working directory associated with this project. When
    /// `devsecrets` runs inside this folder, the project is auto-selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default)]
    pub environments: IndexMap<String, Environment>,
}

/// A single environment / instance: an ordered set of key/value secrets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Environment {
    #[serde(default)]
    pub values: IndexMap<String, String>,
}

impl Store {
    pub fn project(&self, name: &str) -> Option<&Project> {
        self.projects.get(name)
    }

    pub fn project_mut(&mut self, name: &str) -> Option<&mut Project> {
        self.projects.get_mut(name)
    }

    /// Look up an environment's resolved-or-raw value across the whole store.
    pub fn value(&self, project: &str, env: &str, key: &str) -> Option<&String> {
        self.projects
            .get(project)?
            .environments
            .get(env)?
            .values
            .get(key)
    }

    /// Find the first project whose associated folder matches `dir`.
    pub fn project_for_folder(&self, dir: &str) -> Option<&str> {
        self.projects
            .iter()
            .find(|(_, p)| p.folder.as_deref() == Some(dir))
            .map(|(name, _)| name.as_str())
    }
}

impl Project {
    /// Resolve which environment to use: explicit argument, else default,
    /// else the only environment if there is exactly one.
    pub fn resolve_env<'a>(&'a self, requested: Option<&'a str>) -> Option<&'a str> {
        if let Some(req) = requested {
            return self.environments.contains_key(req).then_some(req);
        }
        if let Some(def) = &self.default_env {
            if self.environments.contains_key(def) {
                return Some(def);
            }
        }
        if self.environments.len() == 1 {
            return self.environments.keys().next().map(|s| s.as_str());
        }
        None
    }
}
