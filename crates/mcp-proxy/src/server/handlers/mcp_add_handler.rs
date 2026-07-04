use axum::{Json, extract::State, response::IntoResponse};
use http::Uri;
use log::{debug, error};
use tracing::instrument;
use uuid::Uuid;

use crate::AppError;
use crate::model::{
    AddRouteParams, HttpResult, McpConfig, McpProtocolPath, McpServerConfig, McpType,
};
use crate::model::{AppState, McpRouterPath};
use crate::server::task::integrate_server_with_axum;
use serde_json::json;

// 修改 add_route_handler 函数，使用新的集成方法
#[instrument]
// #[axum::debug_handler]
pub async fn add_route_handler(
    State(state): State<AppState>,
    uri: Uri,
    Json(params): Json<AddRouteParams>,
) -> Result<impl IntoResponse, AppError> {
    // 获取请求路径
    let request_path = uri.path().to_string();
    debug!("Request path: {}", request_path);
    debug!("Full URI: {:?}", uri);

    let mcp_protocol = McpRouterPath::from_uri_prefix_protocol(&request_path);
    if let Some(mcp_protocol) = mcp_protocol {
        // 生成mcp_id, 使用uuid,去掉-
        let mcp_id = Uuid::now_v7().to_string().replace("-", "");

        //根据 mcp_id 和协议,生成 mcp_router_path
        let mcp_router_path = McpRouterPath::new(mcp_id, mcp_protocol)
            .map_err(|e| AppError::mcp_server_error(e.to_string()))?;

        let mcp_plugin_json = params.mcp_json_config;
        // 将mcp_plugin_json转换为 McpServerConfig 结构体
        let mcp_server_config =
            McpServerConfig::try_from(mcp_plugin_json.clone()).expect("解析 MCP 配置失败");

        let mcp_type = params.mcp_type.unwrap_or(McpType::default());

        debug!("Client protocol: {:?}", mcp_router_path.mcp_protocol);

        // 构建完整的 McpConfig（用于自动重启）
        let full_mcp_config = McpConfig {
            mcp_id: mcp_router_path.mcp_id.clone(),
            client_protocol: mcp_router_path.mcp_protocol.clone(),
            mcp_type: mcp_type.clone(),
            mcp_json_config: Some(mcp_plugin_json),
            server_config: Some(mcp_server_config.clone()),
        };

        // 使用新的集成方法，后端协议在函数内部解析
        let _ = integrate_server_with_axum(
            mcp_server_config.clone(),
            mcp_router_path.clone(),
            full_mcp_config,
        )
        .await
        .map_err(|e| {
            error!("Failed to start MCP service: {e}");
            AppError::mcp_server_error(e.to_string())
        })?;

        //区分 mcp协议
        let mcp_protocol = mcp_router_path.mcp_protocol_path.clone();

        let data = match mcp_protocol {
            McpProtocolPath::SsePath(sse_path) => {
                // 返回 mcp_id 和 sse_path
                let data = json!({
                    "mcp_id": mcp_router_path.mcp_id,
                    "sse_path": sse_path.sse_path,
                    "message_path": sse_path.message_path,
                    "mcp_type": mcp_type
                });
                data
            }

            McpProtocolPath::StreamPath(stream_path) => {
                // 返回 mcp_id 和 stream_path
                let data = json!({
                    "mcp_id": mcp_router_path.mcp_id,
                    "stream_path": stream_path.stream_path,
                    "mcp_type": mcp_type
                });
                data
            }
        };

        Ok(HttpResult::success(data, None))
    } else {
        //返回 bad request,400 错误
        return Err(AppError::invalid_parameter("无效的请求路径"));
    }
}
