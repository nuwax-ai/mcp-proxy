use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use crate::models::Config;
use crate::server::handlers;
use crate::openapi;
use crate::server::middleware_config::set_layer;

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
        // Task management endpoints (asynchronous)
        .route("/tasks/transcribe", post(handlers::async_transcribe_handler))
        .route("/tasks/{task_id}", get(handlers::get_task_handler))
        .route("/tasks/{task_id}", axum::routing::delete(handlers::cancel_task_handler))
        .route("/tasks/{task_id}/result", get(handlers::get_task_result_handler))
        .route("/tasks/{task_id}/cancel", post(handlers::cancel_task_post_handler))
        .route("/tasks/{task_id}/retry", post(handlers::retry_task_handler))
        .route("/tasks/stats", get(handlers::get_tasks_stats_handler))
        // Add shared state
        .with_state(shared_state.clone())
        // Merge Swagger UI routes
        .merge(openapi::create_swagger_ui());

    // 统一中间件挂载
    let app = set_layer(
        app,
        shared_state,
        config.server.max_file_size,
        config.server.cors_enabled,
    );

    Ok(app)
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
