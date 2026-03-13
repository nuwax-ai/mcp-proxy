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
        anyhow::bail!("--allow-tools 和 --deny-tools 不能同时使用，请只选择其中一个");
    }

    // 创建工具过滤器
    let tool_filter = if let Some(allow_tools) = args.allow_tools.clone() {
        tracing::info!("工具白名单: {:?}", allow_tools);
        ToolFilter::allow(allow_tools)
    } else if let Some(deny_tools) = args.deny_tools.clone() {
        tracing::info!("工具黑名单: {:?}", deny_tools);
        ToolFilter::deny(deny_tools)
    } else {
        tracing::debug!("工具过滤器: 未启用");
        ToolFilter::default()
    };

    // 解析配置
    tracing::debug!("开始解析配置...");
    let config_source = parse_convert_config(&args)?;
    tracing::info!("配置解析成功");

    // 提取 MCP 名称用于日志文件命名
    let mcp_name = match &config_source {
        crate::client::support::McpConfigSource::RemoteService { name, .. } => {
            tracing::info!("MCP 服务名称: {}", name);
            Some(name.as_str())
        }
        crate::client::support::McpConfigSource::LocalCommand { name, .. } => {
            tracing::info!("MCP 服务名称: {}", name);
            Some(name.as_str())
        }
        _ => {
            tracing::info!("MCP 服务名称: 未指定（使用 direct URL）");
            None
        }
    };

    // 初始化日志系统
    init_logging(&args, mcp_name, quiet, verbose)?;
    tracing::debug!("日志系统初始化完成");

    // 记录命令启动（必须在日志系统初始化之后）
    tracing::info!("========================================");
    tracing::info!("MCP-Proxy CLI 启动");
    tracing::info!("命令: convert (stdio 桥接模式)");
    tracing::info!("版本: {}", env!("CARGO_PKG_VERSION"));
    tracing::info!("诊断模式: {}", args.logging.diagnostic);
    tracing::info!("========================================");

    // 根据配置源执行不同逻辑
    match config_source {
        crate::client::support::McpConfigSource::DirectUrl { url } => {
            tracing::info!("模式: 直接 URL 模式");
            tracing::info!("目标 URL: {}", url);
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
            tracing::info!("模式: 远程服务配置模式");
            tracing::info!("服务名称: {}", name);
            tracing::info!("服务 URL: {}", url);
            if let Some(proto) = &protocol {
                tracing::info!("配置协议: {:?}", proto);
            }
            if !headers.is_empty() {
                tracing::debug!("自定义 headers: {:?}", headers);
            }
            if let Some(timeout) = timeout {
                tracing::debug!("超时设置: {}s", timeout);
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
