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
//!
//! # Architecture
//!
//! ```text
//! Client → SSE → SseHandler → Backend MCP Service
//! ```

pub mod sse_handler;
pub mod server;
pub mod config;

// Re-export main types
pub use sse_handler::{SseHandler, ToolFilter};
pub use server::{run_sse_server, run_sse_server_from_config, McpServiceConfig};

// Re-export commonly used rmcp types
pub use rmcp::{
    RoleClient, RoleServer, ServerHandler,
    model::{ServerInfo, ClientInfo},
    service::{RunningService, Peer},
};
