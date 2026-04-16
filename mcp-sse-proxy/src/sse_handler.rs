use arc_swap::ArcSwapOption;
pub use mcp_common::ToolFilter;
/**
 * Create a local SSE server that proxies requests to a stdio MCP server.
 */
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler,
    model::{
        CallToolRequestParam, CallToolResult, ClientInfo, Content, Implementation, ListToolsResult,
        PaginatedRequestParam, ProtocolVersion, ServerInfo,
    },
    service::{NotificationContext, Peer, RequestContext, RunningService},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime};
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

/// A SSE proxy handler that forwards requests to a client based on the server's capabilities
/// 使用 ArcSwap 实现后端热替换，支持断开时立即返回错误
///
/// **SSE 模式**：使用 rmcp 0.10，稳定的 SSE 传输协议
#[derive(Clone, Debug)]
pub struct SseHandler {
    /// 后端连接（ArcSwap 支持无锁原子替换）
    /// None 表示后端断开/重连中
    peer: Arc<ArcSwapOption<PeerInner>>,
    /// 缓存的服务器信息（保持不变，重连后应一致）
    cached_info: ServerInfo,
    /// MCP ID 用于日志记录
    mcp_id: String,
    /// 工具过滤配置
    tool_filter: ToolFilter,
}

impl ServerHandler for SseHandler {
    fn get_info(&self) -> ServerInfo {
        self.cached_info.clone()
    }

    #[tracing::instrument(skip(self, request, context), fields(
        mcp_id = %self.mcp_id,
        request = ?request,
    ))]
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
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
                                })
                            }
                            Err(err) => {
                                error!("Error listing tools: {:?}", err);
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
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // 生成唯一请求 ID 用于追踪
        let request_id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        let start_time = SystemTime::now();

        info!(
            "[call_tool:{}] Start - Tool: {}, MCP ID: {}, Time: {:?}",
            request_id, request.name, self.mcp_id, start_time
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

                // 使用 tokio::select! 同时等待取消和结果
                tokio::select! {
                    result = inner.peer.call_tool(request.clone()) => {
                        let elapsed = start.elapsed();
                        match &result {
                            Ok(call_result) => {
                                // 记录工具调用结果，这些结果会通过 SSE 推送给客户端
                                let is_error = call_result.is_error.unwrap_or(false);
                                info!(
                                    "[call_tool:{}] Response received - tool: {}, time taken: {}ms, is_error: {}, MCP ID: {}",
                                    request_id, request.name, elapsed.as_millis(), is_error, self.mcp_id
                                );
                                if is_error {
                                    // 记录错误响应的内容（用于调试）
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
                                    request_id, request.name, elapsed.as_millis(), err, self.mcp_id
                                );
                                // Return an error result instead of propagating the error
                                Ok(CallToolResult::error(vec![Content::text(format!(
                                    "Error: {err}"
                                ))]))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        let elapsed = start.elapsed();
                        warn!(
                            "[call_tool:{}] Request canceled - Tool: {}, Time taken: {}ms, MCP ID: {}",
                            request_id, request.name, elapsed.as_millis(), self.mcp_id
                        );
                        Ok(CallToolResult::error(vec![Content::text(
                            "Request cancelled"
                        )]))
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
        request: Option<PaginatedRequestParam>,
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
        request: rmcp::model::ReadResourceRequestParam,
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
            return Err(ErrorData::internal_error(
                "Backend connection closed, please retry".to_string(),
                None,
            ));
        }

        // Check if the server has resources capability and forward the request
        match self.capabilities().resources {
            Some(_) => {
                tokio::select! {
                    result = inner.peer.read_resource(rmcp::model::ReadResourceRequestParam {
                        uri: request.uri.clone(),
                    }) => {
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
                Ok(rmcp::model::ReadResourceResult {
                    contents: Vec::new(),
                })
            }
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParam>,
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
        request: Option<PaginatedRequestParam>,
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
        request: rmcp::model::GetPromptRequestParam,
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
                // Server doesn't support prompts, return error
                warn!("Server doesn't support prompts capability");
                Ok(rmcp::model::GetPromptResult {
                    description: None,
                    messages: Vec::new(),
                })
            }
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParam,
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
            return;
        }

        match inner.peer.notify_progress(notification).await {
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
            return;
        }

        match inner.peer.notify_cancelled(notification).await {
            Ok(_) => {
                debug!("Proxying cancelled notification");
            }
            Err(err) => {
                error!("Error notifying cancelled: {:?}", err);
            }
        }
    }
}

impl SseHandler {
    /// 获取 capabilities 的引用，避免 clone
    #[inline]
    fn capabilities(&self) -> &rmcp::model::ServerCapabilities {
        &self.cached_info.capabilities
    }

    /// 创建一个默认的 ServerInfo（用于断开状态）
    fn default_server_info(mcp_id: &str) -> ServerInfo {
        warn!(
            "[SseHandler] Create default ServerInfo - MCP ID: {}",
            mcp_id
        );
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            server_info: Implementation {
                name: "MCP Proxy".to_string(),
                version: "0.1.0".to_string(),
                title: None,
                website_url: None,
                icons: None,
            },
            instructions: None,
            capabilities: Default::default(),
        }
    }

    /// 从 RunningService 提取 ServerInfo
    fn extract_server_info(
        client: &RunningService<RoleClient, ClientInfo>,
        mcp_id: &str,
    ) -> ServerInfo {
        client
            .peer_info()
            .map(|peer_info| ServerInfo {
                protocol_version: peer_info.protocol_version.clone(),
                server_info: Implementation {
                    name: peer_info.server_info.name.clone(),
                    version: peer_info.server_info.version.clone(),
                    title: None,
                    website_url: None,
                    icons: None,
                },
                instructions: peer_info.instructions.clone(),
                capabilities: peer_info.capabilities.clone(),
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
            "[SseHandler] Create a disconnected handler - MCP ID: {}",
            mcp_id
        );

        // 记录过滤器配置
        if tool_filter.is_enabled() {
            if let Some(ref allow_list) = tool_filter.allow_tools {
                info!(
                    "[SseHandler] Tool whitelist enabled - MCP ID: {}, allowed tools: {:?}",
                    mcp_id, allow_list
                );
            }
            if let Some(ref deny_list) = tool_filter.deny_tools {
                info!(
                    "[SseHandler] Tool blacklist enabled - MCP ID: {}, excluded tools: {:?}",
                    mcp_id, deny_list
                );
            }
        }

        Self {
            peer: Arc::new(ArcSwapOption::empty()),
            cached_info: default_info,
            mcp_id,
            tool_filter,
        }
    }

    pub fn new(client: RunningService<RoleClient, ClientInfo>) -> Self {
        Self::with_mcp_id(client, "unknown".to_string())
    }

    pub fn with_mcp_id(client: RunningService<RoleClient, ClientInfo>, mcp_id: String) -> Self {
        Self::with_tool_filter(client, mcp_id, ToolFilter::default())
    }

    /// 创建带工具过滤器的 SseHandler（带初始后端连接）
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
                    "[SseHandler] Tool whitelist enabled - MCP ID: {}, allowed tools: {:?}",
                    mcp_id, allow_list
                );
            }
            if let Some(ref deny_list) = tool_filter.deny_tools {
                info!(
                    "[SseHandler] Tool blacklist enabled - MCP ID: {}, excluded tools: {:?}",
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
        }
    }

    /// 原子性替换后端连接
    /// - Some(client): 设置新的后端连接
    /// - None: 标记后端断开
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
                    "[SseHandler] Backend connection updated - MCP ID: {}",
                    self.mcp_id
                );
            }
            None => {
                self.peer.store(None);
                info!(
                    "[SseHandler] Backend connection disconnected - MCP ID: {}",
                    self.mcp_id
                );
            }
        }
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

    /// Update backend from an SseClientConnection
    ///
    /// This method allows updating the backend connection using the high-level
    /// `SseClientConnection` type, which is more convenient than the raw
    /// `RunningService` type.
    ///
    /// # Arguments
    /// * `conn` - Some(connection) to set new backend, None to mark disconnected
    pub fn swap_backend_from_connection(&self, conn: Option<crate::client::SseClientConnection>) {
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

/// A handler that bridges an external backend to the SSE server
///
/// Uses the `BackendBridge` trait (defined in `mcp-common`) to communicate with
/// any backend implementation (e.g., rmcp 1.4.0's ProxyHandler) without directly
/// depending on the concrete type.
#[derive(Clone)]
pub struct BackendSessionHandler {
    /// Backend connection via protocol-agnostic trait
    backend: Arc<dyn mcp_common::BackendBridge>,
    /// MCP ID for logging
    mcp_id: String,
    /// Cached server info (bridged from backend's rmcp version via JSON)
    cached_info: rmcp::model::ServerInfo,
}

impl std::fmt::Debug for BackendSessionHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendSessionHandler")
            .field("mcp_id", &self.mcp_id)
            .field("cached_info", &self.cached_info)
            .finish()
    }
}

impl BackendSessionHandler {
    pub fn new(backend: Arc<dyn mcp_common::BackendBridge>, mcp_id: String) -> Self {
        // 从后端获取 ServerInfo（JSON 桥接跨 rmcp 版本）
        // 这确保 SSE 客户端能看到后端实际支持的 capabilities（tools、resources 等）
        let backend_info_json = backend.get_server_info_json();
        let mut cached_info: rmcp::model::ServerInfo =
            serde_json::from_value(backend_info_json).unwrap_or_else(|e| {
                warn!(
                    "[BackendSessionHandler] Failed to deserialize backend ServerInfo: {}, \
                     using default - MCP ID: {}",
                    e, mcp_id
                );
                rmcp::model::ServerInfo::default()
            });

        // 关键：强制覆盖 protocol_version 为 SSE 客户端支持的版本
        // 因为前端是 SSE 协议（rmcp 0.10），后端可能返回更新的版本（如 2025-06-18）
        // Java SSE SDK 等客户端只支持旧版本，若透传新版本会导致握手失败
        let backend_version = cached_info.protocol_version.clone();
        cached_info.protocol_version = rmcp::model::ProtocolVersion::V_2024_11_05;
        info!(
            "[BackendSessionHandler] Override protocol_version: backend={:?} → SSE={:?} \
             - MCP ID: {}",
            backend_version, cached_info.protocol_version, mcp_id
        );

        info!(
            "[BackendSessionHandler] Created with backend capabilities - MCP ID: {}, \
             tools: {}, resources: {}, prompts: {}",
            mcp_id,
            cached_info.capabilities.tools.is_some(),
            cached_info.capabilities.resources.is_some(),
            cached_info.capabilities.prompts.is_some(),
        );

        Self {
            backend,
            mcp_id,
            cached_info,
        }
    }

    /// 获取 MCP ID
    pub fn mcp_id(&self) -> &str {
        &self.mcp_id
    }

    /// 检查后端是否可用（快速检查，不发送请求）
    pub fn is_backend_available(&self) -> bool {
        self.backend.is_backend_available()
    }

    /// 检查 mcp 服务是否正常（异步版本，会发送验证请求）
    pub async fn is_mcp_server_ready(&self) -> bool {
        self.backend.is_mcp_server_ready().await
    }

    /// 异步检查后端连接是否已断开（会发送验证请求）
    pub async fn is_terminated_async(&self) -> bool {
        self.backend.is_terminated_async().await
    }
}

impl ServerHandler for BackendSessionHandler {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        self.cached_info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let start = std::time::Instant::now();
        info!(
            "[BackendSessionHandler] list_tools request - MCP ID: {}",
            self.mcp_id
        );

        let params = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
        let backend = self.backend.clone();

        tokio::select! {
            result = backend.call_peer_method("tools/list", params) => {
                let elapsed = start.elapsed();
                match result {
                    Ok(value) => {
                        let result: ListToolsResult = serde_json::from_value(value)
                            .map_err(|e| {
                                error!(
                                    "[BackendSessionHandler] list_tools deserialize error \
                                     - MCP ID: {}, error: {}, time: {}ms",
                                    self.mcp_id, e, elapsed.as_millis()
                                );
                                ErrorData::internal_error(format!("deserialize error: {}", e), None)
                            })?;
                        info!(
                            "[BackendSessionHandler] list_tools success \
                             - MCP ID: {}, tools: {}, time: {}ms",
                            self.mcp_id, result.tools.len(), elapsed.as_millis()
                        );
                        debug!(
                            "[BackendSessionHandler] list_tools tool names: {:?}",
                            result.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
                        );
                        Ok(result)
                    }
                    Err(e) => {
                        error!(
                            "[BackendSessionHandler] list_tools backend error \
                             - MCP ID: {}, error: {}, time: {}ms",
                            self.mcp_id, e, elapsed.as_millis()
                        );
                        Err(ErrorData::internal_error(e, None))
                    }
                }
            }
            _ = context.ct.cancelled() => {
                warn!(
                    "[BackendSessionHandler] list_tools cancelled - MCP ID: {}",
                    self.mcp_id
                );
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        info!(
            "[BackendSessionHandler] call_tool request - MCP ID: {}, tool: {}, args: {:?}",
            self.mcp_id, request.name, request.arguments
        );

        let params = serde_json::to_value(&request)
            .map_err(|e| ErrorData::internal_error(format!("serialize error: {}", e), None))?;
        let backend = self.backend.clone();

        tokio::select! {
            result = backend.call_peer_method("tools/call", params) => {
                let elapsed = start.elapsed();
                match result {
                    Ok(value) => {
                        let result: CallToolResult = serde_json::from_value(value)
                            .map_err(|e| {
                                error!(
                                    "[BackendSessionHandler] call_tool deserialize error \
                                     - MCP ID: {}, tool: {}, error: {}, time: {}ms",
                                    self.mcp_id, request.name, e, elapsed.as_millis()
                                );
                                ErrorData::internal_error(format!("deserialize error: {}", e), None)
                            })?;
                        let is_error = result.is_error.unwrap_or(false);
                        info!(
                            "[BackendSessionHandler] call_tool response \
                             - MCP ID: {}, tool: {}, is_error: {}, time: {}ms",
                            self.mcp_id, request.name, is_error, elapsed.as_millis()
                        );
                        if is_error {
                            debug!(
                                "[BackendSessionHandler] call_tool error content: {:?}",
                                result.content
                            );
                        }
                        Ok(result)
                    }
                    Err(e) => {
                        error!(
                            "[BackendSessionHandler] call_tool backend error \
                             - MCP ID: {}, tool: {}, error: {}, time: {}ms",
                            self.mcp_id, request.name, e, elapsed.as_millis()
                        );
                        Err(ErrorData::internal_error(e, None))
                    }
                }
            }
            _ = context.ct.cancelled() => {
                warn!(
                    "[BackendSessionHandler] call_tool cancelled \
                     - MCP ID: {}, tool: {}",
                    self.mcp_id, request.name
                );
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        let params = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
        let backend = self.backend.clone();

        tokio::select! {
            result = backend.call_peer_method("resources/list", params) => {
                match result {
                    Ok(value) => {
                        let result: rmcp::model::ListResourcesResult = serde_json::from_value(value)
                            .map_err(|e| ErrorData::internal_error(format!("deserialize error: {}", e), None))?;
                        Ok(result)
                    }
                    Err(e) => Err(ErrorData::internal_error(e, None)),
                }
            }
            _ = context.ct.cancelled() => {
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        let params = serde_json::to_value(&request)
            .map_err(|e| ErrorData::internal_error(format!("serialize error: {}", e), None))?;

        tokio::select! {
            result = self.backend.call_peer_method("resources/read", params) => {
                match result {
                    Ok(value) => {
                        let result: rmcp::model::ReadResourceResult = serde_json::from_value(value)
                            .map_err(|e| ErrorData::internal_error(format!("deserialize error: {}", e), None))?;
                        Ok(result)
                    }
                    Err(e) => Err(ErrorData::internal_error(e, None)),
                }
            }
            _ = context.ct.cancelled() => {
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        let params = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
        let backend = self.backend.clone();

        tokio::select! {
            result = backend.call_peer_method("resources/templates/list", params) => {
                match result {
                    Ok(value) => {
                        serde_json::from_value(value)
                            .map_err(|e| ErrorData::internal_error(format!("deserialize error: {}", e), None))
                    }
                    Err(e) => Err(ErrorData::internal_error(e, None)),
                }
            }
            _ = context.ct.cancelled() => {
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        let params = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
        let backend = self.backend.clone();

        tokio::select! {
            result = backend.call_peer_method("prompts/list", params) => {
                match result {
                    Ok(value) => {
                        serde_json::from_value(value)
                            .map_err(|e| ErrorData::internal_error(format!("deserialize error: {}", e), None))
                    }
                    Err(e) => Err(ErrorData::internal_error(e, None)),
                }
            }
            _ = context.ct.cancelled() => {
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, ErrorData> {
        let params = serde_json::to_value(&request)
            .map_err(|e| ErrorData::internal_error(format!("serialize error: {}", e), None))?;
        let backend = self.backend.clone();

        tokio::select! {
            result = backend.call_peer_method("prompts/get", params) => {
                match result {
                    Ok(value) => {
                        serde_json::from_value(value)
                            .map_err(|e| ErrorData::internal_error(format!("deserialize error: {}", e), None))
                    }
                    Err(e) => Err(ErrorData::internal_error(e, None)),
                }
            }
            _ = context.ct.cancelled() => {
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        let params = serde_json::to_value(&request)
            .map_err(|e| ErrorData::internal_error(format!("serialize error: {}", e), None))?;
        let backend = self.backend.clone();

        tokio::select! {
            result = backend.call_peer_method("completion/complete", params) => {
                match result {
                    Ok(value) => {
                        serde_json::from_value(value)
                            .map_err(|e| ErrorData::internal_error(format!("deserialize error: {}", e), None))
                    }
                    Err(e) => Err(ErrorData::internal_error(e, None)),
                }
            }
            _ = context.ct.cancelled() => {
                Err(ErrorData::internal_error("Request cancelled".to_string(), None))
            }
        }
    }

    async fn on_progress(
        &self,
        _notification: rmcp::model::ProgressNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        // TODO: Notification forwarding requires extending BackendBridge trait
        // with a send_notification() method. For now, notifications are logged
        // but not forwarded to the Streamable HTTP backend.
        debug!(
            "[BackendSessionHandler] Received progress notification (not forwarded) \
             - MCP ID: {}",
            self.mcp_id
        );
    }

    async fn on_cancelled(
        &self,
        _notification: rmcp::model::CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        // TODO: Same as on_progress — requires BackendBridge extension to forward
        warn!(
            "[BackendSessionHandler] Received cancelled notification (not forwarded) \
             - MCP ID: {}",
            self.mcp_id
        );
    }
}

/// Unified handler enum for SSE server
///
/// This enum wraps either `SseHandler` (for Stdio/SSE URL backends) or
/// `BackendSessionHandler` (for Streamable HTTP backends), providing a
/// common type that implements `ServerHandler` trait.
///
/// Both handler types have the same underlying functionality but use different
/// MCP client implementations:
/// - `SseHandler` uses rmcp 0.10 `Peer<RoleClient>` directly
/// - `BackendSessionHandler` delegates to `ProxyHandler` from mcp-streamable-proxy (rmcp 1.4.0)
#[derive(Clone, Debug)]
pub enum SseServerHandler {
    /// Standard SSE handler for Stdio/SSE URL backends
    Sse(SseHandler),
    /// Backend session handler for Streamable HTTP backends
    BackendSession(BackendSessionHandler),
}

impl ServerHandler for SseServerHandler {
    fn get_info(&self) -> ServerInfo {
        match self {
            SseServerHandler::Sse(h) => h.get_info(),
            SseServerHandler::BackendSession(h) => h.get_info(),
        }
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_tools(request, context).await,
            SseServerHandler::BackendSession(h) => h.list_tools(request, context).await,
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.call_tool(request, context).await,
            SseServerHandler::BackendSession(h) => h.call_tool(request, context).await,
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_resources(request, context).await,
            SseServerHandler::BackendSession(h) => h.list_resources(request, context).await,
        }
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.read_resource(request, context).await,
            SseServerHandler::BackendSession(h) => h.read_resource(request, context).await,
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_resource_templates(request, context).await,
            SseServerHandler::BackendSession(h) => {
                h.list_resource_templates(request, context).await
            }
        }
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.list_prompts(request, context).await,
            SseServerHandler::BackendSession(h) => h.list_prompts(request, context).await,
        }
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.get_prompt(request, context).await,
            SseServerHandler::BackendSession(h) => h.get_prompt(request, context).await,
        }
    }

    async fn complete(
        &self,
        request: rmcp::model::CompleteRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, ErrorData> {
        match self {
            SseServerHandler::Sse(h) => h.complete(request, context).await,
            SseServerHandler::BackendSession(h) => h.complete(request, context).await,
        }
    }

    async fn on_progress(
        &self,
        notification: rmcp::model::ProgressNotificationParam,
        context: NotificationContext<RoleServer>,
    ) {
        match self {
            SseServerHandler::Sse(h) => h.on_progress(notification, context).await,
            SseServerHandler::BackendSession(h) => h.on_progress(notification, context).await,
        }
    }

    async fn on_cancelled(
        &self,
        notification: rmcp::model::CancelledNotificationParam,
        context: NotificationContext<RoleServer>,
    ) {
        match self {
            SseServerHandler::Sse(h) => h.on_cancelled(notification, context).await,
            SseServerHandler::BackendSession(h) => h.on_cancelled(notification, context).await,
        }
    }
}

impl SseServerHandler {
    /// 获取 MCP ID
    pub fn mcp_id(&self) -> &str {
        match self {
            SseServerHandler::Sse(h) => h.mcp_id(),
            SseServerHandler::BackendSession(h) => h.mcp_id(),
        }
    }

    /// 检查后端是否可用（快速检查，不发送请求）
    pub fn is_backend_available(&self) -> bool {
        match self {
            SseServerHandler::Sse(h) => h.is_backend_available(),
            SseServerHandler::BackendSession(h) => h.is_backend_available(),
        }
    }

    /// 检查 mcp 服务是否正常（异步版本，会发送验证请求）
    pub async fn is_mcp_server_ready(&self) -> bool {
        match self {
            SseServerHandler::Sse(h) => h.is_mcp_server_ready().await,
            SseServerHandler::BackendSession(h) => h.is_mcp_server_ready().await,
        }
    }

    /// 异步检查后端连接是否已断开（会发送验证请求）
    pub async fn is_terminated_async(&self) -> bool {
        match self {
            SseServerHandler::Sse(h) => h.is_terminated_async().await,
            SseServerHandler::BackendSession(h) => h.is_terminated_async().await,
        }
    }
}
