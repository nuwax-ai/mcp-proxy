//! Protocol Detection (Re-export Layer)
//!
//! This module re-exports the protocol detection function from mcp-streamable-proxy
//! and provides a convenient combined detection function.
//!
//! Detection logic: If Streamable HTTP → Stream, else → SSE (default)

// Re-export the detection function
pub use mcp_streamable_proxy::is_streamable_http;

use crate::model::McpProtocol;
use anyhow::Result;
use log::info;

/// Automatically detect the MCP service protocol type
///
/// Detection logic:
/// 1. First try to detect Streamable HTTP protocol
/// 2. If not Streamable HTTP, default to SSE (backward compatible)
///
/// # Arguments
///
/// * `url` - The URL to detect
///
/// # Returns
///
/// Returns the detected protocol type (Stream or Sse)
pub async fn detect_mcp_protocol(url: &str) -> Result<McpProtocol> {
    info!("开始自动检测 MCP 服务协议: {}", url);

    // Try Streamable HTTP first
    if is_streamable_http(url).await {
        info!("检测到 Streamable HTTP 协议: {}", url);
        return Ok(McpProtocol::Stream);
    }

    // Default to SSE (not Streamable HTTP means SSE)
    info!("默认使用 SSE 协议: {}", url);
    Ok(McpProtocol::Sse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_invalid_url() {
        // Invalid URL should default to SSE
        let result = detect_mcp_protocol("not-a-url").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), McpProtocol::Sse);
    }

    #[tokio::test]
    async fn test_detect_nonexistent_server() {
        // Non-existent server should default to SSE
        let result = detect_mcp_protocol("http://localhost:99999/mcp").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), McpProtocol::Sse);
    }
}
