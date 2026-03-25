//! 协议转换核心逻辑
//!
//! 处理协议转换的主要流程，包括 URL 模式、协议检测等

use anyhow::Result;
use std::collections::HashMap;

use crate::client::support::{ConvertArgs, protocol_name};
use crate::proxy::{McpClientConfig, ToolFilter};
use crate::t;

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
    tracing::info!("{}", t!("cli.convert.starting"));
    tracing::info!("{}", t!("cli.convert.target_url", url = url));
    tracing::debug!("Headers 数量: {}", merged_headers.len());
    tracing::debug!(
        "Ping 间隔: {}s, Ping 超时: {}s",
        args.ping_interval,
        args.ping_timeout
    );
    tracing::debug!("重试次数: {} (0=无限)", args.retries);

    if !quiet && merged_headers.is_empty() {
        eprintln!("🚀 MCP-Stdio-Proxy: {} → stdio", url);
    }

    // 显示过滤器配置
    if !quiet {
        if let Some(ref allow_tools) = args.allow_tools {
            tracing::info!("{}", t!("cli.convert.tool_whitelist", tools = format!("{:?}", allow_tools)));
        }
        if let Some(ref deny_tools) = args.deny_tools {
            tracing::info!("{}", t!("cli.convert.tool_blacklist", tools = format!("{:?}", deny_tools)));
        }
    }

    // 确定协议类型：命令行参数 > 配置文件 > 自动检测
    let protocol = if let Some(ref proto) = args.protocol {
        let detected = match proto {
            crate::client::proxy_server::ProxyProtocol::Sse => {
                crate::client::protocol::McpProtocol::Sse
            }
            crate::client::proxy_server::ProxyProtocol::Stream => {
                crate::client::protocol::McpProtocol::Stream
            }
        };
        tracing::info!("{}", t!("cli.convert.protocol_specified", protocol = protocol_name(&detected)));
        if !quiet {
            eprintln!("🔧 使用指定协议: {}", protocol_name(&detected));
        }
        detected
    } else if let Some(proto) = config_protocol {
        tracing::info!("{}", t!("cli.convert.protocol_config", protocol = protocol_name(&proto)));
        if !quiet {
            eprintln!("🔧 使用配置协议: {}", protocol_name(&proto));
        }
        proto
    } else {
        tracing::info!("{}", t!("cli.convert.detecting_protocol"));
        if !quiet {
            eprintln!("🔍 正在检测协议...");
        }
        let detection_start = std::time::Instant::now();
        let detected = crate::client::protocol::detect_mcp_protocol(url)
            .await
            .map_err(|e| {
                tracing::error!("{}", t!("cli.convert.detect_failed", error = e.to_string()));
                e
            })?;
        let detection_duration = detection_start.elapsed();
        tracing::info!(
            "{}",
            t!("cli.convert.detect_complete",
                protocol = protocol_name(&detected),
                duration = format!("{:?}", detection_duration)
            )
        );
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
    tracing::info!("{}", t!("cli.convert.using_protocol", protocol = protocol_name(&protocol)));
    match protocol {
        crate::client::protocol::McpProtocol::Sse => {
            run_sse_mode(config, args.clone(), tool_filter, verbose, quiet).await
        }
        crate::client::protocol::McpProtocol::Stream => {
            run_stream_mode(config, args.clone(), tool_filter, verbose, quiet).await
        }
        crate::client::protocol::McpProtocol::Stdio => {
            tracing::error!("{}", t!("cli.convert.stdio_url_not_supported"));
            anyhow::bail!("Stdio protocol does not support URL conversion, please use --config for local commands")
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
