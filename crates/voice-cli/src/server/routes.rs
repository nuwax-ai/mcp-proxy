use crate::models::Config;
use crate::openapi;
use crate::server::handlers;
use crate::server::middleware_config::set_layer;
use crate::server::stt_stream;
use crate::server::tts_stream;
use axum::{
    Router,
    routing::{delete, get, post},
};
use std::sync::Arc;

/// Create routes for the server
pub async fn create_routes(config: Arc<Config>) -> crate::Result<Router> {
    let shared_state = handlers::AppState::new(config.clone()).await?;
    create_routes_with_state(shared_state).await
}

/// Create routes with pre-created AppState
pub async fn create_routes_with_state(shared_state: handlers::AppState) -> crate::Result<Router> {
    let config = shared_state.config.clone();

    let app = Router::new()
        // Health check endpoint
        .route("/health", get(handlers::health_handler))
        // Models management endpoints
        .route("/models", get(handlers::models_list_handler))
        // Transcription endpoint (synchronous)
        .route("/transcribe", post(handlers::transcribe_handler))
        // TTS 同步合成 + 音色查询（sherpa-onnx Kokoro，重新设计的 /api/v1/ 风格）
        .route("/api/v1/tts", post(handlers::tts_sync_handler))
        .route("/api/v1/tts/voices", get(handlers::tts_voices_handler))
        // STT 流式 WebSocket（LocalAgreement 2）
        .route(
            "/api/v1/stream/transcribe",
            get(stt_stream::ws_transcribe_handler),
        )
        // TTS 流式 WebSocket（sherpa-onnx callback → 增量 PCM）
        .route("/api/v1/stream/tts", get(tts_stream::ws_tts_handler))
        // Task management endpoints under /api/v1/tasks
        .nest("/api/v1/tasks", task_routes())
        // Add shared state
        .with_state(shared_state.clone())
        // Merge Swagger UI routes
        .merge(openapi::create_swagger_ui())
        // Merge Scalar 风格文档 routes（与 Swagger UI 并存）
        .merge(openapi::create_scalar_docs());

    // 统一中间件挂载
    let app = set_layer(
        app,
        shared_state,
        config.server.max_file_size,
        config.server.cors_enabled,
    );

    Ok(app)
}

/// Create task management routes
fn task_routes() -> Router<handlers::AppState> {
    Router::new()
        // Task submission
        .route("/transcribe", post(handlers::async_transcribe_handler))
        // URL-based task submission
        .route(
            "/transcribeFromUrl",
            post(handlers::transcribe_from_url_handler),
        )
        // TTS 异步任务（literal /tts 前缀，与 /{task_id} 共存：matchit 静态段优先于参数段）
        .route("/tts", post(handlers::tts_async_handler))
        .route("/tts/{task_id}", get(handlers::tts_task_status_handler))
        .route(
            "/tts/{task_id}/audio",
            get(handlers::tts_task_audio_handler),
        )
        .route("/tts/stats", get(handlers::tts_tasks_stats_handler))
        // Task status and management
        .route("/{task_id}", get(handlers::get_task_handler))
        .route("/{task_id}", delete(handlers::delete_task_handler))
        .route("/{task_id}/result", get(handlers::get_task_result_handler))
        .route("/{task_id}/cancel", post(handlers::cancel_task_handler))
        .route("/{task_id}/retry", post(handlers::retry_task_handler))
        // Task statistics
        .route("/stats", get(handlers::get_tasks_stats_handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Config;

    #[tokio::test]
    async fn test_create_routes() {
        let config = Arc::new(Config::default());
        let app = create_routes(config).await;
        assert!(app.is_ok());
    }
}
