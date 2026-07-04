pub mod model;
pub mod tts;

pub use tts::TtsAction;

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
    /// TTS management commands
    Tts {
        #[command(subcommand)]
        action: TtsAction,
    },
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Initialize server configuration
    Init {
        /// Configuration file output path (default: ./server-config.yml)
        #[arg(short, long)]
        config: Option<std::path::PathBuf>,

        /// Force overwrite existing configuration file
        #[arg(long)]
        force: bool,
    },
    /// Run server in foreground mode
    Run {
        /// Configuration file path
        #[arg(short, long)]
        config: Option<std::path::PathBuf>,
    },
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
    /// Diagnose issues with a downloaded model
    Diagnose {
        /// Model name to diagnose
        model_name: String,
    },
}

// Daemon mode is no longer supported
// Use foreground mode with shell scripts for background operation
