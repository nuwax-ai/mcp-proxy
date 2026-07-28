use super::direct_core::PeerInner;
use super::*;

impl SseHandler {
    pub(super) fn backend_peer(&self) -> Result<Peer<RoleClient>, ErrorData> {
        self.load_backend_peer()
            .map_err(ForwardError::into_error_data)
    }

    /// 获取 capabilities 快照
    #[inline]
    pub(super) fn capabilities(&self) -> rmcp::model::ServerCapabilities {
        self.discovery.capabilities()
    }

    pub(super) fn filter_tools(&self, mut result: ListToolsResult) -> ListToolsResult {
        if self.tool_filter.is_enabled() {
            result
                .tools
                .retain(|tool| self.tool_filter.is_allowed(&tool.name));
        }
        result
    }

    pub(super) fn cached_tools_or_error(
        &self,
        message: &str,
    ) -> Result<ListToolsResult, ErrorData> {
        if let Some(cached) = self.discovery.tools() {
            warn!("{message}; returning cached tools snapshot");
            return Ok(self.filter_tools(cached));
        }
        Err(ErrorData::internal_error(message.to_string(), None))
    }

    pub(super) fn update_cached_tools(&self, tools: ListToolsResult) {
        self.discovery.update_tools(tools);
    }

    fn update_discovery(&self, info: ServerInfo, tools: Option<ListToolsResult>) {
        self.discovery.update(info, tools);
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
            .cloned()
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

        Self::new_disconnected_with_fallback(mcp_id, tool_filter, default_info, None)
    }

    pub fn new_disconnected_with_fallback(
        mcp_id: String,
        tool_filter: ToolFilter,
        default_info: ServerInfo,
        fallback_tools: Option<ListToolsResult>,
    ) -> Self {
        Self {
            peer: Arc::new(ArcSwapOption::empty()),
            discovery: DiscoveryCache::new(default_info, fallback_tools),
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
            discovery: DiscoveryCache::new(cached_info, None),
            mcp_id,
            tool_filter,
        }
    }

    /// 原子性替换后端连接
    /// - Some(client): 设置新的后端连接
    /// - None: 标记后端断开
    pub fn swap_backend(&self, new_client: Option<RunningService<RoleClient, ClientInfo>>) {
        self.swap_backend_with_discovery(new_client, None);
    }

    pub fn swap_backend_with_discovery(
        &self,
        new_client: Option<RunningService<RoleClient, ClientInfo>>,
        complete_tools: Option<ListToolsResult>,
    ) {
        use std::ops::Deref;

        match new_client {
            Some(client) => {
                let info = Self::extract_server_info(&client, &self.mcp_id);
                self.update_discovery(info, complete_tools);
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
        match self.peer.load_full() {
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
        let inner = match self.peer.load_full() {
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
