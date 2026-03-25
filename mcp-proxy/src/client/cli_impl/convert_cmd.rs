//! Convert 命令的 CLI 实现
//!
//! 处理 convert 命令的参数解析和路由到核心逻辑

use anyhow::Result;

use crate::client::core::{run_command_mode, run_url_mode_with_retry};
use crate::client::support::{ConvertArgs, init_logging, parse_convert_config};
use crate::proxy::ToolFilter;
use crate::t;

/// 运行转换命令 - 核心功能
pub async fn run_convert_command(args: ConvertArgs, verbose: bool, quiet: bool) -> Result<()> {
    // 检查 --allow-tools 和 --deny-tools 互斥
    if args.allow_tools.is_some() && args.deny_tools.is_some() {
        anyhow::bail!("--allow-tools and --deny-tools cannot be used together, please choose only one");
    }

    // 创建工具过滤器
    let tool_filter = if let Some(allow_tools) = args.allow_tools.clone() {
        tracing::info!("{}", t!("cli.convert.tool_whitelist", tools = format!("{:?}", allow_tools)));
        ToolFilter::allow(allow_tools)
    } else if let Some(deny_tools) = args.deny_tools.clone() {
        tracing::info!("{}", t!("cli.convert.tool_blacklist", tools = format!("{:?}", deny_tools)));
        ToolFilter::deny(deny_tools)
    } else {
        tracing::debug!("工具过滤器: 未启用");
        ToolFilter::default()
    };

    // 解析配置
    tracing::debug!("开始解析配置...");
    let config_source = parse_convert_config(&args)?;
    tracing::info!("{}", t!("cli.convert.config_parsed"));

    // 提取 MCP 名称用于日志文件命名
    let mcp_name = match &config_source {
        crate::client::support::McpConfigSource::RemoteService { name, .. } => {
            tracing::info!("{}", t!("cli.convert.service_name", name = name));
            Some(name.as_str())
        }
        crate::client::support::McpConfigSource::LocalCommand { name, .. } => {
            tracing::info!("{}", t!("cli.convert.service_name", name = name));
            Some(name.as_str())
        }
        _ => {
            tracing::info!("{}", t!("cli.convert.service_name_not_specified"));
            None
        }
    };

    // 初始化日志系统
    init_logging(&args, mcp_name, quiet, verbose)?;
    tracing::debug!("日志系统初始化完成");

    // 记录命令启动（必须在日志系统初始化之后）
    tracing::info!("========================================");
    tracing::info!("{}", t!("cli.convert.cli_starting"));
    tracing::info!("{}", t!("cli.convert.command"));
    tracing::info!("{}", t!("cli.convert.version", version = env!("CARGO_PKG_VERSION")));
    tracing::info!("{}", t!("cli.convert.diagnostic_mode", enabled = args.logging.diagnostic));
    tracing::info!("========================================");

    // 根据配置源执行不同逻辑
    match config_source {
        crate::client::support::McpConfigSource::DirectUrl { url } => {
            tracing::info!("{}", t!("cli.convert.mode_direct_url"));
            tracing::info!("{}", t!("cli.convert.target_url", url = url));
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
            tracing::info!("{}", t!("cli.convert.mode_remote_service"));
            tracing::info!("{}", t!("cli.convert.service_name", name = name));
            tracing::info!("{}", t!("cli.convert.service_url", url = url));
            if let Some(proto) = &protocol {
                tracing::info!("{}", t!("cli.convert.config_protocol", protocol = format!("{:?}", proto)));
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
