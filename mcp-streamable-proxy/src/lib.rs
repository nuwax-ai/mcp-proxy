//! MCP Streamable HTTP Proxy Module
//!
//! This module provides a proxy implementation for MCP (Model Context Protocol)
//! using Streamable HTTP transport with stateful session management.
//!
//! # Features
//!
//! - **Streamable HTTP Support**: Uses rmcp 0.12 with enhanced Streamable HTTP transport
//! - **Stateful Sessions**: Custom SessionManager with backend version tracking
//! - **Hot Swap**: Supports backend connection replacement without downtime
//! - **Version Control**: Automatically invalidates sessions when backend reconnects
//! - **High-level Client API**: Simple connection interface hiding transport details
//!
//! # Architecture
//!
//! ```text
//! Client → Streamable HTTP → ProxyAwareSessionManager → ProxyHandler → Backend MCP Service
//!                                    ↓
//!                            Version Tracking
//!                            (DashMap<SessionId, BackendVersion>)
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use mcp_streamable_proxy::{StreamClientConnection, McpClientConfig};
//!
//! // Connect to an MCP server
//! let config = McpClientConfig::new("http://localhost:8080/mcp");
//! let conn = StreamClientConnection::connect(config).await?;
//!
//! // List available tools
//! let tools = conn.list_tools().await?;
//! ```

pub mod client;
pub mod config;
pub mod detector;
pub mod proxy_handler;
pub mod server;
pub mod server_builder;
pub mod session_manager;

// Re-export main types
pub use proxy_handler::{ProxyHandler, ToolFilter};
pub use server::{run_stream_server, run_stream_server_from_config};
pub use session_manager::ProxyAwareSessionManager;
pub use mcp_common::McpServiceConfig;

// Re-export protocol detection function
pub use detector::is_streamable_http;

// Re-export server builder API
pub use server_builder::{BackendConfig, StreamServerBuilder, StreamServerConfig};

// Re-export client connection types
pub use client::{StreamClientConnection, ToolInfo};
pub use mcp_common::McpClientConfig;

// Re-export commonly used rmcp types
pub use rmcp::{
    RoleClient, RoleServer, ServerHandler, ServiceExt,
    model::{ClientCapabilities, ClientInfo, Implementation, ServerInfo},
    service::{Peer, RunningService},
};

// Re-export transport types for Streamable HTTP protocol (rmcp 0.12)
pub use rmcp::transport::{
    StreamableHttpServerConfig,
    child_process::TokioChildProcess,
    stdio, // stdio transport for CLI mode
    streamable_http_client::StreamableHttpClientTransport,
    streamable_http_client::StreamableHttpClientTransportConfig,
};

// Re-export server-side Streamable HTTP types
pub use rmcp::transport::streamable_http_server::{
    StreamableHttpService, session::local::LocalSessionManager,
};
