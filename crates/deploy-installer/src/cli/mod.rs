mod doctor;
pub mod document_parser;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub use document_parser::run as run_document_parser;

#[derive(Parser)]
#[command(
    name = "deploy-installer",
    about = "Unified deployment installer for nuwax services (systemd / launchd)",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Check prerequisites (uv, ports, disk, platform)
    Doctor,
    /// document-parser deployment commands
    DocumentParser {
        #[command(subcommand)]
        action: DocumentParserAction,
    },
}

#[derive(Subcommand)]
pub enum DocumentParserAction {
    /// Initialize install directory, copy binaries/templates, set up Python venv
    Setup(SetupArgs),
    /// Full install: setup + service registration (prompts for OSS keys if missing)
    Install(SetupArgs),
    /// Upgrade document-parser binary in an existing install directory
    Upgrade {
        #[arg(long)]
        install_dir: PathBuf,
    },
    /// Service lifecycle (systemd on Linux, launchd on macOS)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(clap::Args, Clone)]
pub struct SetupArgs {
    /// Deployment root directory
    #[arg(long)]
    pub install_dir: Option<PathBuf>,
    /// Download pre-built Python venv from OSS instead of uv-init
    #[arg(long)]
    pub use_prebuilt_venv: bool,
    /// OSS base URL for optional assets (venv tarball)
    #[arg(long)]
    pub oss_base: Option<String>,
}

#[derive(Subcommand)]
pub enum ServiceAction {
    Install(ServiceDirArgs),
    Uninstall(ServiceDirArgs),
    Status(ServiceDirArgs),
    Restart(ServiceDirArgs),
}

#[derive(clap::Args, Clone)]
pub struct ServiceDirArgs {
    #[arg(long, default_value = ".")]
    pub install_dir: PathBuf,
    #[arg(long)]
    pub user: Option<String>,
    #[arg(long)]
    pub no_start: bool,
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Doctor => doctor::run(),
        Commands::DocumentParser { action } => document_parser::run(action),
    }
}
