//! Backend client handler that bridges server notifications to upstream peers.
//!
//! Two isolation modes share one [`BackendNotificationBridge`] type so
//! [`crate::proxy_handler::BackendRunningService`] stays a single concrete type:
//!
//! - **Shared** ([`NotifyTarget::Shared`]): one backend, fan-out / progress·task
//!   routing via [`UpstreamPeerRegistry`] (stdio proxy).
//! - **Per-session** ([`NotifyTarget::Session`]): one backend connection owns one
//!   upstream [`Peer`](Peer); notifications are 1:1 (URL proxy).

use dashmap::DashMap;
use futures::future::join_all;
use rmcp::{
    ClientHandler, RoleClient, RoleServer,
    model::{
        ClientCapabilities, ClientInfo, Implementation, ProgressNotificationParam, ProgressToken,
        ResourceUpdatedNotificationParam, ServerNotification, TaskStatus, TaskStatusNotification,
        TaskStatusNotificationParam, TasksCapability,
    },
    service::{NotificationContext, Peer, ServiceError},
};
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, warn};

/// Delay before unregistering a progress route after the owning request finishes.
///
/// Gives in-flight backend progress notifications a short window to arrive after
/// the JSON-RPC response has already been returned to the upstream client.
pub const PROGRESS_ROUTE_GRACE: Duration = Duration::from_millis(500);

/// Slot for the single upstream peer in per-session isolation mode.
#[derive(Clone, Default, Debug)]
pub struct UpstreamPeerSlot {
    peer: Arc<RwLock<Option<Peer<RoleServer>>>>,
}

impl UpstreamPeerSlot {
    /// Create an empty slot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the upstream session peer (typically from `on_initialized`).
    pub fn set(&self, peer: Peer<RoleServer>) {
        match self.peer.write() {
            Ok(mut guard) => *guard = Some(peer),
            Err(poisoned) => *poisoned.into_inner() = Some(peer),
        }
    }

    /// Clone the bound peer, if any.
    pub fn get(&self) -> Option<Peer<RoleServer>> {
        match self.peer.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Clear the slot (session closing).
    pub fn clear(&self) {
        match self.peer.write() {
            Ok(mut guard) => *guard = None,
            Err(poisoned) => *poisoned.into_inner() = None,
        }
    }
}

/// How backend notifications are delivered to upstream Streamable HTTP sessions.
#[derive(Clone, Debug)]
pub enum NotifyTarget {
    /// Shared backend: fan-out list-changed / logging; route progress/task.
    Shared(Arc<UpstreamPeerRegistry>),
    /// Per-session backend: deliver only to this session's upstream peer.
    Session(UpstreamPeerSlot),
}

/// Registry of upstream session peers that should receive backend notifications.
#[derive(Debug, Default)]
pub struct UpstreamPeerRegistry {
    next_id: AtomicU64,
    peers: DashMap<u64, Peer<RoleServer>>,
    /// progressToken → upstream peer (request-scoped; not broadcast)
    progress_routes: DashMap<ProgressToken, Peer<RoleServer>>,
    /// task_id → upstream peer (task-scoped; not broadcast)
    task_routes: DashMap<String, Peer<RoleServer>>,
}

impl UpstreamPeerRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an upstream peer (typically from `ServerHandler::on_initialized`).
    pub fn register(&self, peer: Peer<RoleServer>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.peers.insert(id, peer);
        id
    }

    /// Explicitly remove a registered peer by id.
    pub fn unregister(&self, id: u64) {
        self.peers.remove(&id);
    }

    /// Number of registered peers (for diagnostics).
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether the registry has no peers.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Bind a progress token to the upstream peer that owns the request.
    pub fn register_progress(&self, token: ProgressToken, peer: Peer<RoleServer>) {
        self.progress_routes.insert(token, peer);
    }

    /// Remove a progress-token route (request finished or cancelled).
    pub fn unregister_progress(&self, token: &ProgressToken) {
        self.progress_routes.remove(token);
    }

    /// Bind a task id to the upstream peer that created/enqueued the task.
    pub fn register_task(&self, task_id: impl Into<String>, peer: Peer<RoleServer>) {
        self.task_routes.insert(task_id.into(), peer);
    }

    /// Remove a task-id route.
    pub fn unregister_task(&self, task_id: &str) {
        self.task_routes.remove(task_id);
    }

    /// Drop peers whose transport is already closed (eager cleanup on session close).
    pub fn reap_closed_peers(&self) {
        self.peers.retain(|_, peer| !peer.is_transport_closed());
    }

    /// Deliver a progress notification to the owning upstream peer only.
    pub async fn deliver_progress(&self, params: ProgressNotificationParam) {
        let token = params.progress_token.clone();
        let Some(peer) = self
            .progress_routes
            .get(&token)
            .map(|entry| entry.value().clone())
        else {
            debug!(
                ?token,
                "No progress route for token; dropping backend progress notification"
            );
            return;
        };

        if peer.is_transport_closed() {
            self.progress_routes.remove(&token);
            return;
        }

        if let Err(err) = peer.notify_progress(params).await {
            warn!(?token, error = ?err, "Failed to deliver progress; removing route");
            self.progress_routes.remove(&token);
        }
    }

    /// Deliver a task-status notification to the owning upstream peer only.
    pub async fn deliver_task_status(&self, params: TaskStatusNotificationParam) {
        let task_id = params.task.task_id.clone();
        let terminal = matches!(
            params.task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        );

        let Some(peer) = self.task_routes.get(&task_id).map(|e| e.value().clone()) else {
            debug!(
                %task_id,
                "No task route for id; dropping backend task status notification"
            );
            return;
        };

        if peer.is_transport_closed() {
            self.task_routes.remove(&task_id);
            return;
        }

        let send_result = peer
            .send_notification(ServerNotification::TaskStatusNotification(
                TaskStatusNotification::new(params),
            ))
            .await;

        if let Err(err) = send_result {
            warn!(%task_id, error = ?err, "Failed to deliver task status; removing route");
            self.task_routes.remove(&task_id);
            return;
        }

        if terminal {
            self.task_routes.remove(&task_id);
        }
    }

    /// Fan-out a notification to all peers; remove peers that fail to send.
    pub async fn fan_out<F, Fut>(&self, mut send: F)
    where
        F: FnMut(Peer<RoleServer>) -> Fut,
        Fut: std::future::Future<Output = Result<(), ServiceError>>,
    {
        let snapshot: Vec<(u64, Peer<RoleServer>)> = self
            .peers
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect();

        let mut dead = Vec::new();
        let mut live = Vec::new();
        for (id, peer) in snapshot {
            if peer.is_transport_closed() {
                dead.push(id);
            } else {
                live.push((id, peer));
            }
        }

        let results = join_all(live.into_iter().map(|(id, peer)| {
            let fut = send(peer);
            async move { (id, fut.await) }
        }))
        .await;

        for (id, result) in results {
            if let Err(err) = result {
                warn!(
                    peer_id = id,
                    error = ?err,
                    "Failed to fan-out notification to upstream peer; removing"
                );
                dead.push(id);
            }
        }

        for id in dead {
            self.peers.remove(&id);
        }
    }
}

/// RAII guard that unregisters a progress route when dropped (after a short grace).
pub struct ProgressRouteGuard {
    registry: Arc<UpstreamPeerRegistry>,
    token: Option<ProgressToken>,
    grace: Duration,
}

impl ProgressRouteGuard {
    /// Register `token` → `peer` and return a guard that cleans up on drop.
    pub fn register(
        registry: Arc<UpstreamPeerRegistry>,
        token: ProgressToken,
        peer: Peer<RoleServer>,
    ) -> Self {
        registry.register_progress(token.clone(), peer);
        Self {
            registry,
            token: Some(token),
            grace: PROGRESS_ROUTE_GRACE,
        }
    }

    /// Register from request meta if a progress token is present.
    pub fn try_from_meta(
        registry: Arc<UpstreamPeerRegistry>,
        meta: &rmcp::model::RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Option<Self> {
        let token = meta.get_progress_token()?;
        Some(Self::register(registry, token, peer))
    }
}

impl Drop for ProgressRouteGuard {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        let registry = Arc::clone(&self.registry);
        let grace = self.grace;
        // Prefer delayed unregister so late progress can still be routed.
        // Fall back to immediate cleanup outside a Tokio runtime.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    tokio::time::sleep(grace).await;
                    registry.unregister_progress(&token);
                });
            }
            Err(_) => {
                registry.unregister_progress(&token);
            }
        }
    }
}

/// Client-side handler used when the proxy connects to a backend MCP server.
#[derive(Clone, Debug)]
pub struct BackendNotificationBridge {
    info: ClientInfo,
    target: NotifyTarget,
}

impl BackendNotificationBridge {
    /// Create a bridge with the given client info and notify target.
    pub fn new(info: ClientInfo, target: NotifyTarget) -> Self {
        Self { info, target }
    }

    /// Shared-backend bridge (fan-out / routed progress).
    pub fn with_default_info(upstream_peers: Arc<UpstreamPeerRegistry>) -> Self {
        Self::new(
            default_backend_client_info(),
            NotifyTarget::Shared(upstream_peers),
        )
    }

    /// Per-session bridge (1:1 notify to [`UpstreamPeerSlot`]).
    pub fn for_session(slot: UpstreamPeerSlot) -> Self {
        Self::new(default_backend_client_info(), NotifyTarget::Session(slot))
    }

    /// Shared upstream peer registry when target is [`NotifyTarget::Shared`].
    pub fn upstream_peers(&self) -> Option<&Arc<UpstreamPeerRegistry>> {
        match &self.target {
            NotifyTarget::Shared(reg) => Some(reg),
            NotifyTarget::Session(_) => None,
        }
    }

    /// Session peer slot when target is [`NotifyTarget::Session`].
    pub fn session_slot(&self) -> Option<&UpstreamPeerSlot> {
        match &self.target {
            NotifyTarget::Session(slot) => Some(slot),
            NotifyTarget::Shared(_) => None,
        }
    }
}

/// Default [`ClientInfo`] for proxy→backend handshake (enables task notifications).
pub fn default_backend_client_info() -> ClientInfo {
    #[allow(deprecated)]
    let capabilities = ClientCapabilities::builder()
        .enable_experimental()
        .enable_tasks_with(TasksCapability::client_default())
        .build();
    ClientInfo::new(
        capabilities,
        Implementation::new("mcp-streamable-proxy-client", env!("CARGO_PKG_VERSION")),
    )
}

async fn with_session_peer<F, Fut>(slot: &UpstreamPeerSlot, send: F)
where
    F: FnOnce(Peer<RoleServer>) -> Fut,
    Fut: std::future::Future<Output = Result<(), ServiceError>>,
{
    let Some(peer) = slot.get() else {
        debug!("Session notify slot empty; dropping backend notification");
        return;
    };
    if peer.is_transport_closed() {
        slot.clear();
        return;
    }
    if let Err(err) = send(peer).await {
        warn!(error = ?err, "Failed to deliver notification to session peer");
        slot.clear();
    }
}

impl ClientHandler for BackendNotificationBridge {
    fn get_info(&self) -> ClientInfo {
        self.info.clone()
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        match &self.target {
            NotifyTarget::Shared(reg) => {
                debug!(uri = %params.uri, "Fan-out resources/updated to upstream peers");
                reg.fan_out(|peer| {
                    let params = params.clone();
                    async move { peer.notify_resource_updated(params).await }
                })
                .await;
            }
            NotifyTarget::Session(slot) => {
                with_session_peer(slot, |peer| async move {
                    peer.notify_resource_updated(params).await
                })
                .await;
            }
        }
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        match &self.target {
            NotifyTarget::Shared(reg) => {
                reg.fan_out(|peer| async move { peer.notify_resource_list_changed().await })
                    .await;
            }
            NotifyTarget::Session(slot) => {
                with_session_peer(slot, |peer| async move {
                    peer.notify_resource_list_changed().await
                })
                .await;
            }
        }
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        match &self.target {
            NotifyTarget::Shared(reg) => {
                reg.fan_out(|peer| async move { peer.notify_tool_list_changed().await })
                    .await;
            }
            NotifyTarget::Session(slot) => {
                with_session_peer(slot, |peer| async move { peer.notify_tool_list_changed().await })
                    .await;
            }
        }
    }

    async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
        match &self.target {
            NotifyTarget::Shared(reg) => {
                reg.fan_out(|peer| async move { peer.notify_prompt_list_changed().await })
                    .await;
            }
            NotifyTarget::Session(slot) => {
                with_session_peer(slot, |peer| async move {
                    peer.notify_prompt_list_changed().await
                })
                .await;
            }
        }
    }

    #[allow(deprecated)]
    async fn on_logging_message(
        &self,
        params: rmcp::model::LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        match &self.target {
            NotifyTarget::Shared(reg) => {
                reg.fan_out(|peer| {
                    let params = params.clone();
                    async move { peer.notify_logging_message(params).await }
                })
                .await;
            }
            NotifyTarget::Session(slot) => {
                with_session_peer(slot, |peer| async move {
                    peer.notify_logging_message(params).await
                })
                .await;
            }
        }
    }

    async fn on_task_status(
        &self,
        params: TaskStatusNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        match &self.target {
            NotifyTarget::Shared(reg) => {
                reg.deliver_task_status(params).await;
            }
            NotifyTarget::Session(slot) => {
                with_session_peer(slot, |peer| async move {
                    peer.send_notification(ServerNotification::TaskStatusNotification(
                        TaskStatusNotification::new(params),
                    ))
                    .await
                })
                .await;
            }
        }
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        match &self.target {
            NotifyTarget::Shared(reg) => {
                reg.deliver_progress(params).await;
            }
            NotifyTarget::Session(slot) => {
                with_session_peer(slot, |peer| async move { peer.notify_progress(params).await })
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_route_register_unregister() {
        let registry = UpstreamPeerRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        registry.unregister(999);
        registry.unregister_progress(&ProgressToken(rmcp::model::NumberOrString::Number(1)));
        registry.unregister_task("nope");
    }

    #[test]
    fn upstream_peer_slot_set_clear() {
        let slot = UpstreamPeerSlot::new();
        assert!(slot.get().is_none());
        slot.clear();
        assert!(slot.get().is_none());
    }
}
