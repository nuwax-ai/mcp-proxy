use axum::{extract::Path, response::IntoResponse};

use crate::{AppError, get_proxy_manager, model::HttpResult};
use anyhow::Result;
use serde_json::json;

// #[axum::debug_handler]
pub async fn delete_route_handler(
    Path(mcp_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // 删除动态路由,以及清理资源
    // 使用强制清理，因为用户显式请求删除
    get_proxy_manager()
        .cleanup_resources_force(&mcp_id)
        .await
        .map_err(AppError::McpServerError)?;

    // 返回成功信息
    let data = json!({
        "mcp_id": mcp_id,
        "message": format!("已删除路由: {}", mcp_id)
    });

    Ok(HttpResult::success(data, None))
}
