//! Command-line interface definition.
//!
//! Running `devsecrets` with no subcommand launches the TUI. Every action
//! available in the TUI is also available as a subcommand so the tool can be
//! scripted.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::model::Kind;

/// Output format for `export`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// `KEY=VALUE` lines (a `.env` file).
    Env,
    /// `export KEY=VALUE` lines, for `eval "$(...)"`.
    Shell,
    /// A JSON object of key/value pairs.
    Json,
    /// TOML `KEY = "value"` lines.
    Toml,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Env => "env",
            Format::Shell => "shell",
            Format::Json => "json",
            Format::Toml => "toml",
        }
    }

    pub fn next(self) -> Format {
        match self {
            Format::Env => Format::Shell,
            Format::Shell => Format::Json,
            Format::Json => Format::Toml,
            Format::Toml => Format::Env,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "devsecrets",
    version,
    about = "Manage local development secrets, organized by project and environment.",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Assign the current folder to a project + environment (interactive).
    Setup {
        /// Folder to configure. Defaults to the current directory.
        folder: Option<PathBuf>,
    },

    /// Show version, config locations, and folder assignments.
    Version,

    /// Manage projects.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },

    /// Manage environments within a project.
    Env {
        #[command(subcommand)]
        action: EnvAction,
    },

    /// Get/set/list secrets within an environment.
    #[command(visible_alias = "secrets")]
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },

    /// Import a .env file into an environment (creating it if needed).
    Import {
        /// Path to the .env file to read.
        file: PathBuf,
        /// Project to import into. Defaults to the project assigned to this folder.
        #[arg(short, long)]
        project: Option<String>,
        /// Environment to import into. Defaults to the folder's assigned env.
        #[arg(short, long)]
        env: Option<String>,
        /// Overwrite every changed key without asking (still keeps other keys).
        #[arg(long)]
        overwrite: bool,
        /// Replace the environment completely (clear it, then load the file).
        #[arg(long)]
        replace: bool,
    },

    /// Export an environment to a file (or stdout) in a chosen format.
    Export {
        /// Output file. If omitted, writes to stdout.
        file: Option<PathBuf>,
        /// Project to export. Defaults to the project assigned to this folder.
        #[arg(short, long)]
        project: Option<String>,
        /// Environment to export. Defaults to the folder's assigned env, then
        /// the project's default env.
        #[arg(short, long)]
        env: Option<String>,
        /// Output format. Defaults to the output file's extension, else env.
        #[arg(short, long, value_enum)]
        format: Option<Format>,
        /// Do not resolve ${project.env.key} references; export raw values.
        #[arg(long)]
        raw: bool,
    },

    /// Duplicate an environment within a project.
    Duplicate {
        #[arg(short, long)]
        project: String,
        /// Source environment to copy from.
        from: String,
        /// New environment name.
        to: String,
    },

    /// List everything in the store (projects, envs, counts).
    List,
}

#[derive(Subcommand, Debug)]
pub enum ProjectAction {
    /// Create a new project.
    Create { name: String },
    /// List all projects.
    List,
    /// Delete a project and all of its environments.
    Delete { name: String },
}

#[derive(Subcommand, Debug)]
pub enum EnvAction {
    /// Create a new environment within a project.
    Create {
        #[arg(short, long)]
        project: String,
        name: String,
    },
    /// List environments within a project.
    List {
        /// Project to list. Defaults to the project assigned to this folder.
        #[arg(short, long)]
        project: Option<String>,
    },
    /// Delete an environment.
    Delete {
        #[arg(short, long)]
        project: String,
        name: String,
    },
    /// Set the default environment for a project.
    SetDefault {
        #[arg(short, long)]
        project: String,
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SecretAction {
    /// Set (create or update) a secret.
    Set {
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        env: String,
        key: String,
        value: String,
        /// Declared value type (validated): text, number, or json.
        #[arg(short = 't', long = "type", value_enum)]
        kind: Option<Kind>,
    },
    /// Get a single secret (resolved unless --raw).
    Get {
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        env: String,
        key: String,
        /// Print the raw value without resolving references.
        #[arg(long)]
        raw: bool,
    },
    /// List secrets in an environment (values shown by default).
    List {
        /// Project to list. Defaults to the project assigned to this folder.
        #[arg(short, long)]
        project: Option<String>,
        /// Environment to list. Defaults to the folder's assigned env.
        #[arg(short, long)]
        env: Option<String>,
        /// Mask values instead of showing them.
        #[arg(long)]
        mask: bool,
    },
    /// Delete a secret.
    Delete {
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        env: String,
        key: String,
    },
}
