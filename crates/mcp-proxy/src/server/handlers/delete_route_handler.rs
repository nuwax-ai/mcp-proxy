use axum::{extract::Path, response::IntoResponse};

use crate::{AppError, get_proxy_manager, model::HttpResult};
use anyhow::Result;
use serde_json::json;

// #[axum::debug_handler]
#[utoipa::path(
    delete,
    path = "/mcp/config/delete/{mcp_id}",
    tag = "mcp-config",
    params(
        ("mcp_id" = String, Path, description = "注册 MCP 服务时返回的服务唯一标识")
    ),
    responses(
        (status = 200, description = "删除成功，data 内含被删除的 mcp_id",
            body = Object,
            example = json!({
                "code": "0000",
                "message": "成功",
                "data": {
                    "mcp_id": "018f6a2b3c4d5e6f7a8b9c0d1e2f3a4b",
                    "message": "已删除路由: 018f6a2b3c4d5e6f7a8b9c0d1e2f3a4b"
                },
                "tid": null,
                "success": true
            }))
    )
)]
pub async fn delete_route_handler(
    Path(mcp_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // 删除动态路由,以及清理资源
    get_proxy_manager()
        .cleanup_resources(&mcp_id)
        .await
        .map_err(|e| AppError::mcp_server_error(e.to_string()))?;

    // 返回成功信息
    let data = json!({
        "mcp_id": mcp_id,
        "message": format!("已删除路由: {}", mcp_id)
    });

    Ok(HttpResult::success(data, None))
}
