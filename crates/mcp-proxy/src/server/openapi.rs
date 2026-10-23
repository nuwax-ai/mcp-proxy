//! mcp-proxy 的 OpenAPI 文档声明与文档 UI 路由。
//!
//! [`ApiDoc`] 为 utoipa 声明（paths / components / tags）；文档 UI 双风格并存：
//! - Swagger UI：`/api/docs`（spec 挂 `/api/docs/openapi.json`，资产编译期内嵌）
//! - Scalar：`/api/docs/scalar`（spec 内嵌 HTML；UI JS 由浏览器从公网 CDN 加载，
//!   内网浏览器不可出网时白屏，用 Swagger UI 不受影响）
//!
//! 两条动态代理路由 `/mcp/{sse|stream}/proxy/{*path}` 是运行时按 mcp_id 动态注册的
//! 透传路径（`DynamicRouterService`），路径空间无法静态枚举，只在 `info.description`
//! 中文字说明，不做 path 声明。
//!
//! paths 条目使用完整模块路径（同 fastembed 惯例），utoipa 经此路径解析各 handler
//! 模块内生成的 `__path_*` 结构体。

use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};
use utoipa_swagger_ui::SwaggerUi;

use crate::model::HttpResult;
use crate::model::{
    AddRouteParams, CheckMcpStatusRequestParams, CheckMcpStatusResponseParams, McpProtocol,
    McpStatusResponseEnum, McpType,
};
use crate::server::handlers::run_code_handler::RunCodeMessageRequest;
use run_code_rmcp::RunCodeHttpResult;

/// OpenAPI 文档结构
#[derive(OpenApi)]
#[openapi(
    info(
        title = "MCP Proxy API",
        version = env!("CARGO_PKG_VERSION"),
        description = r#"
MCP（Model Context Protocol）透明代理服务：动态注册 MCP 后端、协议透传与状态管理。

## 动态代理路由（不在此文档的 path 清单中）

以下两条是运行时按 mcp_id 动态注册的透传路由，路径空间由注册结果决定，故仅文字说明：

- `GET /mcp/sse/proxy/{mcp_id}/sse` + `POST /mcp/sse/proxy/{mcp_id}/message` —— SSE 协议透传
  （客户端以 SSE 接入，后端协议由 add 时的配置决定，支持协议探测与转换）
- `ANY /mcp/stream/proxy/{mcp_id}/{path}` —— Streamable HTTP 协议透传

接入路径在调用 `POST /mcp/sse/add` 注册成功后由 data 中的 `sse_path` / `message_path` /
`stream_path` 返回。

## 典型流程

1. `POST /mcp/sse/add` 提交 MCP 后端 JSON 配置，获得 mcp_id 与接入路径
2. `POST /mcp/{sse|stream}/check_status` 按 mcp_id + 配置懒启动并轮询（PENDING → READY / ERROR）
3. 客户端经接入路径以对应协议直连透传
4. `DELETE /mcp/config/delete/{mcp_id}` 清理服务与资源
"#,
    ),
    servers(
        (url = "http://localhost:8085", description = "本地开发环境（默认端口，可经 MCP_PROXY_PORT 覆盖）"),
    ),
    paths(
        crate::server::handlers::health::get_health,
        crate::server::handlers::health::get_ready,
        crate::server::handlers::mcp_add_handler::add_route_handler,
        crate::server::handlers::delete_route_handler::delete_route_handler,
        crate::server::handlers::check_mcp_is_status::check_mcp_is_status_handler,
        crate::server::handlers::mcp_check_status_handler::check_mcp_status_handler_sse,
        crate::server::handlers::mcp_check_status_handler::check_mcp_status_handler_stream,
        crate::server::handlers::run_code_handler::run_code_handler,
    ),
    components(
        schemas(
            AddRouteParams,
            McpType,
            McpProtocol,
            CheckMcpStatusRequestParams,
            CheckMcpStatusResponseParams,
            McpStatusResponseEnum,
            HttpResult<CheckMcpStatusResponseParams>,
            RunCodeMessageRequest,
            RunCodeHttpResult,
        )
    ),
    tags(
        (name = "system", description = "系统健康检查与就绪探针"),
        (name = "mcp-sse", description = "SSE 协议域：MCP 服务注册与懒启动状态检查"),
        (name = "mcp-stream", description = "Streamable HTTP 协议域：MCP 服务懒启动状态检查"),
        (name = "mcp-config", description = "MCP 服务生命周期管理：删除与运行状态查询"),
        (name = "code-run", description = "代码执行（JS/TS/Python，uv/deno 沙箱）"),
    )
)]
pub struct ApiDoc;

/// 文档 UI 路由（Swagger UI + Scalar 双风格，共用同一份文档）。
/// 返回 `Router<AppState>`，由 `router_layer::get_router` 在 `with_state` 之前 merge。
pub fn create_docs_router() -> axum::Router<crate::model::AppState> {
    axum::Router::new()
        .merge(SwaggerUi::new("/api/docs").url("/api/docs/openapi.json", ApiDoc::openapi()))
        .merge(Scalar::with_url("/api/docs/scalar", ApiDoc::openapi()))
}
