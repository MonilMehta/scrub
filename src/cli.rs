use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about = "A modern terminal application for macOS to safely reclaim disk space.", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Launch the TUI dashboard (default when no args provided)
    Dashboard {
        /// Optional specific paths to scan
        paths: Vec<String>,
    },
    /// Scan for reclaimable space and print a report
    Scan {
        #[arg(long)]
        json: bool,
        /// Optional specific paths to scan
        paths: Vec<String>,
    },
    /// Clean specific caches
    Clean {
        /// Target ecosystem to clean (e.g., xcode, node, all)
        target: String,
    },
    /// Run diagnostics
    Doctor,
    /// Show statistics
    Stats,
    /// Plugin management
    Plugin {
        #[command(subcommand)]
        cmd: PluginCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    List,
    Validate,
}
