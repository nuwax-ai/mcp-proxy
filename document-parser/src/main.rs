use anyhow::Result;
use clap::{Parser, Subcommand};
use document_parser::{
    APP_NAME, APP_VERSION, AppConfig, AppError, AppState,
    config::{CudaStatus, init_global_config, init_global_cuda_status},
    routes::create_routes,
    utils::environment_manager::{
        CleanupRisk, DirectoryValidationResult, EnvironmentManager, EnvironmentStatus, InstallStage,
    },
};
use log::{error, info, warn};
use std::backtrace::Backtrace;
use std::env;
use std::path::PathBuf;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};

/// Document Parser - 文档解析服务
///
/// 使用当前目录虚拟环境 (./venv/) 进行Python依赖管理
///
/// 快速开始:
///   1. document-parser uv-init    # 初始化虚拟环境和依赖
///   2. document-parser server     # 启动服务器
///
/// 虚拟环境激活:
///   source ./venv/bin/activate    # Linux/macOS
///   .\venv\Scripts\activate       # Windows
#[derive(Parser)]
#[command(name = "document-parser")]
#[command(about = "A document parsing service with CLI support")]
#[command(version = APP_VERSION)]
#[command(long_about = "
Document Parser 是一个多格式文档解析服务，支持PDF、Word、Excel、PowerPoint等格式。

环境管理:
  本服务使用当前目录下的虚拟环境 (./venv/) 来管理Python依赖。
  首次使用请运行 'document-parser uv-init' 来自动设置环境。

支持的格式:
  • PDF (通过 MinerU 引擎)
  • Word, Excel, PowerPoint (通过 MarkItDown 引擎)
  • Markdown, HTML, Text 等

故障排除:
  • 运行 'document-parser check' 检查环境状态
  • 运行 'document-parser troubleshoot' 获取详细故障排除指南
  • 查看日志文件: logs/ 目录
")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// 配置文件路径
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// 服务器端口
    #[arg(short, long)]
    port: Option<u16>,

    /// 服务器主机地址
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动服务器模式
    Server {
        /// 后台运行
        #[arg(short, long)]
        daemon: bool,
    },
    /// 解析单个文件
    Parse {
        /// 输入文件路径
        #[arg(short, long)]
        input: PathBuf,
        /// 输出文件路径
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 解析器类型 (mineru, markitdown)
        #[arg(short, long, default_value = "mineru")]
        parser: String,
    },
    /// 检查环境状态和虚拟环境配置
    #[command(long_about = "
检查Python环境、虚拟环境状态和依赖安装情况。

检查内容:
  • Python版本和可用性
  • UV工具安装状态
  • 虚拟环境 (./venv/) 状态
  • MinerU和MarkItDown依赖
  • CUDA支持情况
  • 路径和权限问题诊断

输出详细的诊断报告和修复建议。")]
    Check,
    /// 安装依赖环境 (已弃用，请使用 uv-init)
    #[command(hide = true)]
    Install,
    /// 初始化当前目录的uv虚拟环境和依赖
    #[command(name = "uv-init")]
    #[command(about = "在当前目录初始化uv虚拟环境，自动安装mineru和markitdown依赖")]
    #[command(long_about = "
在当前工作目录创建虚拟环境 (./venv/) 并安装所需的Python依赖。

执行步骤:
  1. 检查并安装UV工具 (如果缺失)
  2. 在当前目录创建虚拟环境: ./venv/
  3. 安装MinerU依赖: uv pip install -U \"mineru[core]\"
  4. 安装MarkItDown依赖: uv pip install markitdown
  5. 验证安装结果

完成后可以使用以下命令激活虚拟环境:
  Linux/macOS: source ./venv/bin/activate
  Windows:     .\\venv\\Scripts\\activate

然后启动服务器: document-parser server")]
    UvInit,
    /// 显示详细的故障排除指南
    #[command(about = "显示虚拟环境和依赖问题的详细故障排除指南")]
    #[command(long_about = "
显示常见问题的详细故障排除指南，包括:

虚拟环境问题:
  • 虚拟环境创建失败
  • 路径和权限问题
  • 依赖安装失败
  • 跨平台兼容性问题

网络和下载问题:
  • 网络连接超时
  • 包下载失败
  • 镜像源配置

系统环境问题:
  • Python版本兼容性
  • UV工具安装
  • CUDA环境配置

每个问题都包含详细的诊断步骤和解决方案。")]
    Troubleshoot,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_locale_from_env();

    let cli = Cli::parse();

    // 加载配置
    let mut app_config = if let Some(config_path) = cli.config {
        // 直接传入配置文件路径
        AppConfig::load_base_config_with_path(Some(config_path.to_string_lossy().to_string()))
            .map_err(|e| anyhow::anyhow!("配置加载失败: {}", e))?
    } else {
        AppConfig::load_config().map_err(|e| anyhow::anyhow!("配置加载失败: {}", e))?
    };

    // 覆盖命令行参数
    if let Some(port) = cli.port {
        app_config.server.port = port;
    }
    app_config.server.host = cli.host.clone();

    // 初始化全局配置
    init_global_config(app_config.clone())
        .map_err(|e| anyhow::anyhow!("全局配置初始化失败: {}", e))?;

    // 检查并缓存CUDA环境状态到全局配置
    info!("Check CUDA environment status...");
    let environment_manager = EnvironmentManager::for_current_directory()
        .map_err(|e| anyhow::anyhow!("无法创建环境管理器: {}", e))?;

    let cuda_status = match environment_manager.check_cuda_environment().await {
        Ok(cuda_info) => {
            let recommended_device = if cuda_info.available && !cuda_info.devices.is_empty() {
                // 选择显存最大的设备作为推荐设备

                cuda_info
                    .devices
                    .iter()
                    .max_by_key(|device| device.memory_total)
                    .map(|device| format!("cuda:{}", device.id))
            } else {
                None
            };

            let status = CudaStatus {
                available: cuda_info.available,
                version: cuda_info.version,
                device_count: cuda_info.devices.len(),
                recommended_device,
            };

            if status.available {
                info!(
                    "CUDA environment is available: version={:?}, devices={}, recommended={}",
                    status.version.as_deref().unwrap_or("unknown"),
                    status.device_count,
                    status.recommended_device.as_deref().unwrap_or("cuda")
                );
            } else {
                info!("CUDA environment is not available, CPU mode will be used");
            }

            status
        }
        Err(e) => {
            warn!("CUDA environment check failed: {e}, CPU mode will be used");
            CudaStatus::default()
        }
    };

    // 初始化全局CUDA状态
    if let Err(e) = init_global_cuda_status(cuda_status) {
        warn!("Failed to initialize global CUDA state: {e}");
    }

    let log_level = app_config.log.level.clone();
    let log_path = app_config.log.path.clone();
    let server_port = app_config.server.port;
    let server_host = app_config.server.host.clone();
    let retain_days = app_config.log.retain_days;

    // 配置日志
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level.clone()));

    // 控制台日志层
    let console_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_writer(std::io::stdout)
        .with_filter(console_filter);

    // 文件日志层 - 使用 Builder 模式配置日志轮转和保留策略
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY) // 按天滚动
        .filename_prefix("log") // 文件名前缀
        .max_log_files(retain_days as usize) // 保留最近 N 个日志文件
        .build(&log_path)?;
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let log_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(non_blocking)
        .with_filter(log_filter);

    // 初始化 tracing 订阅器
    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    info!("=== {APP_NAME} v{APP_VERSION} Start ===");
    info!("Configuration summary: {}", app_config.summary());

    // 创建环境管理器 - 使用当前目录方法
    let environment_manager = EnvironmentManager::for_current_directory()
        .map_err(|e| anyhow::anyhow!("无法创建环境管理器: {}", e))?;

    // 处理命令行子命令
    match cli.command {
        Some(Commands::Check) => {
            return handle_check_command(&environment_manager).await;
        }
        Some(Commands::Install) => {
            return handle_install_command(&environment_manager).await;
        }
        Some(Commands::Parse {
            input,
            output,
            parser,
        }) => {
            return handle_parse_command(&app_config, &environment_manager, input, output, parser)
                .await;
        }
        Some(Commands::UvInit) => {
            return handle_uv_init_command(&environment_manager).await;
        }
        Some(Commands::Troubleshoot) => {
            return handle_troubleshoot_command(&environment_manager).await;
        }
        Some(Commands::Server { daemon: _ }) | None => {
            // 继续执行服务器模式
        }
    }

    info!("Service listening address: {server_host}:{server_port}");

    // 初始化环境管理器并检查Python环境
    info!("Start checking and initializing the Python environment...");
    let environment_manager = EnvironmentManager::for_current_directory()
        .map_err(|e| anyhow::anyhow!("无法创建环境管理器: {}", e))?;

    // 自动激活虚拟环境（如果存在且未激活）
    info!("Check and activate virtual environment...");
    if let Err(e) = environment_manager
        .auto_activate_virtual_environment()
        .await
    {
        warn!("Automatic activation of virtual environment failed: {e}");
        info!("Please activate the virtual environment manually: source ./venv/bin/activate");
    } else {
        info!("The virtual environment has been automatically activated");
    }

    // 检查环境状态（非阻塞）
    let env_status = match environment_manager.check_environment().await {
        Ok(status) => status,
        Err(e) => {
            warn!(
                "The environment check failed and will be automatically installed in the background: {e}"
            );
            // 创建默认状态，表示需要安装
            EnvironmentStatus::default()
        }
    };

    // 启动后台环境安装任务（非阻塞）
    if !env_status.mineru_available || !env_status.markitdown_available {
        let env_manager = environment_manager.clone();
        tokio::spawn(async move {
            if !env_status.mineru_available {
                info!(
                    "MinerU dependencies are not installed, and automatic background installation starts..."
                );
            }
            if !env_status.markitdown_available {
                info!(
                    "The MarkItDown dependency is not installed, and automatic background installation starts..."
                );
            }

            match env_manager.setup_python_environment().await {
                Ok(_) => {
                    info!("The background Python environment installation is completed");
                }
                Err(e) => {
                    error!("Background Python environment installation failed: {e}");
                }
            }
        });
        info!(
            "The Python dependency installation task has been started (in the background) and the service will start normally."
        );
    } else {
        info!(
            "MinerU dependency has been installed, version: {:?}",
            env_status.mineru_version
        );
        info!("MarkItDown dependencies are installed");
        info!("Python environment check is completed and all dependencies are in place");
    }

    // 创建应用状态
    let state = AppState::new(app_config)
        .await
        .map_err(|e| anyhow::anyhow!("无法创建应用状态: {}", e))?;

    // 健康检查
    if let Err(e) = state.health_check().await {
        error!("Application health check failed: {e}");
        return Err(anyhow::anyhow!("应用健康检查失败: {}", e));
    }

    info!("Application status initialization successful");

    // 监听地址
    let addr = format!("{server_host}:{server_port}");
    let listener = TcpListener::bind(&addr).await?;

    // 构建 axum 路由
    let app = create_router(state.clone()).await?;
    info!("HTTP routing initialization successful");

    // 启动定时任务
    tokio::spawn(start_background_tasks(state.clone()));
    info!("Background task started");

    // 注册关闭处理函数
    tokio::spawn(async move {
        std::panic::set_hook(Box::new(move |panic_info| {
            warn!("The program panics, perform cleanup...");

            if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                error!("Panic reason: {s}");
            } else if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                error!("Panic reason: {s}");
            } else {
                error!("Panic Reason: Unknown");
            }

            if let Some(location) = panic_info.location() {
                error!("Panic Location: {}:{}", location.file(), location.line());
            }

            error!("Stack trace:");
            let backtrace = Backtrace::capture();
            error!("{backtrace:?}");
        }));
    });

    info!("The service started successfully and started listening for connections...");

    // 启动服务器
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Service is down");
    Ok(())
}

const AVAILABLE_LOCALES: &[&str] = &["en", "zh-CN", "zh-TW"];
const DEFAULT_LOCALE: &str = "en";

fn init_locale_from_env() {
    for env_key in ["DEFAULT_LOCALE", "LANG"] {
        let Ok(raw_locale) = std::env::var(env_key) else {
            continue;
        };

        let normalized = if env_key == "LANG" {
            parse_lang_env(&raw_locale)
        } else {
            normalize_locale(&raw_locale)
        };

        if AVAILABLE_LOCALES.contains(&normalized.as_str()) {
            rust_i18n::set_locale(&normalized);
            return;
        }
    }

    rust_i18n::set_locale(DEFAULT_LOCALE);
}

fn parse_lang_env(lang: &str) -> String {
    let lang = lang.split('.').next().unwrap_or(lang);
    let lang = lang.split('@').next().unwrap_or(lang);
    normalize_locale(lang)
}

fn normalize_locale(input: &str) -> String {
    let input = input.trim();
    let input = input.split('.').next().unwrap_or(input);
    let input = input.split('@').next().unwrap_or(input);

    match input.to_lowercase().as_str() {
        "en" | "en_us" | "en-us" | "en_gb" | "en-gb" => "en".to_string(),
        "zh-cn" | "zh_cn" | "zh-hans" | "zh" => "zh-CN".to_string(),
        "zh-tw" | "zh_tw" | "zh-hant" => "zh-TW".to_string(),
        _ => input.to_string(),
    }
}

/// 处理uv环境初始化命令
async fn handle_uv_init_command(_environment_manager: &EnvironmentManager) -> Result<()> {
    println!(
        "🚀 Start initializing the uv virtual environment and dependencies in the current directory..."
    );
    println!();

    // 检查当前目录
    let current_dir = env::current_dir().map_err(|e| anyhow::anyhow!("无法获取当前目录: {}", e))?;
    println!("📁 Current working directory: {}", current_dir.display());
    println!("📁 The virtual environment will be created in: ./venv/");
    println!();

    // 创建基于当前目录的环境管理器
    let local_env_manager = EnvironmentManager::for_current_directory()
        .map_err(|e| anyhow::anyhow!("无法创建环境管理器: {}", e))?;

    // 1. 验证当前目录设置（任务12的核心功能）
    println!("🔍 Verify current directory settings...");
    let _validation_result = match local_env_manager.check_current_directory_readiness().await {
        Ok(result) => {
            if result.is_valid {
                println!("✅ Directory verification passed");
                if !result.warnings.is_empty() {
                    println!("⚠️ Found {} warnings", result.warnings.len());
                    for warning in &result.warnings {
                        println!("      • {}", warning.message);
                    }
                }
            } else {
                println!(
                    "❌ Directory verification failed, {} problems found",
                    result.issues.len()
                );
                for issue in &result.issues {
                    println!(
                        "      • [{}] {}",
                        format!("{:?}", issue.severity).to_uppercase(),
                        issue.message
                    );
                }

                // 尝试自动修复可修复的问题
                let auto_fixable: Vec<_> = result
                    .issues
                    .iter()
                    .filter(|issue| issue.auto_fixable)
                    .collect();

                if !auto_fixable.is_empty() {
                    println!(
                        "🔧 Try to automatically fix {} problems...",
                        auto_fixable.len()
                    );

                    for cleanup_option in &result.cleanup_options {
                        if cleanup_option.risk_level == CleanupRisk::Low
                            || cleanup_option.risk_level == CleanupRisk::Medium
                        {
                            match local_env_manager
                                .execute_cleanup_option(cleanup_option.option_type.clone())
                                .await
                            {
                                Ok(message) => println!("      ✅ {message}"),
                                Err(e) => println!("❌ Cleanup failed: {e}"),
                            }
                        }
                    }
                } else {
                    println!("💡 Please solve the following problems manually:");
                    for recommendation in &result.recommendations {
                        println!("      • {recommendation}");
                    }
                    println!();
                    return Err(anyhow::anyhow!("目录验证失败，请解决上述问题后重试"));
                }
            }
            result
        }
        Err(e) => {
            println!("⚠️ Directory verification failed: {e}");
            println!("Proceed with the installation, but you may encounter problems...");
            // 创建一个默认的验证结果以继续执行
            DirectoryValidationResult {
                is_valid: false,
                current_directory: current_dir.clone(),
                venv_path: current_dir.join("venv"),
                issues: Vec::new(),
                warnings: Vec::new(),
                cleanup_options: Vec::new(),
                recommendations: Vec::new(),
            }
        }
    };
    println!();

    // 1. 检查当前环境状态
    println!("🔍 Check current environment status...");
    let env_status = match local_env_manager.check_environment().await {
        Ok(status) => {
            println!("Environmental check completed:");
            println!(
                "   Python:     {}",
                if status.python_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!(
                "uv tool: {}",
                if status.uv_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!(
                "Virtual environment: {}",
                if status.virtual_env_active {
                    "✅ Active"
                } else {
                    "❌ Inactive"
                }
            );
            println!(
                "   MinerU:     {}",
                if status.mineru_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!(
                "   MarkItDown: {}",
                if status.markitdown_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!();
            status
        }
        Err(e) => {
            println!("⚠️ Environment check failed: {e}");
            println!("Proceed with the installation...");
            println!();
            EnvironmentStatus::default()
        }
    };

    // 2. 检查是否需要安装
    let needs_setup = !env_status.uv_available
        || !env_status.virtual_env_active
        || !env_status.mineru_available
        || !env_status.markitdown_available;

    if !needs_setup {
        println!("✨ All dependencies are ready, no installation required!");
        print_success_message(&current_dir);
        return Ok(());
    }

    // 3. 显示安装计划
    println!("📋 Installation plan:");
    if !env_status.uv_available {
        println!("• Install uv tools");
    }
    if !env_status.virtual_env_active {
        println!("• Create a virtual environment (./venv/)");
    }
    if !env_status.mineru_available {
        println!("• Install MinerU dependencies");
    }
    if !env_status.markitdown_available {
        println!("• Install MarkItDown dependencies");
    }
    println!();

    // 4. 执行环境设置
    println!("⚙️ Start setting up the Python environment and dependencies...");
    println!("This may take a few minutes, please be patient...");
    println!();

    // 创建进度监控
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let env_manager_with_progress = local_env_manager.clone().with_progress_sender(progress_tx);

    // 启动进度显示任务
    let progress_task = tokio::spawn(async move {
        let mut last_package = String::new();
        let mut last_progress = 0.0;

        while let Some(progress) = progress_rx.recv().await {
            // 只在包或进度有显著变化时显示
            if progress.package != last_package || (progress.progress - last_progress).abs() > 10.0
            {
                let stage_icon = match progress.stage {
                    InstallStage::Preparing => "🔧",
                    InstallStage::Downloading => "⬇️",
                    InstallStage::Installing => "📦",
                    InstallStage::Configuring => "⚙️",
                    InstallStage::Verifying => "✅",
                    InstallStage::Completed => "🎉",
                    InstallStage::Failed(_) => "❌",
                    InstallStage::Retrying {
                        attempt,
                        max_attempts,
                    } => {
                        println!(
                            "🔄 Try again {}/{}: {}",
                            attempt, max_attempts, progress.message
                        );
                        continue;
                    }
                };

                let progress_bar = create_progress_bar(progress.progress);
                println!(
                    "   {} {} [{}] {:.0}% - {}",
                    stage_icon, progress.package, progress_bar, progress.progress, progress.message
                );

                last_package = progress.package.clone();
                last_progress = progress.progress;
            }
        }
    });

    // 预检查：诊断潜在的路径问题
    let path_issues = env_manager_with_progress.diagnose_venv_path_issues().await;
    if !path_issues.is_empty() {
        println!("⚠️ Potential routing issue detected:");
        for issue in &path_issues {
            println!("   • {issue}");
        }
        println!();

        // 尝试自动修复
        println!("🔧 Try to fix the problem automatically...");
        match env_manager_with_progress.auto_fix_venv_path_issues().await {
            Ok(fixed) => {
                if !fixed.is_empty() {
                    println!("✅ The following issues have been fixed:");
                    for fix in &fixed {
                        println!("   • {fix}");
                    }
                    println!();
                } else {
                    println!(
                        "Unable to be repaired automatically, please solve the above problem manually"
                    );
                    println!();

                    // 显示详细的恢复建议
                    let suggestions = env_manager_with_progress
                        .get_venv_recovery_suggestions()
                        .await;
                    for suggestion in suggestions {
                        println!("   {suggestion}");
                    }
                    println!();

                    return Err(anyhow::anyhow!("存在无法自动修复的路径问题"));
                }
            }
            Err(e) => {
                println!("❌ Automatic repair failed: {e}");
                println!();

                // 显示详细的恢复建议
                println!("💡 Manual repair suggestions:");
                for suggestion in e.get_path_recovery_suggestions() {
                    println!("   • {suggestion}");
                }
                println!();

                return Err(anyhow::anyhow!("路径问题修复失败: {}", e));
            }
        }
    }

    // 执行安装
    let install_result = env_manager_with_progress.setup_python_environment().await;

    // 停止进度显示
    drop(env_manager_with_progress);
    let _ = progress_task.await;

    match install_result {
        Ok(_) => {
            println!();
            println!("✅ Python environment setup completed!");
        }
        Err(e) => {
            println!();
            println!("❌ Python environment setting failed: {e}");
            println!();

            // 提供基于错误类型的详细建议
            println!("💡 Detailed troubleshooting suggestions:");
            match &e {
                AppError::VirtualEnvironmentPath(_)
                | AppError::Permission(_)
                | AppError::Path(_) => {
                    for suggestion in e.get_path_recovery_suggestions() {
                        println!("   • {suggestion}");
                    }
                }
                AppError::Environment(msg) if msg.contains("超时") => {
                    println!(
                        "• The network connection may be slow, please check the network status"
                    );
                    println!("• Try to use domestic mirror sources");
                    println!("• Increase the timeout and try again");
                }
                AppError::Environment(msg) if msg.contains("权限") => {
                    println!("• Run the command with administrator privileges");
                    println!("• Check directory permission settings");
                    if cfg!(unix) {
                        println!("• Run: chmod 755 .");
                        println!("• Run: chown $USER .");
                    }
                }
                _ => {
                    println!("• Check network connection");
                    println!("• Make sure there is enough disk space (at least 500MB)");
                    println!("• Check firewall settings");
                    println!("• Try rerunning the command");
                    println!("• Check if antivirus software is blocking the operation");
                }
            }

            // 提供诊断命令
            println!();
            println!("🔍 Diagnostic commands:");
            println!("• Check environment status: document-parser check");
            println!("• View detailed logs: Check the logs/ directory");

            return Err(anyhow::anyhow!("Python环境设置失败: {}", e));
        }
    }

    // 5. 验证安装结果
    println!();
    println!("🔍 Verify installation results...");
    match local_env_manager.check_environment().await {
        Ok(status) => {
            println!("Verification completed:");
            println!(
                "   Python:     {}",
                if status.python_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.python_version {
                println!("Version: {version}");
            }
            println!(
                "uv tool: {}",
                if status.uv_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.uv_version {
                println!("Version: {version}");
            }
            println!(
                "Virtual environment: {}",
                if status.virtual_env_active {
                    "✅ Active"
                } else {
                    "❌ Inactive"
                }
            );
            println!(
                "   MinerU:     {}",
                if status.mineru_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.mineru_version {
                println!("Version: {version}");
            }
            println!(
                "   MarkItDown: {}",
                if status.markitdown_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.markitdown_version {
                println!("Version: {version}");
            }
            println!();

            if status.is_ready() {
                print_success_message(&current_dir);
            } else {
                println!("⚠️ There may be problems with the installation of some dependencies");
                println!();
                let critical_issues = status.get_critical_issues();
                if !critical_issues.is_empty() {
                    println!("🔧 Problems that need to be solved:");
                    for issue in critical_issues {
                        println!("   • {}: {}", issue.component, issue.message);
                        println!("Suggestion: {}", issue.suggestion);
                    }
                }
                return Err(anyhow::anyhow!("环境初始化未完全成功"));
            }
        }
        Err(e) => {
            println!("❌ Verification failed: {e}");
            return Err(anyhow::anyhow!("环境验证失败: {}", e));
        }
    }

    Ok(())
}

/// 创建进度条字符串
fn create_progress_bar(progress: f32) -> String {
    let width = 20;
    let filled = ((progress / 100.0) * width as f32) as usize;
    let empty = width - filled;

    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

/// 打印成功消息和下一步指引
fn print_success_message(_current_dir: &std::path::Path) {
    println!("🎉 The uv environment initialization is completed!");
    println!();
    println!("✨ All dependencies are in place, now you can start the server");
    println!();

    // 提供激活虚拟环境的指令
    println!("📋 Virtual environment activation instructions:");

    // 检测当前shell类型并提供相应的激活命令
    if let Ok(shell) = std::env::var("SHELL") {
        if shell.contains("fish") {
            println!("   source ./venv/bin/activate.fish");
        } else if shell.contains("zsh") || shell.contains("bash") {
            println!("   source ./venv/bin/activate");
        } else {
            println!("   source ./venv/bin/activate");
        }
    } else if cfg!(windows) {
        println!("   .\\venv\\Scripts\\activate");
    } else {
        println!("   source ./venv/bin/activate");
    }

    println!();
    println!("🚀 Start the server:");
    println!("   document-parser server");
    println!();
    println!("🔧 Or use uv to run the command directly:");
    println!("   uv run mineru -h");
    println!("   uv run python -m markitdown --help");
    println!();
    println!("📚 More help:");
    println!("   document-parser --help");
    println!("document-parser check # Check environment status");
    println!("document-parser troubleshoot # Troubleshooting guide");
    println!();
    println!("💡 Tips:");
    println!("• Virtual environment location: ./venv/");
    println!(
        "• Python executable file: ./venv/bin/python (Linux/macOS) or .\\\\venv\\\\Scripts\\\\python.exe (Windows)"
    );
    println!(
        "• If you encounter problems, run 'document-parser troubleshoot' for detailed guidance"
    );
}

/// 创建路由
async fn create_router(state: AppState) -> Result<axum::Router, AppError> {
    let app = create_routes(state);
    Ok(app)
}

/// 启动后台任务
async fn start_background_tasks(state: AppState) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // 每小时执行一次

    loop {
        interval.tick().await;

        // 清理过期数据
        if let Err(e) = state.cleanup_expired_data().await {
            error!("Failed to clear expired data: {e}");
        } else {
            info!("Background cleanup task execution completed");
        }
    }
}

/// 关闭信号处理
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("无法监听 Ctrl+C 信号");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("无法监听 terminate 信号")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Receive Ctrl+C signal and start graceful shutdown...");
        }
        _ = terminate => {
            info!("Receive terminate signal and start graceful shutdown...");
        }
    }

    info!("Closing service...");
}

/// 处理故障排除命令
async fn handle_troubleshoot_command(environment_manager: &EnvironmentManager) -> Result<()> {
    println!("🔧 Document Parser Troubleshooting Guide");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // 显示当前环境概览
    println!("📊 Current environment overview:");
    let current_dir = env::current_dir().map_err(|e| anyhow::anyhow!("无法获取当前目录: {}", e))?;
    println!("Working directory: {}", current_dir.display());
    println!("Virtual environment: ./venv/");
    println!(
        "Operating system: {}",
        if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        }
    );
    println!();

    // 1. 虚拟环境问题
    println!("🏠 1. Virtual environment issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: Virtual environment creation failed");
    println!("🔍 Diagnosis steps:");
    println!("1. Check the current directory permissions: ls -la (Linux/macOS) or dir (Windows)");
    println!("2. Check disk space: df -h (Linux/macOS) or dir (Windows)");
    println!("3. Check whether a file with the same name exists: ls -la venv");
    println!();
    println!("💡 Solution:");
    println!("• Make sure the current directory has write permissions");
    if cfg!(unix) {
        println!("• Modify permissions: chmod 755.");
        println!("• Change owner: chown $USER .");
    } else if cfg!(windows) {
        println!("• Run command prompt as administrator");
        println!("• Check User Account Control (UAC) settings");
    }
    println!(
        "• Delete existing venv files: rm -rf ./venv (Linux/macOS) or rmdir /s .\\\\venv (Windows)"
    );
    println!("• Make sure there is at least 500MB of free disk space");
    println!();

    println!("❓ Problem: Virtual environment activation failed");
    println!("🔍 Diagnosis steps:");
    println!(
        "1. Check whether the virtual environment exists: ls ./venv/bin/ (Linux/macOS) or dir .\\\\venv\\\\Scripts\\\\ (Windows)"
    );
    println!("2. Check activation script permissions");
    println!();
    println!("💡 Solution:");
    if cfg!(windows) {
        println!("   • Windows: .\\venv\\Scripts\\activate");
        println!("   • PowerShell: .\\venv\\Scripts\\Activate.ps1");
        println!(
            "• If PowerShell enforcement policy restrictions, run: Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser"
        );
    } else {
        println!("   • Bash/Zsh: source ./venv/bin/activate");
        println!("   • Fish: source ./venv/bin/activate.fish");
        println!("• Check script permissions: chmod +x ./venv/bin/activate");
    }
    println!();

    // 2. 依赖安装问题
    println!("📦 2. Dependency installation issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: UV tool is not installed or unavailable");
    println!("💡 Solution:");
    println!(
        "• Use the official installation script: curl -LsSf https://astral.sh/uv/install.sh | sh"
    );
    println!("• Or install using pip: pip install uv");
    println!("• Or use a package manager:");
    if cfg!(target_os = "macos") {
        println!("     - macOS: brew install uv");
    } else if cfg!(unix) {
        println!("- Ubuntu/Debian: See https://docs.astral.sh/uv/getting-started/installation/");
    } else if cfg!(windows) {
        println!("     - Windows: winget install astral-sh.uv");
    }
    println!("• Restart the terminal and try again");
    println!();

    println!("❓ Problem: MinerU or MarkItDown installation failed");
    println!("🔍 Diagnosis steps:");
    println!("1. Check network connection: ping pypi.org");
    println!("2. Check Python version: python --version (requires 3.8+)");
    println!("3. Check pip in the virtual environment: ./venv/bin/pip --version");
    println!();
    println!("💡 Solution:");
    println!("• Use domestic mirror sources:");
    println!("     uv pip install -i https://pypi.tuna.tsinghua.edu.cn/simple/ mineru[core]");
    println!("• Increase timeout: uv pip install --timeout 300 mineru[core]");
    println!("• Step-by-step installation:");
    println!("     1. uv pip install --upgrade pip");
    println!("     2. uv pip install mineru[core]");
    println!("     3. uv pip install markitdown");
    println!("• Clean the cache and try again: uv cache clean");
    println!();

    // 3. 网络和下载问题
    println!("🌐 3. Network and download issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: Network connection timed out or download failed");
    println!("💡 Solution:");
    println!("• Check network connections and firewall settings");
    println!("• Using a proxy (if required):");
    println!("     export HTTP_PROXY=http://proxy:port");
    println!("     export HTTPS_PROXY=http://proxy:port");
    println!("• Use domestic mirror sources:");
    println!("- Tsinghua source: https://pypi.tuna.tsinghua.edu.cn/simple/");
    println!("- Ali source: https://mirrors.aliyun.com/pypi/simple/");
    println!("• Retry installation: document-parser uv-init");
    println!();

    // 4. 系统环境问题
    println!("⚙️ 4. System environment issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: Python version is incompatible");
    println!("🔍 Check command: python --version or python3 --version");
    println!("💡 Solution:");
    println!("• Requires Python 3.8 or higher");
    if cfg!(target_os = "macos") {
        println!("• macOS installation: brew install python@3.11");
    } else if cfg!(unix) {
        println!("   • Ubuntu/Debian: sudo apt update && sudo apt install python3.11");
        println!("   • CentOS/RHEL: sudo yum install python311");
    } else if cfg!(windows) {
        println!("• Windows: Download and install from https://python.org");
    }
    println!();

    println!("❓ Question: CUDA environment configuration (optional, for GPU acceleration)");
    println!("🔍 Check command: nvidia-smi");
    println!("💡 Solution:");
    println!("• Install NVIDIA driver");
    println!("• Install CUDA Toolkit (11.8 or 12.x recommended)");
    println!("• Verify installation: nvidia-smi and nvcc --version");
    println!("• Note: CPU mode also works normally, GPU is only used for acceleration");
    println!();

    // 5. 常用诊断命令
    println!("🔍 5. Common diagnostic commands");
    println!("───────────────────────────────────────────────────────────────");
    println!();
    println!("Environmental inspection:");
    println!("document-parser check # Complete environment check");
    println!("document-parser uv-init # Reinitialize the environment");
    println!();
    println!("Manual verification:");
    println!("uv --version # Check UV version");
    println!("./venv/bin/python --version # Check virtual environment Python (Linux/macOS)");
    println!(
        ".\\\\venv\\\\Scripts\\\\python --version # Check the virtual environment Python (Windows)"
    );
    println!("./venv/bin/mineru --help # Check MinerU (Linux/macOS)");
    println!(".\\\\venv\\\\Scripts\\\\mineru --help # Check MinerU (Windows)");
    println!();
    println!("Log view:");
    println!("tail -f logs/log.$(date +%Y-%m-%d) # View today’s logs (Linux/macOS)");
    println!("type logs\\\\log.%date:~0,10% # View today’s log (Windows)");
    println!();

    // 6. 获取帮助
    println!("🆘 6. Get more help");
    println!("───────────────────────────────────────────────────────────────");
    println!();
    println!("If none of the above resolves the issue, please:");
    println!("1. Run detailed diagnostics: document-parser check");
    println!("2. Collect error information:");
    println!("• Complete error message");
    println!("• Operating system version");
    println!("• Python version");
    println!("• Current working directory");
    println!("3. View log files: logs/ directory");
    println!("4. Try reinitializing in a new directory");
    println!();

    // 执行实时诊断
    println!("🔬 Real-time environment diagnosis");
    println!("───────────────────────────────────────────────────────────────");
    match environment_manager.check_environment().await {
        Ok(status) => {
            if status.is_ready() {
                println!(
                    "✅ The environment is in good condition and all dependencies are in place"
                );
            } else {
                println!("⚠️ Found the following issues:");
                let issues = status.get_critical_issues();
                for issue in issues {
                    println!("   • {}: {}", issue.component, issue.message);
                    println!("Suggestion: {}", issue.suggestion);
                }
            }
        }
        Err(e) => {
            println!("❌ Environment check failed: {e}");
            println!("Please follow the above guide to troubleshoot");
        }
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("💡 Tip: Most problems can be solved by re-running 'document-parser uv-init'");

    Ok(())
}

/// 处理环境检查命令
async fn handle_check_command(environment_manager: &EnvironmentManager) -> Result<()> {
    info!("Check Python environment status...");

    // 首先进行路径诊断
    println!("🔍 Diagnose virtual environment path...");
    let path_issues = environment_manager.diagnose_venv_path_issues().await;
    if !path_issues.is_empty() {
        println!("⚠️ Found path related issues:");
        for issue in &path_issues {
            println!("   • {issue}");
        }
        println!();

        println!("💡 Suggestions for solving path problems:");
        let suggestions = environment_manager.get_venv_recovery_suggestions().await;
        for suggestion in suggestions {
            println!("   {suggestion}");
        }
        println!();
    } else {
        println!("✅Virtual environment path check passed");
        println!();
    }

    match environment_manager.get_detailed_status_report().await {
        Ok(detailed_report) => {
            // 输出详细的诊断报告
            println!("{detailed_report}");

            // 输出增强的依赖验证报告
            println!("🔬 Perform enhanced dependency verification...");
            match environment_manager.get_enhanced_dependency_report().await {
                Ok(enhanced_report) => {
                    println!("{enhanced_report}");
                }
                Err(e) => {
                    println!("⚠️ Enhanced dependency verification failed: {e}");
                }
            }

            // 检查环境状态以确定退出码
            match environment_manager.check_environment().await {
                Ok(status) => {
                    if status.is_ready() {
                        println!(
                            "✅ Environmental inspection passed! All dependencies are in place."
                        );
                        Ok(())
                    } else {
                        let critical_issues = status.get_critical_issues();
                        if !critical_issues.is_empty() {
                            println!(
                                "❌ Found {} key issues that need to be resolved",
                                critical_issues.len()
                            );
                            for issue in critical_issues {
                                println!("  • {}: {}", issue.component, issue.message);
                                println!("Suggestion: {}", issue.suggestion);
                            }
                        }

                        let auto_fixable = status.get_auto_fixable_issues();
                        if !auto_fixable.is_empty() {
                            println!(
                                "💡 {} problems can be fixed automatically, run 'document-parser uv-init' to fix them",
                                auto_fixable.len()
                            );
                        }

                        // 如果有路径问题，提供额外的建议
                        if !path_issues.is_empty() {
                            println!();
                            println!("🔧 Path problem fix:");
                            println!(
                                "• Running 'document-parser uv-init' will try to fix path issues automatically"
                            );
                            println!(
                                "• Or manually solve the path problem by following the suggestions above"
                            );
                        }

                        Err(anyhow::anyhow!(
                            "环境未就绪，健康评分: {}/100",
                            status.health_score()
                        ))
                    }
                }
                Err(e) => {
                    println!("❌ Environment status check failed: {e}");

                    // 如果是路径相关错误，提供详细建议
                    match &e {
                        AppError::VirtualEnvironmentPath(_)
                        | AppError::Permission(_)
                        | AppError::Path(_) => {
                            println!();
                            println!("💡 Suggestions for solving path errors:");
                            for suggestion in e.get_path_recovery_suggestions() {
                                println!("   • {suggestion}");
                            }
                        }
                        _ => {}
                    }

                    Err(anyhow::anyhow!("环境状态检查失败: {}", e))
                }
            }
        }
        Err(e) => {
            println!("❌ Environment check failed: {e}");

            // 如果是路径相关错误，提供详细建议
            match &e {
                AppError::VirtualEnvironmentPath(_)
                | AppError::Permission(_)
                | AppError::Path(_) => {
                    println!();
                    println!("💡 Suggestions for solving path errors:");
                    for suggestion in e.get_path_recovery_suggestions() {
                        println!("   • {suggestion}");
                    }
                }
                _ => {}
            }

            Err(anyhow::anyhow!("环境检查失败: {}", e))
        }
    }
}

/// 处理依赖安装命令
async fn handle_install_command(environment_manager: &EnvironmentManager) -> Result<()> {
    info!("Start installing Python dependencies...");

    match environment_manager.setup_python_environment().await {
        Ok(_) => {
            info!("Python dependency installation is complete!");

            // 验证安装结果
            match environment_manager.check_environment().await {
                Ok(status) => {
                    if status.mineru_available && status.markitdown_available {
                        info!(
                            "The installation verification was successful and all dependencies are in place!"
                        );
                    } else {
                        warn!(
                            "The installation is completed but verification fails. Some dependencies may not be installed correctly."
                        );
                    }
                }
                Err(e) => {
                    warn!("Installation completed but verification failed: {e}");
                }
            }
        }
        Err(e) => {
            error!("Python dependency installation failed: {e}");
            return Err(anyhow::anyhow!("Python依赖安装失败: {}", e));
        }
    }

    Ok(())
}

/// 处理文件解析命令
async fn handle_parse_command(
    app_config: &AppConfig,
    environment_manager: &EnvironmentManager,
    input: PathBuf,
    output: Option<PathBuf>,
    parser: String,
) -> Result<()> {
    info!("Start parsing file: {input:?}");
    info!("Use parser: {parser}");

    // 检查输入文件是否存在
    if !input.exists() {
        return Err(anyhow::anyhow!("输入文件不存在: {:?}", input));
    }

    // 检查环境
    let env_status = environment_manager
        .check_environment()
        .await
        .map_err(|e| anyhow::anyhow!("环境检查失败: {}", e))?;

    match parser.as_str() {
        "mineru" => {
            if !env_status.mineru_available {
                return Err(anyhow::anyhow!(
                    "MinerU未安装，请先运行 'document-parser install'"
                ));
            }
        }
        "markitdown" => {
            if !env_status.markitdown_available {
                return Err(anyhow::anyhow!(
                    "MarkItDown未安装，请先运行 'document-parser install'"
                ));
            }
        }
        _ => {
            return Err(anyhow::anyhow!(
                "不支持的解析器: {}，支持的解析器: mineru, markitdown",
                parser
            ));
        }
    }

    // 创建应用状态（用于解析器）
    let _state = AppState::new(app_config.clone())
        .await
        .map_err(|e| anyhow::anyhow!("无法创建应用状态: {}", e))?;

    // TODO: 实现实际的文件解析逻辑
    // 这里需要调用相应的解析器服务
    info!("The file parsing function is under development...");

    // 确定输出路径
    let output_path = output.unwrap_or_else(|| {
        let mut path = input.clone();
        path.set_extension("md");
        path
    });

    info!("The analysis is completed and the results will be saved to: {output_path:?}");

    Ok(())
}
