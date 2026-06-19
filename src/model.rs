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
    #[serde(default)]
    pub environments: IndexMap<String, Environment>,
}

/// The declared type of a secret's value. Affects input validation and how
/// the value is presented; stored values are always strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Text,
    Number,
    Json,
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Number => "number",
            Kind::Json => "json",
        }
    }

    pub fn next(&self) -> Kind {
        match self {
            Kind::Text => Kind::Number,
            Kind::Number => Kind::Json,
            Kind::Json => Kind::Text,
        }
    }

    /// Validate `value` for this kind. Returns an error message if invalid.
    pub fn validate(&self, value: &str) -> Result<(), String> {
        match self {
            Kind::Text => Ok(()),
            Kind::Number => value
                .trim()
                .parse::<f64>()
                .map(|_| ())
                .map_err(|_| format!("`{value}` is not a number")),
            Kind::Json => serde_json::from_str::<serde_json::Value>(value)
                .map(|_| ())
                .map_err(|e| format!("invalid JSON: {e}")),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Environment {
    #[serde(default)]
    pub values: IndexMap<String, String>,
    /// Per-key declared types. Absent keys are [`Kind::Text`].
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub types: IndexMap<String, Kind>,
}

impl Environment {
    pub fn kind(&self, key: &str) -> Kind {
        self.types.get(key).copied().unwrap_or_default()
    }

    /// Set (or clear, when Text) the declared type for a key.
    pub fn set_kind(&mut self, key: &str, kind: Kind) {
        if kind == Kind::Text {
            self.types.shift_remove(key);
        } else {
            self.types.insert(key.to_string(), kind);
        }
    }
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
