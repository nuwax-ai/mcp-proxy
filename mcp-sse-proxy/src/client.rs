//! SSE Client Connection Module
//!
//! Provides a high-level API for connecting to MCP servers via SSE protocol.
//! This module encapsulates the rmcp transport details and exposes a simple interface.

use anyhow::{Context, Result};
use mcp_common::McpClientConfig;
use rmcp::{
    RoleClient, ServiceExt,
    model::{ClientCapabilities, ClientInfo, Implementation},
    service::RunningService,
    transport::{SseClientTransport, sse_client::SseClientConfig},
};
use std::time::Instant;
use tracing::{debug, info};

use crate::sse_handler::SseHandler;
use mcp_common::ToolFilter;

/// Opaque wrapper for SSE client connection
///
/// This type encapsulates an active connection to an MCP server via SSE protocol.
/// It hides the internal `RunningService` type and provides only the methods
/// needed by consuming code.
///
/// Note: This type is not Clone because the underlying RunningService
/// is designed for single-owner use. Use `into_handler()` or `into_running_service()`
/// to consume the connection.
///
/// # Example
///
/// ```rust,ignore
/// use mcp_sse_proxy::{SseClientConnection, McpClientConfig};
///
/// let config = McpClientConfig::new("http://localhost:8080/sse")
///     .with_header("Authorization", "Bearer token");
///
/// let conn = SseClientConnection::connect(config).await?;
/// let tools = conn.list_tools().await?;
/// println!("Available tools: {:?}", tools);
/// ```
pub struct SseClientConnection {
    inner: RunningService<RoleClient, ClientInfo>,
}

impl SseClientConnection {
    /// Connect to an SSE MCP server
    ///
    /// # Arguments
    /// * `config` - Client configuration including URL and headers
    ///
    /// # Returns
    /// * `Ok(SseClientConnection)` - Successfully connected client
    /// * `Err` - Connection failed
    pub async fn connect(config: McpClientConfig) -> Result<Self> {
        let start = Instant::now();
        info!("🔗 开始建立 SSE 连接: {}", config.url);

        debug!("构建 HTTP 客户端配置...");
        let http_client = build_http_client(&config)?;

        let sse_config = SseClientConfig {
            sse_endpoint: config.url.clone().into(),
            ..Default::default()
        };

        debug!("启动 SSE 传输层...");
        let transport: SseClientTransport<reqwest::Client> =
            SseClientTransport::start_with_client(http_client, sse_config)
                .await
                .context("Failed to start SSE transport")?;

        let transport_elapsed = start.elapsed();
        debug!("SSE 传输层启动完成，耗时: {:?}", transport_elapsed);

        debug!("初始化 MCP 客户端握手...");
        let client_info = create_default_client_info();
        let running = client_info
            .serve(transport)
            .await
            .context("Failed to initialize MCP client")?;

        let total_elapsed = start.elapsed();

        // 记录详细的连接状态信息
        {
            use std::ops::Deref;
            let transport_closed = running.deref().is_transport_closed();
            let peer_info = running.peer_info();
            info!(
                "✅ SSE 连接建立成功 - 总耗时: {:?}, transport_closed: {}, peer_info: {:?}",
                total_elapsed, transport_closed, peer_info
            );
            if let Some(info) = peer_info {
                info!(
                    "   服务器信息: name={}, version={}, capabilities={:?}",
                    info.server_info.name,
                    info.server_info.version,
                    info.capabilities
                );
            }
        }

        Ok(Self { inner: running })
    }

    /// List available tools from the MCP server
    pub async fn list_tools(&self) -> Result<Vec<ToolInfo>> {
        let result = self.inner.list_tools(None).await?;
        Ok(result
            .tools
            .into_iter()
            .map(|t| ToolInfo {
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()),
            })
            .collect())
    }

    /// Check if the connection is closed
    pub fn is_closed(&self) -> bool {
        use std::ops::Deref;
        self.inner.deref().is_transport_closed()
    }

    /// Get the peer info from the server
    pub fn peer_info(&self) -> Option<&rmcp::model::ServerInfo> {
        self.inner.peer_info()
    }

    /// Convert this connection into an SseHandler for serving
    ///
    /// This consumes the connection and creates an SseHandler that can
    /// proxy requests to the backend MCP server.
    ///
    /// # Arguments
    /// * `mcp_id` - Identifier for logging purposes
    /// * `tool_filter` - Tool filtering configuration
    pub fn into_handler(self, mcp_id: String, tool_filter: ToolFilter) -> SseHandler {
        SseHandler::with_tool_filter(self.inner, mcp_id, tool_filter)
    }

    /// Extract the internal RunningService for use with swap_backend
    ///
    /// This is used internally to support backend hot-swapping.
    pub fn into_running_service(self) -> RunningService<RoleClient, ClientInfo> {
        self.inner
    }
}

/// Simplified tool information
#[derive(Clone, Debug)]
pub struct ToolInfo {
    /// Tool name
    pub name: String,
    /// Tool description (optional)
    pub description: Option<String>,
}

/// Build an HTTP client with the given configuration
fn build_http_client(config: &McpClientConfig) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (key, value) in &config.headers {
        let header_name = key
            .parse::<reqwest::header::HeaderName>()
            .with_context(|| format!("Invalid header name: {}", key))?;
        let header_value = value
            .parse()
            .with_context(|| format!("Invalid header value for {}: {}", key, value))?;
        headers.insert(header_name, header_value);
    }

    let mut builder = reqwest::Client::builder().default_headers(headers);

    if let Some(timeout) = config.connect_timeout {
        builder = builder.connect_timeout(timeout);
    }

    if let Some(timeout) = config.read_timeout {
        builder = builder.timeout(timeout);
    }

    builder.build().context("Failed to build HTTP client")
}

/// Create default client info for MCP handshake
fn create_default_client_info() -> ClientInfo {
    ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build(),
        client_info: Implementation {
            name: "mcp-sse-proxy-client".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: None,
            website_url: None,
            icons: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_info() {
        let info = ToolInfo {
            name: "test_tool".to_string(),
            description: Some("A test tool".to_string()),
        };
        assert_eq!(info.name, "test_tool");
        assert_eq!(info.description, Some("A test tool".to_string()));
    }
}
