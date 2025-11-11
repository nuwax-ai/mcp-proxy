use axum::Router;
use dashmap::DashMap;
use log::{debug, info};
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::ProxyHandler;

use super::{CheckMcpStatusResponseStatus, McpProtocol, McpRouterPath, McpType};

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

//根据用户的 mcp_id ,获取对应的 ProxyHandler;
//定义结构体
#[derive(Debug, Clone)]
pub struct ProxyHandlerManager {
    // 存储 ProxyHandler 透明代理服务
    proxy_handlers: DashMap<String, ProxyHandler>,
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
        }
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
        proxy_handler: Option<ProxyHandler>,
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
        self.mcp_service_statuses
            .get_mut(mcp_id)
            .iter_mut()
            .for_each(|entry| {
                entry.value_mut().update_last_accessed();
            })
    }

    //修改 mcp服务状态,Ready/Pending/Error
    pub fn update_mcp_service_status(&self, mcp_id: &str, status: CheckMcpStatusResponseStatus) {
        if let Some(mut mcp_service_status) = self.mcp_service_statuses.get_mut(mcp_id) {
            mcp_service_status.check_mcp_status_response_status = status;
        }
    }

    pub fn get_proxy_handler(&self, mcp_id: &str) -> Option<ProxyHandler> {
        self.proxy_handlers
            .get(mcp_id)
            .map(|entry| entry.value().clone())
    }

    pub fn add_proxy_handler(&self, mcp_id: &str, proxy_handler: ProxyHandler) {
        self.proxy_handlers
            .insert(mcp_id.to_string(), proxy_handler);
    }

    // 清理资源,根据 mcp_id 清理资源
    pub async fn cleanup_resources(&self, mcp_id: &str) {
        let mcp_sse_router_path = McpRouterPath::new(mcp_id.to_string(), McpProtocol::Sse);
        let base_sse_path = mcp_sse_router_path.base_path;

        let mcp_stream_router_path = McpRouterPath::new(mcp_id.to_string(), McpProtocol::Stream);
        let base_stream_path = mcp_stream_router_path.base_path;
        // 移除相关资源
        DynamicRouterService::delete_route(&base_sse_path);
        DynamicRouterService::delete_route(&base_stream_path);

        if let Some(status) = self.mcp_service_statuses.get_mut(mcp_id) {
            info!("Cleaning up resources for mcp_id: {mcp_id}",);
            // 取消与此 mcp_id 关联的 SseServer/command 终端的 CancellationToken
            status.cancellation_token.cancel();
            info!("CancellationToken cancelled for mcp_id: {mcp_id}");
        }

        self.proxy_handlers.remove(mcp_id);
        self.mcp_service_statuses.remove(mcp_id);

        info!("MCP 服务 {mcp_id} 的 command 终端资源清理已触发");
    }

    // 系统关闭,清理所有资源
    pub async fn cleanup_all_resources(&self) {
        for mcp_service_entry in self.mcp_service_statuses.iter() {
            self.cleanup_resources(mcp_service_entry.key()).await;
        }
    }
}

// 提供一个便捷的函数来获取全局 ProxyHandlerManager
pub fn get_proxy_manager() -> &'static ProxyHandlerManager {
    &GLOBAL_PROXY_MANAGER
}
