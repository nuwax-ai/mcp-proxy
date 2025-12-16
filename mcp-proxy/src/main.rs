mod config;
use anyhow::Result;
use backtrace::Backtrace;
use log::{error, info, warn};
use mcp_stdio_proxy::{
    AppConfig, AppState, get_proxy_manager, get_router, init_tracer_provider, log_service_info,
    start_schedule_task, Cli, run_cli,
};
use run_code_rmcp::warm_up_all_envs;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    // 解析命令行参数
    let cli = Cli::parse();
    
    // 如果有子命令，运行 CLI 模式
    if cli.command.is_some() || cli.url.is_some() {
        return run_cli_mode(cli).await;
    }
    
    // 否则运行传统的服务器模式
    run_server_mode().await
}

/// 运行 CLI 模式
async fn run_cli_mode(cli: Cli) -> Result<()> {
    // 设置基本的日志配置
    let log_level = if cli.verbose {
        "debug"
    } else if cli.quiet {
        "error"
    } else {
        "info"
    };
    
    // 初始化日志
    if std::env::var("RUST_LOG").is_err() {
        unsafe { std::env::set_var("RUST_LOG", log_level); }
    }
    
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .without_time()
        .init();
    
    // 运行 CLI 命令
    run_cli(cli).await
}

/// 运行传统的服务器模式
async fn run_server_mode() -> Result<()> {
    // 配置日志（保持原有的完整日志配置）
    let app_config = AppConfig::load_config()?;
    app_config.log_path_init()?;
    let log_level = app_config.log.level.clone();
    let log_path = app_config.log.path.clone();
    let server_port = app_config.server.port;
    let retain_days = app_config.log.retain_days;

    // 解析 RUST_LOG 环境变量
    let log_level_for_console = log_level.clone();
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level_for_console));

    // 使用 tracing-subscriber 初始化日志记录器
    let console_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_writer(std::io::stdout)
        .with_filter(console_filter);
    
    // 日志写入到文件，使用 Builder 模式配置日志轮转和保留策略
    let log_path_for_file = log_path.clone();
    let file_appender = Builder::new()
        .rotation(Rotation::DAILY) // 按天滚动
        .filename_prefix("log") // 文件名前缀
        .max_log_files(retain_days as usize) // 保留最近 N 个日志文件
        .build(&log_path_for_file)?;
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let log_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));
    
    // 配置文件日志层：使用 compact 格式，避免显示完整的 span 嵌套链，减少日志膨胀
    let file_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(non_blocking)
        .with_filter(log_filter);

    // 初始化 OpenTelemetry tracer provider
    init_tracer_provider("mcp-proxy", "0.1.0")?;

    // 配置 OpenTelemetry
    let telemetry_layer = tracing_opentelemetry::layer();

    // 初始化 tracing 订阅器
    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .with(telemetry_layer)
        .init();

    // 记录服务信息
    log_service_info("mcp-proxy", "0.1.0")?;
    tracing::info!("服务启动，监听端口: {}", server_port);

    // 监听地址
    let addr = format!("0.0.0.0:{server_port}");
    let listener = TcpListener::bind(&addr).await?;
    // 构建 axum 路由
    let state = AppState::new(app_config).await;

    // 初始化 MCP 路由
    let app = get_router(state.clone()).await?;
    info!("服务启动，监听地址: {addr}");

    // 启动定时任务，定期检查MCP服务状态
    tokio::spawn(start_schedule_task());
    info!("MCP服务状态检查定时任务已启动");
    info!("日志自动轮转已配置（保留最近 {} 个日志文件）", retain_days);

    // 注册关闭处理函数，确保在程序退出前执行清理
    tokio::spawn(async move {
        // 确保在程序退出前执行清理
        std::panic::set_hook(Box::new(move |panic_info| {
            // 记录详细的 panic 信息
            warn!("程序发生panic，执行清理...");

            // 记录 panic 消息
            if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                error!("Panic 原因: {s}");
            } else if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                error!("Panic 原因: {s}");
            } else {
                error!("Panic 原因: 未知");
            }

            // 记录 panic 位置
            if let Some(location) = panic_info.location() {
                error!("Panic 位置: {}:{}", location.file(), location.line());
            }

            // 尝试获取堆栈跟踪
            error!("堆栈跟踪:");
            let backtrace = Backtrace::new();
            error!("{backtrace:?}");
        }));
    });

    // 预热 uv/deno 环境依赖
    tokio::spawn(async move {
        info!("开始预热 uv/deno 环境依赖...");
        if let Err(e) = warm_up_all_envs(None, None, None, None).await {
            error!("预热 uv/deno 环境依赖失败: {e}");
        }
        info!("预热 uv/deno 环境依赖完成");
    });

    // 启动服务器，监听多种信号以实现优雅关闭
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal());

    // 运行服务器
    if let Err(e) = server.await {
        error!("服务运行错误: {e}");
    }

    // 服务器关闭后执行清理逻辑
    info!("服务器已关闭，开始清理资源...");

    // 清理所有SSE服务
    if let Err(e) = get_proxy_manager().cleanup_all_resources().await {
        error!("清理资源时出错: {}", e);
    }

    // 等待一小段时间确保所有资源都被清理
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    info!("资源清理完成，服务已完全关闭");
    Ok(())
}

// 监听多种终止信号
async fn shutdown_signal() {
    signal::ctrl_c()
        .await
        .expect("无法安装Ctrl+C处理器");
}