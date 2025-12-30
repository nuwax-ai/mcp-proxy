//! Proxy module - re-exports handler types from proxy libraries
//!
//! This module provides a unified interface for proxy handlers by re-exporting
//! types from mcp-sse-proxy and mcp-common libraries.

// Re-export SseHandler as ProxyHandler for backward compatibility
// SseHandler is used because it's based on rmcp 0.10 which supports both
// SSE server mode and CLI stdio mode used in the main project
pub use mcp_sse_proxy::SseHandler as ProxyHandler;

// Re-export ToolFilter from mcp-common
pub use mcp_common::ToolFilter;
