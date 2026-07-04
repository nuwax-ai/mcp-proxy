//! Convert 命令的 CLI 实现
//!
//! 处理 convert 命令的参数解析和路由到核心逻辑

use anyhow::Result;

use crate::client::core::{run_command_mode, run_url_mode_with_retry};
use crate::client::support::{ConvertArgs, init_logging, parse_convert_config};
use crate::proxy::ToolFilter;

/// 运行转换命令 - 核心功能
pub async fn run_convert_command(args: ConvertArgs, verbose: bool, quiet: bool) -> Result<()> {
    // 检查 --allow-tools 和 --deny-tools 互斥
    if args.allow_tools.is_some() && args.deny_tools.is_some() {
        anyhow::bail!(
            "--allow-tools and --deny-tools cannot be used together, please choose only one"
        );
    }

    // 创建工具过滤器
    let tool_filter = if let Some(allow_tools) = args.allow_tools.clone() {
        tracing::info!("Tool allowlist enabled: {:?}", allow_tools);
        ToolFilter::allow(allow_tools)
    } else if let Some(deny_tools) = args.deny_tools.clone() {
        tracing::info!("Tool denylist enabled: {:?}", deny_tools);
        ToolFilter::deny(deny_tools)
    } else {
        tracing::debug!("Tool filter disabled");
        ToolFilter::default()
    };

    // 解析配置
    tracing::debug!("Parsing convert configuration...");
    let config_source = parse_convert_config(&args)?;
    tracing::info!("Configuration parsed");

    // 提取 MCP 名称用于日志文件命名
    let mcp_name = match &config_source {
        crate::client::support::McpConfigSource::RemoteService { name, .. } => {
            tracing::info!("Service name: {}", name);
            Some(name.as_str())
        }
        crate::client::support::McpConfigSource::LocalCommand { name, .. } => {
            tracing::info!("Service name: {}", name);
            Some(name.as_str())
        }
        _ => {
            tracing::info!("Service name not specified");
            None
        }
    };

    // 初始化日志系统
    init_logging(&args, mcp_name, quiet, verbose)?;
    tracing::debug!("Logging initialized");

    // 记录命令启动（必须在日志系统初始化之后）
    tracing::info!("========================================");
    tracing::info!("Starting convert command");
    tracing::info!("Command: convert");
    tracing::info!("Version: {}", env!("CARGO_PKG_VERSION"));
    tracing::info!("Diagnostic mode: {}", args.logging.diagnostic);
    tracing::info!("========================================");

    // 根据配置源执行不同逻辑
    match config_source {
        crate::client::support::McpConfigSource::DirectUrl { url } => {
            tracing::info!("Mode: direct URL");
            tracing::info!("Target URL: {}", url);
            // 直接 URL 模式（带自动重连）
            run_url_mode_with_retry(
                &args,
                &url,
                std::collections::HashMap::new(),
                None,
                tool_filter,
                verbose,
                quiet,
            )
            .await
        }
        crate::client::support::McpConfigSource::RemoteService {
            name,
            url,
            protocol,
            headers,
            timeout,
        } => {
            // 远程服务配置模式
            tracing::info!("Mode: remote service config");
            tracing::info!("Service name: {}", name);
            tracing::info!("Service URL: {}", url);
            if let Some(proto) = &protocol {
                tracing::info!("Configured protocol: {:?}", proto);
            }
            if !headers.is_empty() {
                tracing::debug!("Custom headers: {:?}", headers);
            }
            if let Some(timeout) = timeout {
                tracing::debug!("Configured timeout: {}s", timeout);
            }

            if !quiet {
                eprintln!("🚀 MCP-Stdio-Proxy: {} ({}) → stdio", name, url);
            }
            // 合并 headers：配置 + 命令行
            let merged_headers =
                crate::client::support::merge_headers(headers, &args.header, args.auth.as_ref());
            run_url_mode_with_retry(
                &args,
                &url,
                merged_headers,
                protocol.or(timeout.map(|_| crate::client::protocol::McpProtocol::Stream)),
                tool_filter,
                verbose,
                quiet,
            )
            .await
        }
        crate::client::support::McpConfigSource::LocalCommand {
            name,
            command,
            args: cmd_args,
            env,
        } => {
            // 本地命令模式（子进程继承父进程环境变量，MCP JSON 的 env 会覆盖同名变量）
            run_command_mode(&name, &command, cmd_args, env, tool_filter, quiet).await
        }
    }
}
