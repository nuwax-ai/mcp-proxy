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
//!
//! # Architecture
//!
//! ```text
//! Client → Streamable HTTP → ProxyAwareSessionManager → ProxyHandler → Backend MCP Service
//!                                    ↓
//!                            Version Tracking
//!                            (DashMap<SessionId, BackendVersion>)
//! ```

pub mod proxy_handler;
pub mod session_manager;
pub mod server;
pub mod config;

// Re-export main types
pub use proxy_handler::{ProxyHandler, ToolFilter};
pub use session_manager::ProxyAwareSessionManager;
pub use server::{run_stream_server, run_stream_server_from_config, McpServiceConfig};

// Re-export commonly used rmcp types
pub use rmcp::{
    RoleClient, RoleServer, ServerHandler,
    model::{ServerInfo, ClientInfo, ClientCapabilities},
    service::{RunningService, Peer},
    ServiceExt,
};

// Re-export transport types for Streamable HTTP protocol (rmcp 0.12)
pub use rmcp::transport::{
    child_process::TokioChildProcess,
    streamable_http_client::StreamableHttpClientTransport,
    streamable_http_client::StreamableHttpClientTransportConfig,
    StreamableHttpServerConfig,
};

// Re-export server-side Streamable HTTP types
pub use rmcp::transport::streamable_http_server::{
    StreamableHttpService,
    session::local::LocalSessionManager,
};
