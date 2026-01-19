use axum::Router;
use dashmap::DashMap;
use log::{debug, error, info};
use moka::future::Cache;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use anyhow::Result;

use crate::proxy::McpHandler;

use super::{CheckMcpStatusResponseStatus, McpConfig, McpProtocol, McpRouterPath, McpType};

// 全局单例路由表
pub static GLOBAL_ROUTES: Lazy<Arc<DashMap<String, Router>>> =
    Lazy::new(|| Arc::new(DashMap::new()));

// 全局单例 ProxyHandlerManager
pub static GLOBAL_PROXY_MANAGER: Lazy<ProxyHandlerManager> =
    Lazy::new(ProxyHandlerManager::default);

/// 动态路由服务
#[derive(Clone)]
pub struct DynamicRouterService(pub McpProtocol);

impl DynamicRouterService {
    // 注册动态 handler
    pub fn register_route(path: &str, handler: Router) {
        debug!("=== 注册路由 ===");
        debug!("注册路径: {}", path);
        GLOBAL_ROUTES.insert(path.to_string(), handler);
        debug!("=== 注册路由完成 ===");
    }

    // 删除动态 handler
    pub fn delete_route(path: &str) {
        debug!("=== 删除路由 ===");
        debug!("删除路径: {}", path);
        GLOBAL_ROUTES.remove(path);
        debug!("=== 删除路由完成 ===");
    }

    // 获取动态 handler
    pub fn get_route(path: &str) -> Option<Router> {
        let result = GLOBAL_ROUTES.get(path).map(|entry| entry.value().clone());
        if result.is_some() {
            debug!("get_route('{}') = Some(Router)", path);
        } else {
            debug!("get_route('{}') = None", path);
        }
        result
    }

    // 获取所有已注册的路由（debug用）
    pub fn get_all_routes() -> Vec<String> {
        GLOBAL_ROUTES
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
}

impl std::fmt::Debug for DynamicRouterService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let routes = GLOBAL_ROUTES
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        write!(f, "DynamicRouterService {{ routes: {routes:?} }}")
    }
}

//mcp 代理管理器,包含路由,取消令牌,透明mcp代理处理器

//根据用户的 mcp_id ,获取对应的 McpHandler;
//定义结构体
#[derive(Debug, Clone)]
pub struct ProxyHandlerManager {
    // 存储 McpHandler 透明代理服务 (支持 SSE 和 Stream 两种类型)
    proxy_handlers: DashMap<String, McpHandler>,
    // 存储 MCP 服务状态,包含路径,类型,取消令牌,mcp_id,状态
    mcp_service_statuses: DashMap<String, McpServiceStatus>,
}
//定义 mcp服务结构,包含:mcpType,McpRouterPath,CancellationToken,mcp_id,CheckMcpStatusResponseStatus
#[derive(Debug, Clone)]
pub struct McpServiceStatus {
    // mcp_id
    pub mcp_id: String,
    // mcp类型
    pub mcp_type: McpType,
    // mcp路由路径
    pub mcp_router_path: McpRouterPath,
    // 用于控制与此 mcp_id 关联的 SseServer 和 command 终端
    pub cancellation_token: CancellationToken,
    // mcp服务状态
    pub check_mcp_status_response_status: CheckMcpStatusResponseStatus,
    // 最后访问时间
    pub last_accessed: Instant,
    // MCP 配置（用于自动重启服务）
    pub mcp_config: Option<McpConfig>,
}

impl McpServiceStatus {
    pub fn new(
        mcp_id: String,
        mcp_type: McpType,
        mcp_router_path: McpRouterPath,
        cancellation_token: CancellationToken,
        check_mcp_status_response_status: CheckMcpStatusResponseStatus,
    ) -> Self {
        Self {
            mcp_id,
            mcp_type,
            mcp_router_path,
            cancellation_token,
            check_mcp_status_response_status,
            last_accessed: Instant::now(),
            mcp_config: None,
        }
    }

    /// 设置 MCP 配置（用于自动重启）
    pub fn with_mcp_config(mut self, mcp_config: McpConfig) -> Self {
        self.mcp_config = Some(mcp_config);
        self
    }

    // 更新最后访问时间
    pub fn update_last_accessed(&mut self) {
        self.last_accessed = Instant::now();
    }
}

impl Default for ProxyHandlerManager {
    fn default() -> Self {
        ProxyHandlerManager {
            proxy_handlers: DashMap::new(),
            mcp_service_statuses: DashMap::new(),
        }
    }
}

impl ProxyHandlerManager {
    // 添加 MCP 服务状态
    pub fn add_mcp_service_status_and_proxy(
        &self,
        mcp_service_status: McpServiceStatus,
        proxy_handler: Option<McpHandler>,
    ) {
        let mcp_id = mcp_service_status.mcp_id.clone();
        self.mcp_service_statuses
            .insert(mcp_id.clone(), mcp_service_status);
        // 如果 proxy_handler 不为 None,则添加到 proxy_handlers; 为空的时候,是记录一个空代理处理器,正在启动中
        if let Some(proxy_handler) = proxy_handler {
            self.proxy_handlers.insert(mcp_id, proxy_handler);
        }
    }
    //获取所有的 mcp 服务状态
    pub fn get_all_mcp_service_status(&self) -> Vec<McpServiceStatus> {
        self.mcp_service_statuses
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    // 获取 MCP 服务状态
    pub fn get_mcp_service_status(&self, mcp_id: &str) -> Option<McpServiceStatus> {
        self.mcp_service_statuses
            .get(mcp_id)
            .map(|entry| entry.value().clone())
    }

    // 更新最后访问时间
    pub fn update_last_accessed(&self, mcp_id: &str) {
        if let Some(mut entry) = self.mcp_service_statuses.get_mut(mcp_id) {
            entry.value_mut().update_last_accessed();
        }
    }

    //修改 mcp服务状态,Ready/Pending/Error
    pub fn update_mcp_service_status(&self, mcp_id: &str, status: CheckMcpStatusResponseStatus) {
        if let Some(mut mcp_service_status) = self.mcp_service_statuses.get_mut(mcp_id) {
            mcp_service_status.check_mcp_status_response_status = status;
        }
    }

    pub fn get_proxy_handler(&self, mcp_id: &str) -> Option<McpHandler> {
        self.proxy_handlers
            .get(mcp_id)
            .map(|entry| entry.value().clone())
    }

    /// 获取服务的 MCP 配置（用于自动重启）
    pub fn get_mcp_config(&self, mcp_id: &str) -> Option<McpConfig> {
        self.mcp_service_statuses
            .get(mcp_id)
            .and_then(|status| status.value().mcp_config.clone())
    }

    pub fn add_proxy_handler(&self, mcp_id: &str, proxy_handler: McpHandler) {
        self.proxy_handlers
            .insert(mcp_id.to_string(), proxy_handler);
    }

    /// 注册 MCP 配置到缓存
    pub async fn register_mcp_config(&self, mcp_id: &str, config: McpConfig) {
        GLOBAL_MCP_CONFIG_CACHE
            .insert(mcp_id.to_string(), config)
            .await;
        info!("MCP 配置已注册到缓存: {}", mcp_id);
    }

    /// 从缓存获取 MCP 配置
    pub async fn get_mcp_config_from_cache(&self, mcp_id: &str) -> Option<McpConfig> {
        if let Some(config) = GLOBAL_MCP_CONFIG_CACHE.get(mcp_id).await {
            debug!("从缓存获取 MCP 配置: {}", mcp_id);
            Some(config)
        } else {
            debug!("缓存中未找到 MCP 配置: {}", mcp_id);
            None
        }
    }

    /// 从缓存删除 MCP 配置
    pub async fn unregister_mcp_config(&self, mcp_id: &str) {
        GLOBAL_MCP_CONFIG_CACHE.invalidate(mcp_id).await;
        info!("MCP 配置已从缓存删除: {}", mcp_id);
    }

    // 清理资源,根据 mcp_id 清理资源
    pub async fn cleanup_resources(&self, mcp_id: &str) -> Result<()> {
        // 创建路径以构建要删除的路由路径
        let mcp_sse_router_path = McpRouterPath::new(mcp_id.to_string(), McpProtocol::Sse)
            .map_err(|e| {
                anyhow::anyhow!("Failed to create SSE router path for {}: {}", mcp_id, e)
            })?;
        let base_sse_path = mcp_sse_router_path.base_path;

        let mcp_stream_router_path = McpRouterPath::new(mcp_id.to_string(), McpProtocol::Stream)
            .map_err(|e| {
                anyhow::anyhow!("Failed to create Stream router path for {}: {}", mcp_id, e)
            })?;
        let base_stream_path = mcp_stream_router_path.base_path;

        // 移除相关路由
        DynamicRouterService::delete_route(&base_sse_path);
        DynamicRouterService::delete_route(&base_stream_path);

        // 取消取消令牌并移除资源
        if let Some(status) = self.mcp_service_statuses.get_mut(mcp_id) {
            info!("Cleaning up resources for mcp_id: {mcp_id}");
            // 取消与此 mcp_id 关联的 SseServer/command 终端的 CancellationToken
            status.cancellation_token.cancel();
            info!("CancellationToken cancelled for mcp_id: {mcp_id}");
        }

        self.proxy_handlers.remove(mcp_id);
        self.mcp_service_statuses.remove(mcp_id);

        // 清理配置缓存
        self.unregister_mcp_config(mcp_id).await;

        // 清理健康状态缓存
        GLOBAL_RESTART_TRACKER.clear_health_status(mcp_id);

        info!("MCP 服务 {mcp_id} 的资源清理已完成");
        Ok(())
    }

    // 系统关闭,清理所有资源
    pub async fn cleanup_all_resources(&self) -> Result<()> {
        // 先收集所有 mcp_id，避免在遍历时修改 DashMap
        let mcp_ids: Vec<String> = self
            .mcp_service_statuses
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        // 再逐个清理资源
        for mcp_id in mcp_ids {
            if let Err(e) = self.cleanup_resources(&mcp_id).await {
                error!("Failed to cleanup resources for {}: {}", mcp_id, e);
                // 继续清理其他资源，不中断整个清理过程
            }
        }
        Ok(())
    }
}

/// MCP 配置缓存（使用 moka 实现 TTL）
///
/// ## 存储架构说明
///
/// MCP 配置存储在两个位置：
///
/// 1. **McpServiceStatus.mcp_config**（服务状态中）
///    - 存储当前运行服务的配置
///    - 随服务清理而被删除
///    - 用于快速访问当前服务的配置
///
/// 2. **GLOBAL_MCP_CONFIG_CACHE**（全局缓存）
///    - 独立于服务状态存储
///    - 有 TTL（24 小时）
///    - 用于服务重启时恢复配置
///
/// ## 为什么需要两处存储？
///
/// - 服务清理后，McpServiceStatus 被删除，但配置仍在缓存中
/// - 下次请求到来时，可以从缓存恢复配置并重启服务
/// - 实现了服务的自动重启能力
///
/// ## 优先级
///
/// 1. 请求 header 中的配置（最新）
/// 2. 缓存中的配置（兜底）
///
/// ## TTL
///
/// - 24 小时（可配置）
/// - max_capacity: 1000（防止内存溢出）
pub struct McpConfigCache {
    cache: Cache<String, McpConfig>,
}

impl McpConfigCache {
    pub fn new() -> Self {
        Self {
            cache: Cache::builder()
                .time_to_live(Duration::from_secs(24 * 60 * 60)) // 24 小时 TTL
                .max_capacity(1000) // 最多缓存 1000 个配置，防止内存溢出
                .build(),
        }
    }

    pub async fn insert(&self, mcp_id: String, config: McpConfig) {
        self.cache.insert(mcp_id.clone(), config).await;
        info!("MCP 配置已缓存: {} (TTL: 24h)", mcp_id);
    }

    pub async fn get(&self, mcp_id: &str) -> Option<McpConfig> {
        self.cache.get(mcp_id).await
    }

    pub async fn invalidate(&self, mcp_id: &str) {
        self.cache.invalidate(mcp_id).await;
    }

    #[allow(dead_code)]
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }
}

impl Default for McpConfigCache {
    fn default() -> Self {
        Self::new()
    }
}

// 全局配置缓存单例
pub static GLOBAL_MCP_CONFIG_CACHE: Lazy<McpConfigCache> = Lazy::new(McpConfigCache::default);

/// MCP 服务重启追踪器
///
/// 用于防止服务频繁重启导致的无限循环
///
/// ## 重启限制
///
/// - 最小重启间隔：30 秒
/// - 如果服务在 30 秒内被标记为需要重启，将跳过重启
/// - 这防止了服务启动失败时的无限重启循环
///
/// ## 健康状态缓存
///
/// - 缓存后端健康状态，避免频繁检查
/// - 缓存时间：5 秒（可配置）
/// - 用于减少 `is_mcp_server_ready()` 调用频率
pub struct RestartTracker {
    // mcp_id -> 最后重启时间
    last_restart: DashMap<String, Instant>,
    // mcp_id -> (健康状态, 检查时间)
    health_status: DashMap<String, (bool, Instant)>,
    // mcp_id -> 启动锁，防止并发启动同一服务
    startup_locks: DashMap<String, Arc<Mutex<()>>>,
}

impl RestartTracker {
    pub fn new() -> Self {
        Self {
            last_restart: DashMap::new(),
            health_status: DashMap::new(),
            startup_locks: DashMap::new(),
        }
    }

    /// 获取缓存的健康状态
    ///
    /// 如果缓存未过期（5秒内），返回缓存值
    /// 否则返回 None，表示需要重新检查
    pub fn get_cached_health_status(&self, mcp_id: &str) -> Option<bool> {
        let cache_duration = Duration::from_secs(5); // 5 秒缓存
        let now = Instant::now();

        self.health_status.get(mcp_id).and_then(|entry| {
            let (is_healthy, check_time) = *entry.value();
            if now.duration_since(check_time) < cache_duration {
                Some(is_healthy)
            } else {
                None
            }
        })
    }

    /// 更新健康状态缓存
    pub fn update_health_status(&self, mcp_id: &str, is_healthy: bool) {
        self.health_status
            .insert(mcp_id.to_string(), (is_healthy, Instant::now()));
    }

    /// 清除健康状态缓存
    pub fn clear_health_status(&self, mcp_id: &str) {
        self.health_status.remove(mcp_id);
    }

    /// 检查是否可以重启服务
    ///
    /// 返回 true 表示可以重启，false 表示在冷却期内
    ///
    /// 注意：此方法仅检查是否可以重启，不会自动插入时间戳。
    /// 时间戳只在服务成功启动后通过 `record_restart()` 方法记录。
    pub fn can_restart(&self, mcp_id: &str) -> bool {
        let now = Instant::now();
        let min_restart_interval = Duration::from_secs(30); // 30 秒最小重启间隔

        // 只检查，不自动插入时间戳
        if let Some(last_restart) = self.last_restart.get(mcp_id) {
            let elapsed = now.duration_since(*last_restart);
            if elapsed < min_restart_interval {
                warn!(
                    "服务 {} 在冷却期内，距离上次重启仅 {} 秒，跳过重启",
                    mcp_id,
                    elapsed.as_secs()
                );
                return false;
            }
        }
        // 不在冷却期内，但不自动更新时间戳
        true
    }

    /// 记录服务成功重启
    ///
    /// 此方法应在服务成功启动后调用，用于记录重启时间戳。
    /// 配合 `can_restart()` 使用，避免在服务启动失败时插入时间戳。
    pub fn record_restart(&self, mcp_id: &str) {
        self.last_restart.insert(mcp_id.to_string(), Instant::now());
        info!("服务启动成功，记录重启时间: {}", mcp_id);
    }

    /// 清除重启时间戳
    ///
    /// 当服务启动失败时，可选择调用此方法清除时间戳，
    /// 允许立即重试而不必等待冷却期。
    #[allow(dead_code)]
    pub fn clear_restart(&self, mcp_id: &str) {
        self.last_restart.remove(mcp_id);
        info!("已清除服务 {} 的重启时间戳", mcp_id);
    }

    /// 尝试获取服务启动锁
    ///
    /// 返回 Some(Arc<Mutex>) 表示获取成功，可以继续启动服务
    /// 返回 None 表示服务正在启动中，应该跳过本次启动
    ///
    /// # 使用方式
    ///
    /// ```ignore
    /// if let Some(lock) = GLOBAL_RESTART_TRACKER.try_acquire_startup_lock(&mcp_id) {
    ///     // 获取到锁，可以启动服务
    ///     let guard = lock.lock().await;
    ///     let result = start_service().await;
    ///     // guard 自动释放
    /// } else {
    ///     // 未获取到锁，服务正在启动中
    ///     return Ok(Response::503);
    /// }
    /// ```
    pub fn try_acquire_startup_lock(&self, mcp_id: &str) -> Option<Arc<Mutex<()>>> {
        // 使用 entry API 确保原子性，避免竞态条件
        let lock = self
            .startup_locks
            .entry(mcp_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();

        // 尝试获取锁，检查是否可用
        if lock.try_lock().is_ok() {
            // 锁可用，返回锁（try_lock 返回的 guard 已被 drop）
            Some(lock)
        } else {
            // 锁被占用，服务正在启动中
            debug!("服务 {} 正在启动中，跳过本次启动", mcp_id);
            None
        }
    }

    /// 清理服务启动锁
    ///
    /// 当服务启动完成或失败后，应该清理启动锁以允许后续重试
    /// 注意：正常情况下锁会随 MutexGuard 自动释放，此方法用于异常清理
    #[allow(dead_code)]
    pub fn cleanup_startup_lock(&self, mcp_id: &str) {
        self.startup_locks.remove(mcp_id);
        debug!("已清理服务 {} 的启动锁", mcp_id);
    }
}

impl Default for RestartTracker {
    fn default() -> Self {
        Self::new()
    }
}

// 全局重启追踪器单例
pub static GLOBAL_RESTART_TRACKER: Lazy<RestartTracker> = Lazy::new(RestartTracker::default);

// 提供一个便捷的函数来获取全局 ProxyHandlerManager
pub fn get_proxy_manager() -> &'static ProxyHandlerManager {
    &GLOBAL_PROXY_MANAGER
}
