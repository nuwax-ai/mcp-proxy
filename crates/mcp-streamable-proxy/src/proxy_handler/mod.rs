/**
 * Create a local SSE server that proxies requests to a stdio MCP server.
 */
use crate::backend_client::{
    BackendNotificationBridge, ProgressRouteGuard, UpstreamPeerRegistry, UpstreamPeerSlot,
};
use crate::backend_connector::{BackendConnector, BackendIsolation};
use arc_swap::ArcSwapOption;
pub use mcp_common::ToolFilter;
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler, ServiceError,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams, ClientRequest,
        ContentBlock, GetPromptResponse, GetTaskParams, GetTaskRequest, GetTaskResult,
        Implementation, InitializeRequestParams, InitializeResult, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ReadResourceResponse, RequestMetaObject,
        RequestParamsMeta, ServerInfo, ServerResult, SetLevelRequestMethod, SubscribeRequestMethod,
        SubscribeRequestParams, UnsubscribeRequestMethod, UnsubscribeRequestParams,
    },
    service::{NotificationContext, Peer, RequestContext, RunningService},
};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::{debug, error, info, warn};

/// 全局请求计数器，用于生成唯一的请求 ID
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Merge `_meta` from the inbound request context into outbound params.
///
/// rmcp lifts wire `params._meta` into [`RequestContext::meta`], leaving
/// `params.meta` often empty. Without this merge, `call_tool_once` / similar
/// helpers send requests to the backend without `progressToken`.
fn merge_context_meta_into_params<P: RequestParamsMeta>(
    params: &mut P,
    context_meta: &RequestMetaObject,
) {
    if context_meta.is_empty() {
        return;
    }
    let dest = params.meta_or_default();
    for (key, value) in context_meta.iter() {
        dest.entry(key.clone()).or_insert_with(|| value.clone());
    }
}

/// Echo client-requested protocol version when known (mirrors rmcp negotiate).
fn negotiate_protocol_version(
    client_requested: &ProtocolVersion,
    server_fallback: ProtocolVersion,
) -> ProtocolVersion {
    if ProtocolVersion::KNOWN_VERSIONS.contains(client_requested) {
        client_requested.clone()
    } else {
        warn!(
            client_requested = %client_requested,
            server_fallback = %server_fallback,
            "client requested unsupported protocol version; falling back to server default"
        );
        server_fallback
    }
}

/// Convert a backend's [`ServerPeerInfo`] (rmcp 3.1.0 `peer_info()`) into the
/// [`ServerInfo`] (= `InitializeResult`) shape stored in the discovery cache.
///
/// rmcp 3.1.0 narrowed the peer-info type from `InitializeResult` to
/// `ServerPeerInfo`; the proxy caches the full `ServerInfo`, so we rebuild it.
/// `InitializeResult` is `#[non_exhaustive]`, so we use its builder instead of a
/// struct literal. `_meta` has no setter and is dropped (consistent with
/// `default_server_info`, which also omits it).
pub(crate) fn peer_info_to_server_info(peer: rmcp::model::ServerPeerInfo) -> ServerInfo {
    let mut info = InitializeResult::new(peer.capabilities).with_protocol_version(peer.protocol_version);
    if let Some(server_info) = peer.server_info {
        info = info.with_server_info(server_info);
    }
    if let Some(instructions) = peer.instructions {
        info = info.with_instructions(instructions);
    }
    info
}

/// Running backend service with notification bridge.
pub type BackendRunningService = RunningService<RoleClient, BackendNotificationBridge>;

/// 包装后端连接和运行服务
/// 用于 ArcSwap 热替换
#[derive(Debug)]
struct PeerInner {
    /// Peer 用于发送请求
    peer: Peer<RoleClient>,
    /// 保持 RunningService 的所有权，确保服务生命周期
    #[allow(dead_code)]
    _running: Arc<BackendRunningService>,
}

/// Lazy-connect state for [`BackendIsolation::PerSession`].
#[derive(Clone)]
struct PerSessionState {
    connector: Arc<dyn BackendConnector>,
    notify_slot: UpstreamPeerSlot,
    connect_lock: Arc<tokio::sync::Mutex<()>>,
}

impl std::fmt::Debug for PerSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerSessionState")
            .field("notify_slot", &self.notify_slot)
            .finish_non_exhaustive()
    }
}

/// A proxy handler that forwards requests to a client based on the server's capabilities
///
/// **Isolation**:
/// - [`BackendIsolation::Shared`]: one backend, clones share Arcs (stdio).
/// - [`BackendIsolation::PerSession`]: factory creates a disconnected handler; backend
///   is opened during `initialize` (so capabilities are real) and dropped with the session.
#[derive(Clone, Debug)]
pub struct ProxyHandler {
    /// 后端连接（ArcSwap 支持无锁原子替换）
    /// None 表示后端断开/重连中
    peer: Arc<ArcSwapOption<PeerInner>>,
    /// 所有 clone 共享、原子更新的发现快照
    discovery: DiscoveryCache,
    /// MCP ID 用于日志记录
    mcp_id: String,
    /// 工具过滤配置
    tool_filter: ToolFilter,
    /// 后端版本号（每次 swap_backend 递增）
    backend_version: Arc<AtomicU64>,
    /// 上游 Streamable HTTP session peers（shared 模式通知桥）
    upstream_peers: Arc<UpstreamPeerRegistry>,
    /// Backend isolation mode
    isolation: BackendIsolation,
    /// Per-session lazy connect (URL); `None` in shared mode
    per_session: Option<PerSessionState>,
}

mod backend_bridge;
mod connection_swap;
mod discovery;
mod forwarding;
mod lifecycle;
mod raw_dispatch;
mod server_handler;

use discovery::DiscoveryCache;
