use axum::{
    routing::{delete, get, post},
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
        // Task management endpoints under /api/v1/tasks
        .nest("/api/v1/tasks", task_routes())
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

/// Create task management routes
fn task_routes() -> Router<handlers::AppState> {
    Router::new()
        // Task submission
        .route("/transcribe", post(handlers::async_transcribe_handler))
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
