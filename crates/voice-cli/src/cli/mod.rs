pub mod model;
pub mod service;
pub mod tts;

pub use service::InstallParams;
pub use tts::TtsAction;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "voice-cli")]
#[command(about = "Speech-to-text HTTP service with CLI interface")]
// 与 /health 的 version 字段同源（CARGO_PKG_VERSION）——此前硬编码 "0.1.0"，
// GPU bundle 构建与 npm 发版线排障时 --version 误导现场
#[command(version = env!("CARGO_PKG_VERSION"))]
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
    /// systemd service registration (Linux)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Initialize server configuration
    Init {
        /// Configuration file output path (default: ./config.yml)
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
pub enum ServiceAction {
    /// Generate unit, install to /etc/systemd/system, enable + start
    Install {
        /// Install root (WorkingDirectory); default: current directory
        #[arg(long, default_value = ".")]
        install_dir: std::path::PathBuf,

        /// systemd User= (default: current user)
        #[arg(long)]
        user: Option<String>,

        /// Register + enable but do not start/restart
        #[arg(long)]
        no_start: bool,

        /// Only print rendered unit; do not write or call systemctl
        #[arg(long)]
        dry_run: bool,

        /// CUDA lib dir for LD_LIBRARY_PATH drop-in (e.g. /usr/local/cuda/lib64)
        #[arg(long)]
        cuda_lib_dir: Option<std::path::PathBuf>,

        /// cuDNN lib dir for LD_LIBRARY_PATH drop-in
        #[arg(long)]
        cudnn_lib_dir: Option<std::path::PathBuf>,
    },
    /// Stop, disable, and remove the unit
    Uninstall,
    /// Show enable/active state, unit, and recent logs
    Status,
    /// Restart the service
    Restart,
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
