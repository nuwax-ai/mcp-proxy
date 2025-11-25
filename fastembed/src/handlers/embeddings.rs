use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use crate::models::{parse_model, get_or_init_model, ModelInfo};
use crate::server::AppState;

/// 文本嵌入请求
#[derive(Debug, Deserialize)]
pub struct EmbedRequest {
    /// 模型名称（变体名或模型代码）
    pub model: Option<String>,
    
    /// 待嵌入的文本列表
    pub texts: Vec<String>,
    
    /// 批处理大小
    pub batch_size: Option<usize>,
    
    /// 最大长度
    pub max_length: Option<usize>,
    
    /// 是否归一化
    pub normalize: Option<bool>,
}

/// 文本嵌入响应
#[derive(Debug, Serialize)]
pub struct EmbedResponse {
    pub model: ModelInfo,
    pub count: usize,
    pub embeddings: Vec<Vec<f32>>,
    pub elapsed_ms: u128,
}

/// 错误响应
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub status: u16,
}

/// 文本嵌入处理器
pub async fn handle_embed(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EmbedRequest>,
) -> Result<Json<EmbedResponse>, (StatusCode, Json<ErrorResponse>)> {
    let start = Instant::now();
    
    // 参数验证
    if req.texts.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "EMPTY_TEXTS".to_string(),
                message: "texts 不能为空".to_string(),
                status: 400,
            }),
        ));
    }
    
    // 检查文本数量限制（最大 1024）
    if req.texts.len() > 1024 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ErrorResponse {
                error: "TOO_MANY_TEXTS".to_string(),
                message: format!("texts 数量不能超过 1024，当前: {}", req.texts.len()),
                status: 413,
            }),
        ));
    }
    
    // 解析模型
    let model_name = req.model.as_deref().unwrap_or(&state.config.fastembed.default_model);
    let embedding_model = parse_model(model_name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "INVALID_MODEL".to_string(),
                message: format!("未知模型: {}, 错误: {}", model_name, e),
                status: 400,
            }),
        )
    })?;
    
    // 获取或初始化模型
    let model_arc = get_or_init_model(
        embedding_model.clone(),
        Some(state.config.fastembed.cache_dir.clone()),
        req.max_length.or(Some(state.config.fastembed.max_length)),
    ).map_err(|e| {
        tracing::error!("模型初始化失败: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "MODEL_INIT_ERROR".to_string(),
                message: format!("模型初始化失败: {}", e),
                status: 500,
            }),
        )
    })?;
    
    // 执行嵌入
    let batch_size = req.batch_size.unwrap_or(state.config.fastembed.batch_size);
    
    let mut model_guard = model_arc.lock().unwrap();
    let embeddings = model_guard.embed(req.texts.clone(), Some(batch_size))
        .map_err(|e| {
            tracing::error!("嵌入计算失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "EMBED_ERROR".to_string(),
                    message: format!("嵌入计算失败: {}", e),
                    status: 500,
                }),
            )
        })?;
    
    // 转换为 Vec<Vec<f32>>
    let embeddings_vec: Vec<Vec<f32>> = embeddings
        .into_iter()
        .map(|e| e.to_vec())
        .collect();
    
    let elapsed = start.elapsed();
    
    Ok(Json(EmbedResponse {
        model: ModelInfo::from_embedding_model(&embedding_model),
        count: embeddings_vec.len(),
        embeddings: embeddings_vec,
        elapsed_ms: elapsed.as_millis(),
    }))
}
