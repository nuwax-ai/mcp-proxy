use crate::models::Config;
use crate::openapi;
use crate::server::{cluster_handlers, handlers};
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Create routes with cluster awareness
pub async fn create_cluster_routes(config: Config) -> crate::Result<Router> {
    let config = Arc::new(config);

    if config.cluster.enabled {
        info!("Creating cluster-aware routes");
        create_cluster_mode_routes(config).await
    } else {
        info!("Creating single-node routes");
        create_single_node_routes(config).await
    }
}

/// Create routes for cluster mode
async fn create_cluster_mode_routes(config: Arc<Config>) -> crate::Result<Router> {
    // Create cluster-aware shared state
    let shared_state = cluster_handlers::ClusterAppState::new(config.clone()).await?;

    let mut app = Router::new()
        // Enhanced health check endpoint with cluster information
        .route("/health", get(cluster_handlers::cluster_health_handler))
        // Cluster status and management endpoints
        .route("/cluster/status", get(cluster_status_handler))
        .route("/cluster/nodes", get(cluster_nodes_handler))
        .route("/cluster/leader", get(cluster_leader_handler))
        .route("/cluster/workers", get(cluster_workers_handler))
        .route("/cluster/capacity", get(cluster_capacity_handler))
        // Models management endpoints (cluster-aware)
        .route(
            "/models",
            get(cluster_handlers::cluster_models_list_handler),
        )
        // Cluster-aware transcription endpoint
        .route(
            "/transcribe",
            post(cluster_handlers::cluster_transcribe_handler),
        )
        // Add shared state
        .with_state(shared_state)
        // Merge Swagger UI routes
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

/// Create routes for single-node mode (existing functionality)
async fn create_single_node_routes(config: Arc<Config>) -> crate::Result<Router> {
    // Create original shared state
    let shared_state = handlers::AppState::new(config.clone()).await?;

    let mut app = Router::new()
        // Original health check endpoint
        .route("/health", get(handlers::health_handler))
        // Models management endpoints
        .route("/models", get(handlers::models_list_handler))
        // Original transcription endpoint
        .route("/transcribe", post(handlers::transcribe_handler))
        // Add shared state
        .with_state(shared_state)
        // Merge Swagger UI routes
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

/// Cluster status endpoint
/// GET /cluster/status
async fn cluster_status_handler(
    axum::extract::State(state): axum::extract::State<cluster_handlers::ClusterAppState>,
) -> axum::response::Json<ClusterStatusResponse> {
    let cluster_stats = state.get_cluster_stats().await;

    let response = ClusterStatusResponse {
        cluster_enabled: state.cluster_enabled,
        node_info: state.cluster_node.clone(),
        stats: cluster_stats,
    };

    axum::response::Json(response)
}

/// Cluster nodes endpoint
/// GET /cluster/nodes
async fn cluster_nodes_handler(
    axum::extract::State(state): axum::extract::State<cluster_handlers::ClusterAppState>,
) -> crate::models::HttpResult<NodesResponse> {
    if !state.cluster_enabled {
        return crate::models::HttpResult::from(crate::VoiceCliError::Config(
            "Cluster mode not enabled".to_string(),
        ));
    }

    let nodes = if let Some(ref metadata_store) = state.metadata_store {
        match metadata_store.get_all_nodes().await {
            Ok(nodes) => nodes,
            Err(e) => {
                return crate::models::HttpResult::from(crate::VoiceCliError::Config(format!(
                    "Failed to get cluster nodes: {}",
                    e
                )))
            }
        }
    } else {
        Vec::new()
    };

    let response = NodesResponse { nodes };
    crate::models::HttpResult::success(response)
}

/// Cluster leader endpoint
/// GET /cluster/leader
async fn cluster_leader_handler(
    axum::extract::State(state): axum::extract::State<cluster_handlers::ClusterAppState>,
) -> crate::models::HttpResult<LeaderResponse> {
    if !state.cluster_enabled {
        return crate::models::HttpResult::from(crate::VoiceCliError::Config(
            "Cluster mode not enabled".to_string(),
        ));
    }

    let leader = state.get_cluster_leader().await;
    let response = LeaderResponse { leader };
    crate::models::HttpResult::success(response)
}

/// Cluster workers endpoint
/// GET /cluster/workers
async fn cluster_workers_handler(
    axum::extract::State(state): axum::extract::State<cluster_handlers::ClusterAppState>,
) -> crate::models::HttpResult<WorkersResponse> {
    if !state.cluster_enabled {
        return crate::models::HttpResult::from(crate::VoiceCliError::Config(
            "Cluster mode not enabled".to_string(),
        ));
    }

    let workers = state.get_healthy_workers().await;
    let total_count = workers.len();
    let response = WorkersResponse {
        workers,
        total_count,
    };
    crate::models::HttpResult::success(response)
}

/// Cluster capacity endpoint
/// GET /cluster/capacity
async fn cluster_capacity_handler(
    axum::extract::State(state): axum::extract::State<cluster_handlers::ClusterAppState>,
) -> crate::models::HttpResult<CapacityResponse> {
    if !state.cluster_enabled {
        return crate::models::HttpResult::from(crate::VoiceCliError::Config(
            "Cluster mode not enabled".to_string(),
        ));
    }

    let has_capacity = state.has_cluster_capacity().await;
    let healthy_workers = state.get_healthy_workers().await;
    let cluster_stats = state.get_cluster_stats().await;

    let response = CapacityResponse {
        has_capacity,
        healthy_workers: healthy_workers.len(),
        total_nodes: cluster_stats.as_ref().map(|s| s.total_nodes).unwrap_or(0),
        can_process_tasks: state.can_process_tasks(),
        is_leader: state.is_cluster_leader(),
    };
    crate::models::HttpResult::success(response)
}

/// Response for cluster status endpoint
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ClusterStatusResponse {
    pub cluster_enabled: bool,
    pub node_info: Option<crate::models::ClusterNode>,
    pub stats: Option<cluster_handlers::ClusterStats>,
}

/// Response for cluster nodes endpoint
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct NodesResponse {
    pub nodes: Vec<crate::models::ClusterNode>,
}

/// Response for cluster leader endpoint
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct LeaderResponse {
    pub leader: Option<crate::models::ClusterNode>,
}

/// Response for cluster workers endpoint
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct WorkersResponse {
    pub workers: Vec<crate::models::ClusterNode>,
    pub total_count: usize,
}

/// Response for cluster capacity endpoint
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct CapacityResponse {
    pub has_capacity: bool,
    pub healthy_workers: usize,
    pub total_nodes: usize,
    pub can_process_tasks: bool,
    pub is_leader: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Config;

    #[tokio::test]
    async fn test_create_single_node_routes() {
        let mut config = Config::default();
        config.cluster.enabled = false;
        let app = create_cluster_routes(config).await;
        assert!(app.is_ok());
    }

    #[tokio::test]
    async fn test_create_cluster_routes() {
        let mut config = Config::default();
        config.cluster.enabled = true;
        let app = create_cluster_routes(config).await;
        assert!(app.is_ok());
    }
}
