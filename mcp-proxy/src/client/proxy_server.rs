//! MCP Proxy Server - 将 stdio MCP 服务代理为 HTTP/SSE 或 Streamable HTTP 服务
//!
//! 支持多个 agent 复用同一个 MCP 服务

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;
use serde::Deserialize;
use tokio::process::Command;

use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        TokioChildProcess,
        sse_server::{SseServer, SseServerConfig},
        streamable_http_server::{StreamableHttpService, session::local::LocalSessionManager},
    },
};

use crate::proxy::{ProxyHandler, ToolFilter};

/// 输出协议类型
#[derive(clap::ValueEnum, Clone, Debug, Default)]
pub enum ProxyProtocol {
    /// SSE 协议
    #[default]
    Sse,
    /// Streamable HTTP 协议
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
    #[arg(long, value_enum, default_value = "sse", help = "输出协议类型")]
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

    // 3. 创建工具过滤器
    let tool_filter = if let Some(allow_tools) = args.allow_tools.clone() {
        ToolFilter::allow(allow_tools)
    } else if let Some(deny_tools) = args.deny_tools.clone() {
        ToolFilter::deny(deny_tools)
    } else {
        ToolFilter::default()
    };

    if !quiet {
        eprintln!("🚀 MCP Proxy 服务");
        eprintln!("   服务名称: {}", parsed.name);
        eprintln!("   命令: {} {:?}", parsed.config.command, parsed.config.args.as_ref().unwrap_or(&vec![]));
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

    // 4. 主循环 - 支持子进程崩溃后自动重启
    loop {
        let result = run_proxy_server(&args, &parsed, tool_filter.clone(), verbose, quiet).await;

        match result {
            Ok(_) => {
                // 正常退出（如 Ctrl+C）
                if !quiet {
                    eprintln!("🛑 服务已停止");
                }
                break;
            }
            Err(e) => {
                // 异常退出，尝试重启
                eprintln!("⚠️  服务异常: {}，3秒后重启...", e);
                tokio::time::sleep(Duration::from_secs(3)).await;
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
    // 1. 创建子进程命令
    let mut command = Command::new(&parsed.config.command);

    if let Some(ref cmd_args) = parsed.config.args {
        command.args(cmd_args);
    }

    if let Some(ref env_vars) = parsed.config.env {
        for (k, v) in env_vars {
            command.env(k, v);
        }
    }

    // 2. 启动子进程
    let tokio_process = TokioChildProcess::new(command)?;

    // 3. 创建客户端信息
    let client_info = ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build(),
        ..Default::default()
    };

    // 4. 连接到子进程
    let client = client_info.serve(tokio_process).await?;

    if !quiet {
        eprintln!("✅ 子进程已启动");
    }

    // 5. 创建 ProxyHandler
    let proxy_handler = ProxyHandler::with_tool_filter(client, parsed.name.clone(), tool_filter);

    // 6. 根据协议类型启动服务器
    let bind_addr = format!("{}:{}", args.host, args.port);

    match args.protocol {
        ProxyProtocol::Sse => {
            run_sse_server(args, proxy_handler, &bind_addr, quiet).await
        }
        ProxyProtocol::Stream => {
            run_stream_server(proxy_handler, &bind_addr, quiet).await
        }
    }
}

/// 运行 SSE 服务器
async fn run_sse_server(
    args: &ProxyArgs,
    proxy_handler: ProxyHandler,
    bind_addr: &str,
    quiet: bool,
) -> Result<()> {
    let config = SseServerConfig {
        bind: bind_addr.parse()?,
        sse_path: args.sse_path.clone(),
        post_path: args.message_path.clone(),
        ct: tokio_util::sync::CancellationToken::new(),
        sse_keep_alive: None,
    };

    if !quiet {
        eprintln!("📡 SSE 服务启动: http://{}", bind_addr);
        eprintln!("   SSE 端点: http://{}{}", bind_addr, args.sse_path);
        eprintln!("   消息端点: http://{}{}", bind_addr, args.message_path);
        eprintln!("💡 MCP 客户端可直接使用 http://{} （自动重定向）", bind_addr);
        eprintln!("💡 按 Ctrl+C 停止服务");
    }

    let (sse_server, sse_router) = SseServer::new(config);
    let ct = sse_server.with_service(move || proxy_handler.clone());

    // 根路径兼容处理器 - 自动重定向到正确的端点
    let sse_path = args.sse_path.clone();
    let message_path = args.message_path.clone();

    let fallback_handler = move |method: axum::http::Method, headers: axum::http::HeaderMap| {
        let sse_path = sse_path.clone();
        let message_path = message_path.clone();
        async move {
            match method {
                axum::http::Method::GET => {
                    // 检查 Accept 头
                    let accept = headers
                        .get("accept")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");

                    if accept.contains("text/event-stream") {
                        // SSE 请求，重定向到 /sse
                        (
                            axum::http::StatusCode::TEMPORARY_REDIRECT,
                            [("Location", sse_path)],
                            "Redirecting to SSE endpoint".to_string(),
                        )
                    } else {
                        // 普通 GET 请求，返回服务信息
                        (
                            axum::http::StatusCode::OK,
                            [("Content-Type", "application/json".to_string())],
                            serde_json::json!({
                                "service": "MCP Proxy (SSE)",
                                "status": "running",
                                "endpoints": {
                                    "sse": sse_path,
                                    "message": message_path
                                },
                                "usage": "Connect your MCP client to this URL or the SSE endpoint directly"
                            }).to_string(),
                        )
                    }
                }
                axum::http::Method::POST => {
                    // POST 请求，重定向到 /message
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("Location", message_path)],
                        "Redirecting to message endpoint".to_string(),
                    )
                }
                _ => {
                    (
                        axum::http::StatusCode::METHOD_NOT_ALLOWED,
                        [("Allow", "GET, POST".to_string())],
                        "Method not allowed".to_string(),
                    )
                }
            }
        }
    };

    // 合并路由：SSE 路由 + 根路径兼容处理
    let router = sse_router.fallback(fallback_handler);

    // 启动 HTTP 服务器
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    // 使用 select 处理 Ctrl+C 和服务器
    tokio::select! {
        result = axum::serve(listener, router) => {
            if let Err(e) = result {
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            if !quiet {
                eprintln!("\n🛑 收到退出信号，正在关闭...");
            }
            ct.cancel();
        }
    }

    Ok(())
}

/// 运行 Streamable HTTP 服务器
async fn run_stream_server(
    proxy_handler: ProxyHandler,
    bind_addr: &str,
    quiet: bool,
) -> Result<()> {
    if !quiet {
        eprintln!("📡 Streamable HTTP 服务启动: http://{}", bind_addr);
        eprintln!("💡 MCP 客户端可直接使用 http://{}", bind_addr);
        eprintln!("💡 按 Ctrl+C 停止服务");
    }

    let service = StreamableHttpService::new(
        move || Ok(proxy_handler.clone()),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    // Streamable HTTP 直接在根路径提供服务
    let router = axum::Router::new().fallback_service(service);

    // 启动 HTTP 服务器
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    // 使用 select 处理 Ctrl+C 和服务器
    tokio::select! {
        result = axum::serve(listener, router) => {
            if let Err(e) = result {
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            if !quiet {
                eprintln!("\n🛑 收到退出信号，正在关闭...");
            }
        }
    }

    Ok(())
}

/// 解析配置
fn parse_config(args: &ProxyArgs) -> Result<ParsedConfig> {
    // 1. 读取配置内容
    let json_str = if let Some(ref config) = args.config {
        config.clone()
    } else if let Some(ref path) = args.config_file {
        std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("读取配置文件失败: {}", e))?
    } else {
        bail!("必须提供 --config 或 --config-file 参数");
    };

    // 2. 解析配置
    let mcp_config: McpConfig = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!(
            "配置解析失败: {}。配置必须是标准 MCP 格式，包含 mcpServers 字段",
            e
        ))?;

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
