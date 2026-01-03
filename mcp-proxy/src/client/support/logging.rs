//! 日志系统初始化
//!
//! 处理日志文件的创建和日志级别配置

use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use super::args::{ConvertArgs, LoggingArgs};

/// 初始化日志系统
/// 只有在 diagnostic=true 时才会创建日志文件并输出详细日志
pub fn init_logging(args: &ConvertArgs, mcp_name: Option<&str>, quiet: bool, verbose: bool) -> Result<()> {
    init_logging_with_config(&args.logging, mcp_name, quiet, verbose)
}

/// 使用日志配置初始化日志系统（更通用的接口）
///
/// 这个函数可以被任何需要日志初始化的地方调用，不依赖完整的 ConvertArgs
pub fn init_logging_with_config(
    logging: &LoggingArgs,
    mcp_name: Option<&str>,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    // 确定日志级别：
    // - diagnostic=true 时使用 debug 级别（详细日志）
    // - diagnostic=false 时使用 warn 级别（只输出警告和错误）
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            if logging.diagnostic {
                EnvFilter::new("debug")
            } else if verbose {
                EnvFilter::new("info")
            } else {
                EnvFilter::new("warn")
            }
        });

    // 只有在 diagnostic=true 或用户明确指定日志文件时才创建日志文件
    let log_file_path = if let Some(log_file) = &logging.log_file {
        // 手动指定文件
        Some(log_file.clone())
    } else if let Some(log_dir) = &logging.log_dir {
        // 指定日志目录
        let session_id = generate_session_id();
        let date = chrono::Local::now().format("%Y%m%d");
        let name_part = mcp_name.unwrap_or("unknown");
        let filename = format!("mcp-proxy-{}-{}-{}.log",
                              name_part, date, session_id);

        std::fs::create_dir_all(log_dir)
            .map_err(|e| anyhow::anyhow!("无法创建日志目录: {}", e))?;
        Some(log_dir.join(filename))
    } else if logging.diagnostic {
        // diagnostic=true 时，使用 tempfile 在系统临时目录创建持久化的日志文件
        let session_id = generate_session_id();
        let date = chrono::Local::now().format("%Y%m%d");
        let name_part = mcp_name.unwrap_or("unknown");
        let filename = format!("mcp-proxy-{}-{}-{}.log",
                              name_part, date, session_id);

        // 使用 tempfile 在系统临时目录创建文件
        // 注意：这里使用 persist(true) 保持文件，程序结束后不会自动删除
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(filename);

        // 尝试创建文件，验证目录可写
        std::fs::File::create(&file_path)
            .map_err(|e| anyhow::anyhow!("无法创建日志文件: {} (路径: {})", e, file_path.display()))?;

        Some(file_path)
    } else {
        // diagnostic=false 且未指定日志文件，不创建日志文件
        None
    };

    // 初始化日志系统
    if let Some(file_path) = log_file_path {
        // 创建日志文件
        let file = std::fs::File::create(&file_path)
            .map_err(|e| anyhow::anyhow!("无法创建日志文件: {}", e))?;

        if !quiet {
            eprintln!("📝 日志文件: {}", file_path.display());
            eprintln!("📋 诊断模式: {} (日志级别: {})",
                     if logging.diagnostic { "启用" } else { "禁用" },
                     if logging.diagnostic { "DEBUG" } else { "WARN" });
        }

        // 同时输出到文件和 stderr
        // 使用 Arc 包装 file 以便共享
        let file_shared = std::sync::Arc::new(file);

        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_writer(std::sync::Mutex::new(file_shared.clone()))
                    .with_ansi(false)  // 文件不使用 ANSI 颜色
            )
            .with(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(true)   // stderr 使用 ANSI 颜色
            )
            .init();
    } else {
        // 仅输出到 stderr（不创建日志文件）
        if !quiet && !logging.diagnostic {
            eprintln!("📋 诊断模式: 禁用 (不创建日志文件)");
        }
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
    }

    Ok(())
}

/// 生成随机会话 ID（8 位十六进制）
pub fn generate_session_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    format!("{:08x}", rng.random::<u32>())
}
