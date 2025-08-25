use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::{interval, timeout};
use tower::ServiceBuilder;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing::{debug, error, info, warn};

// Import necessary types
use crate::models::{
    ClusterError, ClusterNode, LoadBalancerConfig, MetadataStore, NodeRole, NodeStatus,
};

/// Built-in HTTP proxy load balancer service
pub struct LoadBalancerService {
    /// Load balancer configuration
    config: LoadBalancerConfig,
    /// HTTP client for proxying requests
    client: Client,
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
    /// Cached cluster nodes
    cluster_nodes: Arc<RwLock<Vec<ClusterNode>>>,
    /// Load balancer statistics
    stats: Arc<RwLock<LoadBalancerStats>>,
    /// Current leader node cache
    leader_node: Arc<RwLock<Option<ClusterNode>>>,
}

/// Load balancer statistics
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LoadBalancerStats {
    /// Total requests processed
    pub total_requests: u64,
    /// Successful requests
    pub successful_requests: u64,
    /// Failed requests
    pub failed_requests: u64,
    /// Requests to each backend
    pub requests_per_backend: HashMap<String, u64>,
    /// Average response time
    pub avg_response_time: f32,
    /// Total response time
    pub total_response_time: f32,
    /// Uptime in seconds
    pub uptime_seconds: u64,
    /// Current active connections
    pub active_connections: u64,
}

/// Application state for the load balancer
#[derive(Clone)]
pub struct LoadBalancerState {
    service: Arc<LoadBalancerService>,
}

impl LoadBalancerService {
    /// Create a new LoadBalancerService
    pub fn new(
        config: LoadBalancerConfig,
        metadata_store: Arc<MetadataStore>,
    ) -> Result<Self, ClusterError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ClusterError::Config(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            config,
            client,
            metadata_store,
            cluster_nodes: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(LoadBalancerStats::default())),
            leader_node: Arc::new(RwLock::new(None)),
        })
    }

    /// Start the load balancer service
    pub async fn start(&self) -> Result<(), ClusterError> {
        info!(
            "Starting load balancer on {}:{}",
            self.config.bind_address, self.config.port
        );

        // Initialize nodes from metadata store
        if let Err(e) = Self::refresh_cluster_nodes(
            &self.metadata_store,
            &self.cluster_nodes,
            &self.leader_node,
        )
        .await
        {
            warn!("Failed to initialize cluster nodes: {}", e);
        }

        // Start background tasks
        let health_check_handle = self.start_health_checker();
        let node_refresh_handle = self.start_node_refresher();

        // Create Axum app
        let app = self.create_app();

        // Start the HTTP server
        let listener = tokio::net::TcpListener::bind(format!(
            "{}:{}",
            self.config.bind_address, self.config.port
        ))
        .await
        .map_err(|e| ClusterError::Network(format!("Failed to bind to address: {}", e)))?;

        info!(
            "Load balancer listening on {}:{}",
            self.config.bind_address, self.config.port
        );

        // Run the server along with background tasks
        tokio::select! {
            result = axum::serve(listener, app) => {
                if let Err(e) = result {
                    error!("Load balancer server error: {}", e);
                }
            }
            result = health_check_handle => {
                if let Err(e) = result {
                    error!("Health checker task failed: {}", e);
                }
            }
            result = node_refresh_handle => {
                if let Err(e) = result {
                    error!("Node refresher task failed: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Create the Axum application
    fn create_app(&self) -> Router {
        let state = LoadBalancerState {
            service: Arc::new(self.clone()),
        };

        Router::new()
            .route("/health", any(health_handler))
            .route("/lb/status", any(lb_status_handler))
            .route("/lb/stats", any(lb_stats_handler))
            .fallback(proxy_handler)
            .with_state(state)
            .layer(
                ServiceBuilder::new()
                    .layer(TraceLayer::new_for_http())
                    .layer(CompressionLayer::new())
                    .layer(CorsLayer::permissive()),
            )
    }

    /// Start the health checker background task
    fn start_health_checker(&self) -> tokio::task::JoinHandle<()> {
        let mut interval = interval(Duration::from_secs(self.config.health_check_interval));
        let metadata_store = Arc::clone(&self.metadata_store);
        let cluster_nodes = Arc::clone(&self.cluster_nodes);
        let leader_node = Arc::clone(&self.leader_node);
        let client = self.client.clone();
        let health_timeout = Duration::from_secs(self.config.health_check_timeout);

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                if let Err(e) = Self::check_cluster_health(
                    &metadata_store,
                    &cluster_nodes,
                    &leader_node,
                    &client,
                    health_timeout,
                )
                .await
                {
                    warn!("Health check failed: {}", e);
                }
            }
        })
    }

    /// Start the node refresher background task
    fn start_node_refresher(&self) -> tokio::task::JoinHandle<()> {
        let mut interval = interval(Duration::from_secs(10)); // Refresh every 10 seconds
        let metadata_store = Arc::clone(&self.metadata_store);
        let cluster_nodes = Arc::clone(&self.cluster_nodes);
        let leader_node = Arc::clone(&self.leader_node);

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                if let Err(e) =
                    Self::refresh_cluster_nodes(&metadata_store, &cluster_nodes, &leader_node).await
                {
                    warn!("Node refresh failed: {}", e);
                }
            }
        })
    }

    /// Check health of all cluster nodes
    async fn check_cluster_health(
        metadata_store: &MetadataStore,
        cluster_nodes: &Arc<RwLock<Vec<ClusterNode>>>,
        leader_node: &Arc<RwLock<Option<ClusterNode>>>,
        client: &Client,
        health_timeout: Duration,
    ) -> Result<(), ClusterError> {
        let nodes = cluster_nodes.read().await.clone();
        let mut healthy_nodes = Vec::new();
        let mut current_leader: Option<ClusterNode> = None;

        for node in nodes {
            let health_url = format!("http://{}:{}/health", node.address, node.http_port);

            let is_healthy = match timeout(health_timeout, client.get(&health_url).send()).await {
                Ok(Ok(response)) => response.status().is_success(),
                Ok(Err(_)) | Err(_) => false,
            };

            if is_healthy {
                // Update node status to healthy
                if let Err(e) = metadata_store
                    .update_node_status(&node.node_id, NodeStatus::Healthy)
                    .await
                {
                    warn!(
                        "Failed to update node {} status to healthy: {}",
                        node.node_id, e
                    );
                }

                let mut healthy_node = node.clone();
                healthy_node.status = NodeStatus::Healthy;
                healthy_nodes.push(healthy_node.clone());

                // Track leader
                if healthy_node.role == NodeRole::Leader {
                    current_leader = Some(healthy_node);
                }
            } else {
                // Update node status to unhealthy
                if let Err(e) = metadata_store
                    .update_node_status(&node.node_id, NodeStatus::Unhealthy)
                    .await
                {
                    warn!(
                        "Failed to update node {} status to unhealthy: {}",
                        node.node_id, e
                    );
                }
            }
        }

        // Update cluster nodes cache
        {
            let mut cluster_nodes_write = cluster_nodes.write().await;
            *cluster_nodes_write = healthy_nodes;
        }

        // Update leader cache
        {
            let mut leader_write = leader_node.write().await;
            *leader_write = current_leader;
        }

        Ok(())
    }

    /// Refresh cluster nodes from metadata store
    async fn refresh_cluster_nodes(
        metadata_store: &MetadataStore,
        cluster_nodes: &Arc<RwLock<Vec<ClusterNode>>>,
        leader_node: &Arc<RwLock<Option<ClusterNode>>>,
    ) -> Result<(), ClusterError> {
        let nodes = metadata_store.get_all_nodes().await?;
        let mut current_leader: Option<ClusterNode> = None;

        // Find the leader
        for node in &nodes {
            if node.role == NodeRole::Leader && node.status == NodeStatus::Healthy {
                current_leader = Some(node.clone());
                break;
            }
        }

        // Update caches
        {
            let mut cluster_nodes_write = cluster_nodes.write().await;
            *cluster_nodes_write = nodes;
        }

        {
            let mut leader_write = leader_node.write().await;
            *leader_write = current_leader;
        }

        Ok(())
    }

    /// Select the best backend for a request (leader node)
    async fn select_backend(&self) -> Option<ClusterNode> {
        // For audio cluster, always route to leader
        self.leader_node.read().await.clone()
    }

    /// Update load balancer statistics
    async fn update_stats(&self, backend_id: &str, response_time: f32, success: bool) {
        let mut stats = self.stats.write().await;

        stats.total_requests += 1;
        if success {
            stats.successful_requests += 1;
        } else {
            stats.failed_requests += 1;
        }

        // Update per-backend stats
        *stats
            .requests_per_backend
            .entry(backend_id.to_string())
            .or_insert(0) += 1;

        // Update average response time
        stats.total_response_time += response_time;
        stats.avg_response_time = stats.total_response_time / stats.total_requests as f32;
    }

    /// Get current load balancer statistics
    pub async fn get_stats(&self) -> LoadBalancerStats {
        self.stats.read().await.clone()
    }

    /// Get current cluster status
    pub async fn get_cluster_status(&self) -> ClusterStatus {
        let nodes = self.cluster_nodes.read().await.clone();
        let leader = self.leader_node.read().await.clone();

        ClusterStatus {
            total_nodes: nodes.len(),
            healthy_nodes: nodes
                .iter()
                .filter(|n| n.status == NodeStatus::Healthy)
                .count(),
            leader_node: leader.map(|n| n.node_id),
            nodes,
        }
    }
}

impl Clone for LoadBalancerService {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            client: self.client.clone(),
            metadata_store: Arc::clone(&self.metadata_store),
            cluster_nodes: Arc::clone(&self.cluster_nodes),
            stats: Arc::clone(&self.stats),
            leader_node: Arc::clone(&self.leader_node),
        }
    }
}

/// Cluster status information
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClusterStatus {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub leader_node: Option<String>,
    pub nodes: Vec<ClusterNode>,
}

/// Main proxy handler that forwards requests to cluster leader
async fn proxy_handler(
    State(state): State<LoadBalancerState>,
    req: Request,
) -> Result<Response, StatusCode> {
    let start_time = Instant::now();

    // Select backend (leader node)
    let backend = match state.service.select_backend().await {
        Some(node) => node,
        None => {
            error!("No healthy leader node available");
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    let backend_url = format!("http://{}:{}", backend.address, backend.http_port);

    // Build the target URL
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|x| x.as_str())
        .unwrap_or("/");

    let target_url = format!("{}{}", backend_url, path_and_query);

    debug!("Proxying request to: {}", target_url);

    // Extract request parts
    let method = req.method().clone();
    let headers = req.headers().clone();
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => {
            error!("Failed to read request body");
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    // Create proxy request
    let mut proxy_req = state
        .service
        .client
        .request(method, &target_url)
        .body(body_bytes);

    // Copy headers (excluding hop-by-hop headers)
    for (name, value) in headers.iter() {
        if !is_hop_by_hop_header(name.as_str()) {
            proxy_req = proxy_req.header(name, value);
        }
    }

    // Send the request
    let response = match proxy_req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Failed to proxy request: {}", e);
            let processing_time = start_time.elapsed().as_secs_f32();
            state
                .service
                .update_stats(&backend.node_id, processing_time, false)
                .await;
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    let processing_time = start_time.elapsed().as_secs_f32();
    let success = response.status().is_success();

    // Update statistics
    state
        .service
        .update_stats(&backend.node_id, processing_time, success)
        .await;

    // Convert response
    let status = response.status();
    let headers = response.headers().clone();
    let body_bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => {
            error!("Failed to read response body");
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    // Build response
    let mut response_builder = Response::builder().status(status);

    // Copy response headers (excluding hop-by-hop headers)
    for (name, value) in headers.iter() {
        if !is_hop_by_hop_header(name.as_str()) {
            response_builder = response_builder.header(name, value);
        }
    }

    response_builder
        .body(axum::body::Body::from(body_bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Health check handler for the load balancer itself
async fn health_handler() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "service": "voice-cli-load-balancer"
    }))
}

/// Load balancer status handler
async fn lb_status_handler(State(state): State<LoadBalancerState>) -> impl IntoResponse {
    let status = state.service.get_cluster_status().await;
    axum::Json(status)
}

/// Load balancer statistics handler
async fn lb_stats_handler(State(state): State<LoadBalancerState>) -> impl IntoResponse {
    let stats = state.service.get_stats().await;
    let cluster_status = state.service.get_cluster_status().await;

    // Create enriched stats response
    let enriched_stats = serde_json::json!({
        "total_requests": stats.total_requests,
        "successful_requests": stats.successful_requests,
        "failed_requests": stats.failed_requests,
        "avg_response_time": stats.avg_response_time,
        "requests_per_backend": stats.requests_per_backend,
        "uptime_seconds": stats.uptime_seconds,
        "active_connections": stats.active_connections,
        "current_leader": cluster_status.leader_node,
        "healthy_nodes": cluster_status.healthy_nodes,
        "total_nodes": cluster_status.total_nodes
    });

    axum::Json(enriched_stats)
}

/// Check if a header is a hop-by-hop header that shouldn't be forwarded
fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}
