//! 解析器信息、统计与健康检查接口

use crate::app_state::AppState;
use crate::handlers::response::ApiResponse;
use crate::models::DocumentFormat;
use axum::{extract::State, response::IntoResponse};
use serde::Serialize;
use std::collections::HashMap;
use tracing::error;
use utoipa::ToSchema;

/// 支持格式响应
#[derive(Debug, Serialize, ToSchema)]
pub struct SupportedFormatsResponse {
    pub formats: Vec<DocumentFormat>,
}

/// 解析器统计响应
#[derive(Debug, Serialize, ToSchema)]
pub struct ParserStatsResponse {
    pub stats: HashMap<String, serde_json::Value>,
}

/// 处理器缓存统计响应
#[derive(Debug, Serialize, ToSchema)]
pub struct ProcessorCacheStatsResponse {
    pub cache_stats: HashMap<String, serde_json::Value>,
}

/// 获取支持的文档格式
#[utoipa::path(
    get,
    path = "/api/v1/documents/formats",
    responses(
        (status = 200, description = "支持的文档格式列表", body = SupportedFormatsResponse)
    ),
    tag = "documents"
)]
pub async fn get_supported_formats(
    State(_state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let formats = vec![
        DocumentFormat::PDF,
        DocumentFormat::Word,
        DocumentFormat::Excel,
        DocumentFormat::PowerPoint,
        DocumentFormat::Image,
        DocumentFormat::Audio,
        DocumentFormat::HTML,
        DocumentFormat::Text,
        DocumentFormat::Txt,
        DocumentFormat::Md,
    ];

    let response = SupportedFormatsResponse { formats };
    ApiResponse::success(response).into_response()
}

/// 获取解析器统计信息
#[utoipa::path(
    get,
    path = "/api/v1/documents/parser/stats",
    responses(
        (status = 200, description = "解析器统计信息", body = ParserStatsResponse)
    ),
    tag = "documents"
)]
pub async fn get_parser_stats(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let stats_data = state.document_service.get_parser_stats();
    let mut stats = HashMap::new();
    stats.insert(
        "mineru_name".to_string(),
        serde_json::Value::String(stats_data.mineru_name),
    );
    stats.insert(
        "mineru_description".to_string(),
        serde_json::Value::String(stats_data.mineru_description),
    );
    stats.insert(
        "markitdown_name".to_string(),
        serde_json::Value::String(stats_data.markitdown_name),
    );
    stats.insert(
        "markitdown_description".to_string(),
        serde_json::Value::String(stats_data.markitdown_description),
    );
    stats.insert(
        "supported_formats".to_string(),
        serde_json::to_value(stats_data.supported_formats).unwrap_or_default(),
    );

    let response = ParserStatsResponse { stats };
    ApiResponse::success(response).into_response()
}

/// 检查解析器健康状态
#[utoipa::path(
    get,
    path = "/api/v1/documents/parser/health",
    responses(
        (status = 200, description = "解析器健康状态"),
        (status = 500, description = "解析器不健康")
    ),
    tag = "documents"
)]
pub async fn check_parser_health(State(state): State<AppState>) -> impl IntoResponse {
    match state.document_service.check_parser_health().await {
        Ok(health_status) => ApiResponse::success(health_status).into_response(),
        Err(e) => {
            error!("Failed to check parser health status: {}", e);
            ApiResponse::from_app_error::<HashMap<String, bool>>(e.into()).into_response()
        }
    }
}

/// 清理处理器缓存
#[utoipa::path(
    delete,
    path = "/api/v1/documents/processor/cache",
    responses(
        (status = 200, description = "处理器缓存已清空")
    ),
    tag = "documents"
)]
pub async fn clear_processor_cache(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    match state.document_service.clear_processor_cache().await {
        Ok(_) => ApiResponse::message("处理器缓存已清空".to_string()).into_response(),
        Err(e) => ApiResponse::from_app_error::<String>(e.into()).into_response(),
    }
}

/// 获取处理器缓存统计
#[utoipa::path(
    get,
    path = "/api/v1/documents/processor/cache/stats",
    responses(
        (status = 200, description = "处理器缓存统计信息", body = ProcessorCacheStatsResponse)
    ),
    tag = "documents"
)]
pub async fn get_processor_cache_stats(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let cache_statistics = state.document_service.get_processor_cache_stats().await;
    let mut cache_stats = std::collections::HashMap::new();
    cache_stats.insert(
        "total_entries".to_string(),
        serde_json::Value::Number(serde_json::Number::from(cache_statistics.total_entries)),
    );
    cache_stats.insert(
        "expired_entries".to_string(),
        serde_json::Value::Number(serde_json::Number::from(cache_statistics.expired_entries)),
    );

    let response = ProcessorCacheStatsResponse { cache_stats };
    ApiResponse::success(response).into_response()
}
