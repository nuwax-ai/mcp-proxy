// MCP-Proxy CLI 简化实现 - 修复版本
// 直接使用 rmcp 库的功能，无需复杂的 trait 抽象

use std::collections::HashMap;

use clap::Parser;
use anyhow::{Result, bail};
use serde::Deserialize;
use tokio::process::Command;

use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{SseClientTransport, StreamableHttpClientTransport, TokioChildProcess, sse_client::SseClientConfig, streamable_http_client::StreamableHttpClientTransportConfig, stdio},
};
use crate::proxy::{ProxyHandler, ToolFilter};

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

    /// 连接超时时间（秒）
    #[arg(long, default_value = "300", help = "连接超时时间（秒），默认5分钟")]
    pub timeout: u64,

    /// 重试次数
    #[arg(long, default_value = "3", help = "重试次数")]
    pub retries: u32,

    /// 工具白名单（逗号分隔），只允许指定的工具
    #[arg(long, value_delimiter = ',', help = "工具白名单（逗号分隔），只允许指定的工具")]
    pub allow_tools: Option<Vec<String>>,

    /// 工具黑名单（逗号分隔），排除指定的工具
    #[arg(long, value_delimiter = ',', help = "工具黑名单（逗号分隔），排除指定的工具")]
    pub deny_tools: Option<Vec<String>>,
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
                    timeout: 300,  // 5分钟，匹配 ProxyHandler 的工具调用超时
                    retries: 3,
                    allow_tools: None,
                    deny_tools: None,
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

    // 配置客户端能力
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

    // 根据配置源执行不同逻辑
    match config_source {
        McpConfigSource::DirectUrl { url } => {
            // 直接 URL 模式（原有逻辑）
            run_url_mode(&args, &url, HashMap::new(), None, client_info, tool_filter, verbose, quiet).await
        }
        McpConfigSource::RemoteService { name, url, protocol, headers, timeout } => {
            // 远程服务配置模式
            if !quiet {
                eprintln!("🚀 MCP-Stdio-Proxy: {} ({}) → stdio", name, url);
            }
            // 合并 headers：配置 + 命令行
            let merged_headers = merge_headers(headers, &args.header, args.auth.as_ref());
            run_url_mode(&args, &url, merged_headers, protocol.or(timeout.map(|_| super::protocol::McpProtocol::Stream)), client_info, tool_filter, verbose, quiet).await
        }
        McpConfigSource::LocalCommand { name, command, args: cmd_args, env } => {
            // 本地命令模式
            run_command_mode(&name, &command, cmd_args, env, client_info, tool_filter, verbose, quiet).await
        }
    }
}

/// URL 模式执行（远程 HTTP/SSE 服务）
async fn run_url_mode(
    args: &ConvertArgs,
    url: &str,
    merged_headers: HashMap<String, String>,
    config_protocol: Option<super::protocol::McpProtocol>,
    client_info: ClientInfo,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    if !quiet && merged_headers.is_empty() {
        eprintln!("🚀 MCP-Stdio-Proxy: {} → stdio", url);
    }

    if verbose && !quiet {
        eprintln!("📡 超时: {}s, 重试: {}", args.timeout, args.retries);
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
        // 命令行指定协议
        let detected = match proto {
            super::proxy_server::ProxyProtocol::Sse => super::protocol::McpProtocol::Sse,
            super::proxy_server::ProxyProtocol::Stream => super::protocol::McpProtocol::Stream,
        };
        if !quiet {
            eprintln!("🔧 使用指定协议: {}", protocol_name(&detected));
        }
        detected
    } else if let Some(proto) = config_protocol {
        // 配置文件指定协议
        if !quiet {
            eprintln!("🔧 使用配置协议: {}", protocol_name(&proto));
        }
        proto
    } else {
        // 自动检测协议
        let detected = super::protocol::detect_mcp_protocol(url).await?;
        if !quiet {
            eprintln!("🔍 检测到 {} 协议", protocol_name(&detected));
        }
        detected
    };

    if !quiet {
        eprintln!("🔗 建立连接...");
    }

    // 构建带认证与自定义头的 HTTP 客户端
    let http_client = create_http_client_with_headers(&merged_headers, &args.header, args.auth.as_ref(), args.timeout)?;

    // 为不同协议创建传输并启动 rmcp 客户端
    let running = match protocol {
        super::protocol::McpProtocol::Sse => {
            let cfg = SseClientConfig {
                sse_endpoint: url.to_string().into(),
                ..Default::default()
            };
            let transport = SseClientTransport::start_with_client(http_client, cfg).await?;
            client_info.serve(transport).await?
        }
        super::protocol::McpProtocol::Stream => {
            let cfg = StreamableHttpClientTransportConfig {
                uri: url.to_string().into(),
                ..Default::default()
            };
            let transport = StreamableHttpClientTransport::with_client(http_client, cfg);
            client_info.serve(transport).await?
        }
        super::protocol::McpProtocol::Stdio => {
            bail!("Stdio 协议不支持通过 URL 转换，请使用 --config 配置本地命令")
        }
    };

    if !quiet {
        eprintln!("✅ 连接成功，开始代理转换...");

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
                        let desc_short = if desc.chars().count() > 50 {
                            format!("{}...", desc.chars().take(50).collect::<String>())
                        } else {
                            desc.to_string()
                        };
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

    // 使用 ProxyHandler + stdio 将远程 MCP 服务透明暴露为本地 stdio
    let proxy_handler = ProxyHandler::with_tool_filter(running, "cli".to_string(), tool_filter);
    let stdio_transport = stdio();
    let server = proxy_handler.serve(stdio_transport).await?;
    server.waiting().await?;

    Ok(())
}

/// 命令模式执行（本地子进程）
async fn run_command_mode(
    name: &str,
    command: &str,
    cmd_args: Vec<String>,
    env: HashMap<String, String>,
    client_info: ClientInfo,
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
                        let desc_short = if desc.chars().count() > 50 {
                            format!("{}...", desc.chars().take(50).collect::<String>())
                        } else {
                            desc.to_string()
                        };
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
    let stdio_transport = stdio();
    let server = proxy_handler.serve(stdio_transport).await?;
    server.waiting().await?;

    Ok(())
}

/// 获取协议名称
fn protocol_name(protocol: &super::protocol::McpProtocol) -> &'static str {
    match protocol {
        super::protocol::McpProtocol::Sse => "SSE",
        super::protocol::McpProtocol::Stream => "Streamable HTTP",
        super::protocol::McpProtocol::Stdio => "Stdio",
    }
}

/// 创建 HTTP 客户端（使用合并后的 headers）
fn create_http_client_with_headers(
    config_headers: &HashMap<String, String>,
    cli_headers: &[(String, String)],
    cli_auth: Option<&String>,
    timeout: u64,
) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();

    // 1. 先添加配置中的 headers
    for (key, value) in config_headers {
        headers.insert(
            key.parse::<reqwest::header::HeaderName>()?,
            value.parse()?,
        );
    }

    // 2. 命令行 -H 参数覆盖
    for (key, value) in cli_headers {
        headers.insert(
            key.parse::<reqwest::header::HeaderName>()?,
            value.parse()?,
        );
    }

    // 3. 命令行 --auth 参数优先级最高
    if let Some(auth) = cli_auth {
        headers.insert("Authorization", auth.parse()?);
    }

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(tokio::time::Duration::from_secs(timeout))
        .build()?;

    Ok(client)
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