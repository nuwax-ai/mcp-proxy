//! Streamable HTTP Server Builder
//!
//! This module provides a high-level Builder API for creating Streamable HTTP MCP servers.
//! It encapsulates all rmcp-specific types and provides a simple interface for mcp-proxy.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Result;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use tokio_util::sync::CancellationToken;
use tracing::info;

use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        TokioChildProcess,
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
        streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService},
    },
};

// Unix 进程组支持
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

// Windows 静默运行支持
#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};

use crate::{ProxyAwareSessionManager, ProxyHandler, ToolFilter};
pub use mcp_common::ToolFilter as CommonToolFilter;

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
        /// Environment variables from MCP JSON config
        env: Option<HashMap<String, String>>,
    },
    /// Connect to a remote URL
    Url {
        /// URL of the MCP service
        url: String,
        /// Custom HTTP headers (including Authorization)
        headers: Option<HashMap<String, String>>,
    },
}

/// Configuration for the Streamable HTTP server
#[derive(Debug, Clone, Default)]
pub struct StreamServerConfig {
    /// Enable stateful mode with session management
    pub stateful_mode: bool,
    /// MCP service identifier for logging
    pub mcp_id: Option<String>,
    /// Tool filter configuration
    pub tool_filter: Option<ToolFilter>,
}

/// Builder for creating Streamable HTTP MCP servers
///
/// Provides a fluent API for configuring and building MCP proxy servers.
///
/// # Example
///
/// ```rust,ignore
/// use mcp_streamable_proxy::server_builder::{StreamServerBuilder, BackendConfig};
///
/// // Create a server with stdio backend
/// let (router, ct) = StreamServerBuilder::new(BackendConfig::Stdio {
///     command: "npx".into(),
///     args: Some(vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into()]),
///     env: None,
/// })
/// .mcp_id("my-server")
/// .stateful(false)
/// .build()
/// .await?;
/// ```
pub struct StreamServerBuilder {
    backend_config: BackendConfig,
    server_config: StreamServerConfig,
}

impl StreamServerBuilder {
    /// Create a new builder with the given backend configuration
    pub fn new(backend: BackendConfig) -> Self {
        Self {
            backend_config: backend,
            server_config: StreamServerConfig::default(),
        }
    }

    /// Set whether to enable stateful mode
    ///
    /// Stateful mode enables session management and server-side push.
    pub fn stateful(mut self, enabled: bool) -> Self {
        self.server_config.stateful_mode = enabled;
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

    /// Build the server and return an axum Router, CancellationToken, and ProxyHandler
    ///
    /// The router can be merged with other axum routers or served directly.
    /// The CancellationToken can be used to gracefully shut down the service.
    /// The ProxyHandler can be used for status checks and management.
    pub async fn build(self) -> Result<(axum::Router, CancellationToken, ProxyHandler)> {
        let mcp_id = self
            .server_config
            .mcp_id
            .clone()
            .unwrap_or_else(|| "stream-proxy".into());

        // Create client info for connecting to backend
        let capabilities = ClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build();
        let client_info = ClientInfo::new(
            capabilities,
            rmcp::model::Implementation::new("mcp-streamable-proxy", env!("CARGO_PKG_VERSION")),
        );

        // Connect to backend based on configuration
        let client = match &self.backend_config {
            BackendConfig::Stdio { command, args, env } => {
                self.connect_stdio(command, args, env, &client_info).await?
            }
            BackendConfig::Url { url, headers } => {
                self.connect_url(url, headers, &client_info).await?
            }
        };

        // Create proxy handler
        let proxy_handler = if let Some(ref tool_filter) = self.server_config.tool_filter {
            ProxyHandler::with_tool_filter(client, mcp_id.clone(), tool_filter.clone())
        } else {
            ProxyHandler::with_mcp_id(client, mcp_id.clone())
        };

        // Clone handler before creating server
        let handler_for_return = proxy_handler.clone();

        // Create server with configured stateful mode
        let (router, ct) = self.create_server(proxy_handler).await?;

        info!(
            "[StreamServerBuilder] Server created - mcp_id: {}, stateful: {}",
            mcp_id, self.server_config.stateful_mode
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
        let args = args.clone();

        // 使用 process-wrap 创建子进程命令（跨平台进程清理）
        // process-wrap 会自动处理进程组（Unix）或 Job Object（Windows）
        // 并且在 Drop 时自动清理子进程树
        // 子进程默认继承父进程的所有环境变量
        let mut wrapped_cmd = CommandWrap::with_new(command, |cmd| {
            if let Some(cmd_args) = &args {
                cmd.args(cmd_args);
            }
            // 设置 MCP JSON 配置中的环境变量（会覆盖继承的同名变量）
            if let Some(env_vars) = env {
                for (k, v) in env_vars {
                    cmd.env(k, v);
                }
            }
        });

        // Unix: 创建进程组，支持 killpg 清理整个进程树
        #[cfg(unix)]
        wrapped_cmd.wrap(ProcessGroup::leader());

        // Windows: 使用 CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP 隐藏控制台窗口
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
            info!(
                "[StreamServerBuilder] Setting CreationFlags: CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP"
            );
            wrapped_cmd.wrap(CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP));
            wrapped_cmd.wrap(JobObject);
        }

        // 所有平台: Drop 时自动清理进程
        wrapped_cmd.wrap(KillOnDrop);

        info!(
            "[StreamServerBuilder] Starting child process - command: {}, args: {:?}",
            command,
            args.as_ref().unwrap_or(&vec![])
        );

        let mcp_id = self.server_config.mcp_id.as_deref().unwrap_or("unknown");

        // 诊断日志：子进程关键环境变量
        mcp_common::diagnostic::log_stdio_spawn_context("StreamServerBuilder", mcp_id, env);

        // MCP 服务通过 stdin/stdout 进行 JSON-RPC 通信，必须使用 piped（默认行为）
        // 使用 builder 模式捕获 stderr，便于诊断子 MCP 服务初始化失败
        let (tokio_process, child_stderr) = TokioChildProcess::builder(wrapped_cmd)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                anyhow::anyhow!(
                    "{}",
                    mcp_common::diagnostic::format_spawn_error(mcp_id, command, &args, e)
                )
            })?;

        // 启动 stderr 日志读取任务
        if let Some(stderr_pipe) = child_stderr {
            mcp_common::spawn_stderr_reader(stderr_pipe, mcp_id.to_string());
        }

        let client = client_info.clone().serve(tokio_process).await?;

        info!("[StreamServerBuilder] Child process connected successfully");
        Ok(client)
    }

    /// Connect to a URL backend (remote MCP service)
    async fn connect_url(
        &self,
        url: &str,
        headers: &Option<HashMap<String, String>>,
        client_info: &ClientInfo,
    ) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>> {
        info!("[StreamServerBuilder] Connecting to URL backend: {}", url);

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
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
        config.auth_header = auth_header;

        let transport = StreamableHttpClientTransport::with_client(http_client, config);
        let client = client_info.clone().serve(transport).await?;

        info!("[StreamServerBuilder] URL backend connected successfully");
        Ok(client)
    }

    /// Create the Streamable HTTP server
    async fn create_server(
        &self,
        proxy_handler: ProxyHandler,
    ) -> Result<(axum::Router, CancellationToken)> {
        let handler = Arc::new(proxy_handler);
        let ct = CancellationToken::new();

        if self.server_config.stateful_mode {
            // Stateful mode with custom session manager
            let session_manager = ProxyAwareSessionManager::new(handler.clone());
            let handler_for_service = handler.clone();

            let mut server_config = StreamableHttpServerConfig::default();
            server_config.stateful_mode = true;
            let service = StreamableHttpService::new(
                move || Ok((*handler_for_service).clone()),
                session_manager.into(),
                server_config,
            );

            let router = axum::Router::new().fallback_service(service);
            Ok((router, ct))
        } else {
            // Stateless mode with local session manager
            use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;

            let handler_for_service = handler.clone();

            let server_config = StreamableHttpServerConfig::default(); // stateless mode
            let service = StreamableHttpService::new(
                move || Ok((*handler_for_service).clone()),
                LocalSessionManager::default().into(),
                server_config,
            );

            let router = axum::Router::new().fallback_service(service);
            Ok((router, ct))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let builder = StreamServerBuilder::new(BackendConfig::Stdio {
            command: "echo".into(),
            args: Some(vec!["hello".into()]),
            env: None,
        })
        .mcp_id("test")
        .stateful(true);

        assert!(builder.server_config.mcp_id.is_some());
        assert_eq!(builder.server_config.mcp_id.as_deref(), Some("test"));
        assert!(builder.server_config.stateful_mode);
    }

    #[test]
    fn test_url_backend_config() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer token123".into());
        headers.insert("X-Custom".into(), "value".into());

        let builder = StreamServerBuilder::new(BackendConfig::Url {
            url: "http://localhost:8080/mcp".into(),
            headers: Some(headers),
        });

        match &builder.backend_config {
            BackendConfig::Url { url, headers } => {
                assert_eq!(url, "http://localhost:8080/mcp");
                assert!(headers.is_some());
            }
            _ => panic!("Expected URL backend"),
        }
    }
}
