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

use std::sync::Arc;
use dashmap::DashMap;
use futures::Stream;
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::{
        streamable_http_server::session::{
            SessionManager, SessionId,
            local::{LocalSessionManager, LocalSessionManagerError, LocalSessionWorker},
        },
        common::server_side_http::ServerSseMessage,
        WorkerTransport,
    },
};
use tracing::{debug, info};

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
    pub fn new(handler: Arc<ProxyHandler>) -> Self {
        info!("创建 ProxyAwareSessionManager");
        Self {
            inner: LocalSessionManager::default(),
            handler,
            session_versions: DashMap::new(),
        }
    }

    fn check_backend_version(&self, session_id: &SessionId) -> bool {
        if let Some(meta) = self.session_versions.get(session_id.as_ref()) {
            let current_version = self.handler.get_backend_version();
            if meta.backend_version != current_version {
                debug!(
                    "Session {} version mismatch: {} != {}",
                    session_id,
                    meta.backend_version,
                    current_version
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

        debug!(
            "Created session {} with backend version {}",
            session_id, version
        );

        Ok((session_id, transport))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        if !self.handler.is_backend_available() {
            info!(
                "Rejecting session initialization {}: backend not available",
                id
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        if !self.check_backend_version(id) {
            info!(
                "Rejecting session initialization {}: version mismatch",
                id
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        self.inner.initialize_session(id, message).await
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        if !self.check_backend_version(id) {
            return Ok(false);
        }
        self.inner.has_session(id).await
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        self.session_versions.remove(id.as_ref());
        self.inner.close_session(id).await
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        if !self.handler.is_backend_available() {
            info!(
                "Rejecting stream creation {}: backend not available",
                id
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        if !self.check_backend_version(id) {
            info!("Rejecting stream creation {}: version mismatch", id);
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        self.inner.create_stream(id, message).await
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        if !self.handler.is_backend_available() {
            info!(
                "Rejecting message for session {}: backend not available",
                id
            );
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        if !self.check_backend_version(id) {
            info!(
                "Rejecting message for session {}: version mismatch",
                id
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
                info!(
                    "Session {} invalidated: backend version changed ({} -> {})",
                    id, meta.backend_version, current_version
                );

                // 清理失效 session
                drop(meta); // 释放 DashMap 的读锁
                self.session_versions.remove(id.as_ref());
                let _ = self.inner.close_session(id).await;

                return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
            }
        }

        if !self.handler.is_backend_available() {
            info!("Cannot resume session {}: backend not available", id);
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }

        self.inner.resume(id, last_event_id).await
    }
}
