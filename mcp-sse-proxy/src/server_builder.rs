//! SSE Server Builder
//!
//! This module provides a high-level Builder API for creating SSE MCP servers.
//! It encapsulates all rmcp-specific types and provides a simple interface for mcp-proxy.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

// 进程组管理（跨平台子进程清理）
use process_wrap::tokio::{KillOnDrop, TokioCommandWrap};

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

#[cfg(windows)]
use process_wrap::tokio::JobObject;

use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo, ProtocolVersion},
    transport::{
        SseClientTransport, TokioChildProcess,
        sse_client::SseClientConfig,
        sse_server::{SseServer, SseServerConfig},
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
    },
};

use crate::{SseHandler, ToolFilter};

/// Performance warning threshold for stdio (child process) backend connections
const STDIO_SLOW_THRESHOLD_SECS: u64 = 30;

/// Performance warning threshold for HTTP-based backend connections (SSE/Stream)
const HTTP_SLOW_THRESHOLD_SECS: u64 = 10;

/// Backend configuration for the MCP server
///
/// Defines how the proxy connects to the upstream MCP service.
#[derive(Debug, Clone)]
pub enum BackendConfig {
    /// Connect to a local command via stdio
    Stdio {
        /// Command to execute (e.g., "npx", "python", etc.)
        command: String,
        /// Arguments for the command
        args: Option<Vec<String>>,
        /// Environment variables
        env: Option<HashMap<String, String>>,
    },
    /// Connect to a remote URL using SSE protocol
    SseUrl {
        /// URL of the MCP SSE service
        url: String,
        /// Custom HTTP headers (including Authorization)
        headers: Option<HashMap<String, String>>,
    },
    /// Connect to a remote URL using Streamable HTTP protocol
    /// (for protocol conversion: Stream backend -> SSE frontend)
    StreamUrl {
        /// URL of the MCP Streamable HTTP service
        url: String,
        /// Custom HTTP headers (including Authorization)
        headers: Option<HashMap<String, String>>,
    },
}

/// Configuration for the SSE server
#[derive(Debug, Clone)]
pub struct SseServerBuilderConfig {
    /// SSE endpoint path (default: "/sse")
    pub sse_path: String,
    /// Message endpoint path (default: "/message")
    pub post_path: String,
    /// MCP service identifier for logging
    pub mcp_id: Option<String>,
    /// Tool filter configuration
    pub tool_filter: Option<ToolFilter>,
    /// Keep-alive interval in seconds (default: 15)
    pub keep_alive_secs: u64,
    /// Enable stateful mode with full MCP initialization (default: true)
    /// When false, uses `with_service_directly` which skips initialization for faster responses
    pub stateful: bool,
}

impl Default for SseServerBuilderConfig {
    fn default() -> Self {
        Self {
            sse_path: "/sse".into(),
            post_path: "/message".into(),
            mcp_id: None,
            tool_filter: None,
            keep_alive_secs: 15,
            stateful: true,
        }
    }
}

/// Log connection timing with optional performance warning
///
/// # Arguments
///
/// * `mcp_id` - MCP service identifier
/// * `backend_type` - Type of backend (e.g., "stdio", "SSE", "Streamable HTTP")
/// * `total_duration` - Total connection time
/// * `breakdown` - Optional breakdown of timing components
/// * `warn_threshold_secs` - Threshold for performance warning
/// * `warn_message` - Message to show if threshold exceeded
fn log_connection_timing(
    mcp_id: &str,
    backend_type: &str,
    total_duration: Duration,
    breakdown: &[(&str, Duration)],
    warn_threshold_secs: u64,
    warn_message: &str,
) {
    let breakdown_str: Vec<String> = breakdown
        .iter()
        .map(|(name, dur)| format!("{}: {:?}", name, dur))
        .collect();

    info!(
        "[SseServerBuilder] {} backend connected successfully - MCP ID: {}, total: {:?} ({})",
        backend_type,
        mcp_id,
        total_duration,
        breakdown_str.join(", ")
    );

    if total_duration.as_secs() >= warn_threshold_secs {
        warn!(
            "[SseServerBuilder] {} 后端连接耗时较长 - MCP ID: {}, 耗时: {:?}, {}",
            backend_type, mcp_id, total_duration, warn_message
        );
    }
}

/// Builder for creating SSE MCP servers
///
/// Provides a fluent API for configuring and building MCP proxy servers.
///
/// # Example
///
/// ```rust,ignore
/// use mcp_sse_proxy::server_builder::{SseServerBuilder, BackendConfig};
///
/// // Create a server with stdio backend
/// let (router, ct) = SseServerBuilder::new(BackendConfig::Stdio {
///     command: "npx".into(),
///     args: Some(vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into()]),
///     env: None,
/// })
/// .mcp_id("my-server")
/// .sse_path("/custom/sse")
/// .post_path("/custom/message")
/// .stateful(false)  // Disable stateful mode for OneShot services (faster responses)
/// .build()
/// .await?;
/// ```
pub struct SseServerBuilder {
    backend_config: BackendConfig,
    server_config: SseServerBuilderConfig,
}

impl SseServerBuilder {
    /// Create a new builder with the given backend configuration
    pub fn new(backend: BackendConfig) -> Self {
        Self {
            backend_config: backend,
            server_config: SseServerBuilderConfig::default(),
        }
    }

    /// Set the SSE endpoint path
    pub fn sse_path(mut self, path: impl Into<String>) -> Self {
        self.server_config.sse_path = path.into();
        self
    }

    /// Set the message endpoint path
    pub fn post_path(mut self, path: impl Into<String>) -> Self {
        self.server_config.post_path = path.into();
        self
    }

    /// Set the MCP service identifier
    ///
    /// Used for logging and service identification.
    pub fn mcp_id(mut self, id: impl Into<String>) -> Self {
        self.server_config.mcp_id = Some(id.into());
        self
    }

    /// Set the tool filter configuration
    pub fn tool_filter(mut self, filter: ToolFilter) -> Self {
        self.server_config.tool_filter = Some(filter);
        self
    }

    /// Set the keep-alive interval in seconds
    pub fn keep_alive(mut self, secs: u64) -> Self {
        self.server_config.keep_alive_secs = secs;
        self
    }

    /// Set stateful mode (default: true)
    ///
    /// When false, uses `with_service_directly` which skips MCP initialization
    /// for faster responses. This is recommended for OneShot services.
    pub fn stateful(mut self, stateful: bool) -> Self {
        self.server_config.stateful = stateful;
        self
    }

    /// Build the server and return an axum Router, CancellationToken, and SseHandler
    ///
    /// The router can be merged with other axum routers or served directly.
    /// The CancellationToken can be used to gracefully shut down the service.
    /// The SseHandler can be used for status checks and management.
    pub async fn build(self) -> Result<(axum::Router, CancellationToken, SseHandler)> {
        let mcp_id = self
            .server_config
            .mcp_id
            .clone()
            .unwrap_or_else(|| "sse-proxy".into());

        // Create client info for connecting to backend
        let client_info = ClientInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ClientCapabilities::builder()
                .enable_experimental()
                .enable_roots()
                .enable_roots_list_changed()
                .enable_sampling()
                .build(),
            ..Default::default()
        };

        // Connect to backend based on configuration
        let client = match &self.backend_config {
            BackendConfig::Stdio { command, args, env } => {
                self.connect_stdio(command, args, env, &client_info).await?
            }
            BackendConfig::SseUrl { url, headers } => {
                self.connect_sse_url(url, headers, &client_info).await?
            }
            BackendConfig::StreamUrl { url, headers } => {
                self.connect_stream_url(url, headers, &client_info).await?
            }
        };

        // Create SSE handler
        let sse_handler = if let Some(ref tool_filter) = self.server_config.tool_filter {
            SseHandler::with_tool_filter(client, mcp_id.clone(), tool_filter.clone())
        } else {
            SseHandler::with_mcp_id(client, mcp_id.clone())
        };

        // Clone handler before creating server (create_server uses sse_handler.clone() internally)
        let handler_for_return = sse_handler.clone();

        // Create SSE server
        let (router, ct) = self.create_server(sse_handler)?;

        info!(
            "[SseServerBuilder] Server created - mcp_id: {}, sse_path: {}, post_path: {}",
            mcp_id, self.server_config.sse_path, self.server_config.post_path
        );

        Ok((router, ct, handler_for_return))
    }

    /// Connect to a stdio backend (child process)
    async fn connect_stdio(
        &self,
        command: &str,
        args: &Option<Vec<String>>,
        env: &Option<HashMap<String, String>>,
        client_info: &ClientInfo,
    ) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>> {
        use std::time::Instant;

        let start_time = Instant::now();
        let mcp_id = self
            .server_config
            .mcp_id
            .clone()
            .unwrap_or_else(|| "unknown".into());

        // 使用 process-wrap 创建子进程命令（跨平台进程清理）
        // process-wrap 会自动处理进程组（Unix）或 Job Object（Windows）
        // 并且在 Drop 时自动清理子进程树
        let mut wrapped_cmd = TokioCommandWrap::with_new(command, |cmd| {
            if let Some(cmd_args) = args {
                cmd.args(cmd_args);
            }
            if let Some(env_vars) = env {
                for (k, v) in env_vars {
                    cmd.env(k, v);
                }
            }
        });

        // Unix: 创建进程组，支持 killpg 清理整个进程树
        #[cfg(unix)]
        wrapped_cmd.wrap(ProcessGroup::leader());
        // Windows: 使用 Job Object 管理进程树
        #[cfg(windows)]
        wrapped_cmd.wrap(JobObject);

        // 所有平台: Drop 时自动清理进程
        wrapped_cmd.wrap(KillOnDrop);

        info!(
            "[SseServerBuilder] Starting child process - MCP ID: {}, command: {}, args: {:?}",
            mcp_id,
            command,
            args.as_ref().unwrap_or(&vec![])
        );

        let process_start = Instant::now();
        let tokio_process = TokioChildProcess::new(wrapped_cmd)?;
        let process_duration = process_start.elapsed();

        debug!(
            "[SseServerBuilder] Child process spawned - MCP ID: {}, spawn time: {:?}",
            mcp_id, process_duration
        );

        let serve_start = Instant::now();
        let client = client_info.clone().serve(tokio_process).await?;
        let serve_duration = serve_start.elapsed();
        let total_duration = start_time.elapsed();

        let warn_msg = "建议的优化方案: \
            1) 检查网络连接速度 (npm 包下载) \
            2) 配置国内 npm 镜像 (如淘宝镜像: npm config set registry https://registry.npmmirror.com) \
            3) 预热服务 (启动 mcp-proxy 时预先加载常用服务) \
            4) 检查命令参数是否正确";

        log_connection_timing(
            &mcp_id,
            "Stdio",
            total_duration,
            &[("spawn", process_duration), ("serve", serve_duration)],
            STDIO_SLOW_THRESHOLD_SECS,
            warn_msg,
        );

        Ok(client)
    }

    /// Connect to an SSE URL backend
    async fn connect_sse_url(
        &self,
        url: &str,
        headers: &Option<HashMap<String, String>>,
        client_info: &ClientInfo,
    ) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>> {
        use std::time::Instant;

        let start_time = Instant::now();
        let mcp_id = self
            .server_config
            .mcp_id
            .clone()
            .unwrap_or_else(|| "unknown".into());

        info!(
            "[SseServerBuilder] Connecting to SSE URL backend - MCP ID: {}, URL: {}",
            mcp_id, url
        );

        // Build HTTP client with custom headers
        let mut req_headers = reqwest::header::HeaderMap::new();

        if let Some(config_headers) = headers {
            for (key, value) in config_headers {
                req_headers.insert(
                    reqwest::header::HeaderName::try_from(key)
                        .map_err(|e| anyhow::anyhow!("Invalid header name '{}': {}", key, e))?,
                    value.parse().map_err(|e| {
                        anyhow::anyhow!("Invalid header value for '{}': {}", key, e)
                    })?,
                );
            }
        }

        let http_client = reqwest::Client::builder()
            .default_headers(req_headers)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

        // Create SSE client configuration
        let sse_config = SseClientConfig {
            sse_endpoint: url.to_string().into(),
            ..Default::default()
        };

        let transport_start = Instant::now();
        let sse_transport = SseClientTransport::start_with_client(http_client, sse_config).await?;
        let transport_duration = transport_start.elapsed();

        let serve_start = Instant::now();
        let client = client_info.clone().serve(sse_transport).await?;
        let serve_duration = serve_start.elapsed();
        let total_duration = start_time.elapsed();

        log_connection_timing(
            &mcp_id,
            "SSE",
            total_duration,
            &[("transport", transport_duration), ("serve", serve_duration)],
            HTTP_SLOW_THRESHOLD_SECS,
            "建议: 检查网络连接和后端服务状态",
        );

        Ok(client)
    }

    /// Connect to a Streamable HTTP URL backend
    async fn connect_stream_url(
        &self,
        url: &str,
        headers: &Option<HashMap<String, String>>,
        client_info: &ClientInfo,
    ) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>> {
        use std::time::Instant;

        let start_time = Instant::now();
        let mcp_id = self
            .server_config
            .mcp_id
            .clone()
            .unwrap_or_else(|| "unknown".into());

        info!(
            "[SseServerBuilder] Connecting to Streamable HTTP URL backend - MCP ID: {}, URL: {}",
            mcp_id, url
        );

        // Build HTTP client with custom headers (excluding Authorization)
        let mut req_headers = reqwest::header::HeaderMap::new();
        let mut auth_header: Option<String> = None;

        if let Some(config_headers) = headers {
            for (key, value) in config_headers {
                // Authorization header is handled separately by rmcp
                if key.eq_ignore_ascii_case("Authorization") {
                    auth_header = Some(value.strip_prefix("Bearer ").unwrap_or(value).to_string());
                    continue;
                }

                req_headers.insert(
                    reqwest::header::HeaderName::try_from(key)
                        .map_err(|e| anyhow::anyhow!("Invalid header name '{}': {}", key, e))?,
                    value.parse().map_err(|e| {
                        anyhow::anyhow!("Invalid header value for '{}': {}", key, e)
                    })?,
                );
            }
        }

        let http_client = reqwest::Client::builder()
            .default_headers(req_headers)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

        // Create transport configuration
        let config = StreamableHttpClientTransportConfig {
            uri: url.to_string().into(),
            auth_header,
            ..Default::default()
        };

        let serve_start = Instant::now();
        let transport = StreamableHttpClientTransport::with_client(http_client, config);
        let client = client_info.clone().serve(transport).await?;
        let serve_duration = serve_start.elapsed();
        let total_duration = start_time.elapsed();

        log_connection_timing(
            &mcp_id,
            "Streamable HTTP",
            total_duration,
            &[("serve", serve_duration)],
            HTTP_SLOW_THRESHOLD_SECS,
            "建议: 检查网络连接和后端服务状态",
        );

        Ok(client)
    }

    /// Create the SSE server
    fn create_server(&self, sse_handler: SseHandler) -> Result<(axum::Router, CancellationToken)> {
        // SSE server uses bind address 0.0.0.0:0 since we're returning a router
        // The actual binding will be done by the caller
        let config = SseServerConfig {
            bind: "0.0.0.0:0".parse()?,
            sse_path: self.server_config.sse_path.clone(),
            post_path: self.server_config.post_path.clone(),
            ct: CancellationToken::new(),
            sse_keep_alive: Some(std::time::Duration::from_secs(
                self.server_config.keep_alive_secs,
            )),
        };

        let (sse_server, router) = SseServer::new(config);

        // Use with_service_directly for non-stateful mode (OneShot services)
        // This skips MCP initialization for faster responses
        let ct = if self.server_config.stateful {
            sse_server.with_service(move || sse_handler.clone())
        } else {
            sse_server.with_service_directly(move || sse_handler.clone())
        };

        Ok((router, ct))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let builder = SseServerBuilder::new(BackendConfig::Stdio {
            command: "echo".into(),
            args: Some(vec!["hello".into()]),
            env: None,
        })
        .mcp_id("test")
        .sse_path("/custom/sse")
        .post_path("/custom/message");

        assert!(builder.server_config.mcp_id.is_some());
        assert_eq!(builder.server_config.mcp_id.as_deref(), Some("test"));
        assert_eq!(builder.server_config.sse_path, "/custom/sse");
        assert_eq!(builder.server_config.post_path, "/custom/message");
    }

    #[test]
    fn test_default_config() {
        let config = SseServerBuilderConfig::default();
        assert_eq!(config.sse_path, "/sse");
        assert_eq!(config.post_path, "/message");
        assert_eq!(config.keep_alive_secs, 15);
        assert!(
            config.stateful,
            "default stateful should be true for backward compatibility"
        );
    }

    #[test]
    fn test_stateful_flag_default() {
        let builder = SseServerBuilder::new(BackendConfig::Stdio {
            command: "echo".into(),
            args: None,
            env: None,
        });
        assert!(
            builder.server_config.stateful,
            "stateful should default to true"
        );
    }

    #[test]
    fn test_stateful_flag_disabled() {
        let builder = SseServerBuilder::new(BackendConfig::Stdio {
            command: "echo".into(),
            args: None,
            env: None,
        })
        .stateful(false);
        assert!(
            !builder.server_config.stateful,
            "stateful should be false when set"
        );
    }

    #[test]
    fn test_stateful_flag_enabled() {
        let builder = SseServerBuilder::new(BackendConfig::Stdio {
            command: "echo".into(),
            args: None,
            env: None,
        })
        .stateful(true);
        assert!(
            builder.server_config.stateful,
            "stateful should be true when set"
        );
    }

    #[test]
    fn test_timing_constants() {
        assert_eq!(STDIO_SLOW_THRESHOLD_SECS, 30);
        assert_eq!(HTTP_SLOW_THRESHOLD_SECS, 10);
    }

    #[test]
    fn test_log_connection_timing_format() {
        use std::time::Duration;
        // Test that the function doesn't panic and formats correctly
        log_connection_timing(
            "test-mcp",
            "TestBackend",
            Duration::from_millis(1500),
            &[
                ("step1", Duration::from_millis(500)),
                ("step2", Duration::from_millis(1000)),
            ],
            10,
            "Test warning message",
        );
        // If we get here, the function works correctly
    }

    #[test]
    fn test_log_connection_timing_no_breakdown() {
        use std::time::Duration;
        // Test with empty breakdown
        log_connection_timing(
            "test-mcp",
            "TestBackend",
            Duration::from_millis(500),
            &[],
            10,
            "Test warning message",
        );
    }
}
