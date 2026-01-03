//! 协议转换核心逻辑
//!
//! 处理协议转换的主要流程，包括 URL 模式、协议检测等

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

use crate::proxy::{ProxyHandler, ToolFilter, McpClientConfig};
use crate::client::support::{ConvertArgs, protocol_name};

use super::sse::run_sse_mode;
use super::stream::run_stream_mode;

/// URL 模式执行（带自动重连）
/// 使用分支逻辑：根据协议类型调用不同的处理函数
pub async fn run_url_mode_with_retry(
    args: &ConvertArgs,
    url: &str,
    merged_headers: HashMap<String, String>,
    config_protocol: Option<crate::client::protocol::McpProtocol>,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    tracing::info!("开始 URL 模式处理");
    tracing::info!("目标 URL: {}", url);
    tracing::debug!("Headers 数量: {}", merged_headers.len());
    tracing::debug!("Ping 间隔: {}s, Ping 超时: {}s", args.ping_interval, args.ping_timeout);
    tracing::debug!("重试次数: {} (0=无限)", args.retries);

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
            crate::client::proxy_server::ProxyProtocol::Sse => crate::client::protocol::McpProtocol::Sse,
            crate::client::proxy_server::ProxyProtocol::Stream => crate::client::protocol::McpProtocol::Stream,
        };
        tracing::info!("使用命令行指定协议: {}", protocol_name(&detected));
        if !quiet {
            eprintln!("🔧 使用指定协议: {}", protocol_name(&detected));
        }
        detected
    } else if let Some(proto) = config_protocol {
        tracing::info!("使用配置文件协议: {}", protocol_name(&proto));
        if !quiet {
            eprintln!("🔧 使用配置协议: {}", protocol_name(&proto));
        }
        proto
    } else {
        tracing::info!("开始自动检测协议...");
        if !quiet {
            eprintln!("🔍 正在检测协议...");
        }
        let detection_start = std::time::Instant::now();
        let detected = crate::client::protocol::detect_mcp_protocol(url).await
            .map_err(|e| {
                tracing::error!("协议检测失败: {}", e);
                e
            })?;
        let detection_duration = detection_start.elapsed();
        tracing::info!("协议检测完成: {} (耗时: {:?})", protocol_name(&detected), detection_duration);
        if !quiet {
            eprintln!("🔍 检测到 {} 协议", protocol_name(&detected));
        }
        detected
    };

    // 构建 McpClientConfig
    tracing::debug!("构建 MCP 客户端配置...");
    let config = build_mcp_config(url, &merged_headers, args.auth.as_ref());
    tracing::debug!("MCP 客户端配置构建完成");

    // 根据协议类型分支处理
    tracing::info!("使用 {} 协议模式", protocol_name(&protocol));
    match protocol {
        crate::client::protocol::McpProtocol::Sse => {
            run_sse_mode(config, args.clone(), tool_filter, verbose, quiet).await
        }
        crate::client::protocol::McpProtocol::Stream => {
            run_stream_mode(config, args.clone(), tool_filter, verbose, quiet).await
        }
        crate::client::protocol::McpProtocol::Stdio => {
            tracing::error!("Stdio 协议不支持通过 URL 转换");
            anyhow::bail!("Stdio 协议不支持通过 URL 转换，请使用 --config 配置本地命令")
        }
    }
}

/// 构建 McpClientConfig
pub fn build_mcp_config(
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
