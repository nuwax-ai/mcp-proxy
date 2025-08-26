use crate::models::Config;
use crate::openapi;
use crate::server::handlers;
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

pub async fn create_routes(config: Config) -> crate::Result<Router> {
    let config = Arc::new(config);

    // Create shared state
    let shared_state = handlers::AppState::new(config.clone()).await?;

    let mut app = Router::new()
        // Health check endpoint
        .route("/health", get(handlers::health_handler))
        // Models management endpoints
        .route("/models", get(handlers::models_list_handler))
        // Main transcription endpoint
        .route("/transcribe", post(handlers::transcribe_handler))
        // Add shared state
        .with_state(shared_state)
        // Merge Swagger UI routes (accessible at /api/docs/)
        // OpenAPI JSON specification available at /api/docs/openapi.json
        .merge(openapi::create_swagger_ui());

    // Add CORS if enabled
    if config.server.cors_enabled {
        app = app.layer(CorsLayer::permissive());
    }

    // Add other middleware
    app = app
        .layer(RequestBodyLimitLayer::new(config.server.max_file_size))
        .layer(TraceLayer::new_for_http());

    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Config;

    #[tokio::test]
    async fn test_create_routes() {
        let config = Config::default();
        let app = create_routes(config).await;
        assert!(app.is_ok());
    }
}
