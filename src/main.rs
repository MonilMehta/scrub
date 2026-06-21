pub mod commands;
pub mod core;
pub mod models;
pub mod plugins;
pub mod scanners;
pub mod ui;
pub mod utils;
pub mod cli;

use clap::Parser;
use cli::{Cli, Commands};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Scan { json, paths }) => {
            commands::scan::execute(json, paths)?;
        }
        Some(Commands::Clean { target }) => {
            println!("Cleaning target: {}", target);
        }
        Some(Commands::Dashboard { paths }) => {
            // Launch TUI
            ui::app::run(paths)?;
        }
        None => {
            ui::app::run(vec![])?;
        }
        _ => {
            println!("Command not fully implemented yet.");
        }
    }

    Ok(())
}

