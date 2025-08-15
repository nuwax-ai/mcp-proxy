use axum::{extract::Path, response::IntoResponse};

use crate::{get_proxy_manager, model::HttpResult};
use serde_json::json;

// #[axum::debug_handler]
pub async fn delete_route_handler(Path(mcp_id): Path<String>) -> impl IntoResponse {
    // 删除动态路由,以及清理资源
    get_proxy_manager().cleanup_resources(&mcp_id).await;

    // 返回成功信息
    let data = json!({
        "mcp_id": mcp_id,
        "message": format!("已删除路由: {}", mcp_id)
    });

    HttpResult::success(data, None)
}
