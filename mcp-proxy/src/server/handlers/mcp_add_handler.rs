use axum::{Json, extract::State, response::IntoResponse};
use http::Uri;
use log::{debug, error};
use tracing::instrument;
use uuid::Uuid;

use crate::model::{AddRouteParams, HttpResult, McpProtocolPath, McpServerCommandConfig, McpType};
use crate::model::{AppState, McpRouterPath};
use crate::server::task::integrate_sse_server_with_axum;
use serde_json::json;

// 修改 add_route_handler 函数，使用新的集成方法
#[instrument]
// #[axum::debug_handler]
pub async fn add_route_handler(
    State(state): State<AppState>,
    uri: Uri,
    Json(params): Json<AddRouteParams>,
) -> impl IntoResponse {
    // 获取请求路径
    let request_path = uri.path().to_string();
    debug!("请求路径: {}", request_path);
    debug!("完整URI: {:?}", uri);

    let mcp_protocol = McpRouterPath::from_uri_prefix_protocol(&request_path);
    if let Some(mcp_protocol) = mcp_protocol {
        let mcp_protocol = mcp_protocol;

        // 生成mcp_id, 使用uuid,去掉-
        let mcp_id = Uuid::now_v7().to_string().replace("-", "");

        //根据 mcp_id 和协议,生成 mcp_router_path
        let mcp_router_path = McpRouterPath::new(mcp_id, mcp_protocol);

        let mcp_plugin_json = params.mcp_json_config;
        // 将mcp_plugin_json转换为 McpJsonServerParameters 结构体
        let mcp_server_config =
            McpServerCommandConfig::try_from(mcp_plugin_json).expect("解析 MCP 配置失败");

        let mcp_type = params.mcp_type.unwrap_or(McpType::default());

        // 使用新的集成方法，而不是单独启动 SSE 服务
        match integrate_sse_server_with_axum(
            mcp_server_config.clone(),
            mcp_router_path.clone(),
            mcp_type.clone(),
        )
        .await
        {
            Ok((_, _)) => {
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

                HttpResult::success(data, None)
            }
            Err(e) => {
                error!("启动 MCP 服务失败: {e}");
                let message = format!("启动 MCP 服务失败: {e}");
                HttpResult::error("0002", &message, None)
            }
        }
    } else {
        //返回 bad request,400 错误
        return HttpResult::error("400", "无效的请求路径", None);
    }
}
