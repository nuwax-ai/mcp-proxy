//! Streamable HTTP Server Builder
//!
//! This module provides a high-level Builder API for creating Streamable HTTP MCP servers.
//! It encapsulates all rmcp-specific types and provides a simple interface for mcp-proxy.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
};

use crate::backend_client::UpstreamPeerRegistry;
use crate::backend_connector::{
    BackendConnector, BackendIsolation, StdioBackendConnector, UrlBackendConnector,
};
use crate::{ProxyAwareSessionManager, ProxyHandler, ToolFilter};

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
#[derive(Debug, Clone)]
pub struct StreamServerConfig {
    /// Enable stateful mode with session management
    pub stateful_mode: bool,
    /// MCP service identifier for logging
    pub mcp_id: Option<String>,
    /// Tool filter configuration
    pub tool_filter: Option<ToolFilter>,
    /// Backend isolation (`None` = default by backend type)
    pub backend_isolation: Option<BackendIsolation>,
    /// Max concurrent upstream sessions (`None` = unlimited)
    pub max_sessions: Option<usize>,
    /// Per-session backend connect timeout (URL lazy connect)
    pub connect_timeout: Duration,
}

impl Default for StreamServerConfig {
    fn default() -> Self {
        Self {
            stateful_mode: false,
            mcp_id: None,
            tool_filter: None,
            backend_isolation: None,
            max_sessions: None,
            connect_timeout: Duration::from_secs(30),
        }
    }
}

/// Builder for creating Streamable HTTP MCP servers
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
    pub fn stateful(mut self, enabled: bool) -> Self {
        self.server_config.stateful_mode = enabled;
        self
    }

    /// Set the MCP service identifier
    pub fn mcp_id(mut self, id: impl Into<String>) -> Self {
        self.server_config.mcp_id = Some(id.into());
        self
    }

    /// Set the tool filter configuration
    pub fn tool_filter(mut self, filter: ToolFilter) -> Self {
        self.server_config.tool_filter = Some(filter);
        self
    }

    /// Override backend isolation (URL defaults to per-session, stdio to shared).
    pub fn backend_isolation(mut self, isolation: BackendIsolation) -> Self {
        self.server_config.backend_isolation = Some(isolation);
        self
    }

    /// Limit concurrent upstream sessions.
    pub fn max_sessions(mut self, max: usize) -> Self {
        self.server_config.max_sessions = Some(max);
        self
    }

    /// Timeout for per-session lazy backend connect.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.server_config.connect_timeout = timeout;
        self
    }

    /// Build the server and return an axum Router, CancellationToken, and a management ProxyHandler.
    ///
    /// For **Shared** isolation the returned handler is the live shared backend.
    /// For **PerSession** isolation it is a disconnected stub (real backends live
    /// inside each session's handler); do not rely on `swap_backend` / availability APIs.
    pub async fn build(self) -> Result<(axum::Router, CancellationToken, ProxyHandler)> {
        let mcp_id = self
            .server_config
            .mcp_id
            .clone()
            .unwrap_or_else(|| "stream-proxy".into());
        let tool_filter = self
            .server_config
            .tool_filter
            .clone()
            .unwrap_or_default();

        let isolation = self.server_config.backend_isolation.unwrap_or(
            match &self.backend_config {
                BackendConfig::Url { .. } => BackendIsolation::for_url(),
                BackendConfig::Stdio { .. } => BackendIsolation::for_stdio(),
            },
        );

        if let (BackendConfig::Stdio { .. }, BackendIsolation::PerSession) =
            (&self.backend_config, isolation)
        {
            bail!("Stdio backend does not support BackendIsolation::PerSession; use Shared");
        }

        let (router, ct, management_handler) = match (&self.backend_config, isolation) {
            (BackendConfig::Stdio { command, args, env }, BackendIsolation::Shared) => {
                let connector = StdioBackendConnector::new(
                    command.clone(),
                    args.clone(),
                    env.clone(),
                    mcp_id.clone(),
                );
                let registry = Arc::new(UpstreamPeerRegistry::new());
                let client = connector.connect_shared(registry).await?;
                let handler = ProxyHandler::with_tool_filter(client, mcp_id.clone(), tool_filter);
                let (router, ct) = self
                    .create_shared_server(handler.clone())
                    .await?;
                (router, ct, handler)
            }
            (BackendConfig::Url { url, headers }, BackendIsolation::Shared) => {
                let connector = Arc::new(
                    UrlBackendConnector::new(url.clone(), headers.clone())
                        .with_connect_timeout(self.server_config.connect_timeout),
                );
                let registry = Arc::new(UpstreamPeerRegistry::new());
                let client = connector.connect_shared(registry).await?;
                let handler = ProxyHandler::with_tool_filter(client, mcp_id.clone(), tool_filter);
                let (router, ct) = self
                    .create_shared_server(handler.clone())
                    .await?;
                (router, ct, handler)
            }
            (BackendConfig::Url { url, headers }, BackendIsolation::PerSession) => {
                let connector: Arc<dyn BackendConnector> = Arc::new(
                    UrlBackendConnector::new(url.clone(), headers.clone())
                        .with_connect_timeout(self.server_config.connect_timeout),
                );
                let (router, ct) = self
                    .create_per_session_server(connector.clone(), mcp_id.clone(), tool_filter.clone())
                    .await?;
                // Management stub (disconnected); real backends live per session.
                let management =
                    ProxyHandler::new_per_session(connector, mcp_id.clone(), tool_filter);
                (router, ct, management)
            }
            (BackendConfig::Stdio { .. }, BackendIsolation::PerSession) => unreachable!(),
        };

        info!(
            "[StreamServerBuilder] Server created - mcp_id: {}, stateful: {}, isolation: {:?}",
            mcp_id, self.server_config.stateful_mode, isolation
        );

        Ok((router, ct, management_handler))
    }

    async fn create_shared_server(
        &self,
        proxy_handler: ProxyHandler,
    ) -> Result<(axum::Router, CancellationToken)> {
        let handler = Arc::new(proxy_handler);
        let ct = CancellationToken::new();

        if self.server_config.stateful_mode {
            let session_manager = ProxyAwareSessionManager::new(handler.clone())
                .with_max_sessions(self.server_config.max_sessions);
            let handler_for_service = handler.clone();
            let mut server_config = StreamableHttpServerConfig::default();
            server_config.stateful_mode = true;
            let service = StreamableHttpService::new(
                move || Ok((*handler_for_service).clone()),
                session_manager.into(),
                server_config,
            );
            Ok((axum::Router::new().fallback_service(service), ct))
        } else {
            use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
            let handler_for_service = handler.clone();
            let server_config = StreamableHttpServerConfig::default();
            let service = StreamableHttpService::new(
                move || Ok((*handler_for_service).clone()),
                LocalSessionManager::default().into(),
                server_config,
            );
            Ok((axum::Router::new().fallback_service(service), ct))
        }
    }

    async fn create_per_session_server(
        &self,
        connector: Arc<dyn BackendConnector>,
        mcp_id: String,
        tool_filter: ToolFilter,
    ) -> Result<(axum::Router, CancellationToken)> {
        if !self.server_config.stateful_mode {
            warn!(
                "Per-session isolation is intended for stateful_mode; enabling stateful_mode"
            );
        }
        let ct = CancellationToken::new();
        // Session manager still needs a template handler for version APIs;
        // use a disconnected per-session template (version stays 0 until connects).
        let template = Arc::new(ProxyHandler::new_per_session(
            connector.clone(),
            mcp_id.clone(),
            tool_filter.clone(),
        ));
        let session_manager = ProxyAwareSessionManager::new(template)
            .with_max_sessions(self.server_config.max_sessions);

        let mut server_config = StreamableHttpServerConfig::default();
        server_config.stateful_mode = true;
        let service = StreamableHttpService::new(
            move || {
                Ok(ProxyHandler::new_per_session(
                    connector.clone(),
                    mcp_id.clone(),
                    tool_filter.clone(),
                ))
            },
            session_manager.into(),
            server_config,
        );
        Ok((axum::Router::new().fallback_service(service), ct))
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

    #[test]
    fn default_isolation_by_backend() {
        assert_eq!(BackendIsolation::for_url(), BackendIsolation::PerSession);
        assert_eq!(BackendIsolation::for_stdio(), BackendIsolation::Shared);
    }
}
