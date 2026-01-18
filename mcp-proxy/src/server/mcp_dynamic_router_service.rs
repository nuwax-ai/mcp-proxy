use std::{
    convert::Infallible,
    task::{Context, Poll},
};

use axum::{
    body::Body,
    extract::Request,
    response::{IntoResponse, Response},
};
use futures::future::BoxFuture;
use log::{debug, error, info, warn};
use tower::Service;

use crate::{
    DynamicRouterService, get_proxy_manager, mcp_start_task,
    model::{GLOBAL_RESTART_TRACKER, HttpResult, McpConfig, McpRouterPath},
    server::middlewares::extract_trace_id,
};

impl Service<Request<Body>> for DynamicRouterService {
    type Response = Response;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().to_string();
        let method = req.method().clone();
        let headers = req.headers().clone();

        // DEBUG: 详细路径解析日志
        debug!("=== 路径解析开始 ===");
        debug!("原始请求路径: {}", path);
        debug!("路径包含的通配符参数: {:?}", req.extensions());

        // 提取 trace_id
        let trace_id = extract_trace_id();

        // 创建根 span (使用 debug_span 减少日志量)
        let span = tracing::debug_span!(
            "DynamicRouterService",
            otel.name = "HTTP Request",
            http.method = %method,
            http.route = %path,
            http.url = %req.uri(),
            mcp.protocol = format!("{:?}", self.0),
            trace_id = %trace_id,
        );

        // 记录请求头信息
        if let Some(content_type) = headers.get("content-type") {
            span.record("http.request.content_type", format!("{:?}", content_type));
        }
        if let Some(content_length) = headers.get("content-length") {
            span.record(
                "http.request.content_length",
                format!("{:?}", content_length),
            );
        }

        debug!("请求路径: {path}");

        // 解析路由路径
        let mcp_router_path = McpRouterPath::from_url(&path);

        match mcp_router_path {
            Some(mcp_router_path) => {
                let mcp_id = mcp_router_path.mcp_id.clone();
                let base_path = mcp_router_path.base_path.clone();

                span.record("mcp.id", &mcp_id);
                span.record("mcp.base_path", &base_path);

                debug!("=== 路径解析结果 ===");
                debug!("解析出的mcp_id: {}", mcp_id);
                debug!("解析出的base_path: {}", base_path);
                debug!("请求路径: {} vs base_path: {}", path, base_path);
                debug!("=== 路径解析结束 ===");

                Box::pin(async move {
                    let _guard = span.enter();

                    // 先尝试查找已注册的路由
                    debug!("=== 路由查找过程 ===");
                    debug!("查找base_path: '{}'", base_path);

                    if let Some(router_entry) = DynamicRouterService::get_route(&base_path) {
                        debug!(
                            "✅ 找到已注册的路由: base_path={}, path={}",
                            base_path, path
                        );

                        // ===== 检查后端健康状态 =====
                        // 提前提取 mcp_id，用于后续配置获取和重启逻辑
                        let mcp_id_for_check = McpRouterPath::from_url(&path);
                        let should_restart = if let Some(router_path) = mcp_id_for_check {
                            let proxy_manager = get_proxy_manager();

                            if let Some(handler) =
                                proxy_manager.get_proxy_handler(&router_path.mcp_id)
                            {
                                // 先检查缓存的健康状态（5秒缓存）
                                let is_healthy = if let Some(cached) = GLOBAL_RESTART_TRACKER
                                    .get_cached_health_status(&router_path.mcp_id)
                                {
                                    debug!(
                                        "使用缓存的健康状态: mcp_id={}, is_healthy={}",
                                        router_path.mcp_id, cached
                                    );
                                    cached
                                } else {
                                    // 缓存未命中，检查实际状态
                                    let status = handler.is_mcp_server_ready().await;
                                    GLOBAL_RESTART_TRACKER
                                        .update_health_status(&router_path.mcp_id, status);
                                    debug!(
                                        "检查后端健康状态: mcp_id={}, is_healthy={}",
                                        router_path.mcp_id, status
                                    );
                                    status
                                };

                                if is_healthy {
                                    debug!(
                                        "后端服务正常，直接使用路由: mcp_id={}",
                                        router_path.mcp_id
                                    );
                                    false // 不需要重启
                                } else {
                                    warn!(
                                        "后端服务已终止，清理并尝试重启: mcp_id={}",
                                        router_path.mcp_id
                                    );
                                    // 清理资源（包括路由）
                                    if let Err(e) =
                                        proxy_manager.cleanup_resources(&router_path.mcp_id).await
                                    {
                                        error!(
                                            "清理资源失败: mcp_id={}, error={}",
                                            router_path.mcp_id, e
                                        );
                                    }
                                    true // 需要重启
                                }
                            } else {
                                // handler 不存在，说明服务已被清理，需要重启
                                warn!(
                                    "路由存在但 handler 不存在，需要重启: base_path={}",
                                    base_path
                                );
                                true
                            }
                        } else {
                            // 无法解析路由路径，直接使用路由（不应该发生）
                            false
                        };

                        if !should_restart {
                            debug!("=== 路由查找结束(成功) ===");
                            return handle_request_with_router(req, router_entry, &path).await;
                        }
                        // 后端已死，继续执行后续逻辑尝试重启
                        debug!("后端已死，进入重启流程");
                    } else {
                        debug!(
                            "❌ 未找到已注册的路由: base_path='{}', path='{}'",
                            base_path, path
                        );

                        // 显示所有已注册的路由
                        let all_routes = DynamicRouterService::get_all_routes();
                        debug!("当前已注册的路由: {:?}", all_routes);
                        debug!("=== 路由查找结束(失败) ===");
                    }

                    // 未找到路由，尝试启动服务
                    warn!("未找到匹配的路径,尝试启动服务:base_path={base_path},path={path}");
                    span.record("error.route_not_found", true);

                    // ===== 提前解析 mcp_id 用于配置获取 =====
                    let mcp_router_path_for_config = McpRouterPath::from_url(&path);

                    // ===== 配置获取优先级 =====
                    let proxy_manager = get_proxy_manager();

                    // 优先级 1: 从请求 header 中获取配置（最新）
                    if let Some(mcp_config) = req.extensions().get::<McpConfig>().cloned()
                        && mcp_config.mcp_json_config.is_some()
                    {
                        // 检查重启限制（防止无限循环）
                        if !GLOBAL_RESTART_TRACKER.can_restart(&mcp_config.mcp_id) {
                            warn!("服务 {} 在重启冷却期内，跳过启动", mcp_config.mcp_id);
                            span.record("error.restart_in_cooldown", true);
                            let message =
                                format!("服务 {} 在重启冷却期内，请稍后再试", mcp_config.mcp_id);
                            let http_result: HttpResult<String> =
                                HttpResult::error("0002", &message, None);
                            span.record("http.response.status_code", 429u16); // Too Many Requests
                            return Ok(http_result.into_response());
                        }

                        // 尝试获取启动锁，防止并发启动同一服务
                        let startup_lock = match GLOBAL_RESTART_TRACKER.try_acquire_startup_lock(&mcp_config.mcp_id) {
                            Some(lock) => lock,
                            None => {
                                warn!("服务 {} 正在启动中，跳过本次启动", mcp_config.mcp_id);
                                span.record("error.startup_in_progress", true);
                                let message =
                                    format!("服务 {} 正在启动中，请稍后再试", mcp_config.mcp_id);
                                let http_result: HttpResult<String> =
                                    HttpResult::error("0003", &message, None);
                                span.record("http.response.status_code", 503u16); // Service Unavailable
                                return Ok(http_result.into_response());
                            }
                        };

                        // 获取锁，确保服务启动期间其他请求等待
                        let _guard = startup_lock.lock().await;

                        info!("使用请求 header 配置启动服务: {}", mcp_config.mcp_id);
                        // 同时更新缓存
                        proxy_manager
                            .register_mcp_config(&mcp_config.mcp_id, mcp_config.clone())
                            .await;

                        // _guard 会在作用域结束时自动释放
                        return start_mcp_and_handle_request(req, mcp_config).await;
                    }

                    // 优先级 2: 从 moka 缓存中获取配置（兜底）
                    if let Some(mcp_id_for_cache) =
                        mcp_router_path_for_config.as_ref().map(|p| &p.mcp_id)
                        && let Some(mcp_config) = proxy_manager
                            .get_mcp_config_from_cache(mcp_id_for_cache)
                            .await
                    {
                        // 检查重启限制（防止无限循环）
                        if !GLOBAL_RESTART_TRACKER.can_restart(mcp_id_for_cache) {
                            warn!("服务 {} 在重启冷却期内，跳过启动", mcp_id_for_cache);
                            span.record("error.restart_in_cooldown", true);
                            let message =
                                format!("服务 {} 在重启冷却期内，请稍后再试", mcp_id_for_cache);
                            let http_result: HttpResult<String> =
                                HttpResult::error("0002", &message, None);
                            span.record("http.response.status_code", 429u16); // Too Many Requests
                            return Ok(http_result.into_response());
                        }

                        // 尝试获取启动锁，防止并发启动同一服务
                        let startup_lock = match GLOBAL_RESTART_TRACKER.try_acquire_startup_lock(mcp_id_for_cache) {
                            Some(lock) => lock,
                            None => {
                                warn!("服务 {} 正在启动中，跳过本次启动", mcp_id_for_cache);
                                span.record("error.startup_in_progress", true);
                                let message =
                                    format!("服务 {} 正在启动中，请稍后再试", mcp_id_for_cache);
                                let http_result: HttpResult<String> =
                                    HttpResult::error("0003", &message, None);
                                span.record("http.response.status_code", 503u16); // Service Unavailable
                                return Ok(http_result.into_response());
                            }
                        };

                        // 获取锁，确保服务启动期间其他请求等待
                        let _guard = startup_lock.lock().await;

                        info!("使用缓存配置启动服务: {}", mcp_id_for_cache);
                        // _guard 会在作用域结束时自动释放
                        return start_mcp_and_handle_request(req, mcp_config).await;
                    }

                    // 优先级 3: 无法获取配置，返回错误
                    warn!("未找到匹配的路径,且未获取到配置,无法启动MCP服务: {path}");
                    span.record("error.mcp_config_missing", true);

                    let message =
                        format!("未找到匹配的路径,且未获取到配置,无法启动MCP服务: {path}");
                    let http_result: HttpResult<String> = HttpResult::error("0001", &message, None);
                    span.record("http.response.status_code", 404u16);
                    span.record("error.message", &message);
                    Ok(http_result.into_response())
                })
            }
            None => {
                warn!("请求路径解析失败: {path}");
                span.record("error.path_parse_failed", true);

                let message = format!("请求路径解析失败: {path}");
                let http_result: HttpResult<String> = HttpResult::error("0001", &message, None);
                Box::pin(async move {
                    let _guard = span.enter();
                    span.record("http.response.status_code", 400u16);
                    span.record("error.message", &message);
                    Ok(http_result.into_response())
                })
            }
        }
    }
}

/// 使用给定的路由处理请求
async fn handle_request_with_router(
    req: Request<Body>,
    router_entry: axum::Router,
    path: &str,
) -> Result<Response, Infallible> {
    // 获取匹配路径的Router，并处理请求
    let trace_id = extract_trace_id();

    let method = req.method().clone();
    let uri = req.uri().clone();

    info!("[handle_request_with_router]处理请求: {} {}", method, path);

    // 记录请求头中的关键信息
    if let Some(content_type) = req.headers().get("content-type")
        && let Ok(content_type_str) = content_type.to_str()
    {
        debug!(
            "[handle_request_with_router] Content-Type: {}",
            content_type_str
        );
    }

    if let Some(content_length) = req.headers().get("content-length")
        && let Ok(content_length_str) = content_length.to_str()
    {
        debug!(
            "[handle_request_with_router] Content-Length: {}",
            content_length_str
        );
    }

    // 记录 x-mcp-json 头信息（如果存在）
    if let Some(mcp_json) = req.headers().get("x-mcp-json")
        && let Ok(mcp_json_str) = mcp_json.to_str()
    {
        debug!(
            "[handle_request_with_router] MCP-JSON Header: {}",
            mcp_json_str
        );
    }

    // 记录查询参数
    if let Some(query) = uri.query() {
        debug!("[handle_request_with_router] Query: {}", query);
    }

    // 使用 debug_span 减少日志量，因为 DynamicRouterService 已经记录了请求信息
    // 移除 #[tracing::instrument] 避免 span 嵌套导致的日志膨胀问题
    let span = tracing::debug_span!(
        "handle_request_with_router",
        otel.name = "Handle Request with Router",
        component = "router",
        trace_id = %trace_id,
        http.method = %method,
        http.path = %path,
    );

    let _guard = span.enter();

    let mut service = router_entry.into_service();
    match service.call(req).await {
        Ok(response) => {
            let status = response.status();

            // 记录响应头信息
            debug!(
                "[handle_request_with_router]响应状态: {}, 响应头: {response:?}",
                status
            );

            span.record("http.response.status_code", status.as_u16());
            Ok(response)
        }
        Err(error) => {
            span.record("error.router_service_error", true);
            span.record("error.message", format!("{:?}", error));
            error!("[handle_request_with_router]错误: {error:?}");
            Ok(axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// 启动MCP服务并处理请求
async fn start_mcp_and_handle_request(
    req: Request<Body>,
    mcp_config: McpConfig,
) -> Result<Response, Infallible> {
    let request_path = req.uri().path().to_string();
    let trace_id = extract_trace_id();
    debug!("请求路径: {request_path}");

    // 使用 debug_span 减少日志量，移除 #[tracing::instrument] 避免 span 嵌套
    let span = tracing::debug_span!(
        "start_mcp_and_handle_request",
        otel.name = "Start MCP and Handle Request",
        component = "mcp_startup",
        mcp.id = %mcp_config.mcp_id,
        mcp.type = ?mcp_config.mcp_type,
        mcp.config.has_config = mcp_config.mcp_json_config.is_some(),
        trace_id = %trace_id,
    );

    let _guard = span.enter();

    let ret = mcp_start_task(mcp_config).await;

    if let Ok((router, _)) = ret {
        span.record("mcp.startup.success", true);
        handle_request_with_router(req, router, &request_path).await
    } else {
        span.record("mcp.startup.failed", true);
        span.record("error.mcp_startup_failed", true);
        span.record("error.message", format!("{:?}", ret));
        warn!("MCP服务启动失败: {ret:?}");
        Ok(axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}
