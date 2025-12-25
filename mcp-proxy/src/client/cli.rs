// MCP-Proxy CLI 简化实现 - 修复版本
// 直接使用 rmcp 库的功能，无需复杂的 trait 抽象

use clap::Parser;
use anyhow::{Result, bail};

use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{SseClientTransport, StreamableHttpClientTransport, sse_client::SseClientConfig, streamable_http_client::StreamableHttpClientTransportConfig, stdio},
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
    /// MCP 服务的 URL 地址
    #[arg(value_name = "URL", help = "MCP 服务的 URL 地址")]
    pub url: String,

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
    #[arg(long, default_value = "30", help = "连接超时时间（秒）")]
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
                    url,
                    protocol: None,
                    auth: None,
                    header: vec![],
                    timeout: 30,
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

    if !quiet {
        eprintln!("🚀 MCP-Stdio-Proxy: {} → stdio", args.url);
        if verbose {
            eprintln!("📡 超时: {}s, 重试: {}", args.timeout, args.retries);
        }
        // 显示过滤器配置
        if let Some(ref allow_tools) = args.allow_tools {
            eprintln!("🔧 工具白名单: {:?}", allow_tools);
        }
        if let Some(ref deny_tools) = args.deny_tools {
            eprintln!("🔧 工具黑名单: {:?}", deny_tools);
        }
    }

    // 确定协议类型：手动指定或自动检测
    let protocol = if let Some(ref proto) = args.protocol {
        // 手动指定协议
        let detected = match proto {
            super::proxy_server::ProxyProtocol::Sse => super::protocol::McpProtocol::Sse,
            super::proxy_server::ProxyProtocol::Stream => super::protocol::McpProtocol::Stream,
        };
        if !quiet {
            let proto_name = match detected {
                super::protocol::McpProtocol::Sse => "SSE",
                super::protocol::McpProtocol::Stream => "Streamable HTTP",
                super::protocol::McpProtocol::Stdio => "Stdio",
            };
            eprintln!("🔧 使用指定协议: {}", proto_name);
        }
        detected
    } else {
        // 自动检测协议
        let detected = super::protocol::detect_mcp_protocol(&args.url).await?;
        if !quiet {
            let proto_name = match detected {
                super::protocol::McpProtocol::Sse => "SSE",
                super::protocol::McpProtocol::Stream => "Streamable HTTP",
                super::protocol::McpProtocol::Stdio => "Stdio",
            };
            eprintln!("🔍 检测到 {} 协议", proto_name);
        }
        detected
    };

    if !quiet {
        eprintln!("🔗 建立连接...");
    }

    // 构建带认证与自定义头的 HTTP 客户端
    let http_client = create_http_client(&args)?;

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

    // 为不同协议创建传输并启动 rmcp 客户端
    let running = match protocol {
        super::protocol::McpProtocol::Sse => {
            let cfg = SseClientConfig {
                sse_endpoint: args.url.clone().into(),
                ..Default::default()
            };
            let transport = SseClientTransport::start_with_client(http_client, cfg).await?;
            client_info.serve(transport).await?
        }
        super::protocol::McpProtocol::Stream => {
            let cfg = StreamableHttpClientTransportConfig {
                uri: args.url.clone().into(),
                ..Default::default()
            };
            let transport = StreamableHttpClientTransport::with_client(http_client, cfg);
            client_info.serve(transport).await?
        }
        super::protocol::McpProtocol::Stdio => {
            bail!("Stdio 协议不支持通过 URL 转换，请使用命令行模式")
        }
    };

    if !quiet {
        eprintln!("✅ 连接成功，开始代理转换...");
        eprintln!("💡 现在可以通过 stdin 发送 JSON-RPC 请求");
    }

    // 使用 ProxyHandler + stdio 将远程 MCP 服务透明暴露为本地 stdio
    let proxy_handler = ProxyHandler::with_tool_filter(running, "cli".to_string(), tool_filter);
    let stdio_transport = stdio();
    let server = proxy_handler.serve(stdio_transport).await?;
    server.waiting().await?;

    Ok(())
}






/// 创建 HTTP 客户端
fn create_http_client(args: &ConvertArgs) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    
    // 添加认证 header
    if let Some(auth) = &args.auth {
        headers.insert("Authorization", auth.parse()?);
    }
    
    // 添加自定义 headers
    for (key, value) in &args.header {
        headers.insert(key.parse::<reqwest::header::HeaderName>()?, value.parse()?);
    }
    
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(tokio::time::Duration::from_secs(args.timeout))
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