//! 协议转换核心逻辑
//!
//! 处理协议转换的主要流程，包括 URL 模式、协议检测等

use anyhow::Result;
use std::collections::HashMap;

use crate::client::support::{ConvertArgs, protocol_name};
use crate::proxy::{McpClientConfig, ToolFilter};

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
    tracing::info!("Starting protocol conversion");
    tracing::info!("Target URL: {url}");
    tracing::debug!("Header count: {}", merged_headers.len());
    tracing::debug!(
        "Ping interval: {}s, ping timeout: {}s",
        args.ping_interval,
        args.ping_timeout
    );
    tracing::debug!("Retry count: {} (0 = unlimited)", args.retries);

    if !quiet && merged_headers.is_empty() {
        eprintln!("🚀 MCP-Stdio-Proxy: {} → stdio", url);
    }

    // 显示过滤器配置
    if !quiet {
        if let Some(ref allow_tools) = args.allow_tools {
            tracing::info!("Tool allowlist: {:?}", allow_tools);
        }
        if let Some(ref deny_tools) = args.deny_tools {
            tracing::info!("Tool denylist: {:?}", deny_tools);
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
        tracing::info!(
            "Using protocol from CLI argument: {}",
            protocol_name(&detected)
        );
        if !quiet {
            eprintln!("🔧 Using protocol from CLI: {}", protocol_name(&detected));
        }
        detected
    } else if let Some(proto) = config_protocol {
        tracing::info!("Using protocol from config: {}", protocol_name(&proto));
        if !quiet {
            eprintln!("🔧 Using protocol from config: {}", protocol_name(&proto));
        }
        proto
    } else {
        tracing::info!("Detecting protocol...");
        if !quiet {
            eprintln!("🔍 Detecting protocol...");
        }
        let detection_start = std::time::Instant::now();
        let detected = crate::client::protocol::detect_mcp_protocol(url)
            .await
            .map_err(|e| {
                tracing::error!("Protocol detection failed: {}", e);
                e
            })?;
        let detection_duration = detection_start.elapsed();
        tracing::info!(
            "Protocol detection completed: protocol={}, duration={:?}",
            protocol_name(&detected),
            detection_duration
        );
        if !quiet {
            eprintln!("🔍 Detected protocol: {}", protocol_name(&detected));
        }
        detected
    };

    // 构建 McpClientConfig
    tracing::debug!("Building MCP client config...");
    let config = build_mcp_config(url, &merged_headers, args.auth.as_ref());
    tracing::debug!("MCP client config ready");

    // 根据协议类型分支处理
    tracing::info!("Using protocol: {}", protocol_name(&protocol));
    match protocol {
        crate::client::protocol::McpProtocol::Sse => {
            run_sse_mode(config, args.clone(), tool_filter, verbose, quiet)
                .await
                .map_err(|e| {
                    tracing::error!("SSE mode failed: {:?}", e);
                    eprintln!("❌ SSE mode failed: {}", e);
                    e
                })
        }
        crate::client::protocol::McpProtocol::Stream => {
            run_stream_mode(config, args.clone(), tool_filter, verbose, quiet)
                .await
                .map_err(|e| {
                    tracing::error!("Stream mode failed: {:?}", e);
                    eprintln!("❌ Stream mode failed: {}", e);
                    e
                })
        }
        crate::client::protocol::McpProtocol::Stdio => {
            tracing::error!("Stdio protocol does not support URL conversion");
            anyhow::bail!(
                "Stdio protocol does not support URL conversion, please use --config for local commands"
            )
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
        // Authorization header: 确保有 "Bearer " 前缀，与 Server 模式行为一致
        if k.eq_ignore_ascii_case("Authorization") {
            let value = if v.starts_with("Bearer ") {
                v.clone()
            } else {
                format!("Bearer {}", v)
            };
            config = config.with_header(k, value);
        } else {
            config = config.with_header(k, v);
        }
    }
    if let Some(auth_value) = auth {
        // 命令行 --auth 参数不带 "Bearer " 前缀，直接添加
        config = config.with_header("Authorization", auth_value);
    }
    config
}
