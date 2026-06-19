//! Command-line interface definition.
//!
//! Running `devsecrets` with no subcommand launches the TUI. Every action
//! available in the TUI is also available as a subcommand so the tool can be
//! scripted.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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

    /// Show or change global settings (where the secrets store lives).
    Settings {
        #[command(subcommand)]
        action: Option<SettingsAction>,
    },

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
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },

    /// Import a .env file into an environment (creating it if needed).
    Import {
        /// Path to the .env file to read.
        file: PathBuf,
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        env: String,
        /// Replace the environment contents instead of merging.
        #[arg(long)]
        replace: bool,
    },

    /// Export an environment to a .env file (or stdout).
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
pub enum SettingsAction {
    /// Show current settings and config locations.
    Show,
    /// Change where the secrets store file is kept (moving existing data).
    Store {
        /// Target directory or `.json` file path.
        path: PathBuf,
    },
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
        #[arg(short, long)]
        project: String,
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
    /// List secrets in an environment.
    List {
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        env: String,
        /// Reveal values instead of masking them.
        #[arg(long)]
        show: bool,
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
