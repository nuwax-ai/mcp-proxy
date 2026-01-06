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
