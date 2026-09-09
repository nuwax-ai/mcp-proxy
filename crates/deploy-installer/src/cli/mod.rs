pub mod assets;
pub mod common;
mod doctor;
pub mod document_parser;
pub mod env_config;
mod probe_vulkan;
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
    /// voice-cli deployment commands (macOS launchd + Linux CUDA/Vulkan systemd)
    VoiceCli {
        #[command(subcommand)]
        action: VoiceCliAction,
    },
    /// Vulkan GPU probe (internal; runs in a crash-isolated subprocess for
    /// voice-cli tier detection — exit code, not for interactive use)
    #[command(hide = true, name = "__probe-vulkan")]
    ProbeVulkan,
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
    /// Download prebuilt voice-cli CUDA bundle from OSS (default tier on Linux
    /// x86_64 when an NVIDIA GPU + CUDA toolkit are detected)
    #[arg(long, conflicts_with = "use_oss_vulkan")]
    pub use_oss_cuda: bool,
    /// Skip OSS CUDA bundle download (auto tier may still pick Vulkan; with
    /// --skip-oss-vulkan forces the CPU build)
    #[arg(long)]
    pub skip_oss_cuda: bool,
    /// Download prebuilt voice-cli Vulkan bundle from OSS (non-NVIDIA GPU on
    /// Linux x86_64; auto tier when CUDA is unavailable but Vulkan is)
    #[arg(long, conflicts_with = "use_oss_cuda")]
    pub use_oss_vulkan: bool,
    /// Skip OSS Vulkan bundle download (with --skip-oss-cuda forces the CPU build)
    #[arg(long)]
    pub skip_oss_vulkan: bool,
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
        Commands::ProbeVulkan => probe_vulkan::run(),
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

    /// --use-oss-cuda 与 --use-oss-vulkan 互斥（显式档位只能给一个；
    /// skip 对不互斥——双 skip 是合法的强制 CPU 组合）
    #[test]
    fn oss_cuda_conflicts_with_oss_vulkan() {
        let combined = Cli::try_parse_from([
            "deploy-installer",
            "voice-cli",
            "install",
            "--use-oss-cuda",
            "--use-oss-vulkan",
        ]);
        assert!(combined.is_err(), "双显式档位应被 clap 拒绝");

        let both_skip = Cli::try_parse_from([
            "deploy-installer",
            "voice-cli",
            "install",
            "--skip-oss-cuda",
            "--skip-oss-vulkan",
        ]);
        assert!(both_skip.is_ok(), "双 skip（强制 CPU）应通过");

        let vulkan_only = Cli::try_parse_from([
            "deploy-installer",
            "voice-cli",
            "install",
            "--use-oss-vulkan",
        ]);
        assert!(vulkan_only.is_ok(), "单独 --use-oss-vulkan 应通过");
    }

    /// 隐藏探针子命令可解析且不出现在 help（面向父进程 reexec，非用户接口）
    #[test]
    fn probe_vulkan_subcommand_hidden_but_parseable() {
        let parsed = Cli::try_parse_from(["deploy-installer", "__probe-vulkan"]);
        assert!(parsed.is_ok(), "探针子命令应可解析");
        // 验证子命令确实 hide（面向父进程 reexec，非用户接口）
        let help = <Cli as clap::CommandFactory>::command()
            .render_help()
            .to_string();
        assert!(
            !help.contains("__probe-vulkan"),
            "探针不应出现在 help: {help}"
        );
    }
}
