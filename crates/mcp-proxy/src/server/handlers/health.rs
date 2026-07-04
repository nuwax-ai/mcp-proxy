use crate::AppError;
use axum::response::IntoResponse;

///健康检查:health
pub async fn get_health() -> Result<impl IntoResponse, AppError> {
    Ok("health".to_string())
}

pub async fn get_ready() -> Result<impl IntoResponse, AppError> {
    Ok("ready".to_string())
}
