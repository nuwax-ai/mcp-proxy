//! SSE Client Connection Module
//!
//! Provides a high-level API for connecting to MCP servers via SSE protocol.
//! This module encapsulates the rmcp transport details and exposes a simple interface.

use anyhow::{Context, Result};
use mcp_common::McpClientConfig;
use rmcp::{
    RoleClient, ServiceExt,
    model::{ClientCapabilities, ClientInfo, Implementation, ProtocolVersion},
    service::RunningService,
    transport::{
        SseClientTransport, common::client_side_sse::SseRetryPolicy, sse_client::SseClientConfig,
    },
};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tracing::{debug, info};

use crate::sse_handler::SseHandler;
use mcp_common::ToolFilter;

/// 自定义的指数退避重试策略，支持最大间隔限制
///
/// 重试间隔按照指数增长，但不会超过 max_interval
/// - 第 1 次重试：base_duration × 2^0
/// - 第 2 次重试：base_duration × 2^1
/// - ...
/// - 第 n 次重试：min(base_duration × 2^(n-1), max_interval)
#[derive(Debug, Clone)]
pub struct CappedExponentialBackoff {
    /// 最大重试次数，None 表示无限制
    pub max_times: Option<usize>,
    /// 基础延迟时间（第一次重试前的等待时间）
    pub base_duration: Duration,
    /// 最大延迟间隔（重试间隔不会超过这个值）
    pub max_interval: Duration,
}

impl CappedExponentialBackoff {
    /// 创建一个新的带上限的指数退避策略
    ///
    /// # Arguments
    /// * `max_times` - 最大重试次数，None 表示无限制
    /// * `base_duration` - 基础延迟时间
    /// * `max_interval` - 最大延迟间隔
    pub fn new(max_times: Option<usize>, base_duration: Duration, max_interval: Duration) -> Self {
        Self {
            max_times,
            base_duration,
            max_interval,
        }
    }
}

impl Default for CappedExponentialBackoff {
    fn default() -> Self {
        Self {
            max_times: None,
            base_duration: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
        }
    }
}

impl SseRetryPolicy for CappedExponentialBackoff {
    fn retry(&self, current_times: usize) -> Option<Duration> {
        // 检查是否超过最大重试次数
        if let Some(max_times) = self.max_times
            && current_times >= max_times
        {
            return None;
        }

        // 计算指数退避时间
        let exponential_delay = self.base_duration * (2u32.pow(current_times as u32));

        // 限制最大间隔
        Some(exponential_delay.min(self.max_interval))
    }
}

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
        info!("🔗 Start establishing SSE connection: {}", config.url);

        debug!("Building the HTTP client configuration...");
        let http_client = build_http_client(&config)?;

        // 配置指数退避重试策略，最大间隔 1 分钟，不限制重试次数
        let retry_policy = CappedExponentialBackoff::new(
            None,                    // 不限制重试次数
            Duration::from_secs(1),  // 基础延迟 1 秒
            Duration::from_secs(60), // 最大间隔 60 秒
        );

        let sse_config = SseClientConfig {
            sse_endpoint: config.url.clone().into(),
            retry_policy: Arc::new(retry_policy),
            ..Default::default()
        };

        debug!("Starting the SSE transport layer...");
        let transport: SseClientTransport<reqwest::Client> =
            SseClientTransport::start_with_client(http_client, sse_config)
                .await
                .context("Failed to start SSE transport")?;

        let transport_elapsed = start.elapsed();
        debug!(
            "SSE transport layer startup completed, time taken: {:?}",
            transport_elapsed
        );

        debug!("Initializing MCP client handshake...");
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
                "✅ SSE connection established successfully - total time taken: {:?}, transport_closed: {}, peer_info: {:?}",
                total_elapsed, transport_closed, peer_info
            );
            if let Some(info) = peer_info {
                info!(
                    "Server information: name={}, version={}, capabilities={:?}",
                    info.server_info.name, info.server_info.version, info.capabilities
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
        protocol_version: ProtocolVersion::V_2024_11_05,
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

    #[test]
    fn test_capped_exponential_backoff() {
        // 测试带上限的指数退避策略
        let policy = CappedExponentialBackoff::new(
            None,                    // 不限制重试次数
            Duration::from_secs(1),  // 基础延迟 1 秒
            Duration::from_secs(60), // 最大间隔 60 秒
        );

        // 验证第 1 次重试：1 秒
        assert_eq!(policy.retry(0), Some(Duration::from_secs(1)));
        // 验证第 2 次重试：2 秒
        assert_eq!(policy.retry(1), Some(Duration::from_secs(2)));
        // 验证第 3 次重试：4 秒
        assert_eq!(policy.retry(2), Some(Duration::from_secs(4)));
        // 验证第 4 次重试：8 秒
        assert_eq!(policy.retry(3), Some(Duration::from_secs(8)));
        // 验证第 5 次重试：16 秒
        assert_eq!(policy.retry(4), Some(Duration::from_secs(16)));
        // 验证第 6 次重试：32 秒
        assert_eq!(policy.retry(5), Some(Duration::from_secs(32)));
        // 验证第 7 次重试：64 秒 -> 会被限制为 60 秒
        assert_eq!(policy.retry(6), Some(Duration::from_secs(60)));
        // 验证第 8 次重试：128 秒 -> 会被限制为 60 秒
        assert_eq!(policy.retry(7), Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_capped_exponential_backoff_with_max_times() {
        // 测试带最大重试次数的限制
        let policy = CappedExponentialBackoff::new(
            Some(3),                 // 最多重试 3 次
            Duration::from_secs(1),  // 基础延迟 1 秒
            Duration::from_secs(60), // 最大间隔 60 秒
        );

        // 验证前 3 次重试都有延迟时间
        assert_eq!(policy.retry(0), Some(Duration::from_secs(1)));
        assert_eq!(policy.retry(1), Some(Duration::from_secs(2)));
        assert_eq!(policy.retry(2), Some(Duration::from_secs(4)));

        // 验证第 4 次重试（重试次数已达到上限）
        assert_eq!(policy.retry(3), None);
    }

    #[test]
    fn test_capped_exponential_backoff_default() {
        // 测试默认配置
        let policy = CappedExponentialBackoff::default();

        // 验证默认配置
        assert_eq!(policy.max_times, None);
        assert_eq!(policy.base_duration, Duration::from_secs(1));
        assert_eq!(policy.max_interval, Duration::from_secs(60));

        // 验证重试行为
        assert_eq!(policy.retry(0), Some(Duration::from_secs(1)));
        assert_eq!(policy.retry(5), Some(Duration::from_secs(32)));
        assert_eq!(policy.retry(10), Some(Duration::from_secs(60)));
    }
}
