// CLI 各子命令的 async 嵌套较深，默认 128 的递归上限会让类型布局查询溢出
#![recursion_limit = "256"]

mod cli;

use anyhow::{Context as _, Result};
use clap::Parser;
use cli::{Cli, Commands};
use document_parser::{
    APP_NAME, APP_VERSION, AppConfig, AppError, AppState,
    config::{CudaStatus, StdEnv, init_global_config, init_global_cuda_status},
    routes::create_routes,
    utils::environment_manager::{EnvironmentManager, EnvironmentStatus},
};
use log::{error, info, warn};
use std::backtrace::Backtrace;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};

#[tokio::main]
async fn main() -> Result<()> {
    cli::locale::init_locale_from_env();

    let cli = Cli::parse();

    let Cli {
        command,
        config,
        port,
        host,
    } = cli;

    // Lightweight path: systemd registration (no server / tracing / CUDA)
    if let Some(Commands::Service { action }) = command {
        return cli::service::handle_service_command(action).await;
    }

    // Load `.document-parser.env` before config (does not override existing env vars).
    // Launchd has no EnvironmentFile=; Linux may also inject via systemd (those win).
    let _ = document_parser::env_file::load_document_parser_env(config.as_deref());

    // 加载配置（`--config` 路径也必须套一层环境变量覆盖，否则 systemd EnvironmentFile
    // 注入的 OSS_ACCESS_KEY_* 不会生效）
    let mut app_config = if let Some(config_path) = config {
        let mut cfg =
            AppConfig::load_base_config_with_path(Some(config_path.to_string_lossy().to_string()))
                .context("配置加载失败")?;
        cfg.load_all_from_env(&StdEnv).context("环境变量覆盖失败")?;
        cfg.validate().context("配置验证失败")?;
        cfg
    } else {
        AppConfig::load_config().context("配置加载失败")?
    };

    // 覆盖命令行参数
    if let Some(port) = port {
        app_config.server.port = port;
    }
    app_config.server.host = host.clone();

    // 初始化全局配置
    init_global_config(app_config.clone()).context("全局配置初始化失败")?;

    // 检查并缓存CUDA环境状态到全局配置
    info!("Check CUDA environment status...");
    let environment_manager =
        EnvironmentManager::for_current_directory().context("无法创建环境管理器")?;

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
    let environment_manager =
        EnvironmentManager::for_current_directory().context("无法创建环境管理器")?;

    // 处理命令行子命令
    match command {
        Some(Commands::Check) => {
            return cli::check::handle_check_command(&environment_manager).await;
        }
        Some(Commands::Install) => {
            return cli::check::handle_install_command(&environment_manager).await;
        }
        Some(Commands::Parse {
            input,
            output,
            parser,
        }) => {
            return cli::parse::handle_parse_command(
                &app_config,
                &environment_manager,
                input,
                output,
                parser,
            )
            .await;
        }
        Some(Commands::UvInit) => {
            return cli::uv_init::handle_uv_init_command(&environment_manager).await;
        }
        Some(Commands::Troubleshoot) => {
            return cli::troubleshoot::handle_troubleshoot_command(&environment_manager).await;
        }
        Some(Commands::Service { .. }) => {
            unreachable!("service command handled before server init")
        }
        Some(Commands::Server { daemon: _ }) | None => {
            // 继续执行服务器模式
        }
    }

    info!("Service listening address: {server_host}:{server_port}");

    // 初始化环境管理器并检查Python环境
    info!("Start checking and initializing the Python environment...");
    let environment_manager =
        EnvironmentManager::for_current_directory().context("无法创建环境管理器")?;

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
        .context("无法创建应用状态")?;

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
