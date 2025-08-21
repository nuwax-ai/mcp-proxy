use crate::app_state::AppState;
use crate::models::HttpResult;
use axum::Json;
use utoipa;

/// 健康检查
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "服务健康", body = HttpResult<String>)
    ),
    tag = "health"
)]
pub async fn health_check() -> Json<HttpResult<String>> {
    Json(HttpResult::success("health".to_string()))
}

/// 就绪检查
#[utoipa::path(
    get,
    path = "/ready",
    responses(
        (status = 200, description = "服务就绪", body = HttpResult<String>),
        (status = 500, description = "服务未就绪", body = HttpResult<String>)
    ),
    tag = "health"
)]
pub async fn ready_check(state: axum::extract::State<AppState>) -> Json<HttpResult<String>> {
    match state.health_check().await {
        Ok(_) => Json(HttpResult::success("ready".to_string())),
        Err(e) => Json(HttpResult::<String>::error(
            "E001".to_string(),
            format!("not ready: {e}"),
        )),
    }
}
