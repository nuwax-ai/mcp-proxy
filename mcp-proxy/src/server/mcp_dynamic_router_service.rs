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
    model::{
        CheckMcpStatusResponseStatus, GLOBAL_RESTART_TRACKER, HttpResult, McpConfig, McpRouterPath,
        McpType,
    },
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
                        let mcp_id_for_check = McpRouterPath::from_url(&path);
                        if let Some(router_path) = mcp_id_for_check {
                            let proxy_manager = get_proxy_manager();

                            // ===== 首先检查服务状态 =====
                            // 如果服务状态是 Pending，说明服务正在初始化中（uvx/npx 下载中）
                            // 此时不应该做健康检查，应该等待
                            if let Some(service_status) =
                                proxy_manager.get_mcp_service_status(&router_path.mcp_id)
                            {
                                match &service_status.check_mcp_status_response_status {
                                    CheckMcpStatusResponseStatus::Pending => {
                                        debug!(
                                            "[MCP状态检查] mcp_id={} 状态为 Pending，服务正在初始化中，返回 503",
                                            router_path.mcp_id
                                        );
                                        let message = format!(
                                            "服务 {} 正在初始化中，请稍后再试",
                                            router_path.mcp_id
                                        );
                                        let http_result: HttpResult<String> =
                                            HttpResult::error("0003", &message, None);
                                        return Ok(http_result.into_response());
                                    }
                                    CheckMcpStatusResponseStatus::Error(err) => {
                                        // Error 状态：只清理，不重启
                                        // 避免有问题的 MCP 服务无限重启循环
                                        warn!(
                                            "[MCP状态检查] mcp_id={} 状态为 Error: {}，清理资源并返回错误",
                                            router_path.mcp_id, err
                                        );
                                        // 清理资源
                                        if let Err(e) = proxy_manager
                                            .cleanup_resources(&router_path.mcp_id)
                                            .await
                                        {
                                            error!(
                                                "[MCP状态检查] mcp_id={} 清理资源失败: {}",
                                                router_path.mcp_id, e
                                            );
                                        }
                                        // 返回错误，不尝试重启
                                        let message = format!(
                                            "服务 {} 启动失败: {}",
                                            router_path.mcp_id, err
                                        );
                                        let http_result: HttpResult<String> =
                                            HttpResult::error("0005", &message, None);
                                        return Ok(http_result.into_response());
                                    }
                                    CheckMcpStatusResponseStatus::Ready => {
                                        debug!(
                                            "[MCP状态检查] mcp_id={} 状态为 Ready，继续检查后端健康状态",
                                            router_path.mcp_id
                                        );
                                    }
                                }
                            }

                            // ===== 检查启动锁状态 =====
                            // 如果锁被占用，说明服务正在启动中
                            let startup_guard = GLOBAL_RESTART_TRACKER
                                .try_acquire_startup_lock(&router_path.mcp_id);

                            if startup_guard.is_none() {
                                // 锁被占用，服务正在启动中，返回 503
                                debug!(
                                    "[启动锁检查] mcp_id={} 启动锁被占用，服务正在启动中，返回 503",
                                    router_path.mcp_id
                                );
                                span.record("mcp.startup_in_progress", true);
                                let message =
                                    format!("服务 {} 正在启动中，请稍后再试", router_path.mcp_id);
                                let http_result: HttpResult<String> =
                                    HttpResult::error("0003", &message, None);
                                span.record("http.response.status_code", 503u16);
                                return Ok(http_result.into_response());
                            }

                            // 获取到锁，现在可以安全地检查健康状态
                            let _startup_guard = startup_guard.unwrap();
                            debug!(
                                "[启动锁检查] mcp_id={} 成功获取启动锁，开始健康检查",
                                router_path.mcp_id
                            );

                            if let Some(handler) =
                                proxy_manager.get_proxy_handler(&router_path.mcp_id)
                            {
                                // ===== 健康检查（带缓存）=====
                                let is_healthy = if let Some(cached) = GLOBAL_RESTART_TRACKER
                                    .get_cached_health_status(&router_path.mcp_id)
                                {
                                    debug!(
                                        "[健康检查] mcp_id={} 使用缓存状态: is_healthy={}",
                                        router_path.mcp_id, cached
                                    );
                                    cached
                                } else {
                                    debug!(
                                        "[健康检查] mcp_id={} 缓存未命中，开始实际健康检查...",
                                        router_path.mcp_id
                                    );
                                    let status = handler.is_mcp_server_ready().await;
                                    GLOBAL_RESTART_TRACKER
                                        .update_health_status(&router_path.mcp_id, status);
                                    debug!(
                                        "[健康检查] mcp_id={} 实际健康检查结果: is_healthy={}",
                                        router_path.mcp_id, status
                                    );
                                    status
                                };

                                if is_healthy {
                                    debug!(
                                        "[健康检查] mcp_id={} 后端服务正常，释放锁并使用路由",
                                        router_path.mcp_id
                                    );
                                    // 释放锁，使用路由
                                    drop(_startup_guard);
                                    debug!("=== 路由查找结束(成功) ===");
                                    return handle_request_with_router(req, router_entry, &path)
                                        .await;
                                }

                                // 不健康，获取服务类型以决定是否重启
                                let mcp_type = proxy_manager
                                    .get_mcp_service_status(&router_path.mcp_id)
                                    .map(|s| s.mcp_type.clone());

                                // 清理资源
                                warn!(
                                    "[健康检查] mcp_id={} 后端服务不健康，清理资源",
                                    router_path.mcp_id
                                );
                                if let Err(e) =
                                    proxy_manager.cleanup_resources(&router_path.mcp_id).await
                                {
                                    error!(
                                        "[清理资源] mcp_id={} 清理资源失败: error={}",
                                        router_path.mcp_id, e
                                    );
                                } else {
                                    debug!("[清理资源] mcp_id={} 清理资源成功", router_path.mcp_id);
                                }

                                // OneShot 类型：只清理，不重启
                                // OneShot 服务执行完成后进程会退出，这是正常行为，不应该自动重启
                                // 用户需要通过 check_status 接口显式启动新的 OneShot 服务
                                if matches!(mcp_type, Some(McpType::OneShot)) {
                                    debug!(
                                        "[健康检查] mcp_id={} 是 OneShot 类型，不自动重启，返回服务已结束",
                                        router_path.mcp_id
                                    );
                                    let message = format!(
                                        "OneShot 服务 {} 已结束，请重新启动",
                                        router_path.mcp_id
                                    );
                                    let http_result: HttpResult<String> =
                                        HttpResult::error("0006", &message, None);
                                    return Ok(http_result.into_response());
                                }

                                // Persistent 类型：清理后重启
                                info!(
                                    "[重启流程] mcp_id={} 是 Persistent 类型，开始重启服务",
                                    router_path.mcp_id
                                );

                                // 从配置获取 mcp_config 并启动服务
                                // 优先从请求 header 获取配置
                                if let Some(mcp_config) =
                                    req.extensions().get::<McpConfig>().cloned()
                                    && mcp_config.mcp_json_config.is_some()
                                {
                                    info!(
                                        "[重启流程] mcp_id={} 使用请求 header 配置重启服务",
                                        mcp_config.mcp_id
                                    );
                                    proxy_manager
                                        .register_mcp_config(&mcp_config.mcp_id, mcp_config.clone())
                                        .await;
                                    return start_mcp_and_handle_request(req, mcp_config).await;
                                }

                                // 从缓存获取配置
                                if let Some(mcp_config) = proxy_manager
                                    .get_mcp_config_from_cache(&router_path.mcp_id)
                                    .await
                                {
                                    info!(
                                        "[重启流程] mcp_id={} 使用缓存配置重启服务",
                                        router_path.mcp_id
                                    );
                                    return start_mcp_and_handle_request(req, mcp_config).await;
                                }

                                // 无法获取配置
                                warn!(
                                    "[重启流程] mcp_id={} 无法获取配置，无法重启服务",
                                    router_path.mcp_id
                                );
                                let message =
                                    format!("服务 {} 不健康且无法获取配置", router_path.mcp_id);
                                let http_result: HttpResult<String> =
                                    HttpResult::error("0004", &message, None);
                                return Ok(http_result.into_response());
                            } else {
                                // handler 不存在，但路由存在
                                // 检查服务类型，OneShot 不自动重启
                                let mcp_type = proxy_manager
                                    .get_mcp_service_status(&router_path.mcp_id)
                                    .map(|s| s.mcp_type.clone());

                                if matches!(mcp_type, Some(McpType::OneShot)) {
                                    debug!(
                                        "[服务检查] mcp_id={} 是 OneShot 类型且 handler 不存在，不自动重启",
                                        router_path.mcp_id
                                    );
                                    // 清理残留状态
                                    if let Err(e) =
                                        proxy_manager.cleanup_resources(&router_path.mcp_id).await
                                    {
                                        error!(
                                            "[清理资源] mcp_id={} 清理资源失败: {}",
                                            router_path.mcp_id, e
                                        );
                                    }
                                    let message = format!(
                                        "OneShot 服务 {} 已结束，请重新启动",
                                        router_path.mcp_id
                                    );
                                    let http_result: HttpResult<String> =
                                        HttpResult::error("0006", &message, None);
                                    return Ok(http_result.into_response());
                                }

                                // Persistent 类型：继续进入启动流程
                                warn!(
                                    "路由存在但 handler 不存在，进入重启流程: base_path={}",
                                    base_path
                                );
                            }
                        } else {
                            // 无法解析路由路径，直接使用路由
                            debug!("=== 路由查找结束(成功) ===");
                            return handle_request_with_router(req, router_entry, &path).await;
                        }
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
                        let _startup_guard = match GLOBAL_RESTART_TRACKER
                            .try_acquire_startup_lock(&mcp_config.mcp_id)
                        {
                            Some(guard) => guard,
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

                        info!("使用请求 header 配置启动服务: {}", mcp_config.mcp_id);
                        // 同时更新缓存
                        proxy_manager
                            .register_mcp_config(&mcp_config.mcp_id, mcp_config.clone())
                            .await;

                        // _startup_guard 会在作用域结束时自动释放
                        return start_mcp_and_handle_request(req, mcp_config).await;
                    }

                    // 优先级 2: 从 moka 缓存中获取配置（兜底）
                    // 注意：OneShot 类型不从缓存自动启动，需要用户显式请求（带 header 配置）
                    if let Some(mcp_id_for_cache) =
                        mcp_router_path_for_config.as_ref().map(|p| &p.mcp_id)
                        && let Some(mcp_config) = proxy_manager
                            .get_mcp_config_from_cache(mcp_id_for_cache)
                            .await
                    {
                        // OneShot 类型不从缓存自动启动
                        // 避免已回收的 OneShot 服务被意外重启
                        if matches!(mcp_config.mcp_type, McpType::OneShot) {
                            info!(
                                "[启动检查] mcp_id={} 是 OneShot 类型，不从缓存自动启动，需要用户显式请求",
                                mcp_id_for_cache
                            );
                            let message = format!(
                                "OneShot 服务 {} 需要通过 check_status 接口启动",
                                mcp_id_for_cache
                            );
                            let http_result: HttpResult<String> =
                                HttpResult::error("0007", &message, None);
                            return Ok(http_result.into_response());
                        }

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
                        let _startup_guard = match GLOBAL_RESTART_TRACKER
                            .try_acquire_startup_lock(mcp_id_for_cache)
                        {
                            Some(guard) => guard,
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

                        info!("使用缓存配置启动服务: {}", mcp_id_for_cache);
                        // _startup_guard 会在作用域结束时自动释放
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
