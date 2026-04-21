//! Session Manager with backend version tracking
//!
//! This module implements ProxyAwareSessionManager that integrates with
//! ProxyHandler's version control mechanism to automatically invalidate
//! sessions when the backend reconnects.
//!
//! # Architecture
//!
//! ```text
//! ProxyAwareSessionManager
//! ├── LocalSessionManager (rmcp 提供的基础实现)
//! ├── ProxyHandler (Arc, 访问 backend_version)
//! └── DashMap<SessionId, SessionMetadata> (跟踪 session 创建时的版本)
//!
//! 工作流程：
//! 1. create_session: 记录当前 backend_version
//! 2. resume: 检查版本是否匹配
//!    - 匹配 → 正常 resume
//!    - 不匹配 → 返回 NotFound，客户端重新创建 session
//! ```

use dashmap::DashMap;
use futures::Stream;
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::{
        WorkerTransport,
        common::server_side_http::ServerSseMessage,
        streamable_http_server::session::{
            SessionId, SessionManager,
            local::{LocalSessionManager, LocalSessionManagerError, LocalSessionWorker},
        },
    },
};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use super::proxy_handler::ProxyHandler;

/// Session 元数据：跟踪 session 创建时的后端版本
#[derive(Debug, Clone)]
struct SessionMetadata {
    backend_version: u64,
}

/// 感知代理状态的 SessionManager
///
/// 职责：
/// 1. 委托 LocalSessionManager 处理核心 session 逻辑
/// 2. 维护 session → backend_version 映射
/// 3. 在 resume 时检查版本一致性
/// 4. 版本不匹配时使 session 失效
pub struct ProxyAwareSessionManager {
    inner: LocalSessionManager,
    handler: Arc<ProxyHandler>,
    session_versions: DashMap<String, SessionMetadata>,
}

impl ProxyAwareSessionManager {
    /// 默认 session keep_alive 超时: 30 分钟
    /// rmcp 默认 5 分钟太短, 对于代理场景, agent 可能长时间不发消息但仍需要保持 session
    const DEFAULT_SESSION_KEEP_ALIVE_SECS: u64 = 30 * 60;

    pub fn new(handler: Arc<ProxyHandler>) -> Self {
        Self::with_keep_alive(handler, Duration::from_secs(Self::DEFAULT_SESSION_KEEP_ALIVE_SECS))
    }

    pub fn with_keep_alive(handler: Arc<ProxyHandler>, keep_alive: Duration) -> Self {
        info!(
            "[Session Manager] Create ProxyAwareSessionManager - MCP ID: {}, keep_alive: {}s",
            handler.mcp_id(),
            keep_alive.as_secs()
        );
        let mut inner = LocalSessionManager::default();
        inner.session_config.keep_alive = Some(keep_alive);
        Self {
            inner,
            handler,
            session_versions: DashMap::new(),
        }
    }

    fn check_backend_version(&self, session_id: &SessionId) -> bool {
        if let Some(meta) = self.session_versions.get(session_id.as_ref()) {
            let current_version = self.handler.get_backend_version();
            if meta.backend_version != current_version {
                warn!(
                    "[Session version mismatch] session_id={}, creation version={}, current version={}, MCP ID: {}",
                    session_id,
                    meta.backend_version,
                    current_version,
                    self.handler.mcp_id()
                );
                return false;
            }
        }
        true
    }
}

// Implement SessionManager trait
impl SessionManager for ProxyAwareSessionManager {
    type Error = LocalSessionManagerError;
    type Transport = WorkerTransport<LocalSessionWorker>;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        let (session_id, transport) = self.inner.create_session().await?;

        let version = self.handler.get_backend_version();
        self.session_versions.insert(
            session_id.to_string(),
            SessionMetadata {
                backend_version: version,
            },
        );

        info!(
            "[SessionCreated] session_id={}, backend_version={}, MCP ID: {}",
            session_id,
            version,
            self.handler.mcp_id()
        );

        Ok((session_id, transport))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        if !self.handler.is_backend_available() {
            warn!(
                "[Session initialization failed] session_id={}, reason: backend is unavailable, MCP ID: {}",
                id,
                self.handler.mcp_id()
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        if !self.check_backend_version(id) {
            warn!(
                "[Session initialization failed] session_id={}, reason: version mismatch, MCP ID: {}",
                id,
                self.handler.mcp_id()
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        debug!(
            "[Session initialization] session_id={}, MCP ID: {}",
            id,
            self.handler.mcp_id()
        );
        self.inner.initialize_session(id, message).await
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        if !self.check_backend_version(id) {
            return Ok(false);
        }
        self.inner.has_session(id).await
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        info!(
            "[SessionClosed] session_id={}, MCP ID: {}",
            id,
            self.handler.mcp_id()
        );
        self.session_versions.remove(id.as_ref());
        self.inner.close_session(id).await
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        if !self.handler.is_backend_available() {
            warn!(
                "[Stream creation failed] session_id={}, reason: backend is unavailable, MCP ID: {}",
                id,
                self.handler.mcp_id()
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        if !self.check_backend_version(id) {
            warn!(
                "[Stream creation failed] session_id={}, reason: version mismatch, MCP ID: {}",
                id,
                self.handler.mcp_id()
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        debug!(
            "[Stream creation] session_id={}, MCP ID: {}",
            id,
            self.handler.mcp_id()
        );
        self.inner.create_stream(id, message).await
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        if !self.handler.is_backend_available() {
            warn!(
                "[Message rejected] session_id={}, reason: backend unavailable, MCP ID: {}",
                id,
                self.handler.mcp_id()
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        if !self.check_backend_version(id) {
            warn!(
                "[Message rejected] session_id={}, reason: version mismatch, MCP ID: {}",
                id,
                self.handler.mcp_id()
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        self.inner.accept_message(id, message).await
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        self.inner.create_standalone_stream(id).await
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        // 关键：检查后端版本
        if let Some(meta) = self.session_versions.get(id.as_ref()) {
            let current_version = self.handler.get_backend_version();
            if meta.backend_version != current_version {
                warn!(
                    "[Session recovery failed] session_id={}, reason: backend version change ({} -> {}), MCP ID: {}",
                    id,
                    meta.backend_version,
                    current_version,
                    self.handler.mcp_id()
                );

                // 清理失效 session
                drop(meta); // 释放 DashMap 的读锁
                self.session_versions.remove(id.as_ref());
                let _ = self.inner.close_session(id).await;

                return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
            }
        }

        if !self.handler.is_backend_available() {
            warn!(
                "[Session recovery failed] session_id={}, reason: backend is unavailable, MCP ID: {}",
                id,
                self.handler.mcp_id()
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        debug!(
            "[SessionResumed] session_id={}, last_event_id={}, MCP ID: {}",
            id,
            last_event_id,
            self.handler.mcp_id()
        );
        self.inner.resume(id, last_event_id).await
    }
}
