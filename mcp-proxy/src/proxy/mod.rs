//! Proxy module - re-exports handler types from proxy libraries
//!
//! This module provides a unified interface for proxy handlers by re-exporting
//! types from mcp-sse-proxy, mcp-streamable-proxy, and mcp-common libraries.

// Re-export SseHandler as ProxyHandler for backward compatibility
// SseHandler is used because it's based on rmcp 0.10 which supports both
// SSE server mode and CLI stdio mode used in the main project
pub use mcp_sse_proxy::SseHandler as ProxyHandler;

// Re-export StreamProxyHandler with an alias to distinguish from SSE ProxyHandler
// Both mcp-sse-proxy and mcp-streamable-proxy export ProxyHandler, so we use an alias
pub use mcp_streamable_proxy::ProxyHandler as StreamProxyHandler;

// Re-export ToolFilter from mcp-common
pub use mcp_common::ToolFilter;

// Re-export client connection types for high-level API (each from its own library)
pub use mcp_sse_proxy::{McpClientConfig, SseClientConnection};
pub use mcp_streamable_proxy::StreamClientConnection;

// Re-export Builder APIs for server creation
pub use mcp_sse_proxy::server_builder::{BackendConfig as SseBackendConfig, SseServerBuilder};
pub use mcp_streamable_proxy::server_builder::{
    BackendConfig as StreamBackendConfig, StreamServerBuilder,
};

/// Unified handler enum that can hold either SSE or Stream handler
///
/// This allows ProxyHandlerManager to store handlers of either type
/// while providing a common interface for status checks.
#[derive(Clone, Debug)]
pub enum McpHandler {
    /// SSE protocol handler (from mcp-sse-proxy)
    Sse(ProxyHandler),
    /// Streamable HTTP protocol handler (from mcp-streamable-proxy)
    Stream(StreamProxyHandler),
}

impl McpHandler {
    /// Check if the underlying MCP server is ready
    pub async fn is_mcp_server_ready(&self) -> bool {
        match self {
            McpHandler::Sse(h) => h.is_mcp_server_ready().await,
            McpHandler::Stream(h) => h.is_mcp_server_ready().await,
        }
    }

    /// Check if the backend connection is terminated
    pub async fn is_terminated_async(&self) -> bool {
        match self {
            McpHandler::Sse(h) => h.is_terminated_async().await,
            McpHandler::Stream(h) => h.is_terminated_async().await,
        }
    }
}

impl From<ProxyHandler> for McpHandler {
    fn from(handler: ProxyHandler) -> Self {
        McpHandler::Sse(handler)
    }
}

impl From<StreamProxyHandler> for McpHandler {
    fn from(handler: StreamProxyHandler) -> Self {
        McpHandler::Stream(handler)
    }
}
