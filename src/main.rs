//! dev-secrets — a Telescope-style TUI and CLI for managing local development
//! secrets, organized by project and environment.

mod cli;
mod clip;
mod commands;
mod config;
mod envfile;
mod fuzzy;
mod meta;
mod model;
mod resolve;
mod store;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() -> Result<()> {
    let args = Cli::parse();

    // Initialise settings + store location the first time the app runs.
    if !config::is_initialised() {
        config::save(&config::Settings::default())?;
    }

    match args.command {
        // No subcommand: launch the interactive TUI.
        None => tui::run(),
        Some(command) => commands::run(command),
    }
}
