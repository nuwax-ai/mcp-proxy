use axum::{
    Router,
    routing::{delete, get, post},
};
use http::Method;
use tower_http::cors::{self, CorsLayer};

use crate::{
    AppError, AppState, DynamicRouterService,
    model::{GLOBAL_SSE_MCP_ROUTES_PREFIX, GLOBAL_STREAM_MCP_ROUTES_PREFIX, McpProtocol},
    server::handlers::check_mcp_is_status_handler,
};

use super::{
    get_health, get_ready,
    handlers::{
        add_route_handler, check_mcp_status_handler_sse, check_mcp_status_handler_stream,
        delete_route_handler, run_code_handler,
    },
    set_layer,
};

/// 获取路由
pub async fn get_router(state: AppState) -> Result<Router, AppError> {
    let health = Router::new()
        .route("/health", get(get_health))
        .route("/ready", get(get_ready));

    let cors = CorsLayer::new()
        // allow `GET` and `POST` when accessing the resource
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::PUT,
        ])
        .allow_origin(cors::Any)
        .allow_headers(cors::Any);

    let api = Router::new()
        // .layer(from_fn_with_state(state.clone(), verify_token::<AppState>))
        //mcp sse 协议路由
        .route_service(
            &format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/proxy/{{*path}}"),
            DynamicRouterService(McpProtocol::Sse),
        )
        .route(
            &format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/add"),
            post(add_route_handler),
        )
        .route("/mcp/config/delete/{mcp_id}", delete(delete_route_handler))
        .route(
            "/mcp/check/status/{mcp_id}",
            get(check_mcp_is_status_handler),
        )
        .route(
            &format!("{GLOBAL_SSE_MCP_ROUTES_PREFIX}/check_status"),
            post(check_mcp_status_handler_sse),
        )
        //mcp stream 协议路由
        .route_service(
            &format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/proxy/{{*path}}"),
            DynamicRouterService(McpProtocol::Stream),
        )
        .route(
            &format!("{GLOBAL_STREAM_MCP_ROUTES_PREFIX}/check_status"),
            post(check_mcp_status_handler_stream),
        )
        .route("/api/run_code_with_log", post(run_code_handler))
        .layer(cors);

    // 创建基本路由
    let app: Router<AppState> = Router::new().merge(health).merge(api);

    // 添加状态
    let app = app.with_state(state.clone());
    let router = set_layer(app, state);
    Ok(router)
}
