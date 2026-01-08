//! CLI 参数定义
//!
//! 定义所有命令行参数结构体

use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;

/// 通用日志配置参数
///
/// 用于多个命令之间共享日志配置
#[derive(Parser, Debug, Clone)]
pub struct LoggingArgs {
    /// 启用详细诊断模式，输出连接和工具调用的详细时间信息（默认启用）
    #[arg(
        long,
        default_value = "false",
        help = "启用详细诊断模式，追踪连接生命周期和超时问题（默认启用）"
    )]
    pub diagnostic: bool,

    /// 日志输出目录（自动生成文件名）
    #[arg(long, help = "日志输出目录，将自动生成日志文件名")]
    pub log_dir: Option<PathBuf>,

    /// 日志文件完整路径（手动指定）
    #[arg(long, conflicts_with = "log_dir", help = "日志文件完整路径")]
    pub log_file: Option<PathBuf>,

    /// OTLP 追踪端点（如 http://localhost:4317）
    ///
    /// 启用 diagnostic 模式时，可配置此参数将追踪数据发送到 Jaeger 等 OTLP 兼容的后端。
    /// 支持 gRPC (端口 4317) 和 HTTP (端口 4318) 协议。
    #[arg(
        long,
        env = "OTEL_EXPORTER_OTLP_ENDPOINT",
        help = "OTLP 追踪端点 (如 http://localhost:4317)"
    )]
    pub otlp_endpoint: Option<String>,

    /// 追踪服务名称（用于 Jaeger 等追踪后端标识）
    #[arg(
        long,
        default_value = "mcp-proxy",
        help = "追踪服务名称（用于 Jaeger 等追踪后端标识）"
    )]
    pub service_name: String,
}

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
    Proxy(crate::client::proxy_server::ProxyArgs),

    /// 健康检查 - 验证 MCP 服务是否可用
    Health(HealthArgs),
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
    pub config_file: Option<PathBuf>,

    /// MCP 服务名称（多服务配置时必需）
    #[arg(short, long, help = "MCP 服务名称（多服务配置时必需）")]
    pub name: Option<String>,

    /// 指定远程服务协议类型（不指定则自动检测）
    #[arg(long, value_enum, help = "指定远程服务协议类型（不指定则自动检测）")]
    pub protocol: Option<crate::client::proxy_server::ProxyProtocol>,

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
    #[arg(
        long,
        value_delimiter = ',',
        help = "工具白名单（逗号分隔），只允许指定的工具"
    )]
    pub allow_tools: Option<Vec<String>>,

    /// 工具黑名单（逗号分隔），排除指定的工具
    #[arg(
        long,
        value_delimiter = ',',
        help = "工具黑名单（逗号分隔），排除指定的工具"
    )]
    pub deny_tools: Option<Vec<String>>,

    /// 客户端 ping 间隔（秒），0 表示禁用
    #[arg(
        long,
        default_value = "30",
        help = "客户端 ping 间隔（秒），0 表示禁用"
    )]
    pub ping_interval: u64,

    /// 客户端 ping 超时（秒）
    #[arg(
        long,
        default_value = "10",
        help = "客户端 ping 超时（秒），超时则认为连接断开"
    )]
    pub ping_timeout: u64,

    /// 日志配置（使用通用结构）
    #[command(flatten)]
    pub logging: LoggingArgs,
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

/// 健康检查参数
#[derive(Parser, Debug)]
#[command(after_help = "\
退出码:
  0  服务健康 - MCP 连接握手成功
  1  服务不健康 - 连接失败、超时或握手失败

示例:
  # 基本用法
  mcp-proxy health http://localhost:8080/mcp

  # 带认证
  mcp-proxy health http://localhost:8080/mcp -a \"Bearer token123\"

  # 指定协议和超时
  mcp-proxy health http://localhost:8080/mcp --protocol sse --timeout 5

  # 静默模式（仅返回退出码，适合脚本使用）
  mcp-proxy health http://localhost:8080/mcp -q

  # 在 shell 脚本中使用
  if mcp-proxy health http://localhost:8080/mcp -q; then
      echo \"MCP 服务正常\"
  else
      echo \"MCP 服务不可用\"
  fi
")]
pub struct HealthArgs {
    /// 要检查的 MCP 服务 URL
    #[arg(value_name = "URL")]
    pub url: String,

    /// 认证 header (如: "Bearer token")
    #[arg(short, long, help = "认证 header")]
    pub auth: Option<String>,

    /// 自定义 HTTP headers
    #[arg(short = 'H', long, value_parser = parse_key_val, help = "自定义 HTTP headers (KEY=VALUE 格式)")]
    pub header: Vec<(String, String)>,

    /// 超时时间（秒）
    #[arg(long, default_value = "10")]
    pub timeout: u64,

    /// 指定远程服务协议类型（不指定则自动检测）
    #[arg(long, value_enum, help = "指定远程服务协议类型（不指定则自动检测）")]
    pub protocol: Option<crate::client::proxy_server::ProxyProtocol>,
}

/// 解析 KEY=VALUE 格式的辅助函数
pub fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("无效的 KEY=VALUE 格式: {}", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}
