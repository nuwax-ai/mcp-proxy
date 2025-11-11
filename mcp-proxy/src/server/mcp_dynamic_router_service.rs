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
    DynamicRouterService, mcp_start_task,
    model::{HttpResult, McpConfig, McpRouterPath},
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

        // 创建根 span
        let span = tracing::info_span!(
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
                        debug!("=== 路由查找结束(成功) ===");
                        return handle_request_with_router(req, router_entry).await;
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

                    // 先检查MCP服务是否存在
                    let proxy_manager = crate::model::get_proxy_manager();
                    if proxy_manager.get_mcp_service_status(&mcp_id).is_none() {
                        // MCP服务不存在
                        warn!("MCP服务不存在: {}", mcp_id);
                        span.record("error.mcp_service_not_found", true);
                        return Ok((
                            axum::http::StatusCode::NOT_FOUND,
                            [("Content-Type", "text/plain")],
                            format!("MCP service '{}' not found", mcp_id),
                        )
                            .into_response());
                    }

                    // 从请求扩展中获取MCP配置
                    if let Some(mcp_config) = req.extensions().get::<McpConfig>().cloned() {
                        //mcp_config.mcp_json_config 非空判断
                        if mcp_config.mcp_json_config.is_some() {
                            return start_mcp_and_handle_request(req, mcp_config).await;
                        }
                    }

                    // 没有配置，无法启动服务
                    warn!(
                        "未找到匹配的路径,且未获取到header[x-mcp-json]配置,无法启动MCP服务: {path}"
                    );
                    span.record("error.mcp_config_missing", true);

                    let message = format!(
                        "未找到匹配的路径,且未获取到header[x-mcp-json]配置,无法启动MCP服务: {path}"
                    );
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
#[tracing::instrument(skip(req, router_entry), fields(
    http.method = %req.method(),
    http.uri = %req.uri(),
))]
async fn handle_request_with_router(
    req: Request<Body>,
    router_entry: axum::Router,
) -> Result<Response, Infallible> {
    // 获取匹配路径的Router，并处理请求
    let trace_id = extract_trace_id();

    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path();

    info!("[handle_request_with_router]处理请求: {} {}", method, path);

    // 记录请求头中的关键信息
    if let Some(content_type) = req.headers().get("content-type") {
        if let Ok(content_type_str) = content_type.to_str() {
            info!(
                "[handle_request_with_router] Content-Type: {}",
                content_type_str
            );
        }
    }

    if let Some(content_length) = req.headers().get("content-length") {
        if let Ok(content_length_str) = content_length.to_str() {
            info!(
                "[handle_request_with_router] Content-Length: {}",
                content_length_str
            );
        }
    }

    // 记录 x-mcp-json 头信息（如果存在）
    if let Some(mcp_json) = req.headers().get("x-mcp-json") {
        if let Ok(mcp_json_str) = mcp_json.to_str() {
            info!(
                "[handle_request_with_router] MCP-JSON Header: {}",
                mcp_json_str
            );
        }
    }

    // 记录查询参数
    if let Some(query) = uri.query() {
        info!("[handle_request_with_router] Query: {}", query);
    }

    let span = tracing::info_span!(
        "handle_request_with_router",
        otel.name = "Handle Request with Router",
        component = "router",
        trace_id = %trace_id,
    );

    let _guard = span.enter();

    let mut service = router_entry.into_service();
    match service.call(req).await {
        Ok(response) => {
            let status = response.status();
            span.record("http.response.status_code", status.as_u16());

            // 记录响应头信息
            info!(
                "[handle_request_with_router]响应状态: {}, 响应头: {response:?}",
                status
            );

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
#[tracing::instrument(skip(req, mcp_config), fields(
    mcp_id = %mcp_config.mcp_id,
    mcp_type = ?mcp_config.mcp_type,
))]
async fn start_mcp_and_handle_request(
    req: Request<Body>,
    mcp_config: McpConfig,
) -> Result<Response, Infallible> {
    let request_path = req.uri().path().to_string();
    let trace_id = extract_trace_id();
    debug!("请求路径: {request_path}");

    let span = tracing::info_span!(
        "start_mcp_and_handle_request",
        otel.name = "Start MCP and Handle Request",
        component = "mcp_startup",
        mcp.config.has_config = mcp_config.mcp_json_config.is_some(),
        trace_id = %trace_id,
    );

    let _guard = span.enter();

    let ret = mcp_start_task(mcp_config).await;

    if let Ok((router, _)) = ret {
        span.record("mcp.startup.success", true);
        handle_request_with_router(req, router).await
    } else {
        span.record("mcp.startup.failed", true);
        span.record("error.mcp_startup_failed", true);
        span.record("error.message", format!("{:?}", ret));
        warn!("MCP服务启动失败: {ret:?}");
        Ok(axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}
