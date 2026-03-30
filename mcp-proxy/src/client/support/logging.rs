//! 日志系统初始化
//!
//! 处理日志文件的创建、日志级别配置和 OpenTelemetry 追踪初始化

use anyhow::Result;
use mcp_common::{TracingConfig, TracingGuard};
use once_cell::sync::OnceCell;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use super::args::{ConvertArgs, LoggingArgs};

/// 全局追踪守卫，保持 OTLP exporter 存活
static TRACING_GUARD: OnceCell<TracingGuard> = OnceCell::new();

/// 初始化日志系统
/// 只有在 diagnostic=true 时才会创建日志文件并输出详细日志
pub fn init_logging(
    args: &ConvertArgs,
    mcp_name: Option<&str>,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    init_logging_with_config(&args.logging, mcp_name, quiet, verbose)
}

/// 使用日志配置初始化日志系统（更通用的接口）
///
/// 这个函数可以被任何需要日志初始化的地方调用，不依赖完整的 ConvertArgs
///
/// # 功能
///
/// - 根据 `diagnostic` 参数控制日志级别
/// - 支持日志文件输出
/// - 支持 OTLP 追踪（当配置了 `otlp_endpoint` 时）
pub fn init_logging_with_config(
    logging: &LoggingArgs,
    mcp_name: Option<&str>,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    // 检查是否需要启用 OTLP 追踪
    let enable_otlp = logging.diagnostic && logging.otlp_endpoint.is_some();

    // 如果启用 OTLP，先初始化 tracer provider
    if enable_otlp {
        init_otlp_tracing(logging, mcp_name, quiet)?;
    }

    // 只有在 diagnostic=true 或用户明确指定日志文件时才创建日志文件
    let log_file_path = determine_log_file_path(logging, mcp_name)?;

    // 根据不同的配置组合初始化 subscriber
    match (log_file_path, enable_otlp) {
        (Some(file_path), true) => {
            init_with_file_and_otlp(logging, &file_path, quiet, verbose)?;
        }
        (Some(file_path), false) => {
            init_with_file_only(logging, &file_path, quiet, verbose)?;
        }
        (None, true) => {
            init_stderr_with_otlp(logging, quiet, verbose)?;
        }
        (None, false) => {
            init_stderr_only(logging, quiet, verbose)?;
        }
    }

    Ok(())
}

/// 确定日志文件路径
fn determine_log_file_path(
    logging: &LoggingArgs,
    mcp_name: Option<&str>,
) -> Result<Option<std::path::PathBuf>> {
    if let Some(log_file) = &logging.log_file {
        // 手动指定文件
        Ok(Some(log_file.clone()))
    } else if let Some(log_dir) = &logging.log_dir {
        // 指定日志目录
        let session_id = generate_session_id();
        let date = chrono::Local::now().format("%Y%m%d");
        let name_part = mcp_name.unwrap_or("unknown");
        let filename = format!("mcp-proxy-{}-{}-{}.log", name_part, date, session_id);

        std::fs::create_dir_all(log_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create log directory: {}", e))?;
        Ok(Some(log_dir.join(filename)))
    } else if logging.diagnostic {
        // diagnostic=true 时，使用系统临时目录
        let session_id = generate_session_id();
        let date = chrono::Local::now().format("%Y%m%d");
        let name_part = mcp_name.unwrap_or("unknown");
        let filename = format!("mcp-proxy-{}-{}-{}.log", name_part, date, session_id);

        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(filename);

        // 尝试创建文件，验证目录可写
        std::fs::File::create(&file_path).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create log file: {} (path: {})",
                e,
                file_path.display()
            )
        })?;

        Ok(Some(file_path))
    } else {
        Ok(None)
    }
}

/// 创建日志过滤器
fn create_filter(logging: &LoggingArgs, verbose: bool) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if logging.diagnostic {
            EnvFilter::new("debug")
        } else if verbose {
            EnvFilter::new("info")
        } else {
            EnvFilter::new("warn")
        }
    })
}

/// 初始化：文件 + OTLP
fn init_with_file_and_otlp(
    logging: &LoggingArgs,
    file_path: &std::path::Path,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    let file = std::fs::File::create(file_path)
        .map_err(|e| anyhow::anyhow!("Failed to create log file: {}", e))?;

    if !quiet {
        eprintln!("📝 Log file: {}", file_path.display());
        eprintln!(
            "📋 Diagnostic mode: {} (log level: {})",
            if logging.diagnostic {
                "enabled"
            } else {
                "disabled"
            },
            if logging.diagnostic { "DEBUG" } else { "WARN" }
        );
    }

    let file_shared = std::sync::Arc::new(file);
    let filter = create_filter(logging, verbose);

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_writer(std::sync::Mutex::new(file_shared.clone()))
                .with_ansi(false),
        )
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(true))
        .with(tracing_opentelemetry::layer())
        .init();

    Ok(())
}

/// 初始化：仅文件
fn init_with_file_only(
    logging: &LoggingArgs,
    file_path: &std::path::Path,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    let file = std::fs::File::create(file_path)
        .map_err(|e| anyhow::anyhow!("Failed to create log file: {}", e))?;

    if !quiet {
        eprintln!("📝 Log file: {}", file_path.display());
        eprintln!(
            "📋 Diagnostic mode: {} (log level: {})",
            if logging.diagnostic {
                "enabled"
            } else {
                "disabled"
            },
            if logging.diagnostic { "DEBUG" } else { "WARN" }
        );
    }

    let file_shared = std::sync::Arc::new(file);
    let filter = create_filter(logging, verbose);

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_writer(std::sync::Mutex::new(file_shared.clone()))
                .with_ansi(false),
        )
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(true))
        .init();

    Ok(())
}

/// 初始化：stderr + OTLP
fn init_stderr_with_otlp(logging: &LoggingArgs, quiet: bool, verbose: bool) -> Result<()> {
    if !quiet && !logging.diagnostic {
        eprintln!("📋 Diagnostic mode: disabled (no log file will be created)");
    }

    let filter = create_filter(logging, verbose);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(tracing_opentelemetry::layer())
        .init();

    Ok(())
}

/// 初始化：仅 stderr
fn init_stderr_only(logging: &LoggingArgs, quiet: bool, verbose: bool) -> Result<()> {
    if !quiet && !logging.diagnostic {
        eprintln!("📋 Diagnostic mode: disabled (no log file will be created)");
    }

    let filter = create_filter(logging, verbose);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();

    Ok(())
}

/// 初始化 OTLP 追踪（Jaeger 等）
fn init_otlp_tracing(logging: &LoggingArgs, mcp_name: Option<&str>, quiet: bool) -> Result<()> {
    if let Some(endpoint) = &logging.otlp_endpoint {
        let service_name = mcp_name.unwrap_or(&logging.service_name);

        let config = TracingConfig::new(service_name)
            .with_otlp(endpoint)
            .with_version(env!("CARGO_PKG_VERSION"));

        let guard = mcp_common::init_tracing(&config)?;

        // 保存到全局静态变量，确保 guard 在程序运行期间保持存活
        let _ = TRACING_GUARD.set(guard);

        if !quiet {
            eprintln!("🔭 OTLP tracing: enabled");
            eprintln!("   Endpoint: {}", endpoint);
            eprintln!("   Service: {}", service_name);
        }
    }

    Ok(())
}

/// 生成随机会话 ID（8 位十六进制）
pub fn generate_session_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    format!("{:08x}", rng.random::<u32>())
}
