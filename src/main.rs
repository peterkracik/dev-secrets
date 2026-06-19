//! dev-secrets — a k9s-style TUI and CLI for managing local development
//! secrets, organized by project and environment.

mod cli;
mod clip;
mod commands;
mod config;
mod envfile;
mod model;
mod resolve;
mod store;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

fn main() -> Result<()> {
    let args = Cli::parse();

    match args.command {
        // No subcommand: launch the interactive TUI. If the app has never been
        // set up, fall back to the default store location automatically.
        None => {
            if !config::is_initialised() {
                let cfg = config::Config::default();
                config::save(&cfg)?;
            }
            tui::run()
        }
        Some(command) => {
            // Every other command needs a store; `setup` creates the config.
            if !matches!(command, Command::Setup { .. }) && !config::is_initialised() {
                let cfg = config::Config::default();
                config::save(&cfg)?;
            }
            commands::run(command)
        }
    }
}
