// MCP-Proxy CLI 实现
// 使用库提供的高层 API，分支处理 SSE 和 Stream 协议

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use anyhow::{Result, bail};
use serde::Deserialize;
use tokio::process::Command;
use tracing::error;

// 使用各自库的高层 API（从 proxy/mod.rs 导入）
use crate::proxy::{
    ProxyHandler, ToolFilter,
    McpClientConfig, SseClientConnection, StreamClientConnection,
};

// SSE 模式需要的类型（rmcp 0.10）- 用于 command 模式和 SSE stdio 服务
use mcp_sse_proxy::{
    ServiceExt as SseServiceExt,
    TokioChildProcess,
    stdio as sse_stdio,
    ClientInfo as SseClientInfo,
    ClientCapabilities as SseClientCapabilities,
    Implementation as SseImplementation,
};

// Stream 模式需要的类型（rmcp 0.12）
use mcp_streamable_proxy::{
    ServiceExt as StreamServiceExt,
    stdio as stream_stdio,
    ProxyHandler as StreamProxyHandler,
};

/// MCP-Proxy CLI 主命令结构
#[derive(Parser, Debug)]
#[command(name = "mcp-proxy")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "MCP 协议转换代理工具", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
    
    /// 直接URL模式（向后兼容）
    #[arg(value_name = "URL", help = "MCP 服务的 URL 地址（直接模式）")]
    pub url: Option<String>,
    
    /// 全局详细输出
    #[arg(short, long, global = true)]
    pub verbose: bool,
    
    /// 全局静默模式
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum Commands {
    /// 协议转换模式 - 将 URL 转换为 stdio
    Convert(ConvertArgs),

    /// 检查服务状态
    Check(CheckArgs),

    /// 协议检测
    Detect(DetectArgs),

    /// 代理模式 - 将 stdio MCP 服务代理为 HTTP/SSE 服务
    Proxy(super::proxy_server::ProxyArgs),
}

/// 协议转换参数
#[derive(Parser, Debug, Clone)]
pub struct ConvertArgs {
    /// MCP 服务的 URL 地址（可选，与 --config/--config-file 二选一）
    #[arg(value_name = "URL", help = "MCP 服务的 URL 地址")]
    pub url: Option<String>,

    /// MCP 服务配置 JSON
    #[arg(long, conflicts_with = "config_file", help = "MCP 服务配置 JSON")]
    pub config: Option<String>,

    /// MCP 服务配置文件路径
    #[arg(long, conflicts_with = "config", help = "MCP 服务配置文件路径")]
    pub config_file: Option<std::path::PathBuf>,

    /// MCP 服务名称（多服务配置时必需）
    #[arg(short, long, help = "MCP 服务名称（多服务配置时必需）")]
    pub name: Option<String>,

    /// 指定远程服务协议类型（不指定则自动检测）
    #[arg(long, value_enum, help = "指定远程服务协议类型（不指定则自动检测）")]
    pub protocol: Option<super::proxy_server::ProxyProtocol>,

    /// 认证 header (如: "Bearer token")
    #[arg(short, long, help = "认证 header")]
    pub auth: Option<String>,

    /// 自定义 HTTP headers
    #[arg(short = 'H', long, value_parser = parse_key_val, help = "自定义 HTTP headers (KEY=VALUE 格式)")]
    pub header: Vec<(String, String)>,

    /// 重试次数
    #[arg(long, default_value = "0", help = "重试次数，0 表示无限重试")]
    pub retries: u32,

    /// 工具白名单（逗号分隔），只允许指定的工具
    #[arg(long, value_delimiter = ',', help = "工具白名单（逗号分隔），只允许指定的工具")]
    pub allow_tools: Option<Vec<String>>,

    /// 工具黑名单（逗号分隔），排除指定的工具
    #[arg(long, value_delimiter = ',', help = "工具黑名单（逗号分隔），排除指定的工具")]
    pub deny_tools: Option<Vec<String>>,

    /// 客户端 ping 间隔（秒），0 表示禁用
    #[arg(long, default_value = "30", help = "客户端 ping 间隔（秒），0 表示禁用")]
    pub ping_interval: u64,

    /// 客户端 ping 超时（秒）
    #[arg(long, default_value = "10", help = "客户端 ping 超时（秒），超时则认为连接断开")]
    pub ping_timeout: u64,
}

/// 检查参数
#[derive(Parser, Debug)]
pub struct CheckArgs {
    /// 要检查的 MCP 服务 URL
    #[arg(value_name = "URL")]
    pub url: String,
    
    /// 认证 header
    #[arg(short, long)]
    pub auth: Option<String>,
    
    /// 超时时间
    #[arg(long, default_value = "10")]
    pub timeout: u64,
}

/// 协议检测参数
#[derive(Parser, Debug)]
pub struct DetectArgs {
    /// 要检测的 MCP 服务 URL
    #[arg(value_name = "URL")]
    pub url: String,
    
    /// 认证 header
    #[arg(short, long)]
    pub auth: Option<String>,
}

/// 解析 KEY=VALUE 格式的辅助函数
fn parse_key_val(s: &str) -> Result<(String, String)> {
    let pos = s.find('=')
        .ok_or_else(|| anyhow::anyhow!("无效的 KEY=VALUE 格式: {}", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

// ============== MCP 配置解析相关 ==============

/// MCP 配置格式
#[derive(Deserialize, Debug)]
struct McpConfig {
    #[serde(rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServerInnerConfig>,
}

/// MCP 服务配置（支持 Command 和 Url 两种类型）
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum McpServerInnerConfig {
    Command(StdioConfig),
    Url(UrlConfig),
}

/// stdio 配置（本地命令）
#[derive(Deserialize, Debug, Clone)]
struct StdioConfig {
    command: String,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
}

/// URL 配置（远程服务）
#[derive(Deserialize, Debug, Clone)]
struct UrlConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        rename = "baseUrl",
        alias = "baseurl",
        alias = "base_url"
    )]
    base_url: Option<String>,
    #[serde(default, rename = "type", alias = "Type")]
    r#type: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, alias = "authToken", alias = "auth_token")]
    pub auth_token: Option<String>,
    pub timeout: Option<u64>,
}

impl UrlConfig {
    fn get_url(&self) -> Option<&str> {
        self.url.as_deref().or(self.base_url.as_deref())
    }
}

/// 解析后的配置源
enum McpConfigSource {
    /// 直接 URL 模式（命令行参数）
    DirectUrl {
        url: String,
    },
    /// 远程服务配置（JSON 配置）
    RemoteService {
        name: String,
        url: String,
        protocol: Option<super::protocol::McpProtocol>,
        headers: HashMap<String, String>,
        timeout: Option<u64>,
    },
    /// 本地命令配置（JSON 配置）
    LocalCommand {
        name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
}

/// 解析 convert 命令的配置
fn parse_convert_config(args: &ConvertArgs) -> Result<McpConfigSource> {
    // 优先级：url > config > config_file
    if let Some(ref url) = args.url {
        return Ok(McpConfigSource::DirectUrl { url: url.clone() });
    }

    // 读取 JSON 配置
    let json_str = if let Some(ref config) = args.config {
        config.clone()
    } else if let Some(ref path) = args.config_file {
        std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("读取配置文件失败: {}", e))?
    } else {
        bail!("必须提供 URL、--config 或 --config-file 参数之一");
    };

    // 解析 JSON 配置
    let mcp_config: McpConfig = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!(
            "配置解析失败: {}。配置必须是标准 MCP 格式，包含 mcpServers 字段",
            e
        ))?;

    let servers = mcp_config.mcp_servers;

    if servers.is_empty() {
        bail!("配置中没有找到任何 MCP 服务");
    }

    // 选择服务
    let (name, inner_config) = if servers.len() == 1 {
        servers.into_iter().next().unwrap()
    } else if let Some(ref name) = args.name {
        let config = servers.get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!(
                "服务 '{}' 不存在。可用服务: {:?}",
                name,
                servers.keys().collect::<Vec<_>>()
            ))?;
        (name.clone(), config)
    } else {
        bail!(
            "配置包含多个服务 {:?}，请使用 --name 指定要使用的服务",
            servers.keys().collect::<Vec<_>>()
        );
    };

    // 根据配置类型返回
    match inner_config {
        McpServerInnerConfig::Command(stdio) => {
            Ok(McpConfigSource::LocalCommand {
                name,
                command: stdio.command,
                args: stdio.args.unwrap_or_default(),
                env: stdio.env.unwrap_or_default(),
            })
        }
        McpServerInnerConfig::Url(url_config) => {
            let url = url_config.get_url()
                .ok_or_else(|| anyhow::anyhow!("URL 配置缺少 url 或 baseUrl 字段"))?
                .to_string();

            // 解析协议类型
            let protocol = url_config.r#type.as_ref().and_then(|t| {
                match t.as_str() {
                    "sse" => Some(super::protocol::McpProtocol::Sse),
                    "http" | "stream" => Some(super::protocol::McpProtocol::Stream),
                    _ => None,
                }
            });

            // 合并 headers：JSON 配置中的 auth_token -> Authorization
            let mut headers = url_config.headers.clone().unwrap_or_default();
            if let Some(auth_token) = &url_config.auth_token {
                headers.insert("Authorization".to_string(), auth_token.clone());
            }

            Ok(McpConfigSource::RemoteService {
                name,
                url,
                protocol,
                headers,
                timeout: url_config.timeout,
            })
        }
    }
}

/// 合并 headers：JSON 配置 + 命令行参数（命令行优先）
fn merge_headers(
    config_headers: HashMap<String, String>,
    cli_headers: &[(String, String)],
    cli_auth: Option<&String>,
) -> HashMap<String, String> {
    let mut merged = config_headers;

    // 命令行 -H 参数覆盖配置
    for (key, value) in cli_headers {
        merged.insert(key.clone(), value.clone());
    }

    // 命令行 --auth 参数优先级最高
    if let Some(auth_value) = cli_auth {
        merged.insert("Authorization".to_string(), auth_value.clone());
    }

    merged
}

/// 运行 CLI 主逻辑
pub async fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Commands::Convert(args)) => {
            run_convert_command(args, cli.verbose, cli.quiet).await
        }
        Some(Commands::Check(args)) => {
            run_check_command(args, cli.verbose, cli.quiet).await
        }
        Some(Commands::Detect(args)) => {
            run_detect_command(args, cli.verbose, cli.quiet).await
        }
        Some(Commands::Proxy(args)) => {
            super::proxy_server::run_proxy_command(args, cli.verbose, cli.quiet).await
        }
        None => {
            // 直接 URL 模式（向后兼容）
            if let Some(url) = cli.url {
                let args = ConvertArgs {
                    url: Some(url),
                    config: None,
                    config_file: None,
                    name: None,
                    protocol: None,
                    auth: None,
                    header: vec![],
                    retries: 0,    // 无限重试
                    allow_tools: None,
                    deny_tools: None,
                    ping_interval: 30,  // 默认 30 秒 ping 一次
                    ping_timeout: 10,   // 默认 10 秒超时
                };
                run_convert_command(args, cli.verbose, cli.quiet).await
            } else {
                bail!("请提供 URL 或使用子命令")
            }
        }
    }
}

/// 运行转换命令 - 核心功能
async fn run_convert_command(args: ConvertArgs, verbose: bool, quiet: bool) -> Result<()> {
    // 检查 --allow-tools 和 --deny-tools 互斥
    if args.allow_tools.is_some() && args.deny_tools.is_some() {
        bail!("--allow-tools 和 --deny-tools 不能同时使用，请只选择其中一个");
    }

    // 创建工具过滤器
    let tool_filter = if let Some(allow_tools) = args.allow_tools.clone() {
        ToolFilter::allow(allow_tools)
    } else if let Some(deny_tools) = args.deny_tools.clone() {
        ToolFilter::deny(deny_tools)
    } else {
        ToolFilter::default()
    };

    // 解析配置
    let config_source = parse_convert_config(&args)?;

    // 根据配置源执行不同逻辑
    match config_source {
        McpConfigSource::DirectUrl { url } => {
            // 直接 URL 模式（带自动重连）
            run_url_mode_with_retry(&args, &url, HashMap::new(), None, tool_filter, verbose, quiet).await
        }
        McpConfigSource::RemoteService { name, url, protocol, headers, timeout } => {
            // 远程服务配置模式
            if !quiet {
                eprintln!("🚀 MCP-Stdio-Proxy: {} ({}) → stdio", name, url);
            }
            // 合并 headers：配置 + 命令行
            let merged_headers = merge_headers(headers, &args.header, args.auth.as_ref());
            run_url_mode_with_retry(&args, &url, merged_headers, protocol.or(timeout.map(|_| super::protocol::McpProtocol::Stream)), tool_filter, verbose, quiet).await
        }
        McpConfigSource::LocalCommand { name, command, args: cmd_args, env } => {
            // 本地命令模式（使用 SSE 库的 rmcp 0.10）
            run_command_mode(&name, &command, cmd_args, env, tool_filter, verbose, quiet).await
        }
    }
}

/// URL 模式执行（带自动重连）
/// 使用分支逻辑：根据协议类型调用不同的处理函数
async fn run_url_mode_with_retry(
    args: &ConvertArgs,
    url: &str,
    merged_headers: HashMap<String, String>,
    config_protocol: Option<super::protocol::McpProtocol>,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    if !quiet && merged_headers.is_empty() {
        eprintln!("🚀 MCP-Stdio-Proxy: {} → stdio", url);
    }

    // 显示过滤器配置
    if !quiet {
        if let Some(ref allow_tools) = args.allow_tools {
            eprintln!("🔧 工具白名单: {:?}", allow_tools);
        }
        if let Some(ref deny_tools) = args.deny_tools {
            eprintln!("🔧 工具黑名单: {:?}", deny_tools);
        }
    }

    // 确定协议类型：命令行参数 > 配置文件 > 自动检测
    let protocol = if let Some(ref proto) = args.protocol {
        let detected = match proto {
            super::proxy_server::ProxyProtocol::Sse => super::protocol::McpProtocol::Sse,
            super::proxy_server::ProxyProtocol::Stream => super::protocol::McpProtocol::Stream,
        };
        if !quiet {
            eprintln!("🔧 使用指定协议: {}", protocol_name(&detected));
        }
        detected
    } else if let Some(proto) = config_protocol {
        if !quiet {
            eprintln!("🔧 使用配置协议: {}", protocol_name(&proto));
        }
        proto
    } else {
        if !quiet {
            eprintln!("🔍 正在检测协议...");
        }
        let detected = super::protocol::detect_mcp_protocol(url).await?;
        if !quiet {
            eprintln!("🔍 检测到 {} 协议", protocol_name(&detected));
        }
        detected
    };

    // 构建 McpClientConfig
    let config = build_mcp_config(url, &merged_headers, args.auth.as_ref());

    // 根据协议类型分支处理
    match protocol {
        super::protocol::McpProtocol::Sse => {
            run_sse_mode(config, args.clone(), tool_filter, verbose, quiet).await
        }
        super::protocol::McpProtocol::Stream => {
            run_stream_mode(config, args.clone(), tool_filter, verbose, quiet).await
        }
        super::protocol::McpProtocol::Stdio => {
            bail!("Stdio 协议不支持通过 URL 转换，请使用 --config 配置本地命令")
        }
    }
}

/// 构建 McpClientConfig
fn build_mcp_config(
    url: &str,
    headers: &HashMap<String, String>,
    auth: Option<&String>,
) -> McpClientConfig {
    let mut config = McpClientConfig::new(url);
    for (k, v) in headers {
        config = config.with_header(k, v);
    }
    if let Some(auth_value) = auth {
        config = config.with_header("Authorization", auth_value);
    }
    config
}

/// SSE 模式处理（使用 mcp-sse-proxy，rmcp 0.10）
async fn run_sse_mode(
    config: McpClientConfig,
    args: ConvertArgs,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    if !quiet {
        eprintln!("🔗 正在连接到后端服务 (SSE)...");
    }

    // 1. 使用高层 API 连接
    let connect_timeout = Duration::from_secs(30);
    let conn = tokio::time::timeout(connect_timeout, SseClientConnection::connect(config.clone()))
        .await
        .map_err(|_| anyhow::anyhow!("连接后端超时 ({}秒)", connect_timeout.as_secs()))?
        .map_err(|e| anyhow::anyhow!("连接后端失败: {}", e))?;

    if !quiet {
        eprintln!("✅ 后端连接成功");
        // 打印工具列表
        print_sse_tools(&conn, quiet).await;
        if args.ping_interval > 0 {
            eprintln!("💓 心跳检测: 每 {}s ping 一次（超时 {}s）", args.ping_interval, args.ping_timeout);
        }
    }

    // 2. 创建 handler（消耗 conn）
    let handler = Arc::new(conn.into_handler("cli".to_string(), tool_filter.clone()));

    // 3. 启动 stdio server
    let server = (*handler).clone().serve(sse_stdio()).await?;

    if !quiet {
        eprintln!("💡 stdio server 已启动，开始代理转换...");
    }

    // 4. 启动 watchdog 任务
    let handler_for_watchdog = handler.clone();
    let mut watchdog_handle = tokio::spawn(run_sse_watchdog(
        handler_for_watchdog,
        args,
        config,
        tool_filter,
        verbose,
        quiet,
    ));

    // 5. 等待 stdio server 退出
    tokio::select! {
        result = server.waiting() => {
            watchdog_handle.abort();
            result?;
        }
        watchdog_result = &mut watchdog_handle => {
            if let Err(e) = watchdog_result {
                if !e.is_cancelled() {
                    error!("SSE Watchdog task failed: {:?}", e);
                }
            }
        }
    }

    Ok(())
}

/// Stream 模式处理（使用 mcp-streamable-proxy，rmcp 0.12）
async fn run_stream_mode(
    config: McpClientConfig,
    args: ConvertArgs,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    if !quiet {
        eprintln!("🔗 正在连接到后端服务 (Stream)...");
    }

    // 1. 使用高层 API 连接
    let connect_timeout = Duration::from_secs(30);
    let conn = tokio::time::timeout(connect_timeout, StreamClientConnection::connect(config.clone()))
        .await
        .map_err(|_| anyhow::anyhow!("连接后端超时 ({}秒)", connect_timeout.as_secs()))?
        .map_err(|e| anyhow::anyhow!("连接后端失败: {}", e))?;

    if !quiet {
        eprintln!("✅ 后端连接成功");
        // 打印工具列表
        print_stream_tools(&conn, quiet).await;
        if args.ping_interval > 0 {
            eprintln!("💓 心跳检测: 每 {}s ping 一次（超时 {}s）", args.ping_interval, args.ping_timeout);
        }
    }

    // 2. 创建 handler（消耗 conn）
    let handler = Arc::new(conn.into_handler("cli".to_string(), tool_filter.clone()));

    // 3. 启动 stdio server（使用 stream_stdio，即 rmcp 0.12 的 stdio）
    let server = (*handler).clone().serve(stream_stdio()).await?;

    if !quiet {
        eprintln!("💡 stdio server 已启动，开始代理转换...");
    }

    // 4. 启动 watchdog 任务
    let handler_for_watchdog = handler.clone();
    let mut watchdog_handle = tokio::spawn(run_stream_watchdog(
        handler_for_watchdog,
        args,
        config,
        tool_filter,
        verbose,
        quiet,
    ));

    // 5. 等待 stdio server 退出
    tokio::select! {
        result = server.waiting() => {
            watchdog_handle.abort();
            result?;
        }
        watchdog_result = &mut watchdog_handle => {
            if let Err(e) = watchdog_result {
                if !e.is_cancelled() {
                    error!("Stream Watchdog task failed: {:?}", e);
                }
            }
        }
    }

    Ok(())
}

/// 打印 SSE 连接的工具列表
async fn print_sse_tools(conn: &SseClientConnection, quiet: bool) {
    if quiet {
        return;
    }
    match conn.list_tools().await {
        Ok(tools) => {
            if tools.is_empty() {
                eprintln!("⚠️  工具列表为空 (tools/list 返回 0 个工具)");
            } else {
                eprintln!("🔧 可用工具 ({} 个):", tools.len());
                for tool in &tools {
                    let desc = tool.description.as_deref().unwrap_or("无描述");
                    let desc_short = truncate_str(desc, 50);
                    eprintln!("   - {} : {}", tool.name, desc_short);
                }
            }
        }
        Err(e) => {
            eprintln!("⚠️  获取工具列表失败: {}", e);
        }
    }
}

/// 打印 Stream 连接的工具列表
async fn print_stream_tools(conn: &StreamClientConnection, quiet: bool) {
    if quiet {
        return;
    }
    match conn.list_tools().await {
        Ok(tools) => {
            if tools.is_empty() {
                eprintln!("⚠️  工具列表为空 (tools/list 返回 0 个工具)");
            } else {
                eprintln!("🔧 可用工具 ({} 个):", tools.len());
                for tool in &tools {
                    let desc = tool.description.as_deref().unwrap_or("无描述");
                    let desc_short = truncate_str(desc, 50);
                    eprintln!("   - {} : {}", tool.name, desc_short);
                }
            }
        }
        Err(e) => {
            eprintln!("⚠️  获取工具列表失败: {}", e);
        }
    }
}

/// 截断字符串（UTF-8 安全）
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        format!("{}...", s.chars().take(max_len - 3).collect::<String>())
    } else {
        s.to_string()
    }
}

/// SSE 模式的 watchdog：负责监控连接健康、断开时重连
async fn run_sse_watchdog(
    handler: Arc<ProxyHandler>,
    args: ConvertArgs,
    config: McpClientConfig,
    _tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) {
    let max_retries = args.retries;
    let mut attempt = 0u32;
    let mut backoff_secs = 1u64;
    const MAX_BACKOFF_SECS: u64 = 30;

    // 首先监控现有连接的健康状态
    let disconnect_reason = monitor_sse_connection(
        &handler,
        args.ping_interval,
        args.ping_timeout,
        quiet,
    ).await;

    // 连接断开，标记后端不可用
    handler.swap_backend(None);

    if !quiet {
        eprintln!("⚠️  连接断开: {}", disconnect_reason);
    }

    // 进入重连循环
    loop {
        attempt += 1;

        if !quiet {
            eprintln!("🔗 正在重新连接 (第{}次尝试)...", attempt);
        }

        // 尝试建立连接
        let connect_result = SseClientConnection::connect(config.clone()).await;

        match connect_result {
            Ok(conn) => {
                // 连接成功，获取 RunningService 并热替换后端
                let running = conn.into_running_service();
                handler.swap_backend(Some(running));
                backoff_secs = 1;

                if !quiet {
                    eprintln!("✅ 重连成功，恢复代理服务");
                }

                // 监控连接健康
                let disconnect_reason = monitor_sse_connection(
                    &handler,
                    args.ping_interval,
                    args.ping_timeout,
                    quiet,
                ).await;

                // 连接断开，标记后端不可用
                handler.swap_backend(None);

                if !quiet {
                    eprintln!("⚠️  连接断开: {}", disconnect_reason);
                }
            }
            Err(e) => {
                let error_type = classify_error(&e);

                if max_retries > 0 && attempt >= max_retries {
                    if !quiet {
                        eprintln!("❌ 连接失败，已达最大重试次数 ({})", max_retries);
                        eprintln!("   错误类型: {}", error_type);
                        eprintln!("   错误详情: {}", e);
                    }
                    break;
                }

                if !quiet {
                    if max_retries == 0 {
                        eprintln!("⚠️  连接失败 [{}]: {}，{}秒后重连 (第{}次)...",
                            error_type, summarize_error(&e), backoff_secs, attempt);
                    } else {
                        eprintln!("⚠️  连接失败 [{}]: {}，{}秒后重连 ({}/{})...",
                            error_type, summarize_error(&e), backoff_secs, attempt, max_retries);
                    }
                }

                if verbose && !quiet {
                    eprintln!("   完整错误: {}", e);
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }
}

/// Stream 模式的 watchdog：负责监控连接健康、断开时重连
async fn run_stream_watchdog(
    handler: Arc<StreamProxyHandler>,
    args: ConvertArgs,
    config: McpClientConfig,
    _tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) {
    let max_retries = args.retries;
    let mut attempt = 0u32;
    let mut backoff_secs = 1u64;
    const MAX_BACKOFF_SECS: u64 = 30;

    // 首先监控现有连接的健康状态
    let disconnect_reason = monitor_stream_connection(
        &handler,
        args.ping_interval,
        args.ping_timeout,
        quiet,
    ).await;

    // 连接断开，标记后端不可用
    handler.swap_backend(None);

    if !quiet {
        eprintln!("⚠️  连接断开: {}", disconnect_reason);
    }

    // 进入重连循环
    loop {
        attempt += 1;

        if !quiet {
            eprintln!("🔗 正在重新连接 (第{}次尝试)...", attempt);
        }

        // 尝试建立连接
        let connect_result = StreamClientConnection::connect(config.clone()).await;

        match connect_result {
            Ok(conn) => {
                // 连接成功，获取 RunningService 并热替换后端
                let running = conn.into_running_service();
                handler.swap_backend(Some(running));
                backoff_secs = 1;

                if !quiet {
                    eprintln!("✅ 重连成功，恢复代理服务");
                }

                // 监控连接健康
                let disconnect_reason = monitor_stream_connection(
                    &handler,
                    args.ping_interval,
                    args.ping_timeout,
                    quiet,
                ).await;

                // 连接断开，标记后端不可用
                handler.swap_backend(None);

                if !quiet {
                    eprintln!("⚠️  连接断开: {}", disconnect_reason);
                }
            }
            Err(e) => {
                let error_type = classify_error(&e);

                if max_retries > 0 && attempt >= max_retries {
                    if !quiet {
                        eprintln!("❌ 连接失败，已达最大重试次数 ({})", max_retries);
                        eprintln!("   错误类型: {}", error_type);
                        eprintln!("   错误详情: {}", e);
                    }
                    break;
                }

                if !quiet {
                    if max_retries == 0 {
                        eprintln!("⚠️  连接失败 [{}]: {}，{}秒后重连 (第{}次)...",
                            error_type, summarize_error(&e), backoff_secs, attempt);
                    } else {
                        eprintln!("⚠️  连接失败 [{}]: {}，{}秒后重连 ({}/{})...",
                            error_type, summarize_error(&e), backoff_secs, attempt, max_retries);
                    }
                }

                if verbose && !quiet {
                    eprintln!("   完整错误: {}", e);
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }
}

/// SSE 模式：监控连接健康，返回断开原因
async fn monitor_sse_connection(
    handler: &ProxyHandler,
    ping_interval: u64,
    ping_timeout: u64,
    quiet: bool,
) -> String {
    if ping_interval == 0 {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !handler.is_backend_available() {
                return "后端连接已关闭".to_string();
            }
        }
    }

    let mut interval = tokio::time::interval(Duration::from_secs(ping_interval));
    interval.tick().await;

    loop {
        interval.tick().await;

        if !handler.is_backend_available() {
            return "后端连接已关闭".to_string();
        }

        let check_result = tokio::time::timeout(
            Duration::from_secs(ping_timeout),
            handler.is_terminated_async()
        ).await;

        match check_result {
            Ok(true) => return "Ping 检测失败（服务错误）".to_string(),
            Ok(false) => {}
            Err(_) => {
                if !quiet {
                    eprintln!("❌ Ping 检测超时（{}s）", ping_timeout);
                }
                return format!("Ping 检测超时（{}s）", ping_timeout);
            }
        }
    }
}

/// Stream 模式：监控连接健康，返回断开原因
async fn monitor_stream_connection(
    handler: &StreamProxyHandler,
    ping_interval: u64,
    ping_timeout: u64,
    quiet: bool,
) -> String {
    if ping_interval == 0 {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !handler.is_backend_available() {
                return "后端连接已关闭".to_string();
            }
        }
    }

    let mut interval = tokio::time::interval(Duration::from_secs(ping_interval));
    interval.tick().await;

    loop {
        interval.tick().await;

        if !handler.is_backend_available() {
            return "后端连接已关闭".to_string();
        }

        let check_result = tokio::time::timeout(
            Duration::from_secs(ping_timeout),
            handler.is_terminated_async()
        ).await;

        match check_result {
            Ok(true) => return "Ping 检测失败（服务错误）".to_string(),
            Ok(false) => {}
            Err(_) => {
                if !quiet {
                    eprintln!("❌ Ping 检测超时（{}s）", ping_timeout);
                }
                return format!("Ping 检测超时（{}s）", ping_timeout);
            }
        }
    }
}

/// 错误分类
fn classify_error(e: &anyhow::Error) -> &'static str {
    let err_str = e.to_string().to_lowercase();

    if err_str.contains("timeout") || err_str.contains("timed out") {
        "超时"
    } else if err_str.contains("connection refused") {
        "连接被拒绝"
    } else if err_str.contains("connection reset") {
        "连接被重置"
    } else if err_str.contains("dns") || err_str.contains("resolve") {
        "DNS解析失败"
    } else if err_str.contains("certificate") || err_str.contains("ssl") || err_str.contains("tls") {
        "SSL/TLS错误"
    } else if err_str.contains("session") {
        "会话错误"
    } else if err_str.contains("sending request") || err_str.contains("network") {
        "网络错误"
    } else if err_str.contains("eof") || err_str.contains("closed") || err_str.contains("shutdown") {
        "连接关闭"
    } else {
        "未知错误"
    }
}

/// 简化错误信息（用于单行日志）
fn summarize_error(e: &anyhow::Error) -> String {
    let full = e.to_string();
    // 截取第一行或前80个字符
    let first_line = full.lines().next().unwrap_or(&full);
    // 使用 chars() 安全处理 UTF-8 字符，避免在多字节字符中间截断
    if first_line.chars().count() > 80 {
        format!("{}...", first_line.chars().take(77).collect::<String>())
    } else {
        first_line.to_string()
    }
}

/// 命令模式执行（本地子进程）
/// 使用 SSE 库（rmcp 0.10）的类型
async fn run_command_mode(
    name: &str,
    command: &str,
    cmd_args: Vec<String>,
    env: HashMap<String, String>,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    if !quiet {
        eprintln!("🚀 MCP-Stdio-Proxy: {} (command) → stdio", name);
        eprintln!("   命令: {} {:?}", command, cmd_args);
        if verbose && !env.is_empty() {
            eprintln!("   环境变量: {:?}", env);
        }
    }

    // 显示过滤器配置
    if !quiet {
        if tool_filter.is_enabled() {
            eprintln!("🔧 工具过滤已启用");
        }
    }

    // 创建子进程命令
    let mut cmd = Command::new(command);
    cmd.args(&cmd_args);
    for (k, v) in &env {
        cmd.env(k, v);
    }

    // 启动子进程
    let tokio_process = TokioChildProcess::new(cmd)?;

    if !quiet {
        eprintln!("🔗 启动子进程...");
    }

    // 创建 ClientInfo（使用 SSE 库的类型，rmcp 0.10）
    let client_info = create_sse_client_info();

    // 连接到子进程
    let running = client_info.serve(tokio_process).await?;

    if !quiet {
        eprintln!("✅ 子进程已启动，开始代理转换...");

        // 打印工具列表
        match running.list_tools(None).await {
            Ok(tools_result) => {
                let tools = &tools_result.tools;
                if tools.is_empty() {
                    eprintln!("⚠️  工具列表为空 (tools/list 返回 0 个工具)");
                } else {
                    eprintln!("🔧 可用工具 ({} 个):", tools.len());
                    for tool in tools {
                        let desc = tool.description.as_deref().unwrap_or("无描述");
                        let desc_short = truncate_str(desc, 50);
                        eprintln!("   - {} : {}", tool.name, desc_short);
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  获取工具列表失败: {}", e);
            }
        }

        eprintln!("💡 现在可以通过 stdin 发送 JSON-RPC 请求");
    }

    // 使用 ProxyHandler + stdio 将本地 MCP 服务透明暴露为 stdio
    let proxy_handler = ProxyHandler::with_tool_filter(running, name.to_string(), tool_filter);
    let server = proxy_handler.serve(sse_stdio()).await?;
    server.waiting().await?;

    Ok(())
}

/// 创建 SSE 库的 ClientInfo（rmcp 0.10）
fn create_sse_client_info() -> SseClientInfo {
    SseClientInfo {
        protocol_version: Default::default(),
        capabilities: SseClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build(),
        client_info: SseImplementation {
            name: "mcp-proxy-cli".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: None,
            website_url: None,
            icons: None,
        },
    }
}

/// 获取协议名称
fn protocol_name(protocol: &super::protocol::McpProtocol) -> &'static str {
    match protocol {
        super::protocol::McpProtocol::Sse => "SSE",
        super::protocol::McpProtocol::Stream => "Streamable HTTP",
        super::protocol::McpProtocol::Stdio => "Stdio",
    }
}

/// 运行检查命令
async fn run_check_command(args: CheckArgs, _verbose: bool, quiet: bool) -> Result<()> {
    if !quiet {
        eprintln!("🔍 检查服务: {}", args.url);
    }

    match super::protocol::detect_mcp_protocol(&args.url).await {
        Ok(protocol) => {
            if !quiet {
                eprintln!("✅ 服务正常，检测到 {} 协议", protocol);
            }
            Ok(())
        }
        Err(e) => {
            if !quiet {
                eprintln!("❌ 服务检查失败: {}", e);
            }
            Err(e)
        }
    }
}

/// 运行协议检测命令
async fn run_detect_command(args: DetectArgs, _verbose: bool, quiet: bool) -> Result<()> {
    let protocol = super::protocol::detect_mcp_protocol(&args.url).await?;

    if quiet {
        println!("{}", protocol);
    } else {
        eprintln!("{}", protocol);
    }

    Ok(())
}