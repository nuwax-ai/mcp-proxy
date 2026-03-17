//! Protocol Detection (Re-export Layer)
//!
//! This module re-exports protocol detection functions and provides
//! a convenient combined detection function.
//!
//! Detection logic: If SSE detected → Sse, else → Stream (default fallback)

// Re-export the detection functions
pub use mcp_sse_proxy::is_sse_with_headers;

use crate::model::McpProtocol;
use anyhow::Result;
use log::info;
use std::collections::HashMap;

/// Automatically detect the MCP service protocol type
///
/// Convenience wrapper around [`detect_mcp_protocol_with_headers`] that passes no
/// custom headers.
pub async fn detect_mcp_protocol(url: &str) -> Result<McpProtocol> {
    detect_mcp_protocol_with_headers(url, None).await
}

/// Automatically detect the MCP service protocol type, with optional custom headers
///
/// Detection logic:
/// 1. First try to detect SSE protocol (GET /sse returns text/event-stream)
/// 2. If not SSE, default to Streamable HTTP (modern MCP standard)
///
/// # Arguments
///
/// * `url` - The URL to detect
/// * `headers` - Optional custom headers to include in the detection request
///
/// # Returns
///
/// Returns the detected protocol type (Sse or Stream)
pub async fn detect_mcp_protocol_with_headers(
    url: &str,
    headers: Option<&HashMap<String, String>>,
) -> Result<McpProtocol> {
    info!("开始自动检测 MCP 服务协议: {}", url);

    // Try SSE first
    if is_sse_with_headers(url, headers).await {
        info!("检测到 SSE 协议: {}", url);
        return Ok(McpProtocol::Sse);
    }

    // Default to Streamable HTTP (modern MCP standard)
    info!("默认使用 Streamable HTTP 协议: {}", url);
    Ok(McpProtocol::Stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_invalid_url() {
        // Invalid URL should default to Stream
        let result = detect_mcp_protocol("not-a-url").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), McpProtocol::Stream);
    }

    #[tokio::test]
    async fn test_detect_nonexistent_server() {
        // Non-existent server should default to Stream
        let result = detect_mcp_protocol("http://localhost:99999/mcp").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), McpProtocol::Stream);
    }

    #[tokio::test]
    async fn test_detect_with_headers_nonexistent_server() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer test-token".to_string());
        let result =
            detect_mcp_protocol_with_headers("http://localhost:99999/mcp", Some(&headers)).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), McpProtocol::Stream);
    }
}
