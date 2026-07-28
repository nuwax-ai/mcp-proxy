//! SSE protocol adapter for the shared remote convert runtime.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use mcp_proxy_args::LoadedFallbackJson;
use mcp_sse_proxy::{FallbackMetadata, ServiceExt, stdio as sse_stdio};

use super::common::{HealthChecker, discovery_with_timeout};
use super::discovery_options::DiscoveryMode;
use super::remote_runtime::{
    DiscoveryExport, PreparedHandler, RemoteProtocolAdapter, RuntimeOptions, ToolPreview,
    run_remote_mode,
};
use crate::proxy::{McpClientConfig, ProxyHandler, SseClientConnection, ToolFilter};

impl HealthChecker for ProxyHandler {
    fn is_backend_available(&self) -> bool {
        self.is_backend_available()
    }

    async fn is_terminated_async(&self) -> bool {
        self.is_terminated_async().await
    }
}

#[derive(Clone, Copy)]
struct SseAdapter;

#[async_trait]
impl RemoteProtocolAdapter for SseAdapter {
    type Connection = SseClientConnection;
    type Handler = ProxyHandler;
    type ImportedFallback = FallbackMetadata;

    fn protocol_name(&self) -> &'static str {
        "SSE"
    }

    fn parse_import(&self, json: LoadedFallbackJson) -> Result<Self::ImportedFallback> {
        let sources = format!(
            "initialize={}, tools={}",
            json.initialize.source, json.tools.source
        );
        FallbackMetadata::from_json(&json.initialize.json, &json.tools.json)
            .with_context(|| format!("failed to parse SSE fallback metadata ({sources})"))
    }

    async fn connect_initial(&self, config: McpClientConfig) -> Result<Self::Connection> {
        SseClientConnection::connect(config).await
    }

    async fn connect_reconnect(
        &self,
        config: McpClientConfig,
        _handler: &Arc<Self::Handler>,
    ) -> Result<Self::Connection> {
        SseClientConnection::connect(config).await
    }

    async fn export_discovery(&self, connection: &Self::Connection) -> Result<DiscoveryExport> {
        let snapshot = discovery_with_timeout(
            self.protocol_name(),
            "export discovery",
            connection.fetch_discovery_snapshot(),
        )
        .await?;
        Ok(DiscoveryExport {
            initialize_json: snapshot.initialize_json()?,
            tools_json: snapshot.tools_json()?,
        })
    }

    async fn preview_tools(&self, connection: &Self::Connection) -> Result<Vec<ToolPreview>> {
        let tools = discovery_with_timeout(
            self.protocol_name(),
            "tool preview",
            connection.list_tools(),
        )
        .await?;
        Ok(tools
            .into_iter()
            .map(|tool| ToolPreview {
                name: tool.name,
                description: tool.description,
            })
            .collect())
    }

    async fn build_handler(
        &self,
        connection: Option<Self::Connection>,
        fallback: Option<Self::ImportedFallback>,
        filter: ToolFilter,
    ) -> Result<PreparedHandler<Self::Handler>> {
        let initially_connected = connection.is_some();
        let live_snapshot = if fallback.is_some() {
            match connection.as_ref() {
                Some(connection) => match discovery_with_timeout(
                    self.protocol_name(),
                    "initial discovery",
                    connection.fetch_discovery_snapshot(),
                )
                .await
                {
                    Ok(snapshot) => Some(snapshot),
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "real SSE tools discovery failed; retaining imported fallback"
                        );
                        None
                    }
                },
                None => None,
            }
        } else {
            None
        };
        let imported = fallback.map(FallbackMetadata::into_snapshot);
        let initial_info = live_snapshot
            .as_ref()
            .map(|snapshot| snapshot.server_info.clone())
            .or_else(|| {
                connection
                    .as_ref()
                    .and_then(|connection| connection.peer_info().cloned())
            })
            .or_else(|| {
                imported
                    .as_ref()
                    .map(|snapshot| snapshot.server_info.clone())
            })
            .context("SSE fallback is missing initialize metadata")?;
        let initial_tools = live_snapshot
            .as_ref()
            .map(|snapshot| snapshot.tools.clone())
            .or_else(|| imported.as_ref().map(|snapshot| snapshot.tools.clone()));
        let handler = Arc::new(ProxyHandler::new_disconnected_with_fallback(
            "cli".to_string(),
            filter,
            initial_info,
            initial_tools,
        ));

        if let Some(connection) = connection {
            let live_tools = live_snapshot.map(|snapshot| snapshot.tools);
            handler
                .swap_backend_with_discovery(Some(connection.into_running_service()), live_tools);
        } else {
            tracing::warn!(
                "starting SSE stdio with imported discovery fallback while disconnected"
            );
        }

        Ok(PreparedHandler {
            handler,
            initially_connected,
        })
    }

    async fn install_reconnected(
        &self,
        connection: Self::Connection,
        handler: &Arc<Self::Handler>,
    ) -> Result<()> {
        let complete_tools = match discovery_with_timeout(
            self.protocol_name(),
            "reconnect discovery",
            connection.fetch_discovery_snapshot(),
        )
        .await
        {
            Ok(snapshot) => Some(snapshot.tools),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "SSE reconnect tools discovery failed; retaining previous cache"
                );
                None
            }
        };
        handler
            .swap_backend_with_discovery(Some(connection.into_running_service()), complete_tools);
        Ok(())
    }

    fn disconnect(&self, handler: &Self::Handler) {
        handler.swap_backend(None);
    }

    async fn serve_stdio(&self, handler: Arc<Self::Handler>) -> Result<()> {
        let server = (*handler)
            .clone()
            .serve(sse_stdio())
            .await
            .context("failed to start SSE stdio server")?;
        server
            .waiting()
            .await
            .context("SSE stdio server terminated with an error")
            .map(|_| ())
    }
}

pub(super) async fn run_sse_mode(
    config: McpClientConfig,
    discovery_mode: DiscoveryMode,
    tool_filter: ToolFilter,
    options: RuntimeOptions,
) -> Result<()> {
    run_remote_mode(SseAdapter, config, discovery_mode, tool_filter, options).await
}
