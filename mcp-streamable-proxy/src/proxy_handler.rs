use arc_swap::ArcSwapOption;
pub use mcp_common::ToolFilter;
/**
 * Create a local SSE server that proxies requests to a stdio MCP server.
 */
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, ClientInfo, Content, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerInfo,
    },
    service::{NotificationContext, Peer, RequestContext, RunningService},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::{debug, error, info, warn};

/// 全局请求计数器，用于生成唯一的请求 ID
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 包装后端连接和运行服务
/// 用于 ArcSwap 热替换
#[derive(Debug)]
struct PeerInner {
    /// Peer 用于发送请求
    peer: Peer<RoleClient>,
    /// 保持 RunningService 的所有权，确保服务生命周期
    #[allow(dead_code)]
    _running: Arc<RunningService<RoleClient, ClientInfo>>,
}

/// A proxy handler that forwards requests to a client based on the server's capabilities
/// 使用 ArcSwap 实现后端热替换，支持断开时立即返回错误
///
/// **增强功能**：
/// - 后端版本控制：每次 swap_backend 都会递增版本号
/// - 支持 Session 版本跟踪：配合 ProxyAwareSessionManager 使用
#[derive(Clone, Debug)]
pub struct ProxyHandler {
    /// 后端连接（ArcSwap 支持无锁原子替换）
    /// None 表示后端断开/重连中
    peer: Arc<ArcSwapOption<PeerInner>>,
    /// 缓存的服务器信息（保持不变，重连后应一致）
    cached_info: ServerInfo,
    /// MCP ID 用于日志记录
    mcp_id: String,
    /// 工具过滤配置
    tool_filter: ToolFilter,
    /// 后端版本号（每次 swap_backend 递增）
    /// 用于跟踪后端连接变化，使旧 session 失效
    backend_version: Arc<AtomicU64>,
}

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> ServerInfo {
        self.cached_info.clone()
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
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        // Check if the server has tools capability and forward the request
        match self.capabilities().tools {
            Some(_) => {
                // 使用 tokio::select! 同时等待取消和结果
                tokio::select! {
                    result = inner.peer.list_tools(request) => {
                        match result {
                            Ok(result) => {
                                // 根据过滤配置过滤工具列表
                                let filtered_tools: Vec<_> = if self.tool_filter.is_enabled() {
                                    result
                                        .tools
                                        .into_iter()
                                        .filter(|tool| self.tool_filter.is_allowed(&tool.name))
                                        .collect()
                                } else {
                                    result.tools
                                };

                                // 记录工具列表结果，这些结果会通过 SSE 推送给客户端
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
                                    meta: result.meta, // rmcp 0.12 新增字段
                                })
                            }
                            Err(err) => {
                                error!("Error listing tools: {:?}", err);
                                // 传输层错误时立即标记后端不可用，触发 watchdog 重连
                                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                                    warn!("[list_tools] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                                    self.swap_backend(None);
                                }
                                Err(ErrorData::internal_error(
                                    format!("Error listing tools: {err}"),
                                    None,
                                ))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        info!("[list_tools] Request canceled - MCP ID: {}", self.mcp_id);
                        Err(ErrorData::internal_error(
                            "Request cancelled".to_string(),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support tools, return empty list
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
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // 生成唯一请求 ID 用于追踪
        let request_id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();

        info!(
            "[call_tool:{}] Start - Tool: {}, MCP ID: {}",
            request_id, request.name, self.mcp_id
        );

        // 首先检查工具是否被过滤
        if !self.tool_filter.is_allowed(&request.name) {
            info!(
                "[call_tool:{}] Tool is filtered - MCP ID: {}, Tool: {}",
                request_id, self.mcp_id, request.name
            );
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Tool '{}' is not allowed by filter configuration",
                request.name
            ))]));
        }

        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = match inner_guard.as_ref() {
            Some(inner) => {
                let transport_closed = inner.peer.is_transport_closed();
                info!(
                    "[call_tool:{}] Backend connection exists - transport_closed: {}",
                    request_id, transport_closed
                );
                inner
            }
            None => {
                error!(
                    "[call_tool:{}] Backend connection unavailable (reconnecting) - MCP ID: {}",
                    request_id, self.mcp_id
                );
                return Ok(CallToolResult::error(vec![Content::text(
                    "Backend connection is not available, reconnecting...",
                )]));
            }
        };

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!(
                "[call_tool:{}] Backend transport is closed - MCP ID: {}",
                request_id, self.mcp_id
            );
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Ok(CallToolResult::error(vec![Content::text(
                "Backend connection closed, please retry",
            )]));
        }

        // Check if the server has tools capability and forward the request
        let result = match self.capabilities().tools {
            Some(_) => {
                // 记录发送请求到后端的时间点
                info!(
                    "[call_tool:{}] Send request to backend... - Tool: {}, Elapsed time: {}ms",
                    request_id,
                    request.name,
                    start.elapsed().as_millis()
                );

                // 创建后端调用的 Future，使用 pin 固定
                let call_future = inner.peer.call_tool(request.clone());
                tokio::pin!(call_future);

                // 等待心跳间隔（30秒）
                const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
                let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
                // 跳过第一次立即触发
                heartbeat_interval.tick().await;

                // 使用循环 + select! 实现等待心跳
                let call_result = loop {
                    tokio::select! {
                        // biased 确保按顺序优先检查，避免心跳检查抢占实际结果
                        biased;

                        result = &mut call_future => {
                            break result;
                        }
                        _ = context.ct.cancelled() => {
                            let elapsed = start.elapsed();
                            warn!(
                                "[call_tool:{}] Request canceled - Tool: {}, Time taken: {}ms, MCP ID: {}",
                                request_id, request.name, elapsed.as_millis(), self.mcp_id
                            );
                            return Ok(CallToolResult::error(vec![Content::text(
                                "Request cancelled"
                            )]));
                        }
                        _ = heartbeat_interval.tick() => {
                            // 定期打印等待日志，证明 mcp-proxy 在等待后端响应
                            let elapsed = start.elapsed();
                            let transport_closed = inner.peer.is_transport_closed();
                            info!(
                                "[call_tool:{}] Waiting for backend response... - Tool: {}, Waiting: {}ms, \\ transport_closed: {}, MCP ID: {}",
                                request_id, request.name, elapsed.as_millis(),
                                transport_closed, self.mcp_id
                            );
                        }
                    }
                };

                let elapsed = start.elapsed();
                match &call_result {
                    Ok(call_result) => {
                        // 记录工具调用结果
                        let is_error = call_result.is_error.unwrap_or(false);
                        info!(
                            "[call_tool:{}] Response received - tool: {}, time taken: {}ms, is_error: {}, MCP ID: {}",
                            request_id,
                            request.name,
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
                        Ok(call_result.clone())
                    }
                    Err(err) => {
                        error!(
                            "[call_tool:{}] Backend returns error - Tool: {}, Time: {}ms, Error: {:?}, MCP ID: {}",
                            request_id,
                            request.name,
                            elapsed.as_millis(),
                            err,
                            self.mcp_id
                        );
                        // 传输层错误时立即标记后端不可用，触发 watchdog 重连
                        // 避免 sessionId 失效后持续报错直到下次 ping 才检测到
                        if is_transport_error(&err, inner.peer.is_transport_closed()) {
                            warn!(
                                "[call_tool:{}] Transport error detected, marking backend unavailable - MCP ID: {}",
                                request_id, self.mcp_id
                            );
                            self.swap_backend(None);
                        }
                        // Return an error result instead of propagating the error
                        Ok(CallToolResult::error(vec![Content::text(format!(
                            "Error: {err}"
                        ))]))
                    }
                }
            }
            None => {
                error!(
                    "[call_tool:{}] The server does not support tools capability - MCP ID: {}",
                    request_id, self.mcp_id
                );
                Ok(CallToolResult::error(vec![Content::text(
                    "Server doesn't support tools capability",
                )]))
            }
        };

        let total_elapsed = start.elapsed();
        info!(
            "[call_tool:{}] Completed - Tool: {}, total time taken: {}ms",
            request_id,
            request.name,
            total_elapsed.as_millis()
        );
        result
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        // Check if the server has resources capability and forward the request
        match self.capabilities().resources {
            Some(_) => {
                tokio::select! {
                    result = inner.peer.list_resources(request) => {
                        match result {
                            Ok(result) => {
                                // 记录资源列表结果，这些结果会通过 SSE 推送给客户端
                                info!(
                                    "[list_resources] Resource list results - MCP ID: {}, resource quantity: {}",
                                    self.mcp_id,
                                    result.resources.len()
                                );

                                debug!("Proxying list_resources response");
                                Ok(result)
                            }
                            Err(err) => {
                                error!("Error listing resources: {:?}", err);
                                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                                    warn!("[list_resources] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                                    self.swap_backend(None);
                                }
                                Err(ErrorData::internal_error(
                                    format!("Error listing resources: {err}"),
                                    None,
                                ))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        info!("[list_resources] Request canceled - MCP ID: {}", self.mcp_id);
                        Err(ErrorData::internal_error(
                            "Request cancelled".to_string(),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support resources, return empty list
                warn!("Server doesn't support resources capability");
                Ok(rmcp::model::ListResourcesResult::default())
            }
        }
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        // Check if the server has resources capability and forward the request
        match self.capabilities().resources {
            Some(_) => {
                tokio::select! {
                    result = inner.peer.read_resource(rmcp::model::ReadResourceRequestParams::new(request.uri.clone())) => {
                        match result {
                            Ok(result) => {
                                // 记录资源读取结果，这些结果会通过 SSE 推送给客户端
                                info!(
                                    "[read_resource] Resource read result - MCP ID: {}, URI: {}",
                                    self.mcp_id, request.uri
                                );

                                debug!("Proxying read_resource response for {}", request.uri);
                                Ok(result)
                            }
                            Err(err) => {
                                error!("Error reading resource: {:?}", err);
                                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                                    warn!("[read_resource] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                                    self.swap_backend(None);
                                }
                                Err(ErrorData::internal_error(
                                    format!("Error reading resource: {err}"),
                                    None,
                                ))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        info!("[read_resource] Request canceled - MCP ID: {}, URI: {}", self.mcp_id, request.uri);
                        Err(ErrorData::internal_error(
                            "Request cancelled".to_string(),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support resources, return error
                error!("Server doesn't support resources capability");
                Ok(rmcp::model::ReadResourceResult::new(vec![]))
            }
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        // Check if the server has resources capability and forward the request
        match self.capabilities().resources {
            Some(_) => {
                tokio::select! {
                    result = inner.peer.list_resource_templates(request) => {
                        match result {
                            Ok(result) => {
                                debug!("Proxying list_resource_templates response");
                                Ok(result)
                            }
                            Err(err) => {
                                error!("Error listing resource templates: {:?}", err);
                                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                                    warn!("[list_resource_templates] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                                    self.swap_backend(None);
                                }
                                Err(ErrorData::internal_error(
                                    format!("Error listing resource templates: {err}"),
                                    None,
                                ))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        info!("[list_resource_templates] request canceled - MCP ID: {}", self.mcp_id);
                        Err(ErrorData::internal_error(
                            "Request cancelled".to_string(),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support resources, return empty list
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
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        // Check if the server has prompts capability and forward the request
        match self.capabilities().prompts {
            Some(_) => {
                tokio::select! {
                    result = inner.peer.list_prompts(request) => {
                        match result {
                            Ok(result) => {
                                debug!("Proxying list_prompts response");
                                Ok(result)
                            }
                            Err(err) => {
                                error!("Error listing prompts: {:?}", err);
                                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                                    warn!("[list_prompts] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                                    self.swap_backend(None);
                                }
                                Err(ErrorData::internal_error(
                                    format!("Error listing prompts: {err}"),
                                    None,
                                ))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        info!("[list_prompts] Request canceled - MCP ID: {}", self.mcp_id);
                        Err(ErrorData::internal_error(
                            "Request cancelled".to_string(),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support prompts, return empty list
                warn!("Server doesn't support prompts capability");
                Ok(rmcp::model::ListPromptsResult::default())
            }
        }
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, ErrorData> {
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        // Check if the server has prompts capability and forward the request
        match self.capabilities().prompts {
            Some(_) => {
                tokio::select! {
                    result = inner.peer.get_prompt(request.clone()) => {
                        match result {
                            Ok(result) => {
                                debug!("Proxying get_prompt response");
                                Ok(result)
                            }
                            Err(err) => {
                                error!("Error getting prompt: {:?}", err);
                                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                                    warn!("[get_prompt] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                                    self.swap_backend(None);
                                }
                                Err(ErrorData::internal_error(
                                    format!("Error getting prompt: {err}"),
                                    None,
                                ))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        info!("[get_prompt] Request canceled - MCP ID: {}, prompt: {:?}", self.mcp_id, request.name);
                        Err(ErrorData::internal_error(
                            "Request cancelled".to_string(),
                            None,
                        ))
                    }
                }
            }
            None => {
                // Server doesn't support prompts, return empty messages
                warn!("Server doesn't support prompts capability");
                let messages = Vec::new();
                Ok(rmcp::model::GetPromptResult::new(messages))
            }
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = inner_guard.as_ref().ok_or_else(|| {
            error!("Backend connection is not available (reconnecting)");
            ErrorData::internal_error(
                "Backend connection is not available, reconnecting...".to_string(),
                None,
            )
        })?;

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        tokio::select! {
            result = inner.peer.complete(request) => {
                match result {
                    Ok(result) => {
                        debug!("Proxying complete response");
                        Ok(result)
                    }
                    Err(err) => {
                        error!("Error completing: {:?}", err);
                        if is_transport_error(&err, inner.peer.is_transport_closed()) {
                            warn!("[complete] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                            self.swap_backend(None);
                        }
                        Err(ErrorData::internal_error(
                            format!("Error completing: {err}"),
                            None,
                        ))
                    }
                }
            }
            _ = context.ct.cancelled() => {
                info!("[complete] Request canceled - MCP ID: {}", self.mcp_id);
                Err(ErrorData::internal_error(
                    "Request cancelled".to_string(),
                    None,
                ))
            }
        }
    }

    async fn on_progress(
        &self,
        notification: rmcp::model::ProgressNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = match inner_guard.as_ref() {
            Some(inner) => inner,
            None => {
                error!("Backend connection is not available, cannot forward progress notification");
                return;
            }
        };

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed, cannot forward progress notification");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return;
        }

        match inner.peer.notify_progress(notification).await {
            Ok(_) => {
                debug!("Proxying progress notification");
            }
            Err(err) => {
                error!("Error notifying progress: {:?}", err);
                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                    warn!("[on_progress] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                    self.swap_backend(None);
                }
            }
        }
    }

    async fn on_cancelled(
        &self,
        notification: rmcp::model::CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        // 原子加载后端连接
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

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed, cannot forward cancelled notification");
            // transport 已关闭，立即标记后端不可用，触发 watchdog 重连
            self.swap_backend(None);
            return;
        }

        match inner.peer.notify_cancelled(notification).await {
            Ok(_) => {
                debug!("Proxying cancelled notification");
            }
            Err(err) => {
                error!("Error notifying cancelled: {:?}", err);
                if is_transport_error(&err, inner.peer.is_transport_closed()) {
                    warn!("[on_cancelled] Transport error detected, marking backend unavailable - MCP ID: {}", self.mcp_id);
                    self.swap_backend(None);
                }
            }
        }
    }
}

/// 判断错误是否属于传输层错误，需要立即标记后端不可用
///
/// 基于 rmcp ServiceError 类型匹配检测传输层错误
///
/// 优先使用类型匹配和 ErrorCode 匹配，不依赖字符串关键字：
/// - `ServiceError::TransportSend(_)` — 传输层发送失败（覆盖 connection reset、broken pipe、HTTP 404 等）
/// - `ServiceError::TransportClosed` — 传输通道已关闭
/// - `ServiceError::McpError(ErrorData)` 中 code == RESOURCE_NOT_FOUND — 资源未找到（如 sessionId 过期）
/// - is_transport_closed 兜底 — Peer 通道已关闭但 ServiceError 尚未捕获的情况
fn is_transport_error(err: &rmcp::ServiceError, is_transport_closed: bool) -> bool {
    match err {
        rmcp::ServiceError::TransportSend(_) | rmcp::ServiceError::TransportClosed => true,
        rmcp::ServiceError::McpError(err_data) => {
            // RESOURCE_NOT_FOUND (-32002): 上游 SSE 服务 sessionId 过期等场景
            err_data.code == rmcp::model::ErrorCode::RESOURCE_NOT_FOUND || is_transport_closed
        }
        _ => is_transport_closed,
    }
}

impl ProxyHandler {
    /// 获取 capabilities 的引用，避免 clone
    #[inline]
    fn capabilities(&self) -> &rmcp::model::ServerCapabilities {
        &self.cached_info.capabilities
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
    fn extract_server_info(
        client: &RunningService<RoleClient, ClientInfo>,
        mcp_id: &str,
    ) -> ServerInfo {
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
            cached_info: default_info,
            mcp_id,
            tool_filter,
            backend_version: Arc::new(AtomicU64::new(0)), // 断开状态版本为 0
        }
    }

    pub fn new(client: RunningService<RoleClient, ClientInfo>) -> Self {
        Self::with_mcp_id(client, "unknown".to_string())
    }

    pub fn with_mcp_id(client: RunningService<RoleClient, ClientInfo>, mcp_id: String) -> Self {
        Self::with_tool_filter(client, mcp_id, ToolFilter::default())
    }

    /// 创建带工具过滤器的 ProxyHandler（带初始后端连接）
    pub fn with_tool_filter(
        client: RunningService<RoleClient, ClientInfo>,
        mcp_id: String,
        tool_filter: ToolFilter,
    ) -> Self {
        use std::ops::Deref;

        // 提取 ServerInfo
        let cached_info = Self::extract_server_info(&client, &mcp_id);

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
            cached_info,
            mcp_id,
            tool_filter,
            backend_version: Arc::new(AtomicU64::new(1)), // 初始版本为 1
        }
    }

    /// 原子性替换后端连接
    /// - Some(client): 设置新的后端连接
    /// - None: 标记后端断开
    ///
    /// **版本控制**：每次调用都会递增 backend_version，使旧 session 失效
    pub fn swap_backend(&self, new_client: Option<RunningService<RoleClient, ClientInfo>>) {
        use std::ops::Deref;

        match new_client {
            Some(client) => {
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
        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = match inner_guard.as_ref() {
            Some(inner) => inner,
            None => return true,
        };

        // 快速检查 transport 状态
        if inner.peer.is_transport_closed() {
            return true;
        }

        // 通过发送轻量级请求来验证连接
        match inner.peer.list_tools(None).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_common::ToolFilter;
    use rmcp::model::{Implementation, ServerCapabilities};

    /// 创建测试用的默认 ServerInfo
    fn test_server_info() -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("test-server", "0.1.0"))
    }

    // ========== is_transport_error 测试 ==========

    #[test]
    fn test_is_transport_error_transport_send() {
        // TransportSend 包含所有传输层发送错误（connection reset、broken pipe、404 等）
        let dyn_err = rmcp::transport::DynamicTransportError {
            transport_name: std::borrow::Cow::Borrowed("test"),
            transport_type_id: std::any::TypeId::of::<()>(),
            error: Box::new(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe")),
        };
        let transport_err = rmcp::ServiceError::TransportSend(dyn_err);
        assert!(is_transport_error(&transport_err, false));
    }

    #[test]
    fn test_is_transport_error_transport_closed() {
        let closed_err = rmcp::ServiceError::TransportClosed;
        assert!(is_transport_error(&closed_err, false));
    }

    #[test]
    fn test_is_transport_error_mcp_error_resource_not_found() {
        // McpError 中 code == RESOURCE_NOT_FOUND（sessionId 过期时上游返回）
        let mcp_err = rmcp::ServiceError::McpError(rmcp::ErrorData::resource_not_found(
            "sessionId expired".to_string(),
            None,
        ));
        assert!(is_transport_error(&mcp_err, false));
    }

    #[test]
    fn test_is_transport_error_mcp_error_internal_error() {
        // 普通 McpError（如 INTERNAL_ERROR）不应触发
        let mcp_err = rmcp::ServiceError::McpError(rmcp::ErrorData::internal_error(
            "Internal server error".to_string(),
            None,
        ));
        assert!(!is_transport_error(&mcp_err, false));
    }

    #[test]
    fn test_is_transport_error_with_is_transport_closed() {
        // is_transport_closed=true 时，即使是非传输错误也触发（兜底机制）
        let mcp_err = rmcp::ServiceError::McpError(rmcp::ErrorData::internal_error(
            "some random error".to_string(),
            None,
        ));
        assert!(is_transport_error(&mcp_err, true));
    }

    #[test]
    fn test_is_transport_error_non_transport_error() {
        // 非传输层错误不应触发
        let cancelled_err = rmcp::ServiceError::Cancelled { reason: Some("user cancelled".to_string()) };
        assert!(!is_transport_error(&cancelled_err, false));

        let timeout_err = rmcp::ServiceError::Timeout { timeout: std::time::Duration::from_secs(30) };
        assert!(!is_transport_error(&timeout_err, false));
    }

    // ========== ProxyHandler 生命周期测试 ==========

    #[test]
    fn test_new_disconnected_handler_is_not_available() {
        let handler = ProxyHandler::new_disconnected(
            "test-mcp".to_string(),
            ToolFilter::default(),
            test_server_info(),
        );
        assert!(!handler.is_backend_available());
        assert!(handler.is_terminated());
        assert_eq!(handler.mcp_id(), "test-mcp");
        // 断开状态版本号为 0
        assert_eq!(handler.get_backend_version(), 0);
    }

    #[test]
    fn test_swap_backend_none_increments_version() {
        let handler = ProxyHandler::new_disconnected(
            "test-mcp".to_string(),
            ToolFilter::default(),
            test_server_info(),
        );
        assert_eq!(handler.get_backend_version(), 0);

        // swap_backend(None) 应递增版本号
        handler.swap_backend(None);
        assert_eq!(handler.get_backend_version(), 1);

        // 再次 swap_backend(None) 继续递增
        handler.swap_backend(None);
        assert_eq!(handler.get_backend_version(), 2);
    }

    #[test]
    fn test_get_info_returns_cached_info() {
        let info = test_server_info();
        let handler = ProxyHandler::new_disconnected(
            "test-mcp".to_string(),
            ToolFilter::default(),
            info.clone(),
        );
        let returned_info = handler.get_info();
        assert_eq!(
            returned_info.server_info.name,
            info.server_info.name
        );
    }

    // ========== 工具过滤测试 ==========

    #[test]
    fn test_tool_filter_whitelist() {
        let filter = ToolFilter::allow(vec!["echo".to_string(), "increment".to_string()]);
        assert!(filter.is_enabled());
        assert!(filter.is_allowed("echo"));
        assert!(filter.is_allowed("increment"));
        assert!(!filter.is_allowed("dangerous_tool"));
    }

    #[test]
    fn test_tool_filter_blacklist() {
        let filter = ToolFilter::deny(vec!["dangerous_tool".to_string()]);
        assert!(filter.is_enabled());
        assert!(filter.is_allowed("echo"));
        assert!(!filter.is_allowed("dangerous_tool"));
    }

    #[test]
    fn test_tool_filter_default_allows_all() {
        let filter = ToolFilter::default();
        assert!(!filter.is_enabled());
        assert!(filter.is_allowed("any_tool"));
        assert!(filter.is_allowed("another_tool"));
    }

    // ========== Streamable HTTP 集成测试：通过 stdio 连接 test_mcp_server ==========

    #[tokio::test]
    async fn test_proxy_handler_with_stdio_backend() {
        use rmcp::{ServiceExt, model::ClientInfo};

        // 1. 启动 test_mcp_server 作为 stdio 子进程
        // rmcp-soddygo 1.1.1: TokioChildProcess::new() 返回 io::Result（同步）
        let mut cmd = tokio::process::Command::new("cargo");
        cmd.args([
            "run",
            "--example",
            "test_mcp_server",
            "-p",
            "mcp-sse-proxy",
            "-q",
        ])
        .stderr(std::process::Stdio::null());
        let transport = match rmcp::transport::child_process::TokioChildProcess::new(cmd) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Skipping integration test: cannot start test_mcp_server: {e}");
                return;
            }
        };

        // 2. 创建 MCP 客户端连接
        // rmcp-soddygo 1.1.1: ClientInfo::new() 需要 (ClientCapabilities, Implementation) 两个参数
        let client_info = ClientInfo::new(
            rmcp::model::ClientCapabilities::builder().enable_roots().build(),
            Implementation::new("test-client", "0.1.0"),
        );

        let running = match client_info.serve(transport).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Skipping integration test: client handshake failed: {e}");
                return;
            }
        };

        // 3. 创建 ProxyHandler
        let handler = ProxyHandler::with_tool_filter(
            running,
            "integration-test".to_string(),
            ToolFilter::default(),
        );

        // 4. 验证后端可用
        assert!(handler.is_backend_available(), "backend should be available after connection");
        assert!(!handler.is_terminated(), "handler should not be terminated after connection");
        assert_eq!(handler.get_backend_version(), 1);

        // 5. 通过 Peer 直接调用 list_tools
        {
            let inner_guard = handler.peer.load();
            let inner = inner_guard.as_ref().expect("backend should exist");
            let result = inner.peer.list_tools(None).await;
            assert!(result.is_ok(), "list_tools via peer should succeed");
            let tools = &result.unwrap().tools;
            assert_eq!(tools.len(), 4, "test_mcp_server should have 4 tools");
        }

        // 6. 通过 Peer 直接调用 call_tool (echo)
        {
            let inner_guard = handler.peer.load();
            let inner = inner_guard.as_ref().expect("backend should exist");
            let mut args = serde_json::Map::new();
            args.insert("message".to_string(), serde_json::json!("hello"));
            let result = inner.peer.call_tool(
                CallToolRequestParams::new("echo").with_arguments(args)
            ).await;
            assert!(result.is_ok(), "call_tool echo should succeed");
        }

        // 7. swap_backend(None) 使后端不可用，版本号递增
        let version_before = handler.get_backend_version();
        handler.swap_backend(None);
        assert!(!handler.is_backend_available(), "backend should be unavailable after swap_backend(None)");
        assert!(handler.is_terminated(), "handler should be terminated after swap_backend(None)");
        assert!(handler.get_backend_version() > version_before);
    }
}

