mod auth;
mod http_logging;
mod mcp_router_json;
mod mcp_update_latest_layer;
mod opentelemetry_middleware;
mod server_time;

use crate::model::AppState;
use axum::Router;
use axum::middleware::from_fn;
use http_logging::http_logging_middleware;
use mcp_router_json::mcp_json_config_extract;
use opentelemetry_middleware::opentelemetry_tracing_middleware;
use server_time::ServerTimeLayer;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;

pub use mcp_update_latest_layer::MySseRouterLayer;
pub use opentelemetry_middleware::extract_trace_id;

// pub use auth::{extract_user, verify_token};

// pub trait TokenVerify {
//     type Error: fmt::Debug;
//     fn verify(&self, token: &str) -> Result<User, Self::Error>;
// }

const REQUEST_ID_HEADER: &str = "x-request-id";
const SERVER_TIME_HEADER: &str = "x-server-time";

pub fn set_layer(app: Router, state: AppState) -> Router {
    app.layer(
        ServiceBuilder::new()
            // HTTP 请求/响应日志中间件 (TRACE level, 用于调试频繁的 MCP API 调用)
            .layer(from_fn(http_logging_middleware))
            // OpenTelemetry 追踪中间件 - 自动生成 trace_id 和 span
            .layer(from_fn(opentelemetry_tracing_middleware))
            // MCP 配置提取中间件
            .layer(from_fn(mcp_json_config_extract))
            // HTTP 压缩
            .layer(CompressionLayer::new().gzip(true).br(true).deflate(true))
            // 服务器时间响应头
            .layer(ServerTimeLayer)
            // SSE 路由层
            .layer(MySseRouterLayer::new(state.clone())),
    )
}
