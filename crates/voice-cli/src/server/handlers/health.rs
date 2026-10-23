//! 健康检查与模型列表端点。

use super::*;

/// 健康检查端点
/// GET /health
#[utoipa::path(
    get,
    path = "/health",
    tag = "健康检查",
    summary = "健康检查",
    description = "检查服务是否正常运行",
    responses(
        (status = 200, description = "服务正常", body = HealthResponse),
        (status = 500, description = "服务异常", body = String)
    ),
)]
pub async fn health_handler(State(state): State<AppState>) -> HttpResult<HealthResponse> {
    let uptime = SystemTime::now()
        .duration_since(state.start_time)
        .unwrap_or_default();

    // 真实已加载模型：STT + TTS 引擎池缓存（首次请求后才加载，故可能为空）
    let mut models_loaded = crate::stt::engine_pool::loaded_model_ids();
    models_loaded.extend(
        crate::tts::engine_pool::loaded_model_ids()
            .into_iter()
            .map(|m| format!("tts:{m}")),
    );

    HttpResult::success(HealthResponse {
        status: "healthy".to_string(),
        models_loaded,
        uptime: uptime.as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// 获取模型列表
/// GET /models
#[utoipa::path(
    get,
    path = "/models",
    tag = "模型管理",
    summary = "获取可用模型列表",
    description = "获取当前支持的语音转录模型列表",
    responses(
        (status = 200, description = "模型列表", body = HttpResult<ModelsResponse>),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn models_list_handler(State(state): State<AppState>) -> HttpResult<ModelsResponse> {
    // 使用配置中的支持模型列表
    let available_models = state.config.whisper.supported_models.clone();

    // 已加载 = STT 引擎池缓存实际存在的模型（首次请求后才加载，可能为空）
    let loaded_models = crate::stt::engine_pool::loaded_model_ids();

    HttpResult::success(ModelsResponse {
        available_models,
        loaded_models,
        model_info: std::collections::HashMap::new(),
    })
}
