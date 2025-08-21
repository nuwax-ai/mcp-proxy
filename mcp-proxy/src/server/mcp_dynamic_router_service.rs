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
use log::{debug, warn};
use tower::Service;

use crate::{
    DynamicRouterService, mcp_start_task,
    model::{HttpResult, McpConfig, McpRouterPath},
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
        debug!("请求路径: {path}");

        // 解析路由路径
        let mcp_router_path = McpRouterPath::from_url(&path);

        match mcp_router_path {
            Some(mcp_router_path) => {
                let mcp_id = mcp_router_path.mcp_id.clone();
                debug!("请求访问MCP ID: {mcp_id}");
                let base_path = mcp_router_path.base_path.clone();

                Box::pin(async move {
                    // 先尝试查找已注册的路由
                    if let Some(router_entry) = DynamicRouterService::get_route(&base_path) {
                        return handle_request_with_router(req, router_entry).await;
                    }

                    // 未找到路由，尝试启动服务
                    warn!(
                        "未找到匹配的路径,尝试启动服务:base_path={base_path},path={path}"
                    );

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
                    let message = format!(
                        "未找到匹配的路径,且未获取到header[x-mcp-json]配置,无法启动MCP服务: {path}"
                    );
                    let http_result: HttpResult<String> = HttpResult::error("0001", &message, None);
                    Ok(http_result.into_response())
                })
            }
            None => {
                warn!("请求路径解析失败: {path}");
                let message = format!("请求路径解析失败: {path}");
                let http_result: HttpResult<String> = HttpResult::error("0001", &message, None);
                Box::pin(async move { Ok(http_result.into_response()) })
            }
        }
    }
}

/// 使用给定的路由处理请求
async fn handle_request_with_router(
    req: Request<Body>,
    router_entry: axum::Router,
) -> Result<Response, Infallible> {
    // 获取匹配路径的Router，并处理请求
    debug!("[handle_request_with_router]处理请求: {req:?}");
    let mut service = router_entry.into_service();
    match service.call(req).await {
        Ok(response) => {
            debug!("[handle_request_with_router]响应: {response:?}");
            Ok(response)
        }
        Err(error) => {
            debug!("[handle_request_with_router]错误: {error:?}");
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
    debug!("请求路径: {request_path}");

    let ret = mcp_start_task(mcp_config).await;

    if let Ok((router, _)) = ret {
        handle_request_with_router(req, router).await
    } else {
        warn!("MCP服务启动失败: {ret:?}");
        Ok(axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}
