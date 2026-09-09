use crate::AppError;
use axum::response::IntoResponse;

///健康检查:health
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "服务存活探针，返回纯文本 health", content_type = "text/plain")
    )
)]
pub async fn get_health() -> Result<impl IntoResponse, AppError> {
    Ok("health".to_string())
}

#[utoipa::path(
    get,
    path = "/ready",
    tag = "system",
    responses(
        (status = 200, description = "服务就绪探针，返回纯文本 ready", content_type = "text/plain")
    )
)]
pub async fn get_ready() -> Result<impl IntoResponse, AppError> {
    Ok("ready".to_string())
}
