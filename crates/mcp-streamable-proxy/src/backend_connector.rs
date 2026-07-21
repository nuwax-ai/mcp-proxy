//! Backend connection factories for shared (stdio) and per-session (URL) isolation.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use rmcp::{
    ServiceExt,
    transport::{
        TokioChildProcess,
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
    },
};
use tracing::info;

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};

use crate::backend_client::{BackendNotificationBridge, UpstreamPeerRegistry, UpstreamPeerSlot};
use crate::proxy_handler::BackendRunningService;

/// How upstream sessions share (or isolate) backend MCP connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendIsolation {
    /// One backend connection shared by all upstream sessions (stdio default).
    #[default]
    Shared,
    /// Each upstream session opens its own backend connection (URL default).
    PerSession,
}

impl BackendIsolation {
    /// Default isolation for a URL backend.
    pub fn for_url() -> Self {
        Self::PerSession
    }

    /// Default isolation for a stdio backend.
    pub fn for_stdio() -> Self {
        Self::Shared
    }
}

/// Creates backend [`BackendRunningService`] instances for the proxy.
#[async_trait]
pub trait BackendConnector: Send + Sync {
    /// Connect using a shared fan-out registry (stdio / shared isolation).
    ///
    /// For stdio, call at most once at process startup.
    async fn connect_shared(
        &self,
        registry: Arc<UpstreamPeerRegistry>,
    ) -> Result<BackendRunningService>;

    /// Connect for one session with a 1:1 notify slot (URL / per-session).
    async fn connect_session(&self, slot: UpstreamPeerSlot) -> Result<BackendRunningService>;

    /// Timeout used when awaiting lazy per-session connect.
    fn connect_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Connect to a remote Streamable HTTP MCP URL (new connection each call).
#[derive(Debug, Clone)]
pub struct UrlBackendConnector {
    pub url: String,
    pub headers: Option<HashMap<String, String>>,
    pub connect_timeout: Duration,
}

impl UrlBackendConnector {
    /// Create a URL connector.
    pub fn new(url: impl Into<String>, headers: Option<HashMap<String, String>>) -> Self {
        Self {
            url: url.into(),
            headers,
            connect_timeout: Duration::from_secs(30),
        }
    }

    /// Override connect timeout used by lazy session connect.
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    async fn serve_bridge(
        &self,
        bridge: BackendNotificationBridge,
    ) -> Result<BackendRunningService> {
        let mut req_headers = reqwest::header::HeaderMap::new();
        let mut auth_header: Option<String> = None;

        if let Some(config_headers) = &self.headers {
            for (key, value) in config_headers {
                if key.eq_ignore_ascii_case("Authorization") {
                    auth_header = Some(value.strip_prefix("Bearer ").unwrap_or(value).to_string());
                    continue;
                }
                req_headers.insert(
                    reqwest::header::HeaderName::try_from(key)
                        .with_context(|| format!("Invalid header name '{key}'"))?,
                    value
                        .parse()
                        .with_context(|| format!("Invalid header value for '{key}'"))?,
                );
            }
        }

        let http_client = reqwest::Client::builder()
            .default_headers(req_headers)
            .connect_timeout(self.connect_timeout)
            .build()
            .context("Failed to create HTTP client")?;

        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url.clone());
        config.auth_header = auth_header;

        let transport = StreamableHttpClientTransport::with_client(http_client, config);
        let client = bridge
            .serve(transport)
            .await
            .context("Failed to connect URL backend")?;
        info!(url = %self.url, "URL backend connected");
        Ok(client)
    }
}

#[async_trait]
impl BackendConnector for UrlBackendConnector {
    async fn connect_shared(
        &self,
        registry: Arc<UpstreamPeerRegistry>,
    ) -> Result<BackendRunningService> {
        self.serve_bridge(BackendNotificationBridge::with_default_info(registry))
            .await
    }

    async fn connect_session(&self, slot: UpstreamPeerSlot) -> Result<BackendRunningService> {
        self.serve_bridge(BackendNotificationBridge::for_session(slot))
            .await
    }

    fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }
}

/// Stdio child-process backend (shared isolation only).
pub struct StdioBackendConnector {
    command: String,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
    mcp_id: String,
}

impl StdioBackendConnector {
    /// Create a stdio connector.
    pub fn new(
        command: impl Into<String>,
        args: Option<Vec<String>>,
        env: Option<HashMap<String, String>>,
        mcp_id: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            env,
            mcp_id: mcp_id.into(),
        }
    }

    async fn spawn(&self, registry: Arc<UpstreamPeerRegistry>) -> Result<BackendRunningService> {
        let args = self.args.clone();
        let mut wrapped_cmd = CommandWrap::with_new(&self.command, |cmd| {
            if let Some(cmd_args) = &args {
                cmd.args(cmd_args);
            }
            if let Some(env_vars) = &self.env {
                for (k, v) in env_vars {
                    cmd.env(k, v);
                }
            }
        });

        #[cfg(unix)]
        wrapped_cmd.wrap(ProcessGroup::leader());

        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
            wrapped_cmd.wrap(CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP));
            wrapped_cmd.wrap(JobObject);
        }

        wrapped_cmd.wrap(KillOnDrop);

        info!(
            command = %self.command,
            args = ?args.as_ref().unwrap_or(&vec![]),
            "Starting stdio backend child process"
        );

        mcp_common::diagnostic::log_stdio_spawn_context(
            "StdioBackendConnector",
            &self.mcp_id,
            &self.env,
        );

        let (tokio_process, child_stderr) = TokioChildProcess::builder(wrapped_cmd)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                anyhow::anyhow!(
                    "{}",
                    mcp_common::diagnostic::format_spawn_error(
                        &self.mcp_id,
                        &self.command,
                        &self.args,
                        e
                    )
                )
            })?;

        if let Some(stderr_pipe) = child_stderr {
            mcp_common::spawn_stderr_reader(stderr_pipe, self.mcp_id.clone());
        }

        let bridge = BackendNotificationBridge::with_default_info(registry);
        let client = bridge
            .serve(tokio_process)
            .await
            .context("Failed to connect stdio backend")?;
        info!(mcp_id = %self.mcp_id, "Stdio backend connected");
        Ok(client)
    }
}

#[async_trait]
impl BackendConnector for StdioBackendConnector {
    async fn connect_shared(
        &self,
        registry: Arc<UpstreamPeerRegistry>,
    ) -> Result<BackendRunningService> {
        self.spawn(registry).await
    }

    async fn connect_session(&self, _slot: UpstreamPeerSlot) -> Result<BackendRunningService> {
        anyhow::bail!(
            "Stdio backend does not support per-session isolation in this release; \
             use BackendIsolation::Shared"
        )
    }
}
