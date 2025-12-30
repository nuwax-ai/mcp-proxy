use tracing::{debug, info, warn, error};
/**
 * Create a local SSE server that proxies requests to a stdio MCP server.
 */
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler,
    model::{
        CallToolRequestParam, CallToolResult, ClientInfo, Content, Implementation, ListToolsResult,
        PaginatedRequestParam, ServerInfo,
    },
    service::{NotificationContext, Peer, RequestContext, RunningService},
};
use std::collections::HashSet;
use std::sync::Arc;
use arc_swap::ArcSwapOption;

/// 工具过滤配置
#[derive(Clone, Debug, Default)]
pub struct ToolFilter {
    /// 白名单（只允许这些工具）
    pub allow_tools: Option<HashSet<String>>,
    /// 黑名单（排除这些工具）
    pub deny_tools: Option<HashSet<String>>,
}

impl ToolFilter {
    /// 创建白名单过滤器
    pub fn allow(tools: Vec<String>) -> Self {
        Self {
            allow_tools: Some(tools.into_iter().collect()),
            deny_tools: None,
        }
    }

    /// 创建黑名单过滤器
    pub fn deny(tools: Vec<String>) -> Self {
        Self {
            allow_tools: None,
            deny_tools: Some(tools.into_iter().collect()),
        }
    }

    /// 检查工具是否被允许
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        // 白名单模式：只有在白名单中的工具才被允许
        if let Some(ref allow_list) = self.allow_tools {
            return allow_list.contains(tool_name);
        }
        // 黑名单模式：不在黑名单中的工具都被允许
        if let Some(ref deny_list) = self.deny_tools {
            return !deny_list.contains(tool_name);
        }
        // 无过滤：全部允许
        true
    }

    /// 检查是否启用了过滤
    pub fn is_enabled(&self) -> bool {
        self.allow_tools.is_some() || self.deny_tools.is_some()
    }
}

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
                                    "[list_tools] 工具列表结果 - MCP ID: {}, 工具数量: {}{}",
                                    self.mcp_id,
                                    filtered_tools.len(),
                                    if self.tool_filter.is_enabled() {
                                        " (已过滤)"
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
                        info!("[list_tools] 请求被取消 - MCP ID: {}", self.mcp_id);
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
        // 首先检查工具是否被过滤
        if !self.tool_filter.is_allowed(&request.name) {
            info!(
                "[call_tool] 工具被过滤 - MCP ID: {}, 工具: {}",
                self.mcp_id, request.name
            );
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Tool '{}' is not allowed by filter configuration",
                request.name
            ))]));
        }

        // 原子加载后端连接
        let inner_guard = self.peer.load();
        let inner = match inner_guard.as_ref() {
            Some(inner) => inner,
            None => {
                error!("Backend connection is not available (reconnecting)");
                return Ok(CallToolResult::error(vec![Content::text(
                    "Backend connection is not available, reconnecting..."
                )]));
            }
        };

        // 检查后端连接是否已关闭
        if inner.peer.is_transport_closed() {
            error!("Backend transport is closed");
            return Ok(CallToolResult::error(vec![Content::text(
                "Backend connection closed, please retry",
            )]));
        }

        // Check if the server has tools capability and forward the request
        match self.capabilities().tools {
            Some(_) => {
                // 使用 tokio::select! 同时等待取消和结果
                tokio::select! {
                    result = inner.peer.call_tool(request.clone()) => {
                        match result {
                            Ok(result) => {
                                // 记录工具调用结果，这些结果会通过 SSE 推送给客户端
                                info!(
                                    "[call_tool] 工具调用成功 - MCP ID: {}, 工具: {}",
                                    self.mcp_id, request.name
                                );

                                debug!("Tool call succeeded");
                                Ok(result)
                            }
                            Err(err) => {
                                error!("Error calling tool: {:?}", err);
                                // Return an error result instead of propagating the error
                                Ok(CallToolResult::error(vec![Content::text(format!(
                                    "Error: {err}"
                                ))]))
                            }
                        }
                    }
                    _ = context.ct.cancelled() => {
                        info!(
                            "[call_tool] 请求被取消 - MCP ID: {}, 工具: {}",
                            self.mcp_id, request.name
                        );
                        Ok(CallToolResult::error(vec![Content::text(
                            "Request cancelled"
                        )]))
                    }
                }
            }
            None => {
                error!("Server doesn't support tools capability");
                Ok(CallToolResult::error(vec![Content::text(
                    "Server doesn't support tools capability",
                )]))
            }
        }
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
                                    "[list_resources] 资源列表结果 - MCP ID: {}, 资源数量: {}",
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
                        info!("[list_resources] 请求被取消 - MCP ID: {}", self.mcp_id);
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
                                    "[read_resource] 资源读取结果 - MCP ID: {}, URI: {}",
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
                        info!("[read_resource] 请求被取消 - MCP ID: {}, URI: {}", self.mcp_id, request.uri);
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
                        info!("[list_resource_templates] 请求被取消 - MCP ID: {}", self.mcp_id);
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
                        info!("[list_prompts] 请求被取消 - MCP ID: {}", self.mcp_id);
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
                        info!("[get_prompt] 请求被取消 - MCP ID: {}, prompt: {:?}", self.mcp_id, request.name);
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
                info!("[complete] 请求被取消 - MCP ID: {}", self.mcp_id);
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
                error!("Backend connection is not available, cannot forward cancelled notification");
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

impl ProxyHandler {
    /// 获取 capabilities 的引用，避免 clone
    #[inline]
    fn capabilities(&self) -> &rmcp::model::ServerCapabilities {
        &self.cached_info.capabilities
    }

    /// 创建一个默认的 ServerInfo（用于断开状态）
    fn default_server_info(mcp_id: &str) -> ServerInfo {
        warn!("[ProxyHandler] 创建默认 ServerInfo - MCP ID: {}", mcp_id);
        ServerInfo {
            protocol_version: Default::default(),
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
    fn extract_server_info(client: &RunningService<RoleClient, ClientInfo>, mcp_id: &str) -> ServerInfo {
        client.peer_info().map(|peer_info| ServerInfo {
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
        }).unwrap_or_else(|| Self::default_server_info(mcp_id))
    }

    /// 创建断开状态的 handler（用于初始化）
    /// 后续通过 swap_backend() 注入实际的后端连接
    pub fn new_disconnected(mcp_id: String, tool_filter: ToolFilter, default_info: ServerInfo) -> Self {
        info!("[ProxyHandler] 创建断开状态的 handler - MCP ID: {}", mcp_id);

        // 记录过滤器配置
        if tool_filter.is_enabled() {
            if let Some(ref allow_list) = tool_filter.allow_tools {
                info!(
                    "[ProxyHandler] 工具白名单已启用 - MCP ID: {}, 允许的工具: {:?}",
                    mcp_id, allow_list
                );
            }
            if let Some(ref deny_list) = tool_filter.deny_tools {
                info!(
                    "[ProxyHandler] 工具黑名单已启用 - MCP ID: {}, 排除的工具: {:?}",
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
                    "[ProxyHandler] 工具白名单已启用 - MCP ID: {}, 允许的工具: {:?}",
                    mcp_id, allow_list
                );
            }
            if let Some(ref deny_list) = tool_filter.deny_tools {
                info!(
                    "[ProxyHandler] 工具黑名单已启用 - MCP ID: {}, 排除的工具: {:?}",
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
                info!("[ProxyHandler] 后端连接已更新 - MCP ID: {}", self.mcp_id);
            }
            None => {
                self.peer.store(None);
                info!("[ProxyHandler] 后端连接已断开 - MCP ID: {}", self.mcp_id);
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
                debug!("后端连接状态检查: 正常");
                false
            }
            Err(e) => {
                info!("后端连接状态检查: 已断开，原因: {e}");
                true
            }
        }
    }

    /// 获取 MCP ID
    pub fn mcp_id(&self) -> &str {
        &self.mcp_id
    }
}
