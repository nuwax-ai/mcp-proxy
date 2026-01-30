use axum::Router;
use dashmap::DashMap;
use log::{debug, error, info};
use moka::future::Cache;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};
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

// =============================================================================
// RAII 进程管理设计
// =============================================================================
//
// 设计目标：当 mcp_id 从 map 中移除时，自动释放对应的 MCP 进程资源
//
// 核心结构：
// - McpProcessGuard: 进程生命周期守护器，实现 Drop trait 自动取消 CancellationToken
// - McpService: 封装 McpHandler + McpProcessGuard + 服务状态，作为 map 的 value
// - ProxyHandlerManager: 使用单一 DashMap<String, McpService> 管理所有服务
//
// 资源释放流程：
// 1. 从 map 中 remove mcp_id
// 2. McpService 被 drop
// 3. McpProcessGuard::drop() 被调用
// 4. CancellationToken 被 cancel
// 5. 监听该 token 的 SseServer/子进程收到信号，自动退出
// =============================================================================

/// MCP 进程生命周期守护器
///
/// 实现 RAII 模式：当此结构体被 drop 时，自动取消 CancellationToken，
/// 触发关联的 SseServer 和子进程退出。
///
/// # 使用场景
///
/// 1. 从 `ProxyHandlerManager` 移除 mcp_id 时，自动清理进程
/// 2. 服务重启时，旧服务自动被清理
/// 3. 系统关闭时，所有服务自动清理
pub struct McpProcessGuard {
    mcp_id: String,
    cancellation_token: CancellationToken,
}

impl McpProcessGuard {
    pub fn new(mcp_id: String, cancellation_token: CancellationToken) -> Self {
        debug!("[RAII] 创建进程守护器: mcp_id={}", mcp_id);
        Self {
            mcp_id,
            cancellation_token,
        }
    }

    /// 克隆 CancellationToken（用于传递给异步任务）
    pub fn clone_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }
}

impl Drop for McpProcessGuard {
    fn drop(&mut self) {
        info!(
            "[RAII] 进程守护器被 drop，取消 CancellationToken: mcp_id={}",
            self.mcp_id
        );
        self.cancellation_token.cancel();
    }
}

// McpProcessGuard 不实现 Clone，确保唯一所有权
impl std::fmt::Debug for McpProcessGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpProcessGuard")
            .field("mcp_id", &self.mcp_id)
            .field("is_cancelled", &self.cancellation_token.is_cancelled())
            .finish()
    }
}

/// MCP 服务封装
///
/// 将 McpHandler、McpProcessGuard 和服务状态封装在一起，
/// 作为 `ProxyHandlerManager` 中 DashMap 的 value。
///
/// # RAII 保证
///
/// 当 McpService 被 drop 时：
/// 1. McpProcessGuard 被 drop，触发 CancellationToken 取消
/// 2. 关联的子进程收到信号，自动退出
pub struct McpService {
    /// 进程守护器（RAII 核心）
    process_guard: McpProcessGuard,
    /// MCP 透明代理处理器（可选，启动中时为 None）
    handler: Option<McpHandler>,
    /// 服务状态信息
    status: McpServiceStatusInfo,
}

/// MCP 服务状态信息（不包含 CancellationToken，由 McpProcessGuard 管理）
#[derive(Debug, Clone)]
pub struct McpServiceStatusInfo {
    pub mcp_id: String,
    pub mcp_type: McpType,
    pub mcp_router_path: McpRouterPath,
    pub check_mcp_status_response_status: CheckMcpStatusResponseStatus,
    pub last_accessed: Instant,
    pub mcp_config: Option<McpConfig>,
}

impl McpServiceStatusInfo {
    pub fn new(
        mcp_id: String,
        mcp_type: McpType,
        mcp_router_path: McpRouterPath,
        check_mcp_status_response_status: CheckMcpStatusResponseStatus,
    ) -> Self {
        Self {
            mcp_id,
            mcp_type,
            mcp_router_path,
            check_mcp_status_response_status,
            last_accessed: Instant::now(),
            mcp_config: None,
        }
    }

    pub fn update_last_accessed(&mut self) {
        self.last_accessed = Instant::now();
    }
}

impl McpService {
    /// 创建新的 MCP 服务
    ///
    /// # 参数
    /// - `mcp_id`: 服务唯一标识
    /// - `mcp_type`: 服务类型
    /// - `mcp_router_path`: 路由路径
    /// - `cancellation_token`: 用于控制进程生命周期的取消令牌
    pub fn new(
        mcp_id: String,
        mcp_type: McpType,
        mcp_router_path: McpRouterPath,
        cancellation_token: CancellationToken,
    ) -> Self {
        let process_guard = McpProcessGuard::new(mcp_id.clone(), cancellation_token);
        let status = McpServiceStatusInfo::new(
            mcp_id,
            mcp_type,
            mcp_router_path,
            CheckMcpStatusResponseStatus::Pending,
        );
        Self {
            process_guard,
            handler: None,
            status,
        }
    }

    /// 设置 MCP Handler
    pub fn set_handler(&mut self, handler: McpHandler) {
        self.handler = Some(handler);
    }

    /// 获取 MCP Handler
    pub fn handler(&self) -> Option<&McpHandler> {
        self.handler.as_ref()
    }

    /// 获取服务状态
    pub fn status(&self) -> &McpServiceStatusInfo {
        &self.status
    }

    /// 获取可变服务状态
    pub fn status_mut(&mut self) -> &mut McpServiceStatusInfo {
        &mut self.status
    }

    /// 克隆 CancellationToken
    pub fn clone_token(&self) -> CancellationToken {
        self.process_guard.clone_token()
    }

    /// 更新服务状态
    pub fn update_status(&mut self, status: CheckMcpStatusResponseStatus) {
        self.status.check_mcp_status_response_status = status;
    }

    /// 更新最后访问时间
    pub fn update_last_accessed(&mut self) {
        self.status.update_last_accessed();
    }

    /// 设置 MCP 配置
    pub fn set_mcp_config(&mut self, config: McpConfig) {
        self.status.mcp_config = Some(config);
    }
}

impl std::fmt::Debug for McpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpService")
            .field("process_guard", &self.process_guard)
            .field("handler", &self.handler.is_some())
            .field("status", &self.status)
            .finish()
    }
}

// =============================================================================
// 兼容层：保留 McpServiceStatus 以兼容现有代码
// =============================================================================

/// MCP 服务状态（兼容层）
///
/// 保留此结构体以兼容现有代码，内部委托给 McpServiceStatusInfo
#[derive(Debug, Clone)]
pub struct McpServiceStatus {
    pub mcp_id: String,
    pub mcp_type: McpType,
    pub mcp_router_path: McpRouterPath,
    pub cancellation_token: CancellationToken,
    pub check_mcp_status_response_status: CheckMcpStatusResponseStatus,
    pub last_accessed: Instant,
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

    pub fn with_mcp_config(mut self, mcp_config: McpConfig) -> Self {
        self.mcp_config = Some(mcp_config);
        self
    }

    pub fn update_last_accessed(&mut self) {
        self.last_accessed = Instant::now();
    }
}

// =============================================================================
// ProxyHandlerManager：使用 RAII 模式管理 MCP 服务
// =============================================================================

/// MCP 代理管理器
///
/// 使用 RAII 模式管理 MCP 服务：
/// - 从 map 中移除 mcp_id 时，自动释放对应的进程资源
/// - 不需要显式调用 cleanup 方法（但仍提供显式清理接口）
#[derive(Debug)]
pub struct ProxyHandlerManager {
    /// 使用单一 DashMap 管理所有 MCP 服务（RAII 核心）
    services: DashMap<String, McpService>,
}

impl Default for ProxyHandlerManager {
    fn default() -> Self {
        ProxyHandlerManager {
            services: DashMap::new(),
        }
    }
}

impl ProxyHandlerManager {
    /// 添加 MCP 服务（RAII 模式）
    ///
    /// 使用新的 RAII 结构创建服务，当服务被移除时会自动清理资源
    pub fn add_mcp_service(
        &self,
        mcp_id: String,
        mcp_type: McpType,
        mcp_router_path: McpRouterPath,
        cancellation_token: CancellationToken,
    ) {
        let service = McpService::new(mcp_id.clone(), mcp_type, mcp_router_path, cancellation_token);

        // RAII: 如果已存在同名服务，insert 会返回旧服务，旧服务被 drop 时自动清理
        if let Some(old_service) = self.services.insert(mcp_id.clone(), service) {
            info!(
                "[RAII] 覆盖已存在的服务，旧服务将被自动清理: mcp_id={}",
                mcp_id
            );
            drop(old_service);
        }
    }

    /// 添加 MCP 服务状态（兼容旧 API）
    ///
    /// 保持与现有代码的兼容性，内部转换为新的 RAII 结构
    ///
    /// 注意：`last_accessed` 会被重置为当前时间（插入视为新访问）
    pub fn add_mcp_service_status_and_proxy(
        &self,
        mcp_service_status: McpServiceStatus,
        proxy_handler: Option<McpHandler>,
    ) {
        let mcp_id = mcp_service_status.mcp_id.clone();

        // 创建 McpService 使用 RAII 模式
        // 注意：last_accessed 会在 McpServiceStatusInfo::new() 中重置为 Instant::now()
        let mut service = McpService::new(
            mcp_id.clone(),
            mcp_service_status.mcp_type,
            mcp_service_status.mcp_router_path,
            mcp_service_status.cancellation_token,
        );

        // 设置初始状态
        service.status_mut().check_mcp_status_response_status =
            mcp_service_status.check_mcp_status_response_status;

        // 设置配置（如果有）
        if let Some(config) = mcp_service_status.mcp_config {
            service.set_mcp_config(config);
        }

        // 设置 handler（如果有）
        if let Some(handler) = proxy_handler {
            service.set_handler(handler);
        }

        // RAII: 如果已存在同名服务，insert 会返回旧服务，旧服务被 drop 时自动清理
        if let Some(old_service) = self.services.insert(mcp_id.clone(), service) {
            info!(
                "[RAII] 覆盖已存在的服务，旧服务将被自动清理: mcp_id={}",
                mcp_id
            );
            // old_service 在此作用域结束时 drop，触发 McpProcessGuard::drop()
            drop(old_service);
        }
    }

    /// 获取所有的 MCP 服务状态（兼容旧 API）
    ///
    /// 优化：先快速收集所有 keys，然后逐个获取详细信息
    /// 避免 iter() 长时间锁住多个分片，让其他写操作有机会执行
    pub fn get_all_mcp_service_status(&self) -> Vec<McpServiceStatus> {
        // 第一步：快速收集所有 keys（只 clone String，锁持有时间短）
        let keys: Vec<String> = self.services
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        // 第二步：逐个获取详细信息（每次只锁一个分片）
        keys.into_iter()
            .filter_map(|mcp_id| self.get_mcp_service_status(&mcp_id))
            .collect()
    }

    /// 获取 MCP 服务状态（兼容旧 API）
    pub fn get_mcp_service_status(&self, mcp_id: &str) -> Option<McpServiceStatus> {
        self.services.get(mcp_id).map(|entry| {
            let service = entry.value();
            let status = service.status();
            McpServiceStatus {
                mcp_id: status.mcp_id.clone(),
                mcp_type: status.mcp_type.clone(),
                mcp_router_path: status.mcp_router_path.clone(),
                cancellation_token: service.clone_token(),
                check_mcp_status_response_status: status.check_mcp_status_response_status.clone(),
                last_accessed: status.last_accessed,
                mcp_config: status.mcp_config.clone(),
            }
        })
    }

    /// 更新最后访问时间
    ///
    /// 使用 entry API 确保原子性操作
    pub fn update_last_accessed(&self, mcp_id: &str) {
        self.services
            .entry(mcp_id.to_string())
            .and_modify(|service| service.update_last_accessed());
    }

    /// 修改 MCP 服务状态 (Ready/Pending/Error)
    ///
    /// 使用 entry API 确保原子性操作
    pub fn update_mcp_service_status(&self, mcp_id: &str, status: CheckMcpStatusResponseStatus) {
        self.services
            .entry(mcp_id.to_string())
            .and_modify(|service| service.update_status(status));
    }

    /// 获取 MCP Handler
    pub fn get_proxy_handler(&self, mcp_id: &str) -> Option<McpHandler> {
        self.services
            .get(mcp_id)
            .and_then(|entry| entry.value().handler().cloned())
    }

    /// 获取服务的 MCP 配置（用于自动重启）
    pub fn get_mcp_config(&self, mcp_id: &str) -> Option<McpConfig> {
        self.services
            .get(mcp_id)
            .and_then(|entry| entry.value().status().mcp_config.clone())
    }

    /// 添加 MCP Handler 到已存在的服务
    ///
    /// 使用 entry API 确保原子性操作
    pub fn add_proxy_handler(&self, mcp_id: &str, proxy_handler: McpHandler) {
        match self.services.entry(mcp_id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                entry.get_mut().set_handler(proxy_handler);
            }
            dashmap::mapref::entry::Entry::Vacant(_) => {
                warn!(
                    "[RAII] 尝试添加 handler 到不存在的服务: mcp_id={}",
                    mcp_id
                );
            }
        }
    }

    /// 检查服务是否存在
    pub fn contains_service(&self, mcp_id: &str) -> bool {
        self.services.contains_key(mcp_id)
    }

    /// 获取服务数量
    pub fn service_count(&self) -> usize {
        self.services.len()
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

    /// 清理资源 (RAII 模式简化版)
    ///
    /// 通过 RAII 模式，从 DashMap 中移除服务会自动：
    /// 1. 触发 McpProcessGuard::drop()
    /// 2. 取消 CancellationToken
    /// 3. 关联的子进程收到信号退出
    ///
    /// 此方法额外清理路由和缓存
    pub async fn cleanup_resources(&self, mcp_id: &str) -> Result<()> {
        info!("[RAII] 开始清理资源: mcp_id={}", mcp_id);

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

        // RAII 核心：从 DashMap 移除会触发 McpProcessGuard::drop()
        // 这会自动取消 CancellationToken，进而触发子进程退出
        if self.services.remove(mcp_id).is_some() {
            info!(
                "[RAII] 服务已从 map 移除，McpProcessGuard 将自动取消令牌: mcp_id={}",
                mcp_id
            );
        } else {
            debug!("[RAII] 服务不存在，跳过移除: mcp_id={}", mcp_id);
        }

        // 清理配置缓存
        self.unregister_mcp_config(mcp_id).await;

        // 清理健康状态缓存
        GLOBAL_RESTART_TRACKER.clear_health_status(mcp_id);

        info!("[RAII] MCP 服务资源清理完成: mcp_id={}", mcp_id);
        Ok(())
    }

    /// 系统关闭，清理所有资源
    ///
    /// RAII 模式下，清除 DashMap 会自动释放所有资源
    pub async fn cleanup_all_resources(&self) -> Result<()> {
        info!("[RAII] 开始清理所有 MCP 服务资源");

        // 收集所有 mcp_id
        let mcp_ids: Vec<String> = self
            .services
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        let count = mcp_ids.len();

        // 逐个清理（包括路由和缓存）
        for mcp_id in mcp_ids {
            if let Err(e) = self.cleanup_resources(&mcp_id).await {
                error!("[RAII] 清理资源失败: mcp_id={}, error={}", mcp_id, e);
                // 继续清理其他资源
            }
        }

        info!("[RAII] 所有 MCP 服务资源清理完成，共清理 {} 个服务", count);
        Ok(())
    }

    /// 仅移除服务（依赖 RAII 自动清理进程）
    ///
    /// 从 DashMap 中移除服务，触发 RAII 自动清理。
    /// 不会清理路由和缓存，适用于需要快速移除服务的场景。
    pub fn remove_service(&self, mcp_id: &str) -> bool {
        if self.services.remove(mcp_id).is_some() {
            info!(
                "[RAII] 服务已移除，进程将自动清理: mcp_id={}",
                mcp_id
            );
            true
        } else {
            debug!("[RAII] 服务不存在: mcp_id={}", mcp_id);
            false
        }
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
    /// 返回 Some(OwnedMutexGuard) 表示获取成功，可以继续启动服务
    /// 返回 None 表示服务正在启动中，应该跳过本次启动
    ///
    /// # 使用方式
    ///
    /// ```ignore
    /// if let Some(_guard) = GLOBAL_RESTART_TRACKER.try_acquire_startup_lock(&mcp_id) {
    ///     // 获取到锁，可以启动服务
    ///     let result = start_service().await;
    ///     // _guard 在作用域结束时自动释放
    /// } else {
    ///     // 未获取到锁，服务正在启动中
    ///     return Ok(Response::503);
    /// }
    /// ```
    pub fn try_acquire_startup_lock(&self, mcp_id: &str) -> Option<OwnedMutexGuard<()>> {
        // 使用 entry API 确保原子性，避免竞态条件
        let lock = self
            .startup_locks
            .entry(mcp_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();

        // 尝试获取 owned 锁，锁会一直保持到返回的 guard 被 drop
        match lock.try_lock_owned() {
            Ok(guard) => Some(guard),
            Err(_) => {
                // 锁被占用，服务正在启动中
                debug!("服务 {} 正在启动中，跳过本次启动", mcp_id);
                None
            }
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
