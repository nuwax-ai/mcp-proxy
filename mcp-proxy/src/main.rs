// Windows 平台配置：隐藏控制台窗口
// 当从 Tauri 等 GUI 应用启动时，不显示 CMD 窗口
// 注意：这会影响所有 Windows 平台的运行，独立运行时也不会有控制台输出
// 日志会写入文件（默认 ./logs/），可以通过日志文件查看运行状态

mod config;

use anyhow::Result;
use backtrace::Backtrace;
use clap::Parser;
use log::{error, info, warn};
use mcp_stdio_proxy::{
    AppConfig, AppState, Cli, get_proxy_manager, get_router, init_locale_from_env,
    init_tracer_provider, log_service_info, run_cli, start_schedule_task,
};
use run_code_rmcp::warm_up_all_envs;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化 Rustls CryptoProvider（必须在任何使用 TLS 的代码之前）
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

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
    // 初始化语言设置
    init_locale_from_env();

    // 检查是否是需要自定义日志初始化的命令
    // convert 和 proxy 命令会根据自己的参数（--log-dir、--log-file）初始化日志，所以这里跳过
    let is_convert_command = matches!(cli.command, Some(mcp_stdio_proxy::Commands::Convert(_)));
    let is_proxy_command = matches!(cli.command, Some(mcp_stdio_proxy::Commands::Proxy(_)));
    let is_health_command = matches!(cli.command, Some(mcp_stdio_proxy::Commands::Health(_)));
    let has_custom_logging = is_convert_command || is_proxy_command;

    // CLI 模式独立的日志配置
    // 跳过会自己初始化日志的命令，避免重复初始化导致 panic
    if !has_custom_logging && !cli.quiet {
        // CLI 模式默认只显示错误，避免 info/debug 日志污染输出
        let log_level = if cli.verbose {
            "debug"
        } else if is_health_command {
            // health 命令使用更严格的过滤，屏蔽 rmcp 库的噪音日志
            "off"
        } else {
            "error" // 默认只显示错误，屏蔽 info/warn/debug
        };

        // CLI 模式的日志配置：
        // 1. 禁用 ANSI 颜色（避免污染 JSON）
        // 2. 输出到 stderr（stdout 用于 JSON-RPC 通信）
        // 3. 简化格式（无时间戳、无目标）
        // 4. 优先使用 RUST_LOG 环境变量，否则使用默认日志级别
        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .without_time()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .compact()
            .init();
    }

    // 运行 CLI 命令
    run_cli(cli).await
}

/// 运行传统的服务器模式
async fn run_server_mode() -> Result<()> {
    // 初始化语言设置
    init_locale_from_env();

    // 配置日志（保持原有的完整日志配置）
    let app_config = AppConfig::load_config()?;

    // 打印配置信息到 stderr（在日志系统初始化之前）
    eprintln!("========================================");
    eprintln!("Starting MCP proxy service...");
    eprintln!("Version: {}", env!("CARGO_PKG_VERSION"));
    eprintln!("Configuration loaded:");
    eprintln!("  - Port: {}", app_config.server.port);
    eprintln!("  - Log directory: {}", &app_config.log.path);
    eprintln!("  - Log level: {}", &app_config.log.level);
    eprintln!("  - Log retention days: {}", app_config.log.retain_days);
    mcp_stdio_proxy::env_init::init(&app_config);
    eprintln!("========================================");

    app_config.log_path_init()?;
    let log_level = app_config.log.level.clone();
    let log_path = app_config.log.path.clone();
    let server_port = app_config.server.port;
    let retain_days = app_config.log.retain_days;

    // 解析 RUST_LOG 环境变量
    let log_level_for_console = log_level.clone();
    let mut console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level_for_console));

    // 修复 rmcp 库的 span clone panic 问题
    // 过滤掉 rmcp 的 trace/debug 级别日志，只保留 warn/error，避免 span 生命周期管理问题
    // see: https://github.com/tokio-rs/tracing/issues/2778
    console_filter = console_filter
        .add_directive("rmcp=warn".parse().unwrap())
        .add_directive("run_code_rmcp=warn".parse().unwrap());

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

    let mut log_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    // 修复 rmcp 库的 span clone panic 问题（同样应用于文件日志）
    log_filter = log_filter
        .add_directive("rmcp=warn".parse().unwrap())
        .add_directive("run_code_rmcp=warn".parse().unwrap());

    // 配置文件日志层：使用 compact 格式，避免显示完整的 span 嵌套链，减少日志膨胀
    let file_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(non_blocking)
        .with_filter(log_filter);

    // 初始化 OpenTelemetry tracer provider
    init_tracer_provider("mcp-proxy", "0.1.0")?;

    // 修复 rmcp 库的 span clone panic 问题（同样应用于 OpenTelemetry 层）
    // 只为 warn 和以上级别创建 span，避免过多的 span 导致 clone 问题
    let telemetry_filter = EnvFilter::new("warn")
        .add_directive("mcp_proxy=debug".parse().unwrap())
        .add_directive("mcp_stdio_proxy=debug".parse().unwrap())
        .add_directive("rmcp=error".parse().unwrap())
        .add_directive("run_code_rmcp=error".parse().unwrap());

    // 配置 OpenTelemetry（添加过滤器以避免 span clone panic）
    let telemetry_layer = tracing_opentelemetry::layer().with_filter(telemetry_filter);

    // 初始化 tracing 订阅器
    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .with(telemetry_layer)
        .init();

    // 记录服务信息
    log_service_info("mcp-proxy", "0.1.0")?;
    tracing::info!("========================================");
    tracing::info!("Starting MCP proxy service...");
    tracing::info!("Running in proxy server mode");
    tracing::info!("Version: {}", env!("CARGO_PKG_VERSION"));
    tracing::info!("Configuration:");
    tracing::info!("  - Listen port: {}", server_port);
    tracing::info!("  - Log directory: {}", log_path);
    tracing::info!("  - Log level: {}", &app_config.log.level);
    tracing::info!("  - Log retention days: {}", retain_days);
    tracing::info!("Environment overrides:");
    if std::env::var("MCP_PROXY_PORT").is_ok() {
        tracing::info!("  - MCP_PROXY_PORT override detected: {}", server_port);
    }
    if let Ok(log_dir) = std::env::var("MCP_PROXY_LOG_DIR") {
        tracing::info!("  - MCP_PROXY_LOG_DIR override detected: {}", log_dir);
    }
    if let Ok(level) = std::env::var("MCP_PROXY_LOG_LEVEL") {
        tracing::info!("  - MCP_PROXY_LOG_LEVEL override detected: {}", level);
    }
    tracing::info!("========================================");

    // 监听地址
    let addr = format!("0.0.0.0:{server_port}");
    tracing::info!("Binding to {addr}...");
    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        tracing::error!("Failed to bind to {addr}: {e}");
        e
    })?;
    tracing::info!("Successfully bound to {addr}");

    // 构建 axum 路由
    tracing::info!("Initializing application state...");
    let state = AppState::new(app_config.clone()).await;
    tracing::info!("Application state initialized");

    // 初始化 MCP 路由
    tracing::info!("Initializing MCP router...");
    let app = get_router(state.clone()).await?;
    tracing::info!("MCP router initialized");

    info!("MCP proxy started: {addr}");
    info!("Health endpoint: http://{addr}/health");
    info!("MCP list endpoint: http://{addr}/mcp/list");

    // 启动定时任务，定期检查MCP服务状态
    tokio::spawn(start_schedule_task());
    info!("Background schedule task started");
    info!("Log rotation configured (keep last {retain_days} files)");

    // 打印系统信息
    tracing::info!("System info:");
    tracing::info!("  - OS: {}", std::env::consts::OS);
    tracing::info!("  - Arch: {}", std::env::consts::ARCH);
    tracing::info!(
        "  - Working directory: {:?}",
        std::env::current_dir().unwrap_or_default()
    );

    // 注册关闭处理函数，确保在程序退出前执行清理
    tokio::spawn(async move {
        // 确保在程序退出前执行清理
        std::panic::set_hook(Box::new(move |panic_info| {
            // 记录详细的 panic 信息
            warn!("Panic hook triggered");

            // 记录 panic 消息
            if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                error!("Panic reason: {s}");
            } else if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                error!("Panic reason: {s}");
            } else {
                error!("Panic reason: <unknown>");
            }

            // 记录 panic 位置
            if let Some(location) = panic_info.location() {
                error!("Panic location: {}:{}", location.file(), location.line());
            }

            // 尝试获取堆栈跟踪
            error!("Panic backtrace:");
            let backtrace = Backtrace::new();
            error!("{backtrace:?}");
        }));
    });

    // 预热 uv/deno 环境依赖
    tokio::spawn(async move {
        info!("Warming up uv/deno runtime dependencies...");
        match warm_up_all_envs(None, None, None, None).await {
            Ok(_) => info!("Runtime dependency warm-up completed"),
            Err(e) => error!("Runtime dependency warm-up failed: {e}"),
        }
    });

    // 启动服务器，监听多种信号以实现优雅关闭
    info!("Starting HTTP server...");
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal());

    // 运行服务器
    if let Err(e) = server.await {
        error!("HTTP server exited with error: {e}");
    }

    // 服务器关闭后执行清理逻辑
    warn!("Shutdown signal received, starting cleanup...");

    // 清理所有SSE服务
    match get_proxy_manager().cleanup_all_resources().await {
        Ok(_) => info!("Resource cleanup completed"),
        Err(e) => error!("Resource cleanup failed: {e}"),
    }

    // 等待一小段时间确保所有资源都被清理
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    info!("Shutdown complete");
    Ok(())
}

// 监听多种终止信号
async fn shutdown_signal() {
    signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_crypto_provider_install() {
        // 测试 CryptoProvider 可以正常安装（或已被安装）
        // 注意：CryptoProvider 全局只能安装一次，多次安装会返回错误
        let _ = rustls::crypto::ring::default_provider().install_default();
        // 无论首次安装还是已安装，只要能获取到默认 provider 即表示成功
        let provider = rustls::crypto::CryptoProvider::get_default();
        assert!(provider.is_some(), "CryptoProvider should be available");
    }

    #[test]
    fn test_crypto_provider_get_default() {
        // 首先确保 CryptoProvider 已安装（忽略已安装的错误）
        let _ = rustls::crypto::ring::default_provider().install_default();

        // 测试可以正常获取默认 CryptoProvider
        let provider = rustls::crypto::CryptoProvider::get_default();
        assert!(
            provider.is_some(),
            "CryptoProvider should be available after installation"
        );
    }
}
