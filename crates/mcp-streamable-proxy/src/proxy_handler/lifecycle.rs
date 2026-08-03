use super::*;

impl ProxyHandler {
    /// Shared upstream peer registry used across reconnects.
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
    pub(super) fn extract_server_info(client: &BackendRunningService, mcp_id: &str) -> ServerInfo {
        client
            .peer_info()
            .map(|peer_info| super::peer_info_to_server_info((*peer_info).clone()))
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
            discovery: DiscoveryCache::new(default_info, None),
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
            discovery: DiscoveryCache::new(cached_info, None),
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
        self.swap_backend_with_discovery(new_client, None);
    }

    pub fn swap_backend_with_discovery(
        &self,
        new_client: Option<BackendRunningService>,
        complete_tools: Option<ListToolsResult>,
    ) {
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
                self.update_discovery(info, complete_tools);

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
        match serde_json::to_value(self.discovery.info()) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(%error, "failed to serialize cached server info");
                serde_json::Value::Null
            }
        }
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
}
