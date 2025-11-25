use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::server::AppState;

/// 健康检查响应
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub uptime_ms: u128,
    pub model_cache_ready: bool,
}

/// 健康检查处理器
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
