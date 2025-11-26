use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::server::AppState;

/// 健康检查响应
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// 服务状态
    #[schema(example = "ok")]
    pub status: String,
    
    /// 服务运行时长（毫秒）
    #[schema(example = 123456)]
    pub uptime_ms: u128,
    
    /// 模型缓存是否就绪
    #[schema(example = true)]
    pub model_cache_ready: bool,
}

/// 健康检查处理器
#[utoipa::path(
    get,
    path = "/health",
    tag = "健康检查",
    responses(
        (status = 200, description = "服务健康", body = HealthResponse)
    )
)]
pub async fn handle_health(
    State(state): State<Arc<AppState>>,
) -> Json<HealthResponse> {
    let uptime = state.start_time.elapsed();
    
    Json(HealthResponse {
        status: "ok".to_string(),
        uptime_ms: uptime.as_millis(),
        model_cache_ready: *state.model_cache_ready.lock().unwrap(),
    })
}
