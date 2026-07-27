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
        CallToolRequest, CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams,
        CancelTaskRequest, ClientRequest, ContentBlock, CreateTaskResult, GetPromptResponse,
        GetTaskParams, GetTaskPayloadParams, GetTaskPayloadRequest, GetTaskRequest, GetTaskResult,
        Implementation, InitializeRequestParams, InitializeResult, ListTasksRequest,
        ListTasksResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ReadResourceResponse, RequestMetaObject, RequestParamsMeta, ServerInfo, ServerResult,
        SetLevelRequestMethod, SubscribeRequestMethod, SubscribeRequestParams,
        UnsubscribeRequestMethod, UnsubscribeRequestParams,
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
    /// 缓存的服务器信息（ArcSwap：swap_backend 时刷新，所有 clone 可见）
    cached_info: Arc<arc_swap::ArcSwap<ServerInfo>>,
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

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> ServerInfo {
        (**self.cached_info.load()).clone()
    }

    /// Connect per-session backends before returning capabilities.
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());

        if let Some(session) = &self.per_session {
            // Bind notify slot before connect so backend notifications during handshake work.
            session.notify_slot.set(context.peer.clone());
            self.ensure_connected().await?;
            info!(
                "[initialize] Per-session backend ready - MCP ID: {}",
                self.mcp_id
            );
        }

        let mut info = self.get_info();
        info.protocol_version =
            negotiate_protocol_version(&request.protocol_version, info.protocol_version);
        Ok(info)
    }

    #[tracing::instrument(skip(self, request, context), fields(
        mcp_id = %self.mcp_id,
        request = ?request,
    ))]
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        match self.capabilities().tools {
            Some(_) => {
                let result = self
                    .forward_backend(
                        &context,
                        |peer| async move { peer.list_tools(request).await },
                    )
                    .await?;

                let filtered_tools: Vec<_> = if self.tool_filter.is_enabled() {
                    result
                        .tools
                        .into_iter()
                        .filter(|tool| self.tool_filter.is_allowed(&tool.name))
                        .collect()
                } else {
                    result.tools
                };

                info!(
                    "[list_tools] Tool list results - MCP ID: {}, number of tools: {}{}",
                    self.mcp_id,
                    filtered_tools.len(),
                    if self.tool_filter.is_enabled() {
                        " (filtered)"
                    } else {
                        ""
                    }
                );

                debug!(
                    "Proxying list_tools response with {} tools",
                    filtered_tools.len()
                );
                Ok(ListToolsResult {
                    tools: filtered_tools,
                    next_cursor: result.next_cursor,
                    meta: result.meta,
                    result_type: result.result_type,
                    ttl_ms: result.ttl_ms,
                    cache_scope: result.cache_scope,
                })
            }
            None => {
                warn!("Server doesn't support tools capability");
                Ok(ListToolsResult::default())
            }
        }
    }

    #[tracing::instrument(skip(self, request, context), fields(
        mcp_id = %self.mcp_id,
        tool_name = %request.name,
        tool_arguments = ?request.arguments,
    ))]
    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let request_id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        let tool_name = request.name.clone();

        info!(
            "[call_tool:{}] Start - Tool: {}, MCP ID: {}",
            request_id, tool_name, self.mcp_id
        );

        if !self.tool_filter.is_allowed(&tool_name) {
            info!(
                "[call_tool:{}] Tool is filtered - MCP ID: {}, Tool: {}",
                request_id, self.mcp_id, tool_name
            );
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Tool '{tool_name}' is not allowed by filter configuration"
            ))])
            .into());
        }

        if self.capabilities().tools.is_none() {
            error!(
                "[call_tool:{}] The server does not support tools capability - MCP ID: {}",
                request_id, self.mcp_id
            );
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "Server doesn't support tools capability",
            )])
            .into());
        }

        merge_context_meta_into_params(&mut request, &context.meta);
        let _progress_guard = self.maybe_progress_guard(&context);

        info!(
            "[call_tool:{}] Send request to backend... - Tool: {}, Elapsed time: {}ms",
            request_id,
            tool_name,
            start.elapsed().as_millis()
        );

        let call_result = self
            .forward_backend_with_heartbeat(
                &context,
                request_id,
                tool_name.as_ref(),
                |peer| async move { peer.call_tool_once(request).await },
            )
            .await;

        let elapsed = start.elapsed();
        let result = match call_result {
            Ok(response) => {
                match &response {
                    CallToolResponse::Complete(call_result) => {
                        let is_error = call_result.is_error.unwrap_or(false);
                        info!(
                            "[call_tool:{}] Response received - tool: {}, time taken: {}ms, is_error: {}, MCP ID: {}",
                            request_id,
                            tool_name,
                            elapsed.as_millis(),
                            is_error,
                            self.mcp_id
                        );
                        if is_error {
                            debug!(
                                "[call_tool:{}] Error response content: {:?}",
                                request_id, call_result.content
                            );
                        }
                    }
                    CallToolResponse::InputRequired(_) => {
                        info!(
                            "[call_tool:{}] InputRequired received - tool: {}, time taken: {}ms, MCP ID: {}",
                            request_id,
                            tool_name,
                            elapsed.as_millis(),
                            self.mcp_id
                        );
                    }
                    _ => {
                        info!(
                            "[call_tool:{}] Response received - tool: {}, time taken: {}ms, MCP ID: {}",
                            request_id,
                            tool_name,
                            elapsed.as_millis(),
                            self.mcp_id
                        );
                    }
                }
                Ok(response)
            }
            Err(err) => {
                // Preserve historical CallToolResult::error shape for callers.
                let message = err.message.clone();
                if message == "Request cancelled" {
                    warn!(
                        "[call_tool:{}] Request canceled - Tool: {}, Time taken: {}ms, MCP ID: {}",
                        request_id,
                        tool_name,
                        elapsed.as_millis(),
                        self.mcp_id
                    );
                    Ok(CallToolResult::error(vec![ContentBlock::text("Request cancelled")]).into())
                } else if message.contains("Backend connection is not available")
                    || message.contains("Backend unavailable")
                    || message.contains("Backend connect")
                {
                    error!(
                        "[call_tool:{}] Backend unavailable - Tool: {}, Error: {}, MCP ID: {}",
                        request_id, tool_name, message, self.mcp_id
                    );
                    Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                        "Backend unavailable: {message}"
                    ))])
                    .into())
                } else if message.contains("Backend connection closed") {
                    error!(
                        "[call_tool:{}] Backend transport is closed - MCP ID: {}",
                        request_id, self.mcp_id
                    );
                    Ok(CallToolResult::error(vec![ContentBlock::text(
                        "Backend connection closed, please retry",
                    )])
                    .into())
                } else {
                    error!(
                        "[call_tool:{}] Backend returns error - Tool: {}, Time: {}ms, Error: {}, MCP ID: {}",
                        request_id,
                        tool_name,
                        elapsed.as_millis(),
                        message,
                        self.mcp_id
                    );
                    Ok(
                        CallToolResult::error(vec![ContentBlock::text(format!(
                            "Error: {message}"
                        ))])
                        .into(),
                    )
                }
            }
        };

        info!(
            "[call_tool:{}] Completed - Tool: {}, total time taken: {}ms",
            request_id,
            tool_name,
            start.elapsed().as_millis()
        );
        result
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        match self.capabilities().resources {
            Some(_) => {
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.list_resources(request).await
                    })
                    .await?;
                info!(
                    "[list_resources] Resource list results - MCP ID: {}, resource quantity: {}",
                    self.mcp_id,
                    result.resources.len()
                );
                debug!("Proxying list_resources response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support resources capability");
                Ok(rmcp::model::ListResourcesResult::default())
            }
        }
    }

    async fn read_resource(
        &self,
        mut request: rmcp::model::ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        match self.capabilities().resources {
            Some(_) => {
                merge_context_meta_into_params(&mut request, &context.meta);
                let _progress_guard = self.maybe_progress_guard(&context);
                let uri = request.uri.clone();
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.read_resource_once(request).await
                    })
                    .await?;
                info!(
                    "[read_resource] Resource read result - MCP ID: {}, URI: {}",
                    self.mcp_id, uri
                );
                debug!("Proxying read_resource response for {}", uri);
                Ok(result)
            }
            None => {
                error!("Server doesn't support resources capability");
                Ok(rmcp::model::ReadResourceResult::new(vec![]).into())
            }
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        match self.capabilities().resources {
            Some(_) => {
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.list_resource_templates(request).await
                    })
                    .await?;
                debug!("Proxying list_resource_templates response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support resources capability");
                Ok(rmcp::model::ListResourceTemplatesResult::default())
            }
        }
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        match self.capabilities().prompts {
            Some(_) => {
                let result =
                    self.forward_backend(&context, |peer| async move {
                        peer.list_prompts(request).await
                    })
                    .await?;
                debug!("Proxying list_prompts response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support prompts capability");
                Ok(rmcp::model::ListPromptsResult::default())
            }
        }
    }

    async fn get_prompt(
        &self,
        mut request: rmcp::model::GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        match self.capabilities().prompts {
            Some(_) => {
                merge_context_meta_into_params(&mut request, &context.meta);
                let _progress_guard = self.maybe_progress_guard(&context);
                let result = self
                    .forward_backend(&context, |peer| async move {
                        peer.get_prompt_once(request).await
                    })
                    .await?;
                debug!("Proxying get_prompt response");
                Ok(result)
            }
            None => {
                warn!("Server doesn't support prompts capability");
                let messages = Vec::new();
                Ok(rmcp::model::GetPromptResult::new(messages).into())
            }
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        let result = self
            .forward_backend(&context, |peer| async move { peer.complete(request).await })
            .await?;
        debug!("Proxying complete response");
        Ok(result)
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        match self
            .capabilities()
            .resources
            .as_ref()
            .and_then(|r| r.subscribe)
        {
            Some(true) => {
                self.forward_backend(
                    &context,
                    |peer| async move { peer.subscribe(request).await },
                )
                .await
            }
            _ => Err(ErrorData::method_not_found::<SubscribeRequestMethod>()),
        }
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        match self
            .capabilities()
            .resources
            .as_ref()
            .and_then(|r| r.subscribe)
        {
            Some(true) => {
                self.forward_backend(
                    &context,
                    |peer| async move { peer.unsubscribe(request).await },
                )
                .await
            }
            _ => Err(ErrorData::method_not_found::<UnsubscribeRequestMethod>()),
        }
    }

    #[allow(deprecated)]
    async fn set_level(
        &self,
        request: rmcp::model::SetLevelRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if self.capabilities().logging.is_none() {
            return Err(ErrorData::method_not_found::<SetLevelRequestMethod>());
        }
        self.forward_backend(
            &context,
            |peer| async move { peer.set_level(request).await },
        )
        .await
    }

    async fn enqueue_task(
        &self,
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CreateTaskResult, ErrorData> {
        let tasks = self.capabilities().tasks.clone();
        if tasks.as_ref().is_none_or(|t| !t.supports_tools_call()) {
            return Err(ErrorData::internal_error(
                "Backend does not support task-based tools/call".to_string(),
                None,
            ));
        }
        merge_context_meta_into_params(&mut request, &context.meta);
        let _progress_guard = self.maybe_progress_guard(&context);
        let result = self
            .forward_backend(&context, |peer| async move {
                let result = peer
                    .send_request(ClientRequest::CallToolRequest(CallToolRequest::new(
                        request,
                    )))
                    .await?;
                match result {
                    ServerResult::CreateTaskResult(r) => Ok(r),
                    _ => Err(ServiceError::UnexpectedResponse),
                }
            })
            .await?;

        // Shared isolation: route task notifications via registry.
        // Per-session: bridge delivers 1:1 via notify_slot; no registry entry needed.
        if self.isolation == BackendIsolation::Shared {
            self.upstream_peers
                .register_task(result.task.task_id.clone(), context.peer.clone());
        }
        Ok(result)
    }

    async fn list_tasks(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListTasksResult, ErrorData> {
        let tasks = self.capabilities().tasks.clone();
        if tasks.as_ref().is_none_or(|t| !t.supports_list()) {
            return Err(ErrorData::method_not_found::<rmcp::model::ListTasksMethod>());
        }
        self.forward_backend(&context, |peer| async move {
            let result = peer
                .send_request(ClientRequest::ListTasksRequest(ListTasksRequest {
                    method: Default::default(),
                    params: request,
                    extensions: Default::default(),
                }))
                .await?;
            match result {
                ServerResult::ListTasksResult(r) => Ok(r),
                _ => Err(ServiceError::UnexpectedResponse),
            }
        })
        .await
    }

    async fn get_task_info(
        &self,
        request: GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, ErrorData> {
        if self.capabilities().tasks.is_none() {
            return Err(ErrorData::method_not_found::<rmcp::model::GetTaskMethod>());
        }
        self.forward_backend(&context, |peer| async move {
            let result = peer
                .send_request(ClientRequest::GetTaskRequest(GetTaskRequest::new(request)))
                .await?;
            match result {
                ServerResult::GetTaskResult(r) => Ok(r),
                _ => Err(ServiceError::UnexpectedResponse),
            }
        })
        .await
    }

    async fn get_task_result(
        &self,
        request: GetTaskPayloadParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetTaskPayloadResult, ErrorData> {
        if self.capabilities().tasks.is_none() {
            return Err(ErrorData::method_not_found::<
                rmcp::model::GetTaskPayloadMethod,
            >());
        }
        self.forward_backend(&context, |peer| async move {
            let result = peer
                .send_request(ClientRequest::GetTaskPayloadRequest(
                    GetTaskPayloadRequest::new(request),
                ))
                .await?;
            match result {
                ServerResult::GetTaskPayloadResult(r) => Ok(r),
                _ => Err(ServiceError::UnexpectedResponse),
            }
        })
        .await
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CancelTaskResult, ErrorData> {
        let tasks = self.capabilities().tasks.clone();
        if tasks.as_ref().is_none_or(|t| !t.supports_cancel()) {
            return Err(ErrorData::method_not_found::<rmcp::model::CancelTaskMethod>());
        }
        let task_id = request.task_id.clone();
        let result = self
            .forward_backend(&context, |peer| async move {
                let result = peer
                    .send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest::new(
                        request,
                    )))
                    .await?;
                match result {
                    ServerResult::CancelTaskResult(r) => Ok(r),
                    _ => Err(ServiceError::UnexpectedResponse),
                }
            })
            .await?;
        self.upstream_peers.unregister_task(&task_id);
        Ok(result)
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if let Some(session) = &self.per_session {
            // Refresh slot (initialize already connected); keep idempotent ensure.
            session.notify_slot.set(context.peer.clone());
            if let Err(err) = self.ensure_connected().await {
                error!(
                    "[on_initialized] Per-session backend ensure failed - MCP ID: {}, err: {:?}",
                    self.mcp_id, err
                );
            }
            return;
        }

        let id = self.upstream_peers.register(context.peer.clone());
        info!(
            "[on_initialized] Registered upstream peer {} - MCP ID: {}, peers: {}",
            id,
            self.mcp_id,
            self.upstream_peers.len()
        );
    }

    async fn on_progress(
        &self,
        notification: rmcp::model::ProgressNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        let peer = {
            let inner_guard = self.peer.load();
            let inner = match inner_guard.as_ref() {
                Some(inner) => inner,
                None => {
                    error!(
                        "Backend connection is not available, cannot forward progress notification"
                    );
                    return;
                }
            };
            if inner.peer.is_transport_closed() {
                error!("Backend transport is closed, cannot forward progress notification");
                return;
            }
            inner.peer.clone()
        };

        match peer.notify_progress(notification).await {
            Ok(_) => {
                debug!("Proxying progress notification");
            }
            Err(err) => {
                error!("Error notifying progress: {:?}", err);
            }
        }
    }

    async fn on_cancelled(
        &self,
        notification: rmcp::model::CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        let peer = {
            let inner_guard = self.peer.load();
            let inner = match inner_guard.as_ref() {
                Some(inner) => inner,
                None => {
                    error!(
                        "Backend connection is not available, cannot forward cancelled notification"
                    );
                    return;
                }
            };
            if inner.peer.is_transport_closed() {
                error!("Backend transport is closed, cannot forward cancelled notification");
                return;
            }
            inner.peer.clone()
        };

        match peer.notify_cancelled(notification).await {
            Ok(_) => {
                debug!("Proxying cancelled notification");
            }
            Err(err) => {
                error!("Error notifying cancelled: {:?}", err);
            }
        }
    }
}

impl ProxyHandler {
    /// 获取 capabilities（从共享 cached_info 加载）
    #[inline]
    fn capabilities(&self) -> rmcp::model::ServerCapabilities {
        self.cached_info.load().capabilities.clone()
    }

    /// Ensure backend is connected (lazy connect for per-session isolation).
    async fn ensure_connected(&self) -> Result<(), ErrorData> {
        if self.is_backend_available() {
            return Ok(());
        }
        let Some(session) = &self.per_session else {
            return Err(ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            ));
        };

        let _lock = session.connect_lock.lock().await;
        if self.is_backend_available() {
            return Ok(());
        }

        let timeout = session.connector.connect_timeout();
        let slot = session.notify_slot.clone();
        let connect = session.connector.connect_session(slot);
        let result = tokio::time::timeout(timeout, connect).await;

        match result {
            Ok(Ok(running)) => {
                self.install_backend(running, true);
                Ok(())
            }
            Ok(Err(err)) => {
                error!(error = %err, "Per-session backend connect failed");
                Err(ErrorData::internal_error(
                    format!("Backend connect failed: {err}"),
                    None,
                ))
            }
            Err(_) => {
                error!(?timeout, "Per-session backend connect timed out");
                Err(ErrorData::internal_error(
                    format!("Backend connect timed out after {timeout:?}"),
                    None,
                ))
            }
        }
    }

    /// Install a backend without going through shared-mode registry checks.
    fn install_backend(&self, client: BackendRunningService, bump_version: bool) {
        use std::ops::Deref;
        let info = Self::extract_server_info(&client, &self.mcp_id);
        self.cached_info.store(Arc::new(info));
        let peer = client.deref().clone();
        let inner = PeerInner {
            peer,
            _running: Arc::new(client),
        };
        self.peer.store(Some(Arc::new(inner)));
        if bump_version {
            let new_version = self.backend_version.fetch_add(1, Ordering::SeqCst) + 1;
            info!(
                "[ProxyHandler] Backend version update: {} - MCP ID: {}",
                new_version, self.mcp_id
            );
        }
    }

    fn maybe_progress_guard(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Option<ProgressRouteGuard> {
        if self.isolation != BackendIsolation::Shared {
            return None;
        }
        ProgressRouteGuard::try_from_meta(
            self.upstream_peers.clone(),
            &context.meta,
            context.peer.clone(),
        )
    }

    /// Load backend peer, check closed, run op with cancel race.
    async fn forward_backend<T, Fut, F>(
        &self,
        context: &RequestContext<RoleServer>,
        op: F,
    ) -> Result<T, ErrorData>
    where
        F: FnOnce(Peer<RoleClient>) -> Fut,
        Fut: Future<Output = Result<T, ServiceError>>,
    {
        self.ensure_connected().await?;

        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        let peer = inner.peer.clone();
        drop(inner_guard);

        tokio::select! {
            result = op(peer) => {
                result.map_err(|err| {
                    error!("Backend request error: {:?}", err);
                    ErrorData::internal_error(format!("Backend error: {err}"), None)
                })
            }
            _ = context.ct.cancelled() => {
                info!("[forward_backend] Request canceled - MCP ID: {}", self.mcp_id);
                Err(ErrorData::internal_error(
                    "Request cancelled".to_string(),
                    None,
                ))
            }
        }
    }

    /// Like [`Self::forward_backend`], but logs heartbeat while waiting (long tools/call).
    async fn forward_backend_with_heartbeat<T, Fut, F>(
        &self,
        context: &RequestContext<RoleServer>,
        request_id: u64,
        tool_name: &str,
        op: F,
    ) -> Result<T, ErrorData>
    where
        F: FnOnce(Peer<RoleClient>) -> Fut,
        Fut: Future<Output = Result<T, ServiceError>>,
    {
        self.ensure_connected().await?;

        let peer = {
            let inner_guard = self.peer.load();
            let inner = inner_guard.as_ref().ok_or_else(|| {
                error!("Backend connection is not available (reconnecting)");
                ErrorData::internal_error(
                    "Backend connection is not available, reconnecting...".to_string(),
                    None,
                )
            })?;

            if inner.peer.is_transport_closed() {
                error!("Backend transport is closed");
                return Err(ErrorData::internal_error(
                    "Backend connection closed, please retry".to_string(),
                    None,
                ));
            }

            let peer = inner.peer.clone();
            drop(inner_guard);
            peer
        };

        let call_future = op(peer.clone());
        tokio::pin!(call_future);

        const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
        let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat_interval.tick().await;

        loop {
            tokio::select! {
                biased;
                result = &mut call_future => {
                    return result.map_err(|err| {
                        error!("Backend request error: {:?}", err);
                        ErrorData::internal_error(format!("Backend error: {err}"), None)
                    });
                }
                _ = context.ct.cancelled() => {
                    info!(
                        "[forward_backend_with_heartbeat:{}] Request canceled - Tool: {}, MCP ID: {}",
                        request_id, tool_name, self.mcp_id
                    );
                    return Err(ErrorData::internal_error(
                        "Request cancelled".to_string(),
                        None,
                    ));
                }
                _ = heartbeat_interval.tick() => {
                    info!(
                        "[forward_backend_with_heartbeat:{}] Waiting for backend... - Tool: {}, transport_closed: {}, MCP ID: {}",
                        request_id,
                        tool_name,
                        peer.is_transport_closed(),
                        self.mcp_id
                    );
                }
            }
        }
    }

    /// Shared upstream peer registry (for reconnect with the same notification bridge).
    pub fn upstream_peers(&self) -> Arc<UpstreamPeerRegistry> {
        self.upstream_peers.clone()
    }

    /// 创建一个默认的 ServerInfo（用于断开状态）
    fn default_server_info(mcp_id: &str) -> ServerInfo {
        warn!(
            "[ProxyHandler] Create default ServerInfo - MCP ID: {}",
            mcp_id
        );
        ServerInfo::new(rmcp::model::ServerCapabilities::default())
            .with_server_info(Implementation::new("MCP Proxy", "0.1.0"))
    }

    /// 从 RunningService 提取 ServerInfo
    fn extract_server_info(client: &BackendRunningService, mcp_id: &str) -> ServerInfo {
        client
            .peer_info()
            .map(|peer_info| {
                ServerInfo::new(peer_info.capabilities.clone())
                    .with_protocol_version(peer_info.protocol_version.clone())
                    .with_server_info(Implementation::new(
                        peer_info.server_info.name.clone(),
                        peer_info.server_info.version.clone(),
                    ))
                    .with_instructions(peer_info.instructions.clone().unwrap_or_default())
            })
            .unwrap_or_else(|| Self::default_server_info(mcp_id))
    }

    /// 创建断开状态的 handler（用于初始化）
    /// 后续通过 swap_backend() 注入实际的后端连接
    pub fn new_disconnected(
        mcp_id: String,
        tool_filter: ToolFilter,
        default_info: ServerInfo,
    ) -> Self {
        info!(
            "[ProxyHandler] Create a disconnected handler - MCP ID: {}",
            mcp_id
        );

        // 记录过滤器配置
        if tool_filter.is_enabled() {
            if let Some(ref allow_list) = tool_filter.allow_tools {
                info!(
                    "[ProxyHandler] Tool whitelist enabled - MCP ID: {}, allowed tools: {:?}",
                    mcp_id, allow_list
                );
            }
            if let Some(ref deny_list) = tool_filter.deny_tools {
                info!(
                    "[ProxyHandler] Tool blacklist enabled - MCP ID: {}, excluded tools: {:?}",
                    mcp_id, deny_list
                );
            }
        }

        Self {
            peer: Arc::new(ArcSwapOption::empty()),
            cached_info: Arc::new(arc_swap::ArcSwap::from_pointee(default_info)),
            mcp_id,
            tool_filter,
            backend_version: Arc::new(AtomicU64::new(0)), // 断开状态版本为 0
            upstream_peers: Arc::new(UpstreamPeerRegistry::new()),
            isolation: BackendIsolation::Shared,
            per_session: None,
        }
    }

    /// Create a disconnected per-session handler (URL isolation).
    ///
    /// Backend is opened during [`ServerHandler::initialize`] so the initialize
    /// response carries real backend capabilities.
    pub fn new_per_session(
        connector: Arc<dyn BackendConnector>,
        mcp_id: String,
        tool_filter: ToolFilter,
    ) -> Self {
        info!(
            "[ProxyHandler] Create per-session disconnected handler - MCP ID: {}",
            mcp_id
        );
        let default_info = Self::default_server_info(&mcp_id);
        Self {
            peer: Arc::new(ArcSwapOption::empty()),
            cached_info: Arc::new(arc_swap::ArcSwap::from_pointee(default_info)),
            mcp_id,
            tool_filter,
            backend_version: Arc::new(AtomicU64::new(0)),
            upstream_peers: Arc::new(UpstreamPeerRegistry::new()),
            isolation: BackendIsolation::PerSession,
            per_session: Some(PerSessionState {
                connector,
                notify_slot: UpstreamPeerSlot::new(),
                connect_lock: Arc::new(tokio::sync::Mutex::new(())),
            }),
        }
    }

    /// Isolation mode for this handler.
    pub fn isolation(&self) -> BackendIsolation {
        self.isolation
    }

    pub fn new(client: BackendRunningService) -> Self {
        Self::with_mcp_id(client, "unknown".to_string())
    }

    pub fn with_mcp_id(client: BackendRunningService, mcp_id: String) -> Self {
        Self::with_tool_filter(client, mcp_id, ToolFilter::default())
    }

    /// 创建带工具过滤器的 ProxyHandler（带初始后端连接，shared 隔离）
    pub fn with_tool_filter(
        client: BackendRunningService,
        mcp_id: String,
        tool_filter: ToolFilter,
    ) -> Self {
        use std::ops::Deref;

        // 提取 ServerInfo 与共享 upstream peer registry
        let cached_info = Self::extract_server_info(&client, &mcp_id);
        let upstream_peers = client
            .service()
            .upstream_peers()
            .cloned()
            .unwrap_or_else(|| Arc::new(UpstreamPeerRegistry::new()));

        // 克隆 Peer 用于并发请求（无需锁）
        let peer = client.deref().clone();

        // 记录过滤器配置
        if tool_filter.is_enabled() {
            if let Some(ref allow_list) = tool_filter.allow_tools {
                info!(
                    "[ProxyHandler] Tool whitelist enabled - MCP ID: {}, allowed tools: {:?}",
                    mcp_id, allow_list
                );
            }
            if let Some(ref deny_list) = tool_filter.deny_tools {
                info!(
                    "[ProxyHandler] Tool blacklist enabled - MCP ID: {}, excluded tools: {:?}",
                    mcp_id, deny_list
                );
            }
        }

        // 创建 PeerInner
        let inner = PeerInner {
            peer,
            _running: Arc::new(client),
        };

        Self {
            peer: Arc::new(ArcSwapOption::from(Some(Arc::new(inner)))),
            cached_info: Arc::new(arc_swap::ArcSwap::from_pointee(cached_info)),
            mcp_id,
            tool_filter,
            backend_version: Arc::new(AtomicU64::new(1)), // 初始版本为 1
            upstream_peers,
            isolation: BackendIsolation::Shared,
            per_session: None,
        }
    }

    /// 原子性替换后端连接
    /// - Some(client): 设置新的后端连接
    /// - None: 标记后端断开
    ///
    /// **版本控制**：每次调用都会递增 backend_version，使旧 session 失效
    ///
    /// **注意**：
    /// - 新 client 应使用与本 handler 相同的 [`UpstreamPeerRegistry`]
    ///   （见 `StreamClientConnection::connect_with_peers`）。
    /// - 对 [`BackendIsolation::PerSession`] 的 **management stub**（builder 返回值）
    ///   调用此方法没有运维意义：真实后端由各 session handler 持有。
    pub fn swap_backend(&self, new_client: Option<BackendRunningService>) {
        use std::ops::Deref;

        if self.isolation == BackendIsolation::PerSession && self.peer.load().is_none() {
            warn!(
                "[ProxyHandler] swap_backend on disconnected per-session handler \
                 (likely management stub) - MCP ID: {}",
                self.mcp_id
            );
        }

        match new_client {
            Some(client) => {
                if let Some(reg) = client.service().upstream_peers()
                    && !Arc::ptr_eq(reg, &self.upstream_peers)
                {
                    warn!(
                        "[ProxyHandler] swap_backend registry Arc differs from handler - MCP ID: {}",
                        self.mcp_id
                    );
                }
                // 刷新 cached capabilities / server info
                let info = Self::extract_server_info(&client, &self.mcp_id);
                self.cached_info.store(Arc::new(info));

                let peer = client.deref().clone();
                let inner = PeerInner {
                    peer,
                    _running: Arc::new(client),
                };
                self.peer.store(Some(Arc::new(inner)));
                info!(
                    "[ProxyHandler] Backend connection updated - MCP ID: {}",
                    self.mcp_id
                );
            }
            None => {
                self.peer.store(None);
                info!(
                    "[ProxyHandler] Backend connection disconnected - MCP ID: {}",
                    self.mcp_id
                );
            }
        }

        // 关键：递增版本号，使所有旧 session 失效
        let new_version = self.backend_version.fetch_add(1, Ordering::SeqCst) + 1;
        info!(
            "[ProxyHandler] Backend version update: {} - MCP ID: {}",
            new_version, self.mcp_id
        );
    }

    /// 检查后端是否可用（快速检查，不发送请求）
    pub fn is_backend_available(&self) -> bool {
        let inner_guard = self.peer.load();
        match inner_guard.as_ref() {
            Some(inner) => !inner.peer.is_transport_closed(),
            None => false,
        }
    }

    /// 检查 mcp 服务是否正常（异步版本，会发送验证请求）
    pub async fn is_mcp_server_ready(&self) -> bool {
        !self.is_terminated_async().await
    }

    /// 检查后端连接是否已关闭（同步版本，仅检查 transport 状态）
    pub fn is_terminated(&self) -> bool {
        !self.is_backend_available()
    }

    /// 异步检查后端连接是否已断开（会发送验证请求）
    pub async fn is_terminated_async(&self) -> bool {
        let peer = {
            let inner_guard = self.peer.load();
            let inner = match inner_guard.as_ref() {
                Some(inner) => inner,
                None => return true,
            };
            if inner.peer.is_transport_closed() {
                return true;
            }
            inner.peer.clone()
        };

        match peer.list_tools(None).await {
            Ok(_) => {
                debug!("Backend connection status check: OK");
                false
            }
            Err(e) => {
                info!("Backend connection status check: Disconnected, reason: {e}");
                true
            }
        }
    }

    /// 获取 MCP ID
    pub fn mcp_id(&self) -> &str {
        &self.mcp_id
    }

    /// 获取后端 ServerInfo 的 JSON 表示
    ///
    /// 用于跨 rmcp 版本桥接：将 rmcp 1.4.0 的 ServerInfo 序列化为 JSON，
    /// 供 rmcp 0.10 侧反序列化使用。
    pub fn get_server_info_json(&self) -> serde_json::Value {
        serde_json::to_value(&**self.cached_info.load()).unwrap_or_default()
    }

    /// 获取当前后端版本号
    ///
    /// 版本号用于跟踪后端连接变化：
    /// - 0: 断开状态
    /// - 1+: 已连接，每次 swap_backend 递增
    ///
    /// **用途**：配合 ProxyAwareSessionManager 实现 session 版本控制
    pub fn get_backend_version(&self) -> u64 {
        self.backend_version.load(Ordering::SeqCst)
    }

    /// 直接调用后端 peer 的方法（用于 BackendSession trait 实现）
    ///
    /// 这是一个低级接口，直接操作 peer 而不经过 ServerHandler 的封装
    pub async fn call_peer_method(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use rmcp::model::PaginatedRequestParams;

        let peer = {
            let inner_guard = self.peer.load();
            let inner = inner_guard
                .as_ref()
                .ok_or_else(|| "Backend connection is not available (reconnecting)".to_string())?;

            if inner.peer.is_transport_closed() {
                return Err("Backend transport is closed".to_string());
            }
            inner.peer.clone()
        };

        match method {
            "tools/list" => {
                let request: Option<PaginatedRequestParams> = serde_json::from_value(params).ok();
                let result = peer
                    .list_tools(request)
                    .await
                    .map_err(|e| format!("list_tools error: {:?}", e))?;
                serde_json::to_value(result).map_err(|e| format!("serialize error: {}", e))
            }
            "tools/call" => {
                let request: rmcp::model::CallToolRequestParams = serde_json::from_value(params)
                    .map_err(|e| format!("Invalid params for tools/call: {}", e))?;
                if request.task.is_some() {
                    let result = peer
                        .send_request(ClientRequest::CallToolRequest(CallToolRequest::new(
                            request,
                        )))
                        .await
                        .map_err(|e| format!("enqueue_task error: {:?}", e))?;
                    match result {
                        ServerResult::CreateTaskResult(r) => {
                            serde_json::to_value(r).map_err(|e| format!("serialize error: {}", e))
                        }
                        other => Err(format!("unexpected enqueue_task response: {other:?}")),
                    }
                } else {
                    let result = peer
                        .call_tool_once(request)
                        .await
                        .map_err(|e| format!("call_tool error: {:?}", e))?;
                    let value = match result {
                        CallToolResponse::Complete(r) => serde_json::to_value(r),
                        CallToolResponse::InputRequired(r) => serde_json::to_value(r),
                        other => {
                            return Err(format!("unsupported CallToolResponse variant: {other:?}"));
                        }
                    };
                    value.map_err(|e| format!("serialize error: {}", e))
                }
            }
            "resources/subscribe" => {
                let request: SubscribeRequestParams = serde_json::from_value(params)
                    .map_err(|e| format!("Invalid params for resources/subscribe: {}", e))?;
                peer.subscribe(request)
                    .await
                    .map_err(|e| format!("subscribe error: {:?}", e))?;
                Ok(serde_json::json!({}))
            }
            "resources/unsubscribe" => {
                let request: UnsubscribeRequestParams = serde_json::from_value(params)
                    .map_err(|e| format!("Invalid params for resources/unsubscribe: {}", e))?;
                peer.unsubscribe(request)
                    .await
                    .map_err(|e| format!("unsubscribe error: {:?}", e))?;
                Ok(serde_json::json!({}))
            }
            "logging/setLevel" => {
                #[allow(deprecated)]
                {
                    let request: rmcp::model::SetLevelRequestParams =
                        serde_json::from_value(params)
                            .map_err(|e| format!("Invalid params for logging/setLevel: {}", e))?;
                    peer.set_level(request)
                        .await
                        .map_err(|e| format!("set_level error: {:?}", e))?;
                }
                Ok(serde_json::json!({}))
            }
            "tasks/list" => {
                let request: Option<PaginatedRequestParams> = serde_json::from_value(params).ok();
                let result = peer
                    .send_request(ClientRequest::ListTasksRequest(ListTasksRequest {
                        method: Default::default(),
                        params: request,
                        extensions: Default::default(),
                    }))
                    .await
                    .map_err(|e| format!("list_tasks error: {:?}", e))?;
                match result {
                    ServerResult::ListTasksResult(r) => {
                        serde_json::to_value(r).map_err(|e| format!("serialize error: {}", e))
                    }
                    other => Err(format!("unexpected list_tasks response: {other:?}")),
                }
            }
            "tasks/get" => {
                let request: GetTaskParams = serde_json::from_value(params)
                    .map_err(|e| format!("Invalid params for tasks/get: {}", e))?;
                let result = peer
                    .send_request(ClientRequest::GetTaskRequest(GetTaskRequest::new(request)))
                    .await
                    .map_err(|e| format!("get_task error: {:?}", e))?;
                match result {
                    ServerResult::GetTaskResult(r) => {
                        serde_json::to_value(r).map_err(|e| format!("serialize error: {}", e))
                    }
                    other => Err(format!("unexpected get_task response: {other:?}")),
                }
            }
            "tasks/result" => {
                let request: GetTaskPayloadParams = serde_json::from_value(params)
                    .map_err(|e| format!("Invalid params for tasks/result: {}", e))?;
                let result = peer
                    .send_request(ClientRequest::GetTaskPayloadRequest(
                        GetTaskPayloadRequest::new(request),
                    ))
                    .await
                    .map_err(|e| format!("get_task_result error: {:?}", e))?;
                match result {
                    ServerResult::GetTaskPayloadResult(r) => {
                        serde_json::to_value(r).map_err(|e| format!("serialize error: {}", e))
                    }
                    other => Err(format!("unexpected get_task_result response: {other:?}")),
                }
            }
            "tasks/cancel" => {
                let request: CancelTaskParams = serde_json::from_value(params)
                    .map_err(|e| format!("Invalid params for tasks/cancel: {}", e))?;
                let result = peer
                    .send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest::new(
                        request,
                    )))
                    .await
                    .map_err(|e| format!("cancel_task error: {:?}", e))?;
                match result {
                    ServerResult::CancelTaskResult(r) => {
                        serde_json::to_value(r).map_err(|e| format!("serialize error: {}", e))
                    }
                    other => Err(format!("unexpected cancel_task response: {other:?}")),
                }
            }
            "resources/list" => {
                let request: Option<PaginatedRequestParams> = serde_json::from_value(params).ok();
                let result = peer
                    .list_resources(request)
                    .await
                    .map_err(|e| format!("list_resources error: {:?}", e))?;
                serde_json::to_value(result).map_err(|e| format!("serialize error: {}", e))
            }
            "resources/read" => {
                let request: rmcp::model::ReadResourceRequestParams =
                    serde_json::from_value(params)
                        .map_err(|e| format!("Invalid params for resources/read: {}", e))?;
                let result = peer
                    .read_resource_once(request)
                    .await
                    .map_err(|e| format!("read_resource error: {:?}", e))?;
                let value = match result {
                    ReadResourceResponse::Complete(r) => serde_json::to_value(r),
                    ReadResourceResponse::InputRequired(r) => serde_json::to_value(r),
                    other => {
                        return Err(format!(
                            "unsupported ReadResourceResponse variant: {other:?}"
                        ));
                    }
                };
                value.map_err(|e| format!("serialize error: {}", e))
            }
            "prompts/list" => {
                let request: Option<PaginatedRequestParams> = serde_json::from_value(params).ok();
                let result = peer
                    .list_prompts(request)
                    .await
                    .map_err(|e| format!("list_prompts error: {:?}", e))?;
                serde_json::to_value(result).map_err(|e| format!("serialize error: {}", e))
            }
            "prompts/get" => {
                let request: rmcp::model::GetPromptRequestParams =
                    serde_json::from_value(params)
                        .map_err(|e| format!("Invalid params for prompts/get: {}", e))?;
                let result = peer
                    .get_prompt_once(request)
                    .await
                    .map_err(|e| format!("get_prompt error: {:?}", e))?;
                let value = match result {
                    GetPromptResponse::Complete(r) => serde_json::to_value(r),
                    GetPromptResponse::InputRequired(r) => serde_json::to_value(r),
                    other => {
                        return Err(format!("unsupported GetPromptResponse variant: {other:?}"));
                    }
                };
                value.map_err(|e| format!("serialize error: {}", e))
            }
            "resources/templates/list" => {
                let request: Option<PaginatedRequestParams> = serde_json::from_value(params).ok();
                let result = peer
                    .list_resource_templates(request)
                    .await
                    .map_err(|e| format!("list_resource_templates error: {:?}", e))?;
                serde_json::to_value(result).map_err(|e| format!("serialize error: {}", e))
            }
            "completion/complete" => {
                let request: rmcp::model::CompleteRequestParams = serde_json::from_value(params)
                    .map_err(|e| format!("Invalid params for completion/complete: {}", e))?;
                let result = peer
                    .complete(request)
                    .await
                    .map_err(|e| format!("complete error: {:?}", e))?;
                serde_json::to_value(result).map_err(|e| format!("serialize error: {}", e))
            }
            _ => Err(format!("Unsupported method: {}", method)),
        }
    }

    /// Update backend from a StreamClientConnection
    ///
    /// This method allows updating the backend connection using the high-level
    /// `StreamClientConnection` type, which is more convenient than the raw
    /// `RunningService` type.
    ///
    /// # Arguments
    /// * `conn` - Some(connection) to set new backend, None to mark disconnected
    pub fn swap_backend_from_connection(
        &self,
        conn: Option<crate::client::StreamClientConnection>,
    ) {
        match conn {
            Some(c) => {
                let running = c.into_running_service();
                self.swap_backend(Some(running));
            }
            None => {
                self.swap_backend(None);
            }
        }
    }
}

impl mcp_common::BackendBridge for ProxyHandler {
    fn mcp_id(&self) -> &str {
        self.mcp_id()
    }

    fn get_server_info_json(&self) -> serde_json::Value {
        self.get_server_info_json()
    }

    fn is_backend_available(&self) -> bool {
        self.is_backend_available()
    }

    fn is_mcp_server_ready(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        Box::pin(self.is_mcp_server_ready())
    }

    fn is_terminated_async(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        Box::pin(self.is_terminated_async())
    }

    fn call_peer_method(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send + '_>,
    > {
        let method = method.to_string();
        Box::pin(async move { ProxyHandler::call_peer_method(self, &method, params).await })
    }
}

#[cfg(test)]
mod meta_merge_tests {
    use super::merge_context_meta_into_params;
    use rmcp::model::{
        CallToolRequestParams, NumberOrString, ProgressToken, RequestMetaObject, RequestParamsMeta,
    };

    #[test]
    fn merge_copies_progress_token_from_context() {
        let mut params = CallToolRequestParams::new("echo");
        assert!(params.meta.is_none());

        let mut context_meta = RequestMetaObject::new();
        context_meta.set_progress_token(ProgressToken(NumberOrString::Number(42)));

        merge_context_meta_into_params(&mut params, &context_meta);

        assert_eq!(
            params.progress_token(),
            Some(ProgressToken(NumberOrString::Number(42)))
        );
    }

    #[test]
    fn merge_keeps_existing_params_meta_keys() {
        let mut params = CallToolRequestParams::new("echo");
        params.set_progress_token(ProgressToken(NumberOrString::Number(1)));

        let mut context_meta = RequestMetaObject::new();
        context_meta.set_progress_token(ProgressToken(NumberOrString::Number(99)));
        context_meta.insert("traceId".to_string(), serde_json::json!("abc"));

        merge_context_meta_into_params(&mut params, &context_meta);

        assert_eq!(
            params.progress_token(),
            Some(ProgressToken(NumberOrString::Number(1))),
            "params win on conflict"
        );
        assert_eq!(
            params.meta.as_ref().and_then(|m| m.get("traceId")),
            Some(&serde_json::json!("abc"))
        );
    }
}
