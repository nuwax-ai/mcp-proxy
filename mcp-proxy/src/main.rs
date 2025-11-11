mod config;
use anyhow::Result;
use backtrace::Backtrace;
use log::{error, info, warn};
use mcp_proxy::{
    AppConfig, AppState, get_proxy_manager, get_router, init_tracer_provider, log_service_info,
    start_schedule_task,
};
use run_code_rmcp::warm_up_all_envs;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};

#[tokio::main]
async fn main() -> Result<()> {
    // 配置日志
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
    // 日志写入到文件
    let log_path_for_file = log_path.clone();
    let file_appender = tracing_appender::rolling::daily(log_path_for_file, "log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let log_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));
    let file_layer = tracing_subscriber::fmt::layer()
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

    // 启动日志清理任务，定期清理超过配置天数的日志文件
    let log_path = log_path.clone();
    tokio::spawn(async move {
        // 先执行一次清理
        info!("清理旧日志文件start,路径: {log_path}, 保留天数: {retain_days}");
        if let Err(e) = clean_old_logs(&log_path, retain_days).await {
            warn!("清理旧日志文件失败: {}", e);
        }

        // 每小时执行一次
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            info!("清理旧日志文件start,路径: {log_path}, 保留天数: {retain_days}");
            if let Err(e) = clean_old_logs(&log_path, retain_days).await {
                warn!("清理旧日志文件失败: {}", e);
            }
        }
    });
    info!("日志清理任务已启动（保留最近{}天的日志）", retain_days);

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

    //预热 uv /deno 环境依赖
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
    get_proxy_manager().cleanup_all_resources().await;

    // 等待一小段时间确保所有资源都被清理
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    info!("资源清理完成，服务已完全关闭");
    Ok(())
}

// 监听多种终止信号
async fn shutdown_signal() {
    signal::ctrl_c().await.expect("无法安装Ctrl+C处理器");
}

/// 清理超过指定天数的日志文件
async fn clean_old_logs(log_path: &str, retain_days: u32) -> Result<()> {
    use std::fs;
    use std::path::Path;

    let log_dir = Path::new(log_path);
    if !log_dir.exists() {
        info!("清理旧日志文件,路径: {log_path} 不存在");
        return Ok(());
    }

    let entries = fs::read_dir(log_dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // 只处理日志文件（文件名包含日期 log.YYYY-MM-DD）
        if path.is_file() {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                // 尝试从文件名中提取日期（格式: log.YYYY-MM-DD）
                if let Some(date_str) = file_name.strip_prefix("log.") {
                    if let Ok(file_date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                        // 基于文件名中的日期判断是否过期
                        let today = chrono::Local::now().date_naive();
                        let age_days = (today - file_date).num_days();
                        if age_days > retain_days as i64 {
                            if let Err(e) = fs::remove_file(&path) {
                                warn!("删除旧日志文件失败: {:?}, 错误: {}", path, e);
                            } else {
                                log::debug!(
                                    "已删除旧日志文件: {:?} (文件日期: {}, 超过{}天)",
                                    path,
                                    file_date,
                                    retain_days
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    info!("清理旧日志文件完成");
    Ok(())
}
