//! MCP SSE Proxy Module
//!
//! This module provides a proxy implementation for MCP (Model Context Protocol)
//! using SSE (Server-Sent Events) transport.
//!
//! # Features
//!
//! - **SSE Support**: Uses rmcp 0.10 with SSE transport (removed in 0.12+)
//! - **Stable Protocol**: Production-ready SSE implementation
//! - **Hot Swap**: Supports backend connection replacement
//! - **Fallback Option**: Used when Streamable HTTP is not supported
//! - **High-level Client API**: Simple connection interface hiding transport details
//!
//! # Architecture
//!
//! ```text
//! Client → SSE → SseHandler → Backend MCP Service
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use mcp_sse_proxy::{SseClientConnection, McpClientConfig};
//!
//! // Connect to an MCP server
//! let config = McpClientConfig::new("http://localhost:8080/sse");
//! let conn = SseClientConnection::connect(config).await?;
//!
//! // List available tools
//! let tools = conn.list_tools().await?;
//! ```

pub mod sse_handler;
pub mod server;
pub mod config;
pub mod client;

// Re-export main types
pub use sse_handler::{SseHandler, ToolFilter};
pub use server::{run_sse_server, run_sse_server_from_config, McpServiceConfig};

// Re-export client connection types
pub use client::{SseClientConnection, ToolInfo};
pub use mcp_common::McpClientConfig;

// Re-export commonly used rmcp types
pub use rmcp::{
    RoleClient, RoleServer, ServerHandler,
    model::{ServerInfo, ClientInfo, ClientCapabilities, CallToolRequestParam, Implementation},
    service::{RunningService, Peer},
    ServiceExt,
};

// Re-export transport types for SSE protocol (rmcp 0.10)
pub use rmcp::transport::{
    SseClientTransport,
    sse_client::SseClientConfig,
    child_process::TokioChildProcess,
    stdio,
    SseServer,
    sse_server::SseServerConfig,
};
