use axum::Json;
use crate::models::HttpResult;
use crate::app_state::AppState;

/// 健康检查
pub async fn health_check() -> Json<HttpResult<String>> {
    Json(HttpResult::success("health".to_string()))
}

/// 就绪检查
pub async fn ready_check(state: axum::extract::State<AppState>) -> Json<HttpResult<String>> {
    match state.health_check().await {
        Ok(_) => Json(HttpResult::success("ready".to_string())),
        Err(e) => Json(HttpResult::<String>::error("E001".to_string(), format!("not ready: {e}"))),
    }
}
