use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::FutureExt;
use mcp_proxy_args::LoadedFallbackJson;

use super::common::{HealthChecker, InitialConnectOutcome, connect_with_initial_retry};
use super::discovery_options::DiscoveryMode;
use crate::client::support::ConvertArgs;
use crate::proxy::{McpClientConfig, ToolFilter};

mod retry;
mod watchdog;

use watchdog::run_watchdog;

pub(super) struct DiscoveryExport {
    pub initialize_json: String,
    pub tools_json: String,
}

pub(super) struct PreparedHandler<H> {
    pub handler: Arc<H>,
    pub initially_connected: bool,
}

pub(super) struct ToolPreview {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone)]
pub(super) struct RuntimeOptions {
    retries: u32,
    ping_interval: u64,
    ping_timeout: u64,
    diagnostic: bool,
    verbose: bool,
    quiet: bool,
}

impl RuntimeOptions {
    pub fn from_args(args: &ConvertArgs, verbose: bool, quiet: bool) -> Self {
        Self {
            retries: args.retries,
            ping_interval: args.ping_interval,
            ping_timeout: args.ping_timeout,
            diagnostic: args.logging.diagnostic,
            verbose,
            quiet,
        }
    }
}

#[async_trait]
pub(super) trait RemoteProtocolAdapter: Clone + Send + Sync + 'static {
    type Connection: Send + Sync;
    type Handler: HealthChecker + Send + Sync + 'static;
    type ImportedFallback: Send;

    fn protocol_name(&self) -> &'static str;

    fn parse_import(&self, json: LoadedFallbackJson) -> Result<Self::ImportedFallback>;

    async fn connect_initial(&self, config: McpClientConfig) -> Result<Self::Connection>;

    async fn connect_reconnect(
        &self,
        config: McpClientConfig,
        handler: &Arc<Self::Handler>,
    ) -> Result<Self::Connection>;

    async fn export_discovery(&self, connection: &Self::Connection) -> Result<DiscoveryExport>;

    async fn preview_tools(&self, connection: &Self::Connection) -> Result<Vec<ToolPreview>>;

    async fn build_handler(
        &self,
        connection: Option<Self::Connection>,
        fallback: Option<Self::ImportedFallback>,
        filter: ToolFilter,
    ) -> Result<PreparedHandler<Self::Handler>>;

    async fn install_reconnected(
        &self,
        connection: Self::Connection,
        handler: &Arc<Self::Handler>,
    ) -> Result<()>;

    fn disconnect(&self, handler: &Self::Handler);

    async fn serve_stdio(&self, handler: Arc<Self::Handler>) -> Result<()>;
}

pub(super) async fn run_remote_mode<A>(
    adapter: A,
    config: McpClientConfig,
    discovery_mode: DiscoveryMode,
    tool_filter: ToolFilter,
    options: RuntimeOptions,
) -> Result<()>
where
    A: RemoteProtocolAdapter,
{
    let protocol = adapter.protocol_name();
    let (fallback, export_targets) = match discovery_mode {
        DiscoveryMode::Normal => (None, None),
        DiscoveryMode::Import(json) => (Some(adapter.parse_import(json)?), None),
        DiscoveryMode::Export(targets) => (None, Some(targets)),
    };

    tracing::info!(
        protocol,
        url = %config.url,
        ping_interval = options.ping_interval,
        ping_timeout = options.ping_timeout,
        "Starting remote convert mode"
    );
    if !options.quiet {
        eprintln!("🔗 Connecting to backend service ({protocol})...");
    }

    let initial = connect_with_initial_retry(protocol, options.quiet, || {
        adapter.connect_initial(config.clone())
    })
    .await;
    let (connection, initial_elapsed) = match initial {
        InitialConnectOutcome::Connected {
            connection,
            elapsed,
        } => (Some(connection), elapsed),
        InitialConnectOutcome::Exhausted { error, elapsed } if fallback.is_some() => {
            tracing::warn!(
                protocol,
                %error,
                "Initial connection exhausted; starting from imported fallback"
            );
            (None, elapsed)
        }
        InitialConnectOutcome::Exhausted { error, elapsed } => {
            anyhow::bail!(
                "all initial {protocol} connection attempts failed after {:.1}s: {error:#}",
                elapsed.as_secs_f64()
            );
        }
    };

    if let Some(targets) = export_targets {
        let connection = connection
            .as_ref()
            .context("export requires a successful real upstream connection")?;
        let export = adapter.export_discovery(connection).await?;
        return super::export::publish_discovery(
            &targets,
            &export.initialize_json,
            &export.tools_json,
        );
    }

    if let Some(connection) = connection.as_ref() {
        tracing::info!(
            protocol,
            elapsed = ?initial_elapsed,
            "Backend connected successfully"
        );
        if !options.quiet {
            eprintln!("✅ Backend connected successfully");
            if fallback.is_none() {
                print_tool_preview(&adapter, connection).await;
            }
        }
    }

    let prepared = adapter
        .build_handler(connection, fallback, tool_filter)
        .await?;
    let handler = prepared.handler;
    let watchdog = tokio::spawn({
        let adapter = adapter.clone();
        let handler = handler.clone();
        let options = options.clone();
        async move {
            if AssertUnwindSafe(run_watchdog(
                adapter,
                handler,
                config,
                options,
                prepared.initially_connected,
            ))
            .catch_unwind()
            .await
            .is_err()
            {
                tracing::error!("Remote watchdog panicked and exited unexpectedly");
            }
        }
    });

    let result = adapter.serve_stdio(handler).await;
    watchdog.abort();
    if let Err(error) = watchdog.await
        && !error.is_cancelled()
    {
        tracing::error!(%error, "Remote watchdog task failed while shutting down");
    }
    result
}

async fn print_tool_preview<A>(adapter: &A, connection: &A::Connection)
where
    A: RemoteProtocolAdapter,
{
    match adapter.preview_tools(connection).await {
        Ok(tools) if tools.is_empty() => {
            eprintln!("⚠️  Tool list is empty (tools/list returned 0 tools)");
        }
        Ok(tools) => {
            eprintln!("🔧 Available tools ({}):", tools.len());
            for tool in tools {
                let description = tool.description.as_deref().unwrap_or("no description");
                eprintln!(
                    "   - {} : {}",
                    tool.name,
                    crate::client::support::utils::truncate_str(description, 50)
                );
            }
        }
        Err(error) => eprintln!("⚠️  Failed to list tools: {error}"),
    }
}

#[cfg(test)]
#[path = "remote_runtime/tests.rs"]
mod tests;
