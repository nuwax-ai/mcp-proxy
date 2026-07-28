//! 协议转换核心逻辑
//!
//! 处理协议转换的主要流程，包括 URL 模式、协议检测等

use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;

use super::remote_runtime::RuntimeOptions;
use super::sse::run_sse_mode;
use super::stream::run_stream_mode;
use crate::client::protocol::McpProtocol;
use crate::client::proxy_server::ProxyProtocol;
use crate::client::support::{ConvertArgs, merge_headers, protocol_name};
use crate::proxy::{McpClientConfig, ToolFilter};

pub struct UrlModeTarget {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub protocol: Option<crate::client::protocol::McpProtocol>,
    pub timeout_secs: Option<u64>,
}

/// URL 模式执行（带自动重连）
/// 使用分支逻辑：根据协议类型调用不同的处理函数
pub async fn run_url_mode_with_retry(
    args: ConvertArgs,
    target: UrlModeTarget,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    let UrlModeTarget {
        url,
        headers,
        protocol: config_protocol,
        timeout_secs,
    } = target;
    tracing::info!("Starting protocol conversion");
    tracing::info!("Target URL: {url}");
    tracing::debug!("Header count: {}", headers.len());
    tracing::debug!(
        "Ping interval: {}s, ping timeout: {}s",
        args.ping_interval,
        args.ping_timeout
    );
    tracing::debug!("Retry count: {} (0 = unlimited)", args.retries);

    if !quiet && headers.is_empty() {
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

    let discovery_mode = super::discovery_options::prepare(&args, config_protocol.as_ref())?;
    let runtime_options = RuntimeOptions::from_args(&args, verbose, quiet);

    let protocol = resolve_protocol(
        args.protocol.as_ref(),
        config_protocol,
        &url,
        &headers,
        quiet,
    )
    .await?;

    // 构建 McpClientConfig
    tracing::debug!("Building MCP client config...");
    let config = build_mcp_config(&url, &headers, None, timeout_secs);
    tracing::debug!("MCP client config ready");

    // 根据协议类型分支处理
    tracing::info!("Using protocol: {}", protocol_name(&protocol));
    match protocol {
        crate::client::protocol::McpProtocol::Sse => {
            run_sse_mode(config, discovery_mode, tool_filter, runtime_options)
                .await
                .map_err(|e| {
                    tracing::error!("SSE mode failed: {:?}", e);
                    eprintln!("❌ SSE mode failed: {}", e);
                    e
                })
        }
        crate::client::protocol::McpProtocol::Stream => {
            run_stream_mode(config, discovery_mode, tool_filter, runtime_options)
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

async fn resolve_protocol(
    cli_protocol: Option<&ProxyProtocol>,
    config_protocol: Option<McpProtocol>,
    url: &str,
    headers: &HashMap<String, String>,
    quiet: bool,
) -> Result<McpProtocol> {
    if let Some(protocol) = cli_protocol {
        let resolved = match protocol {
            ProxyProtocol::Sse => McpProtocol::Sse,
            ProxyProtocol::Stream => McpProtocol::Stream,
        };
        tracing::info!(protocol = protocol_name(&resolved), "Using CLI protocol");
        if !quiet {
            eprintln!("🔧 Using protocol from CLI: {}", protocol_name(&resolved));
        }
        return Ok(resolved);
    }

    if let Some(protocol) = config_protocol {
        tracing::info!(
            protocol = protocol_name(&protocol),
            "Using configured protocol"
        );
        if !quiet {
            eprintln!(
                "🔧 Using protocol from config: {}",
                protocol_name(&protocol)
            );
        }
        return Ok(protocol);
    }

    if !quiet {
        eprintln!("🔍 Detecting protocol...");
    }
    let started = std::time::Instant::now();
    let detected =
        crate::client::protocol::detect_mcp_protocol_with_headers(url, Some(headers)).await?;
    tracing::info!(
        protocol = protocol_name(&detected),
        elapsed = ?started.elapsed(),
        "Protocol detection completed"
    );
    if !quiet {
        eprintln!("🔍 Detected protocol: {}", protocol_name(&detected));
    }
    Ok(detected)
}

/// 构建 McpClientConfig
pub fn build_mcp_config(
    url: &str,
    headers: &HashMap<String, String>,
    auth: Option<&String>,
    timeout_secs: Option<u64>,
) -> McpClientConfig {
    let mut config = McpClientConfig::new(url);
    let final_headers = merge_headers(headers.clone(), &[], auth);
    for (key, value) in final_headers {
        config = config.with_header(key, value);
    }
    if let Some(timeout_secs) = timeout_secs {
        let timeout = Duration::from_secs(timeout_secs);
        config = config
            .with_connect_timeout(timeout)
            .with_read_timeout(timeout);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_timeout_applies_to_connect_and_read() {
        let config = build_mcp_config("http://localhost", &HashMap::new(), None, Some(7));

        assert_eq!(config.connect_timeout, Some(Duration::from_secs(7)));
        assert_eq!(config.read_timeout, Some(Duration::from_secs(7)));
    }

    #[test]
    fn complete_authorization_header_is_not_rewritten() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "ApiKey secret".to_string());

        let config = build_mcp_config("http://localhost", &headers, None, None);

        assert_eq!(
            config.headers.get("Authorization").map(String::as_str),
            Some("ApiKey secret")
        );
    }

    #[test]
    fn explicit_auth_argument_remains_complete_and_wins() {
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer old".to_string());
        let auth = "Basic final".to_string();

        let config = build_mcp_config("http://localhost", &headers, Some(&auth), None);

        assert_eq!(config.headers.len(), 1);
        assert_eq!(
            config.headers.get("Authorization").map(String::as_str),
            Some("Basic final")
        );
    }
}
