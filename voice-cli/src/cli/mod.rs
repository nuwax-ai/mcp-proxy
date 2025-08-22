pub mod server;
pub mod model;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "voice-cli")]
#[command(about = "Speech-to-text HTTP service with CLI interface")]
#[command(version = "0.1.0")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Configuration file path
    #[arg(short, long, default_value = "config.yml")]
    pub config: String,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Server management commands
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },
    /// Model management commands
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
    /// Internal daemon command (used by daemon service)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Run server in foreground mode
    Run,
    /// Start server in background mode (daemon)
    Start,
    /// Stop background server
    Stop,
    /// Restart background server
    Restart,
    /// Check server status
    Status,
}

#[derive(Subcommand)]
pub enum ModelAction {
    /// Download a specific model
    Download {
        /// Model name to download (e.g., base, small, large)
        model_name: String,
    },
    /// List available and downloaded models
    List,
    /// Validate downloaded models
    Validate,
    /// Remove a downloaded model
    Remove {
        /// Model name to remove
        model_name: String,
    },
}

#[derive(Subcommand)]
pub enum DaemonAction {
    /// Serve HTTP requests (internal command used by daemon)
    Serve,
}