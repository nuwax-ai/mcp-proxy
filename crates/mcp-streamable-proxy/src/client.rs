//! Streamable HTTP Client Connection Module
//!
//! Provides a high-level API for connecting to MCP servers via Streamable HTTP protocol.
//! This module encapsulates the rmcp transport details and exposes a simple interface.

use anyhow::{Context, Result};
use mcp_common::McpClientConfig;
use rmcp::{
    ServiceExt,
    transport::{
        common::client_side_sse::SseRetryPolicy,
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
    },
};
use std::sync::Arc;
use std::time::Duration;

use crate::backend_client::{BackendNotificationBridge, UpstreamPeerRegistry};
use crate::fallback::DiscoverySnapshot;
use crate::proxy_handler::{BackendRunningService, ProxyHandler};
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

        Some(mcp_common::capped_exponential_delay(
            self.base_duration,
            self.max_interval,
            current_times,
        ))
    }
}

/// Opaque wrapper for Streamable HTTP client connection
///
/// This type encapsulates an active connection to an MCP server via Streamable HTTP protocol.
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
/// use mcp_streamable_proxy::{StreamClientConnection, McpClientConfig};
///
/// let config = McpClientConfig::new("http://localhost:8080/mcp")
///     .with_header("Authorization", "Bearer token");
///
/// let conn = StreamClientConnection::connect(config).await?;
/// let tools = conn.list_tools().await?;
/// println!("Available tools: {:?}", tools);
/// ```
pub struct StreamClientConnection {
    inner: BackendRunningService,
}

impl StreamClientConnection {
    /// Connect to a Streamable HTTP MCP server (new upstream peer registry).
    pub async fn connect(config: McpClientConfig) -> Result<Self> {
        Self::connect_with_peers(config, Arc::new(UpstreamPeerRegistry::new())).await
    }

    /// Connect using a shared [`UpstreamPeerRegistry`] (for proxy reconnect / hot-swap).
    pub async fn connect_with_peers(
        config: McpClientConfig,
        upstream_peers: Arc<UpstreamPeerRegistry>,
    ) -> Result<Self> {
        let http_client = build_http_client(&config)?;

        // 配置指数退避重试策略，最大间隔 1 分钟，不限制重试次数
        let retry_policy = CappedExponentialBackoff::new(
            None,                    // 不限制重试次数
            Duration::from_secs(1),  // 基础延迟 1 秒
            Duration::from_secs(60), // 最大间隔 60 秒
        );

        let mut transport_config =
            StreamableHttpClientTransportConfig::with_uri(config.url.clone());
        transport_config.retry_config = Arc::new(retry_policy);

        let transport = StreamableHttpClientTransport::with_client(http_client, transport_config);

        let bridge = BackendNotificationBridge::with_default_info(upstream_peers);
        let running = bridge
            .serve(transport)
            .await
            .context("Failed to initialize MCP client")?;

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

    /// Fetch an unfiltered, complete discovery snapshot from the real upstream.
    pub async fn fetch_discovery_snapshot(&self) -> Result<DiscoverySnapshot> {
        use std::collections::HashSet;

        const MAX_PAGES: usize = 10_000;
        let server_info = self
            .peer_info()
            .map(|info| (*info).clone())
            .context("Streamable HTTP upstream did not return initialize server info")?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut response_envelope = None;

        for _ in 0..MAX_PAGES {
            let request = cursor.clone().map(|cursor| {
                rmcp::model::PaginatedRequestParams::default().with_cursor(Some(cursor))
            });
            let mut result = self
                .inner
                .list_tools(request)
                .await
                .context("failed to list Streamable HTTP upstream tools")?;
            tools.append(&mut result.tools);
            let next_cursor = result.next_cursor.take();
            if response_envelope.is_none() {
                response_envelope = Some(result);
            }
            match next_cursor {
                Some(next) => {
                    if !seen.insert(next.clone()) {
                        anyhow::bail!(
                            "Streamable HTTP tools pagination returned a repeated cursor"
                        );
                    }
                    cursor = Some(next);
                }
                None => {
                    let mut complete = response_envelope
                        .context("Streamable HTTP tools pagination returned no response")?;
                    complete.tools = tools;
                    complete.next_cursor = None;
                    return Ok(DiscoverySnapshot {
                        server_info,
                        tools: complete,
                    });
                }
            }
        }
        anyhow::bail!("Streamable HTTP tools pagination exceeded {MAX_PAGES} pages")
    }

    /// Check if the connection is closed
    pub fn is_closed(&self) -> bool {
        use std::ops::Deref;
        self.inner.deref().is_transport_closed()
    }

    /// Get the peer info from the server
    pub fn peer_info(&self) -> Option<Arc<rmcp::model::ServerInfo>> {
        // rmcp 3.1.0 narrowed peer_info() to ServerPeerInfo; rebuild the full
        // ServerInfo (InitializeResult) shape the proxy caches/exposes.
        self.inner
            .peer_info()
            .map(|p| Arc::new(crate::proxy_handler::peer_info_to_server_info((*p).clone())))
    }

    /// Convert this connection into a ProxyHandler for serving
    ///
    /// This consumes the connection and creates a ProxyHandler that can
    /// proxy requests to the backend MCP server.
    ///
    /// # Arguments
    /// * `mcp_id` - Identifier for logging purposes
    /// * `tool_filter` - Tool filtering configuration
    pub fn into_handler(self, mcp_id: String, tool_filter: ToolFilter) -> ProxyHandler {
        ProxyHandler::with_tool_filter(self.inner, mcp_id, tool_filter)
    }

    /// Extract the internal RunningService for use with swap_backend
    ///
    /// This is used internally to support backend hot-swapping.
    pub fn into_running_service(self) -> BackendRunningService {
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
            .with_context(|| format!("Invalid header value for {key}"))?;
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
    fn invalid_header_error_does_not_expose_value() {
        let secret = "secret-token\ninvalid";
        let config = McpClientConfig::new("http://localhost").with_header("Authorization", secret);
        let error = build_http_client(&config).expect_err("invalid header must fail");

        assert!(error.to_string().contains("Authorization"));
        assert!(!error.to_string().contains("secret-token"));
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
        // 验证第 7 次重试：64 秒，但被限制为 60 秒
        assert_eq!(policy.retry(6), Some(Duration::from_secs(60)));
        // 验证第 10 次重试：仍然是 60 秒
        assert_eq!(policy.retry(9), Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_capped_exponential_backoff_with_max_times() {
        let policy =
            CappedExponentialBackoff::new(Some(3), Duration::from_secs(1), Duration::from_secs(60));

        assert_eq!(policy.retry(0), Some(Duration::from_secs(1)));
        assert_eq!(policy.retry(1), Some(Duration::from_secs(2)));
        assert_eq!(policy.retry(2), Some(Duration::from_secs(4)));
        // 超过最大次数
        assert_eq!(policy.retry(3), None);
    }

    #[test]
    fn test_capped_exponential_backoff_never_overflows() {
        let policy = CappedExponentialBackoff::default();

        assert_eq!(policy.retry(31), Some(Duration::from_secs(60)));
        assert_eq!(policy.retry(32), Some(Duration::from_secs(60)));
        assert_eq!(policy.retry(usize::MAX), Some(Duration::from_secs(60)));
    }
}
