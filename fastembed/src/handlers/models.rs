use axum::{extract::{Query, State}, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::models::{list_available_models, ModelInfo};
use crate::server::AppState;
use crate::handlers::embeddings::ErrorResponse;

/// 查询参数
#[derive(Debug, Deserialize)]
pub struct ModelsQuery {
    /// 模型类型: text | image | sparse
    #[serde(rename = "type")]
    pub model_type: Option<String>,
}

/// 模型列表响应
#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub r#type: String,
    pub count: usize,
    pub models: Vec<ModelInfo>,
}

/// 列出可用模型处理器
pub async fn handle_list_models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ModelsQuery>,
) -> Result<Json<ModelsResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 验证类型参数
    let model_type = query.model_type.as_deref().unwrap_or("text");
    
    // 目前仅支持 text 类型
    if model_type != "text" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "INVALID_TYPE".to_string(),
                message: format!("不支持的模型类型: {}，当前仅支持 text", model_type),
                status: 400,
            }),
        ));
    }
    
    // 列出可用模型
    let models = list_available_models(&state.config.fastembed.cache_dir)
        .map_err(|e| {
            tracing::error!("列出可用模型失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "LIST_ERROR".to_string(),
                    message: format!("列出可用模型失败: {}", e),
                    status: 500,
                }),
            )
        })?;
    
    Ok(Json(ModelsResponse {
        r#type: model_type.to_string(),
        count: models.len(),
        models,
    }))
}
