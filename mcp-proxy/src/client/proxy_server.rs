//! MCP Proxy Server - 将 stdio MCP 服务代理为 HTTP/SSE 或 Streamable HTTP 服务
//!
//! 支持多个 agent 复用同一个 MCP 服务

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::client::support::{LoggingArgs, init_logging_with_config};
use crate::proxy::ToolFilter;

/// 输出协议类型
#[derive(clap::ValueEnum, Clone, Debug, Default)]
pub enum ProxyProtocol {
    /// SSE 协议
    Sse,
    /// Streamable HTTP 协议
    #[default]
    Stream,
}

/// 代理模式参数 - 将 stdio MCP 服务代理为 HTTP 服务
#[derive(Parser, Debug, Clone)]
pub struct ProxyArgs {
    /// 监听端口
    #[arg(short, long, default_value = "8080", help = "监听端口")]
    pub port: u16,

    /// 监听地址
    #[arg(long, default_value = "127.0.0.1", help = "监听地址")]
    pub host: String,

    /// MCP 服务名称（当配置包含多个服务时必需）
    #[arg(short, long, help = "MCP 服务名称（多服务配置时必需）")]
    pub name: Option<String>,

    /// MCP 服务配置 JSON
    #[arg(long, conflicts_with = "config_file", help = "MCP 服务配置 JSON")]
    pub config: Option<String>,

    /// MCP 服务配置文件路径
    #[arg(long, conflicts_with = "config", help = "MCP 服务配置文件路径")]
    pub config_file: Option<PathBuf>,

    /// 输出协议类型
    #[arg(long, value_enum, default_value = "stream", help = "输出协议类型")]
    pub protocol: ProxyProtocol,

    /// SSE 端点路径（仅 SSE 协议）
    #[arg(long, default_value = "/sse", help = "SSE 端点路径")]
    pub sse_path: String,

    /// 消息端点路径（仅 SSE 协议）
    #[arg(long, default_value = "/message", help = "消息端点路径")]
    pub message_path: String,

    /// 工具白名单（逗号分隔），只允许指定的工具
    #[arg(long, value_delimiter = ',', help = "工具白名单（逗号分隔）")]
    pub allow_tools: Option<Vec<String>>,

    /// 工具黑名单（逗号分隔），排除指定的工具
    #[arg(long, value_delimiter = ',', help = "工具黑名单（逗号分隔）")]
    pub deny_tools: Option<Vec<String>>,

    /// 日志配置（使用通用结构）
    #[command(flatten)]
    pub logging: LoggingArgs,
}

/// MCP 配置格式
#[derive(Deserialize, Debug)]
struct McpConfig {
    #[serde(rename = "mcpServers")]
    mcp_servers: HashMap<String, StdioConfig>,
}

/// stdio 配置
#[derive(Deserialize, Debug, Clone)]
struct StdioConfig {
    command: String,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
}

/// 解析后的服务配置（包含服务名）
struct ParsedConfig {
    name: String,
    config: StdioConfig,
}

/// 运行代理命令
pub async fn run_proxy_command(args: ProxyArgs, verbose: bool, quiet: bool) -> Result<()> {
    // 1. 验证互斥参数
    if args.allow_tools.is_some() && args.deny_tools.is_some() {
        bail!("--allow-tools 和 --deny-tools 不能同时使用，请只选择其中一个");
    }

    // 2. 解析配置
    let parsed = parse_config(&args)?;

    // 3. 初始化日志系统（在启动服务之前）
    init_logging_with_config(&args.logging, Some(&parsed.name), quiet, verbose)?;

    // 4. 创建工具过滤器
    let tool_filter = if let Some(allow_tools) = args.allow_tools.clone() {
        ToolFilter::allow(allow_tools)
    } else if let Some(deny_tools) = args.deny_tools.clone() {
        ToolFilter::deny(deny_tools)
    } else {
        ToolFilter::default()
    };

    let protocol_name = match args.protocol {
        ProxyProtocol::Sse => "SSE",
        ProxyProtocol::Stream => "Streamable HTTP",
    };

    // 记录服务启动信息到日志文件
    info!(
        "[服务启动] MCP Proxy 服务启动 - 协议: {}, 服务名: {}, 命令: {} {:?}",
        protocol_name,
        parsed.name,
        parsed.config.command,
        parsed.config.args.as_ref().unwrap_or(&vec![])
    );

    if let Some(ref allow_tools) = args.allow_tools {
        info!("[服务启动] 工具白名单: {:?}", allow_tools);
    }
    if let Some(ref deny_tools) = args.deny_tools {
        info!("[服务启动] 工具黑名单: {:?}", deny_tools);
    }

    if !quiet {
        eprintln!("🚀 MCP Proxy 服务");
        eprintln!("   协议类型: {}", protocol_name);
        eprintln!("   服务名称: {}", parsed.name);
        eprintln!(
            "   命令: {} {:?}",
            parsed.config.command,
            parsed.config.args.as_ref().unwrap_or(&vec![])
        );
        if verbose {
            if let Some(ref env) = parsed.config.env {
                eprintln!("   环境变量: {:?}", env);
            }
        }
        // 显示过滤器配置
        if let Some(ref allow_tools) = args.allow_tools {
            eprintln!("   工具白名单: {:?}", allow_tools);
        }
        if let Some(ref deny_tools) = args.deny_tools {
            eprintln!("   工具黑名单: {:?}", deny_tools);
        }
    }

    // 5. 主循环 - 支持子进程崩溃后自动重启
    loop {
        let result = run_proxy_server(&args, &parsed, tool_filter.clone(), verbose, quiet).await;

        match result {
            Ok(_) => {
                // 正常退出（如 Ctrl+C）
                info!("[服务停止] MCP Proxy 服务正常停止 - 服务名: {}", parsed.name);
                if !quiet {
                    eprintln!("🛑 服务已停止");
                }
                break;
            }
            Err(e) => {
                // 异常退出，尝试重启
                error!(
                    "[服务异常] MCP Proxy 服务异常退出 - 服务名: {}, 错误: {}, 3秒后重启",
                    parsed.name, e
                );
                eprintln!("⚠️  服务异常: {}，3秒后重启...", e);
                tokio::time::sleep(Duration::from_secs(3)).await;
                warn!(
                    "[服务重启] 正在重启 MCP Proxy 服务 - 服务名: {}",
                    parsed.name
                );
                if !quiet {
                    eprintln!("🔄 正在重启服务...");
                }
                continue;
            }
        }
    }

    Ok(())
}

/// 运行代理服务器（单次运行）
async fn run_proxy_server(
    args: &ProxyArgs,
    parsed: &ParsedConfig,
    tool_filter: ToolFilter,
    _verbose: bool,
    quiet: bool,
) -> Result<()> {
    let bind_addr = format!("{}:{}", args.host, args.port);

    // 根据协议类型选择对应的库并启动服务器
    // 每个库使用自己的 rmcp 版本创建完整的生命周期
    match args.protocol {
        ProxyProtocol::Sse => {
            // 使用 mcp-sse-proxy 库（rmcp 0.10）
            let config = mcp_sse_proxy::McpServiceConfig {
                name: parsed.name.clone(),
                command: parsed.config.command.clone(),
                args: parsed.config.args.clone(),
                env: parsed.config.env.clone(),
                tool_filter: Some(tool_filter),
            };
            mcp_sse_proxy::run_sse_server_from_config(config, &bind_addr, quiet).await
        }
        ProxyProtocol::Stream => {
            // 使用 mcp-streamable-proxy 库（rmcp 0.12）
            let config = mcp_streamable_proxy::McpServiceConfig {
                name: parsed.name.clone(),
                command: parsed.config.command.clone(),
                args: parsed.config.args.clone(),
                env: parsed.config.env.clone(),
                tool_filter: Some(tool_filter),
            };
            mcp_streamable_proxy::run_stream_server_from_config(config, &bind_addr, quiet).await
        }
    }
}

// Note: 两个库现在都使用 mcp-common::ToolFilter，所以直接传递即可

/// 解析配置
fn parse_config(args: &ProxyArgs) -> Result<ParsedConfig> {
    // 1. 读取配置内容
    let json_str = if let Some(ref config) = args.config {
        config.clone()
    } else if let Some(ref path) = args.config_file {
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("读取配置文件失败: {}", e))?
    } else {
        bail!("必须提供 --config 或 --config-file 参数");
    };

    // 2. 解析配置
    let mcp_config: McpConfig = serde_json::from_str(&json_str).map_err(|e| {
        anyhow::anyhow!(
            "配置解析失败: {}。配置必须是标准 MCP 格式，包含 mcpServers 字段",
            e
        )
    })?;

    let servers = mcp_config.mcp_servers;

    if servers.is_empty() {
        bail!("配置中没有找到任何 MCP 服务");
    }

    // 3. 根据服务数量和 --name 参数选择服务
    if servers.len() == 1 {
        // 单服务：自动使用，无需 --name
        let (name, config) = servers.into_iter().next().unwrap();
        Ok(ParsedConfig { name, config })
    } else if let Some(ref name) = args.name {
        // 多服务：根据 --name 选择
        servers
            .get(name)
            .cloned()
            .map(|config| ParsedConfig {
                name: name.clone(),
                config,
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "服务 '{}' 不存在。可用服务: {:?}",
                    name,
                    servers.keys().collect::<Vec<_>>()
                )
            })
    } else {
        // 多服务但未指定 --name
        bail!(
            "配置包含多个服务 {:?}，请使用 --name 指定要启动的服务",
            servers.keys().collect::<Vec<_>>()
        );
    }
}
