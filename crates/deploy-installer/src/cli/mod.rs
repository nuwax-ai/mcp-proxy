pub mod assets;
pub mod common;
mod doctor;
pub mod document_parser;
pub mod env_config;
pub mod voice_cli;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    /// voice-cli deployment commands (macOS launchd + Linux CUDA systemd)
    VoiceCli {
        #[command(subcommand)]
        action: VoiceCliAction,
    },
}

#[derive(Subcommand)]
pub enum DocumentParserAction {
    /// Initialize install directory, copy binaries/templates, set up Python venv
    Setup(SetupArgs),
    /// Full install: setup + service registration (requires OSS keys or
    /// custom-upload env; bails with guidance if neither is configured)
    Install(SetupArgs),
    /// Upgrade document-parser binary in an existing install directory
    Upgrade {
        /// Install directory (default: ~/document-parser)
        #[arg(long)]
        install_dir: Option<PathBuf>,
    },
    /// Service lifecycle (systemd on Linux, launchd on macOS)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
pub enum VoiceCliAction {
    /// Initialize install directory and copy bundled binary + config
    Setup(VoiceCliSetupArgs),
    /// Setup, download Whisper models from OSS (default large-v3), register service
    Install(VoiceCliSetupArgs),
    /// Upgrade voice-cli binary in an existing install directory
    Upgrade {
        /// Install directory (default: ~/voice-cli)
        #[arg(long)]
        install_dir: Option<PathBuf>,
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
    /// Skip pre-built venv on macOS (default: use OSS venv on macOS)
    #[arg(long)]
    pub no_prebuilt_venv: bool,
    /// Install Python venv from a local tarball (offline environments;
    /// mutually exclusive with --use-prebuilt-venv / --no-prebuilt-venv)
    #[arg(long, conflicts_with_all = ["use_prebuilt_venv", "no_prebuilt_venv"])]
    pub venv_file: Option<PathBuf>,
    /// OSS base URL for optional assets (venv tarball)
    #[arg(long)]
    pub oss_base: Option<String>,
}

#[derive(clap::Args, Clone)]
pub struct VoiceCliSetupArgs {
    /// Deployment root directory (default: ~/voice-cli)
    #[arg(long)]
    pub install_dir: Option<PathBuf>,
    /// Download prebuilt Whisper ggml models from OSS
    #[arg(long)]
    pub use_prebuilt_models: bool,
    /// Skip Whisper model download (offline or models already present)
    #[arg(long)]
    pub skip_models: bool,
    /// Which OSS model pack: large-v3 (default) or all
    #[arg(long, default_value = "large-v3", value_name = "PACK")]
    pub models: String,
    /// OSS base URL prefix for voice-cli optional assets
    #[arg(long)]
    pub oss_base: Option<String>,
    /// Download prebuilt voice-cli CUDA bundle from OSS (default on Linux x86_64)
    #[arg(long)]
    pub use_oss_cuda: bool,
    /// Skip OSS CUDA bundle download (use vendor binary or existing install)
    #[arg(long)]
    pub skip_oss_cuda: bool,
    /// NVIDIA CUDA toolkit lib dir for systemd LD_LIBRARY_PATH (Linux CUDA)
    #[arg(long)]
    pub cuda_lib_dir: Option<PathBuf>,
    /// cuDNN lib dir for systemd LD_LIBRARY_PATH (Linux CUDA)
    #[arg(long)]
    pub cudnn_lib_dir: Option<PathBuf>,
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
    /// Install directory (default: ~/document-parser or ~/voice-cli per service)
    #[arg(long, default_value = ".")]
    pub install_dir: PathBuf,
    #[arg(long)]
    pub user: Option<String>,
    #[arg(long)]
    pub no_start: bool,
    #[arg(long)]
    pub dry_run: bool,
    /// NVIDIA CUDA toolkit lib dir (voice-cli Linux CUDA only)
    #[arg(long)]
    pub cuda_lib_dir: Option<PathBuf>,
    /// cuDNN lib dir (voice-cli Linux CUDA only)
    #[arg(long)]
    pub cudnn_lib_dir: Option<PathBuf>,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Doctor => doctor::run(),
        Commands::DocumentParser { action } => document_parser::run(action),
        Commands::VoiceCli { action } => voice_cli::run(action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// --venv-file 与 --use/--no-prebuilt-venv 互斥（组合传参直接解析失败）
    #[test]
    fn venv_file_conflicts_with_prebuilt_flags() {
        let combined = Cli::try_parse_from([
            "deploy-installer",
            "document-parser",
            "install",
            "--venv-file",
            "x.tar.gz",
            "--use-prebuilt-venv",
        ]);
        assert!(combined.is_err(), "组合传参应被 clap 拒绝");

        let solo = Cli::try_parse_from([
            "deploy-installer",
            "document-parser",
            "install",
            "--venv-file",
            "x.tar.gz",
        ]);
        assert!(solo.is_ok(), "单独传 --venv-file 应通过");
    }
}
