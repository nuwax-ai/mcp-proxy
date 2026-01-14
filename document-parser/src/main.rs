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
    info!("检查CUDA环境状态...");
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
                    "CUDA环境可用: version={:?}, devices={}, recommended={}",
                    status.version.as_deref().unwrap_or("unknown"),
                    status.device_count,
                    status.recommended_device.as_deref().unwrap_or("cuda")
                );
            } else {
                info!("CUDA环境不可用，将使用CPU模式");
            }

            status
        }
        Err(e) => {
            warn!("CUDA环境检查失败: {e}, 将使用CPU模式");
            CudaStatus::default()
        }
    };

    // 初始化全局CUDA状态
    if let Err(e) = init_global_cuda_status(cuda_status) {
        warn!("初始化全局CUDA状态失败: {e}");
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

    info!("=== {APP_NAME} v{APP_VERSION} 启动 ===");
    info!("配置摘要: {}", app_config.summary());

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

    info!("服务监听地址: {server_host}:{server_port}");

    // 初始化环境管理器并检查Python环境
    info!("开始检查和初始化Python环境...");
    let environment_manager = EnvironmentManager::for_current_directory()
        .map_err(|e| anyhow::anyhow!("无法创建环境管理器: {}", e))?;

    // 自动激活虚拟环境（如果存在且未激活）
    info!("检查并激活虚拟环境...");
    if let Err(e) = environment_manager
        .auto_activate_virtual_environment()
        .await
    {
        warn!("虚拟环境自动激活失败: {e}");
        info!("请手动激活虚拟环境: source ./venv/bin/activate");
    } else {
        info!("虚拟环境已自动激活");
    }

    // 检查环境状态（非阻塞）
    let env_status = match environment_manager.check_environment().await {
        Ok(status) => status,
        Err(e) => {
            warn!("环境检查失败，将在后台自动安装: {e}");
            // 创建默认状态，表示需要安装
            EnvironmentStatus::default()
        }
    };

    // 启动后台环境安装任务（非阻塞）
    if !env_status.mineru_available || !env_status.markitdown_available {
        let env_manager = environment_manager.clone();
        tokio::spawn(async move {
            if !env_status.mineru_available {
                info!("MinerU依赖未安装，开始后台自动安装...");
            }
            if !env_status.markitdown_available {
                info!("MarkItDown依赖未安装，开始后台自动安装...");
            }

            match env_manager.setup_python_environment().await {
                Ok(_) => {
                    info!("后台Python环境安装完成");
                }
                Err(e) => {
                    error!("后台Python环境安装失败: {e}");
                }
            }
        });
        info!("Python依赖安装任务已启动（后台进行），服务将正常启动");
    } else {
        info!("MinerU依赖已安装，版本: {:?}", env_status.mineru_version);
        info!("MarkItDown依赖已安装");
        info!("Python环境检查完成，所有依赖已就绪");
    }

    // 创建应用状态
    let state = AppState::new(app_config)
        .await
        .map_err(|e| anyhow::anyhow!("无法创建应用状态: {}", e))?;

    // 健康检查
    if let Err(e) = state.health_check().await {
        error!("应用健康检查失败: {e}");
        return Err(anyhow::anyhow!("应用健康检查失败: {}", e));
    }

    info!("应用状态初始化成功");

    // 监听地址
    let addr = format!("{server_host}:{server_port}");
    let listener = TcpListener::bind(&addr).await?;

    // 构建 axum 路由
    let app = create_router(state.clone()).await?;
    info!("HTTP路由初始化成功");

    // 启动定时任务
    tokio::spawn(start_background_tasks(state.clone()));
    info!("后台任务已启动");

    // 注册关闭处理函数
    tokio::spawn(async move {
        std::panic::set_hook(Box::new(move |panic_info| {
            warn!("程序发生panic，执行清理...");

            if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                error!("Panic 原因: {s}");
            } else if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                error!("Panic 原因: {s}");
            } else {
                error!("Panic 原因: 未知");
            }

            if let Some(location) = panic_info.location() {
                error!("Panic 位置: {}:{}", location.file(), location.line());
            }

            error!("堆栈跟踪:");
            let backtrace = Backtrace::capture();
            error!("{backtrace:?}");
        }));
    });

    info!("服务启动成功，开始监听连接...");

    // 启动服务器
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("服务已关闭");
    Ok(())
}

/// 处理uv环境初始化命令
async fn handle_uv_init_command(_environment_manager: &EnvironmentManager) -> Result<()> {
    println!("🚀 开始在当前目录初始化uv虚拟环境和依赖...");
    println!();

    // 检查当前目录
    let current_dir = env::current_dir().map_err(|e| anyhow::anyhow!("无法获取当前目录: {}", e))?;
    println!("📁 当前工作目录: {}", current_dir.display());
    println!("📁 虚拟环境将创建在: ./venv/");
    println!();

    // 创建基于当前目录的环境管理器
    let local_env_manager = EnvironmentManager::for_current_directory()
        .map_err(|e| anyhow::anyhow!("无法创建环境管理器: {}", e))?;

    // 1. 验证当前目录设置（任务12的核心功能）
    println!("🔍 验证当前目录设置...");
    let _validation_result = match local_env_manager.check_current_directory_readiness().await {
        Ok(result) => {
            if result.is_valid {
                println!("   ✅ 目录验证通过");
                if !result.warnings.is_empty() {
                    println!("   ⚠️  发现 {} 个警告", result.warnings.len());
                    for warning in &result.warnings {
                        println!("      • {}", warning.message);
                    }
                }
            } else {
                println!("   ❌ 目录验证失败，发现 {} 个问题", result.issues.len());
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
                    println!("   🔧 尝试自动修复 {} 个问题...", auto_fixable.len());

                    for cleanup_option in &result.cleanup_options {
                        if cleanup_option.risk_level == CleanupRisk::Low
                            || cleanup_option.risk_level == CleanupRisk::Medium
                        {
                            match local_env_manager
                                .execute_cleanup_option(cleanup_option.option_type.clone())
                                .await
                            {
                                Ok(message) => println!("      ✅ {message}"),
                                Err(e) => println!("      ❌ 清理失败: {e}"),
                            }
                        }
                    }
                } else {
                    println!("   💡 请手动解决以下问题:");
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
            println!("   ⚠️  目录验证失败: {e}");
            println!("   继续进行安装，但可能遇到问题...");
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
    println!("🔍 检查当前环境状态...");
    let env_status = match local_env_manager.check_environment().await {
        Ok(status) => {
            println!("   环境检查完成:");
            println!(
                "   Python:     {}",
                if status.python_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            println!(
                "   uv工具:     {}",
                if status.uv_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            println!(
                "   虚拟环境:   {}",
                if status.virtual_env_active {
                    "✅ 激活"
                } else {
                    "❌ 未激活"
                }
            );
            println!(
                "   MinerU:     {}",
                if status.mineru_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            println!(
                "   MarkItDown: {}",
                if status.markitdown_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            println!();
            status
        }
        Err(e) => {
            println!("   ⚠️  环境检查失败: {e}");
            println!("   继续进行安装...");
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
        println!("✨ 所有依赖都已就绪，无需安装！");
        print_success_message(&current_dir);
        return Ok(());
    }

    // 3. 显示安装计划
    println!("📋 安装计划:");
    if !env_status.uv_available {
        println!("   • 安装 uv 工具");
    }
    if !env_status.virtual_env_active {
        println!("   • 创建虚拟环境 (./venv/)");
    }
    if !env_status.mineru_available {
        println!("   • 安装 MinerU 依赖");
    }
    if !env_status.markitdown_available {
        println!("   • 安装 MarkItDown 依赖");
    }
    println!();

    // 4. 执行环境设置
    println!("⚙️  开始设置Python环境和依赖...");
    println!("   这可能需要几分钟时间，请耐心等待...");
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
                            "   🔄 重试 {}/{}: {}",
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
        println!("⚠️  检测到潜在的路径问题:");
        for issue in &path_issues {
            println!("   • {issue}");
        }
        println!();

        // 尝试自动修复
        println!("🔧 尝试自动修复问题...");
        match env_manager_with_progress.auto_fix_venv_path_issues().await {
            Ok(fixed) => {
                if !fixed.is_empty() {
                    println!("✅ 已修复以下问题:");
                    for fix in &fixed {
                        println!("   • {fix}");
                    }
                    println!();
                } else {
                    println!("   无法自动修复，请手动解决上述问题");
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
                println!("❌ 自动修复失败: {e}");
                println!();

                // 显示详细的恢复建议
                println!("💡 手动修复建议:");
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
            println!("✅ Python环境设置完成！");
        }
        Err(e) => {
            println!();
            println!("❌ Python环境设置失败: {e}");
            println!();

            // 提供基于错误类型的详细建议
            println!("💡 详细故障排除建议:");
            match &e {
                AppError::VirtualEnvironmentPath(_)
                | AppError::Permission(_)
                | AppError::Path(_) => {
                    for suggestion in e.get_path_recovery_suggestions() {
                        println!("   • {suggestion}");
                    }
                }
                AppError::Environment(msg) if msg.contains("超时") => {
                    println!("   • 网络连接可能较慢，请检查网络状态");
                    println!("   • 尝试使用国内镜像源");
                    println!("   • 增加超时时间后重试");
                }
                AppError::Environment(msg) if msg.contains("权限") => {
                    println!("   • 使用管理员权限运行命令");
                    println!("   • 检查目录权限设置");
                    if cfg!(unix) {
                        println!("   • 运行: chmod 755 .");
                        println!("   • 运行: chown $USER .");
                    }
                }
                _ => {
                    println!("   • 检查网络连接");
                    println!("   • 确保有足够的磁盘空间 (至少500MB)");
                    println!("   • 检查防火墙设置");
                    println!("   • 尝试重新运行命令");
                    println!("   • 检查防病毒软件是否阻止操作");
                }
            }

            // 提供诊断命令
            println!();
            println!("🔍 诊断命令:");
            println!("   • 检查环境状态: document-parser check");
            println!("   • 查看详细日志: 检查 logs/ 目录");

            return Err(anyhow::anyhow!("Python环境设置失败: {}", e));
        }
    }

    // 5. 验证安装结果
    println!();
    println!("🔍 验证安装结果...");
    match local_env_manager.check_environment().await {
        Ok(status) => {
            println!("   验证完成:");
            println!(
                "   Python:     {}",
                if status.python_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            if let Some(ref version) = status.python_version {
                println!("               版本: {version}");
            }
            println!(
                "   uv工具:     {}",
                if status.uv_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            if let Some(ref version) = status.uv_version {
                println!("               版本: {version}");
            }
            println!(
                "   虚拟环境:   {}",
                if status.virtual_env_active {
                    "✅ 激活"
                } else {
                    "❌ 未激活"
                }
            );
            println!(
                "   MinerU:     {}",
                if status.mineru_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            if let Some(ref version) = status.mineru_version {
                println!("               版本: {version}");
            }
            println!(
                "   MarkItDown: {}",
                if status.markitdown_available {
                    "✅ 可用"
                } else {
                    "❌ 不可用"
                }
            );
            if let Some(ref version) = status.markitdown_version {
                println!("               版本: {version}");
            }
            println!();

            if status.is_ready() {
                print_success_message(&current_dir);
            } else {
                println!("⚠️  部分依赖安装可能存在问题");
                println!();
                let critical_issues = status.get_critical_issues();
                if !critical_issues.is_empty() {
                    println!("🔧 需要解决的问题:");
                    for issue in critical_issues {
                        println!("   • {}: {}", issue.component, issue.message);
                        println!("     建议: {}", issue.suggestion);
                    }
                }
                return Err(anyhow::anyhow!("环境初始化未完全成功"));
            }
        }
        Err(e) => {
            println!("❌ 验证失败: {e}");
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
fn print_success_message(current_dir: &std::path::Path) {
    println!("🎉 uv环境初始化完成！");
    println!();
    println!("✨ 所有依赖都已就绪，现在可以启动服务器了");
    println!();

    // 提供激活虚拟环境的指令
    println!("📋 虚拟环境激活指令:");

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
    println!("🚀 启动服务器:");
    println!("   document-parser server");
    println!();
    println!("🔧 或者使用 uv 直接运行命令:");
    println!("   uv run mineru -h");
    println!("   uv run python -m markitdown --help");
    println!();
    println!("📚 更多帮助:");
    println!("   document-parser --help");
    println!("   document-parser check         # 检查环境状态");
    println!("   document-parser troubleshoot  # 故障排除指南");
    println!();
    println!("💡 提示:");
    println!("   • 虚拟环境位置: ./venv/");
    println!(
        "   • Python可执行文件: ./venv/bin/python (Linux/macOS) 或 .\\venv\\Scripts\\python.exe (Windows)"
    );
    println!("   • 如遇问题，请运行 'document-parser troubleshoot' 查看详细指南");
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
            error!("清理过期数据失败: {e}");
        } else {
            info!("后台清理任务执行完成");
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
            info!("收到 Ctrl+C 信号，开始优雅关闭...");
        }
        _ = terminate => {
            info!("收到 terminate 信号，开始优雅关闭...");
        }
    }

    info!("正在关闭服务...");
}

/// 处理故障排除命令
async fn handle_troubleshoot_command(environment_manager: &EnvironmentManager) -> Result<()> {
    println!("🔧 Document Parser 故障排除指南");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // 显示当前环境概览
    println!("📊 当前环境概览:");
    let current_dir = env::current_dir().map_err(|e| anyhow::anyhow!("无法获取当前目录: {}", e))?;
    println!("   工作目录: {}", current_dir.display());
    println!("   虚拟环境: ./venv/");
    println!(
        "   操作系统: {}",
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
    println!("🏠 1. 虚拟环境问题");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ 问题: 虚拟环境创建失败");
    println!("🔍 诊断步骤:");
    println!("   1. 检查当前目录权限: ls -la (Linux/macOS) 或 dir (Windows)");
    println!("   2. 检查磁盘空间: df -h (Linux/macOS) 或 dir (Windows)");
    println!("   3. 检查是否存在同名文件: ls -la venv");
    println!();
    println!("💡 解决方案:");
    println!("   • 确保当前目录有写入权限");
    if cfg!(unix) {
        println!("   • 修改权限: chmod 755 .");
        println!("   • 修改所有者: chown $USER .");
    } else if cfg!(windows) {
        println!("   • 以管理员身份运行命令提示符");
        println!("   • 检查用户账户控制(UAC)设置");
    }
    println!("   • 删除现有的venv文件: rm -rf ./venv (Linux/macOS) 或 rmdir /s .\\venv (Windows)");
    println!("   • 确保至少有500MB可用磁盘空间");
    println!();

    println!("❓ 问题: 虚拟环境激活失败");
    println!("🔍 诊断步骤:");
    println!(
        "   1. 检查虚拟环境是否存在: ls ./venv/bin/ (Linux/macOS) 或 dir .\\venv\\Scripts\\ (Windows)"
    );
    println!("   2. 检查激活脚本权限");
    println!();
    println!("💡 解决方案:");
    if cfg!(windows) {
        println!("   • Windows: .\\venv\\Scripts\\activate");
        println!("   • PowerShell: .\\venv\\Scripts\\Activate.ps1");
        println!(
            "   • 如果PowerShell执行策略限制，运行: Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser"
        );
    } else {
        println!("   • Bash/Zsh: source ./venv/bin/activate");
        println!("   • Fish: source ./venv/bin/activate.fish");
        println!("   • 检查脚本权限: chmod +x ./venv/bin/activate");
    }
    println!();

    // 2. 依赖安装问题
    println!("📦 2. 依赖安装问题");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ 问题: UV工具未安装或不可用");
    println!("💡 解决方案:");
    println!("   • 使用官方安装脚本: curl -LsSf https://astral.sh/uv/install.sh | sh");
    println!("   • 或使用pip安装: pip install uv");
    println!("   • 或使用包管理器:");
    if cfg!(target_os = "macos") {
        println!("     - macOS: brew install uv");
    } else if cfg!(unix) {
        println!(
            "     - Ubuntu/Debian: 参考 https://docs.astral.sh/uv/getting-started/installation/"
        );
    } else if cfg!(windows) {
        println!("     - Windows: winget install astral-sh.uv");
    }
    println!("   • 重启终端后重试");
    println!();

    println!("❓ 问题: MinerU或MarkItDown安装失败");
    println!("🔍 诊断步骤:");
    println!("   1. 检查网络连接: ping pypi.org");
    println!("   2. 检查Python版本: python --version (需要3.8+)");
    println!("   3. 检查虚拟环境中的pip: ./venv/bin/pip --version");
    println!();
    println!("💡 解决方案:");
    println!("   • 使用国内镜像源:");
    println!("     uv pip install -i https://pypi.tuna.tsinghua.edu.cn/simple/ mineru[core]");
    println!("   • 增加超时时间: uv pip install --timeout 300 mineru[core]");
    println!("   • 分步安装:");
    println!("     1. uv pip install --upgrade pip");
    println!("     2. uv pip install mineru[core]");
    println!("     3. uv pip install markitdown");
    println!("   • 清理缓存后重试: uv cache clean");
    println!();

    // 3. 网络和下载问题
    println!("🌐 3. 网络和下载问题");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ 问题: 网络连接超时或下载失败");
    println!("💡 解决方案:");
    println!("   • 检查网络连接和防火墙设置");
    println!("   • 使用代理 (如果需要):");
    println!("     export HTTP_PROXY=http://proxy:port");
    println!("     export HTTPS_PROXY=http://proxy:port");
    println!("   • 使用国内镜像源:");
    println!("     - 清华源: https://pypi.tuna.tsinghua.edu.cn/simple/");
    println!("     - 阿里源: https://mirrors.aliyun.com/pypi/simple/");
    println!("   • 重试安装: document-parser uv-init");
    println!();

    // 4. 系统环境问题
    println!("⚙️  4. 系统环境问题");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ 问题: Python版本不兼容");
    println!("🔍 检查命令: python --version 或 python3 --version");
    println!("💡 解决方案:");
    println!("   • 需要Python 3.8或更高版本");
    if cfg!(target_os = "macos") {
        println!("   • macOS安装: brew install python@3.11");
    } else if cfg!(unix) {
        println!("   • Ubuntu/Debian: sudo apt update && sudo apt install python3.11");
        println!("   • CentOS/RHEL: sudo yum install python311");
    } else if cfg!(windows) {
        println!("   • Windows: 从 https://python.org 下载安装");
    }
    println!();

    println!("❓ 问题: CUDA环境配置 (可选，用于GPU加速)");
    println!("🔍 检查命令: nvidia-smi");
    println!("💡 解决方案:");
    println!("   • 安装NVIDIA驱动程序");
    println!("   • 安装CUDA Toolkit (推荐11.8或12.x)");
    println!("   • 验证安装: nvidia-smi 和 nvcc --version");
    println!("   • 注意: CPU模式也可正常工作，GPU仅用于加速");
    println!();

    // 5. 常用诊断命令
    println!("🔍 5. 常用诊断命令");
    println!("───────────────────────────────────────────────────────────────");
    println!();
    println!("环境检查:");
    println!("   document-parser check           # 完整环境检查");
    println!("   document-parser uv-init         # 重新初始化环境");
    println!();
    println!("手动验证:");
    println!("   uv --version                    # 检查UV版本");
    println!("   ./venv/bin/python --version     # 检查虚拟环境Python (Linux/macOS)");
    println!("   .\\venv\\Scripts\\python --version  # 检查虚拟环境Python (Windows)");
    println!("   ./venv/bin/mineru --help        # 检查MinerU (Linux/macOS)");
    println!("   .\\venv\\Scripts\\mineru --help    # 检查MinerU (Windows)");
    println!();
    println!("日志查看:");
    println!("   tail -f logs/log.$(date +%Y-%m-%d)  # 查看当天日志 (Linux/macOS)");
    println!("   type logs\\log.%date:~0,10%          # 查看当天日志 (Windows)");
    println!();

    // 6. 获取帮助
    println!("🆘 6. 获取更多帮助");
    println!("───────────────────────────────────────────────────────────────");
    println!();
    println!("如果上述方法都无法解决问题，请:");
    println!("   1. 运行详细诊断: document-parser check");
    println!("   2. 收集错误信息:");
    println!("      • 完整的错误消息");
    println!("      • 操作系统版本");
    println!("      • Python版本");
    println!("      • 当前工作目录");
    println!("   3. 查看日志文件: logs/ 目录");
    println!("   4. 尝试在新的目录中重新初始化");
    println!();

    // 执行实时诊断
    println!("🔬 实时环境诊断");
    println!("───────────────────────────────────────────────────────────────");
    match environment_manager.check_environment().await {
        Ok(status) => {
            if status.is_ready() {
                println!("✅ 环境状态良好，所有依赖都已就绪");
            } else {
                println!("⚠️  发现以下问题:");
                let issues = status.get_critical_issues();
                for issue in issues {
                    println!("   • {}: {}", issue.component, issue.message);
                    println!("     建议: {}", issue.suggestion);
                }
            }
        }
        Err(e) => {
            println!("❌ 环境检查失败: {e}");
            println!("   请按照上述指南进行故障排除");
        }
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("💡 提示: 大多数问题可以通过重新运行 'document-parser uv-init' 解决");

    Ok(())
}

/// 处理环境检查命令
async fn handle_check_command(environment_manager: &EnvironmentManager) -> Result<()> {
    info!("检查Python环境状态...");

    // 首先进行路径诊断
    println!("🔍 诊断虚拟环境路径...");
    let path_issues = environment_manager.diagnose_venv_path_issues().await;
    if !path_issues.is_empty() {
        println!("⚠️  发现路径相关问题:");
        for issue in &path_issues {
            println!("   • {issue}");
        }
        println!();

        println!("💡 路径问题解决建议:");
        let suggestions = environment_manager.get_venv_recovery_suggestions().await;
        for suggestion in suggestions {
            println!("   {suggestion}");
        }
        println!();
    } else {
        println!("✅ 虚拟环境路径检查通过");
        println!();
    }

    match environment_manager.get_detailed_status_report().await {
        Ok(detailed_report) => {
            // 输出详细的诊断报告
            println!("{detailed_report}");

            // 输出增强的依赖验证报告
            println!("🔬 执行增强依赖验证...");
            match environment_manager.get_enhanced_dependency_report().await {
                Ok(enhanced_report) => {
                    println!("{enhanced_report}");
                }
                Err(e) => {
                    println!("⚠️  增强依赖验证失败: {e}");
                }
            }

            // 检查环境状态以确定退出码
            match environment_manager.check_environment().await {
                Ok(status) => {
                    if status.is_ready() {
                        println!("✅ 环境检查通过！所有依赖都已就绪。");
                        Ok(())
                    } else {
                        let critical_issues = status.get_critical_issues();
                        if !critical_issues.is_empty() {
                            println!("❌ 发现 {} 个关键问题需要解决", critical_issues.len());
                            for issue in critical_issues {
                                println!("  • {}: {}", issue.component, issue.message);
                                println!("    建议: {}", issue.suggestion);
                            }
                        }

                        let auto_fixable = status.get_auto_fixable_issues();
                        if !auto_fixable.is_empty() {
                            println!(
                                "💡 {} 个问题可以自动修复，运行 'document-parser uv-init' 进行修复",
                                auto_fixable.len()
                            );
                        }

                        // 如果有路径问题，提供额外的建议
                        if !path_issues.is_empty() {
                            println!();
                            println!("🔧 路径问题修复:");
                            println!("   • 运行 'document-parser uv-init' 会尝试自动修复路径问题");
                            println!("   • 或者手动按照上述建议解决路径问题");
                        }

                        Err(anyhow::anyhow!(
                            "环境未就绪，健康评分: {}/100",
                            status.health_score()
                        ))
                    }
                }
                Err(e) => {
                    println!("❌ 环境状态检查失败: {e}");

                    // 如果是路径相关错误，提供详细建议
                    match &e {
                        AppError::VirtualEnvironmentPath(_)
                        | AppError::Permission(_)
                        | AppError::Path(_) => {
                            println!();
                            println!("💡 路径错误解决建议:");
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
            println!("❌ 环境检查失败: {e}");

            // 如果是路径相关错误，提供详细建议
            match &e {
                AppError::VirtualEnvironmentPath(_)
                | AppError::Permission(_)
                | AppError::Path(_) => {
                    println!();
                    println!("💡 路径错误解决建议:");
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
    info!("开始安装Python依赖...");

    match environment_manager.setup_python_environment().await {
        Ok(_) => {
            info!("Python依赖安装完成！");

            // 验证安装结果
            match environment_manager.check_environment().await {
                Ok(status) => {
                    if status.mineru_available && status.markitdown_available {
                        info!("安装验证成功，所有依赖都已就绪！");
                    } else {
                        warn!("安装完成但验证失败，部分依赖可能未正确安装");
                    }
                }
                Err(e) => {
                    warn!("安装完成但验证失败: {e}");
                }
            }
        }
        Err(e) => {
            error!("Python依赖安装失败: {e}");
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
    info!("开始解析文件: {input:?}");
    info!("使用解析器: {parser}");

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
    info!("文件解析功能正在开发中...");

    // 确定输出路径
    let output_path = output.unwrap_or_else(|| {
        let mut path = input.clone();
        path.set_extension("md");
        path
    });

    info!("解析完成，结果将保存到: {output_path:?}");

    Ok(())
}
