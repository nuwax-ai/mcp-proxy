use crate::cluster::{ClusterState, ServiceDiscovery, ServiceDiscoveryConfig, SimpleTaskScheduler};
use crate::grpc::server::{AudioClusterGrpcServer, GrpcServerConfig};
use crate::models::{ClusterNode, Config, MetadataStore, NodeStatus};
use crate::{log_cluster_event, log_cluster_state_change, log_performance_metric};

use crate::error::ClusterResultExt;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Service manager for coordinating all cluster services
pub struct ClusterServiceManager {
    /// Configuration
    config: Arc<Config>,
    /// Cluster node information
    node: ClusterNode,
    /// Shared cluster state
    cluster_state: Arc<ClusterState>,
    /// Metadata store for persistence
    metadata_store: Option<Arc<MetadataStore>>,
    /// Task scheduler
    task_scheduler: Option<Arc<SimpleTaskScheduler>>,
    /// Service discovery
    service_discovery: Option<Arc<ServiceDiscovery>>,
    /// Shutdown signal broadcaster
    shutdown_tx: broadcast::Sender<()>,
}

impl ClusterServiceManager {
    /// Create a new ClusterServiceManager
    pub fn new(
        config: Arc<Config>,
        node: ClusterNode,
        cluster_state: Arc<ClusterState>,
        metadata_store: Option<Arc<MetadataStore>>,
    ) -> Self {
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(16);

        Self {
            config,
            node,
            cluster_state,
            metadata_store,
            task_scheduler: None,
            service_discovery: None,
            shutdown_tx,
        }
    }

    /// Create a new ClusterServiceManager with integrated MetadataStore and ClusterState
    pub fn new_integrated(
        config: Arc<Config>,
        node: ClusterNode,
        cluster_state: Arc<ClusterState>,
        db_path: impl AsRef<std::path::Path>,
    ) -> Result<Self, anyhow::Error> {
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(16);

        // Create MetadataStore with ClusterState integration
        let metadata_store = Arc::new(
            MetadataStore::new_with_cluster_state(db_path, cluster_state.clone())
                .context("Failed to create integrated metadata store")?,
        );

        Ok(Self {
            config,
            node,
            cluster_state,
            metadata_store: Some(metadata_store),
            task_scheduler: None,
            service_discovery: None,
            shutdown_tx,
        })
    }

    /// Initialize the service manager with task scheduler
    pub fn with_task_scheduler(mut self, task_scheduler: Arc<SimpleTaskScheduler>) -> Self {
        self.task_scheduler = Some(task_scheduler);
        self
    }

    /// Initialize the service manager with service discovery
    pub fn with_service_discovery(mut self, service_discovery: Arc<ServiceDiscovery>) -> Self {
        self.service_discovery = Some(service_discovery);
        self
    }

    /// Create service discovery with default configuration
    pub fn create_service_discovery(&self) -> Arc<ServiceDiscovery> {
        let discovery_config = ServiceDiscoveryConfig::default();
        Arc::new(ServiceDiscovery::new(
            self.node.clone(),
            self.cluster_state.clone(),
            discovery_config,
        ))
    }

    /// Start all cluster services concurrently
    pub async fn start(&mut self) -> Result<()> {
        let start_time = std::time::Instant::now();

        log_cluster_event!(
            info,
            &self.node.node_id,
            "cluster_service_manager",
            "start_services",
            "Starting cluster service manager",
            node_role = ?self.node.role,
            node_address = %self.node.address,
            grpc_port = self.node.grpc_port
        );

        // Initialize cluster state with metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            // Sync existing data from database to cluster state
            metadata_store
                .sync_to_cluster_state()
                .await
                .context("Failed to sync metadata store to cluster state")?;
        }

        // Register this node in cluster state (atomic operation)
        self.cluster_state.upsert_node(self.node.clone());

        // Also register in metadata store if available (will update both cluster state and database)
        if let Some(ref metadata_store) = self.metadata_store {
            metadata_store
                .add_node(&self.node)
                .await
                .with_node_context(&self.node.node_id)
                .context("Failed to register node in metadata store")?;
        }

        // Perform initial cluster discovery if service discovery is available
        if let Some(ref service_discovery) = self.service_discovery {
            info!("Performing initial cluster discovery");
            match service_discovery.discover_cluster().await {
                Ok(discovered_nodes) => {
                    info!("Initial discovery found {} nodes", discovered_nodes.len());
                    for node in discovered_nodes {
                        info!(
                            "Discovered node: {} at {}:{}",
                            node.node_id, node.address, node.grpc_port
                        );

                        // Add discovered nodes to metadata store if available
                        if let Some(ref metadata_store) = self.metadata_store {
                            if let Err(e) = metadata_store.add_node(&node).await {
                                warn!("Failed to add discovered node to metadata store: {:?}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Initial cluster discovery failed: {:?}", e);
                }
            }
        }

        // Start all services concurrently
        tokio::select! {
            result = self.start_http_server() => {
                if let Err(e) = result {
                    error!("HTTP server failed: {:?}", e);
                    return Err(e);
                }
            }
            result = self.start_grpc_server() => {
                if let Err(e) = result {
                    error!("gRPC server failed: {:?}", e);
                    return Err(e);
                }
            }
            result = self.start_heartbeat_service() => {
                if let Err(e) = result {
                    error!("Heartbeat service failed: {:?}", e);
                    return Err(e);
                }
            }
            result = self.start_task_scheduler() => {
                if let Err(e) = result {
                    error!("Task scheduler failed: {:?}", e);
                    return Err(e);
                }
            }
            result = self.start_service_discovery() => {
                if let Err(e) = result {
                    error!("Service discovery failed: {:?}", e);
                    return Err(e);
                }
            }
            _ = self.wait_for_shutdown() => {
                info!("Received shutdown signal");
            }
        }

        // Log performance metrics
        let duration = start_time.elapsed();
        log_performance_metric!(
            "start_services",
            duration.as_millis() as u64,
            &self.node.node_id,
            "cluster_service_manager",
            success = true
        );

        // Perform graceful shutdown
        self.shutdown().await?;
        Ok(())
    }

    /// Start HTTP server
    async fn start_http_server(&self) -> Result<()> {
        info!(
            "Starting HTTP server on port {}",
            self.config.cluster.http_port
        );

        // Create router using the existing routes function
        let app = crate::server::routes::create_routes(Arc::new((*self.config).clone()))
            .await
            .map_err(|e| anyhow::Error::new(e).context("Failed to create HTTP routes"))?;

        // Bind and serve
        let listener =
            tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.config.cluster.http_port))
                .await
                .context("Failed to bind HTTP server")?;

        info!(
            "HTTP server listening on 0.0.0.0:{}",
            self.config.cluster.http_port
        );

        // Start server with graceful shutdown
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.recv().await;
                info!("HTTP server shutting down gracefully");
            })
            .await
            .context("HTTP server error")?;

        Ok(())
    }

    /// Start gRPC server
    async fn start_grpc_server(&self) -> Result<()> {
        info!(
            "Starting gRPC server on port {}",
            self.config.cluster.grpc_port
        );

        let grpc_config = GrpcServerConfig {
            bind_address: "0.0.0.0".to_string(),
            port: self.config.cluster.grpc_port,
            max_message_size: 4 * 1024 * 1024, // 4MB
        };

        let grpc_server = AudioClusterGrpcServer::new(
            grpc_config,
            self.node.clone(),
            self.metadata_store.clone().unwrap_or_else(|| {
                // Create a temporary metadata store if none provided
                Arc::new(MetadataStore::new_temp().expect("Failed to create temp metadata store"))
            }),
            self.task_scheduler.clone(),
            None, // heartbeat_service
        );

        let mut shutdown_rx = self.shutdown_tx.subscribe();
        grpc_server
            .start_with_shutdown(async move {
                let _ = shutdown_rx.recv().await;
                info!("gRPC server shutting down gracefully");
            })
            .await
            .map_err(|e| anyhow::Error::new(e).context("gRPC server error"))?;

        Ok(())
    }

    /// Start heartbeat service with atomic health monitoring
    async fn start_heartbeat_service(&self) -> Result<()> {
        info!("Starting heartbeat service for node: {}", self.node.node_id);

        let cluster_state = self.cluster_state.clone();
        let metadata_store = self.metadata_store.clone();
        let node_id = self.node.node_id.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // Heartbeat interval (30 seconds)
        let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));
        // Health check interval (15 seconds) - more frequent than heartbeat but not too aggressive
        let mut health_check_interval = tokio::time::interval(std::time::Duration::from_secs(15));

        loop {
            tokio::select! {
                _ = heartbeat_interval.tick() => {
                    debug!("Sending heartbeat for node: {}", node_id);

                    // Update heartbeat in cluster state (atomic)
                    if let Some(mut node) = cluster_state.get_node(&node_id) {
                        node.update_heartbeat();
                        cluster_state.upsert_node(node);
                    }

                    // Update heartbeat in metadata store if available (will also update cluster state)
                    if let Some(metadata_store) = metadata_store.as_ref() {
                        if let Err(e) = metadata_store.update_heartbeat(&node_id).await {
                            warn!("Failed to update heartbeat in metadata store: {:?}", e);
                        }
                    }
                }
                _ = health_check_interval.tick() => {
                    // Perform atomic node health monitoring
                    self.perform_health_check(&cluster_state, &metadata_store).await;
                }
                _ = shutdown_rx.recv() => {
                    info!("Heartbeat service shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Perform atomic health check on all nodes
    async fn perform_health_check(
        &self,
        cluster_state: &Arc<ClusterState>,
        metadata_store: &Option<Arc<MetadataStore>>,
    ) {
        let heartbeat_timeout = 90; // 90 seconds timeout
        let joining_timeout = 60; // 60 seconds for joining nodes to become healthy
        let current_time = chrono::Utc::now().timestamp();

        // Get all nodes from cluster state (atomic read)
        let all_nodes = cluster_state.get_all_nodes();

        for node in all_nodes {
            let time_since_heartbeat = current_time - node.last_heartbeat;
            let is_heartbeat_healthy = time_since_heartbeat <= heartbeat_timeout;
            
            // For joining nodes, also check if they've been running long enough
            let is_joining_ready = if node.status == crate::models::NodeStatus::Joining {
                time_since_heartbeat <= joining_timeout
            } else {
                false
            };

            // Determine new status and if update is needed
            let (new_status, needs_update) = match node.status {
                crate::models::NodeStatus::Joining => {
                    if is_joining_ready && self.check_node_service_availability(&node).await {
                        info!(
                            "Node {} successfully joined cluster and is now healthy (last heartbeat: {}s ago)",
                            node.node_id, time_since_heartbeat
                        );
                        (Some(crate::models::NodeStatus::Healthy), true)
                    } else if time_since_heartbeat > joining_timeout * 2 {
                        warn!(
                            "Node {} failed to join cluster within timeout (last heartbeat: {}s ago)",
                            node.node_id, time_since_heartbeat
                        );
                        (Some(crate::models::NodeStatus::Unhealthy), true)
                    } else {
                        debug!(
                            "Node {} still joining (last heartbeat: {}s ago)",
                            node.node_id, time_since_heartbeat
                        );
                        (None, false)
                    }
                }
                crate::models::NodeStatus::Healthy => {
                    if !is_heartbeat_healthy {
                        warn!(
                            "Node {} became unhealthy (last heartbeat: {}s ago)",
                            node.node_id, time_since_heartbeat
                        );
                        (Some(crate::models::NodeStatus::Unhealthy), true)
                    } else {
                        (None, false)
                    }
                }
                crate::models::NodeStatus::Unhealthy => {
                    if is_heartbeat_healthy && self.check_node_service_availability(&node).await {
                        info!("Node {} recovered and is now healthy", node.node_id);
                        (Some(crate::models::NodeStatus::Healthy), true)
                    } else {
                        (None, false)
                    }
                }
                crate::models::NodeStatus::Leaving => {
                    // Don't update leaving nodes
                    (None, false)
                }
            };

            if needs_update {
                if let Some(target_status) = new_status {
                    // Perform atomic health update
                    if let Some(metadata_store) = metadata_store {
                        if let Err(e) = metadata_store
                            .update_node_status(&node.node_id, target_status)
                            .await
                        {
                            warn!(
                                "Failed to update status for node {} to {:?}: {:?}",
                                node.node_id, target_status, e
                            );
                        }
                    } else {
                        // Fallback to cluster state only
                        if let Err(e) = cluster_state.update_node_status(&node.node_id, target_status) {
                            warn!("Failed to update node status in cluster state: {:?}", e);
                        }
                    }
                }
            }
        }
        
        // Also check self node if it's in joining state
        if let Some(self_node) = cluster_state.get_node(&self.node.node_id) {
            if self_node.status == crate::models::NodeStatus::Joining {
                let time_since_heartbeat = current_time - self_node.last_heartbeat;
                if time_since_heartbeat <= joining_timeout {
                    info!(
                        "Self node {} successfully joined cluster and is now healthy",
                        self.node.node_id
                    );
                    
                    if let Some(metadata_store) = metadata_store {
                        if let Err(e) = metadata_store
                            .update_node_status(&self.node.node_id, crate::models::NodeStatus::Healthy)
                            .await
                        {
                            warn!("Failed to update self node status: {:?}", e);
                        }
                    } else {
                        if let Err(e) = cluster_state.update_node_status(&self.node.node_id, crate::models::NodeStatus::Healthy) {
                            warn!("Failed to update self node status in cluster state: {:?}", e);
                        }
                    }
                }
            }
        }
    }
    
    /// Check if a node's services are available (active health check)
    async fn check_node_service_availability(&self, node: &ClusterNode) -> bool {
        // Skip self check to avoid circular dependency
        if node.node_id == self.node.node_id {
            return true;
        }
        
        // Try to connect to the node's gRPC service
        match self.ping_node_grpc(node).await {
            Ok(true) => {
                debug!("Node {} gRPC service is available", node.node_id);
                true
            }
            Ok(false) => {
                debug!("Node {} gRPC service responded but not ready", node.node_id);
                false
            }
            Err(e) => {
                debug!("Node {} gRPC service unavailable: {}", node.node_id, e);
                false
            }
        }
    }
    
    /// Ping a node's gRPC service to check availability
    async fn ping_node_grpc(&self, node: &ClusterNode) -> Result<bool> {
        use crate::grpc::client::AudioClusterClient;
        
        let address = format!(
            "http://{}:{}",
            node.address,
            node.grpc_port
        );
        
        // Create a quick connection with short timeout
        match AudioClusterClient::connect(&address, Some(std::time::Duration::from_secs(5))).await {
            Ok(mut client) => {
                // Try to send a simple health check ping
                match client.ping().await {
                    Ok(_) => Ok(true),
                    Err(_) => Ok(false)
                }
            }
            Err(e) => {
                debug!("Failed to connect to node {} at {}: {}", node.node_id, address, e);
                Err(anyhow::Error::new(e))
            }
        }
    }

    /// Start task scheduler service
    async fn start_task_scheduler(&self) -> Result<()> {
        if let Some(ref task_scheduler) = self.task_scheduler {
            info!("Starting task scheduler service");

            // Clone the scheduler to get ownership
            let _scheduler = task_scheduler.clone();
            let mut shutdown_rx = self.shutdown_tx.subscribe();

            // We need to use a different approach since start() requires &mut self
            // For now, we'll just wait for shutdown signal
            // TODO: Implement a proper async task scheduler that doesn't require &mut self
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("Task scheduler shutting down");
                }
            }
        } else {
            info!("No task scheduler configured, skipping");
            // Just wait for shutdown signal
            let mut shutdown_rx = self.shutdown_tx.subscribe();
            let _ = shutdown_rx.recv().await;
        }

        Ok(())
    }

    /// Wait for shutdown signal
    async fn wait_for_shutdown(&self) -> Result<()> {
        // Wait for SIGINT or SIGTERM
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received SIGINT (Ctrl+C)");
            }
            _ = self.wait_for_sigterm() => {
                info!("Received SIGTERM");
            }
        }

        // Broadcast shutdown signal to all services
        if let Err(e) = self.shutdown_tx.send(()) {
            warn!("Failed to send shutdown signal: {:?}", e);
        }

        Ok(())
    }

    /// Wait for SIGTERM signal (Unix only)
    #[cfg(unix)]
    async fn wait_for_sigterm(&self) -> Result<()> {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).context("Failed to register SIGTERM handler")?;
        sigterm.recv().await;
        Ok(())
    }

    /// Wait for SIGTERM signal (Windows - no-op)
    #[cfg(not(unix))]
    async fn wait_for_sigterm(&self) -> Result<()> {
        // On Windows, we only handle Ctrl+C
        std::future::pending::<()>().await;
        Ok(())
    }

    /// Start service discovery service
    async fn start_service_discovery(&self) -> Result<()> {
        if let Some(ref _service_discovery) = self.service_discovery {
            info!("Starting service discovery service");

            // Clone the service discovery to get ownership
            let mut discovery = ServiceDiscovery::new(
                self.node.clone(),
                self.cluster_state.clone(),
                ServiceDiscoveryConfig::default(),
            );

            let mut shutdown_rx = self.shutdown_tx.subscribe();

            tokio::select! {
                result = discovery.start() => {
                    if let Err(e) = result {
                        error!("Service discovery failed: {:?}", e);
                        return Err(anyhow::Error::new(e).context("Service discovery error"));
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Service discovery shutting down");
                    if let Err(e) = discovery.shutdown().await {
                        warn!("Failed to shutdown service discovery gracefully: {:?}", e);
                    }
                }
            }
        } else {
            info!("No service discovery configured, skipping");
            // Just wait for shutdown signal
            let mut shutdown_rx = self.shutdown_tx.subscribe();
            let _ = shutdown_rx.recv().await;
        }

        Ok(())
    }

    /// Perform graceful shutdown
    async fn shutdown(&self) -> Result<()> {
        info!(
            "Performing graceful shutdown for node: {}",
            self.node.node_id
        );

        // Announce leaving via service discovery if available
        if let Some(ref service_discovery) = self.service_discovery {
            if let Err(e) = service_discovery.announce_leaving().await {
                warn!("Failed to announce leaving via service discovery: {:?}", e);
            }
        }

        // Update node status to indicate shutdown
        if let Some(mut node) = self.cluster_state.get_node(&self.node.node_id) {
            node.status = NodeStatus::Leaving;
            self.cluster_state.upsert_node(node);
        }

        // Update status in metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            if let Err(e) = metadata_store
                .update_node_status(&self.node.node_id, NodeStatus::Leaving)
                .await
            {
                warn!(
                    "Failed to update node status in metadata store during shutdown: {:?}",
                    e
                );
            }
        }

        // Shutdown service discovery
        if let Some(ref service_discovery) = self.service_discovery {
            if let Err(e) = service_discovery.shutdown().await {
                warn!("Failed to shutdown service discovery: {:?}", e);
            }
        }

        // Give services time to shut down gracefully
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        info!(
            "Graceful shutdown completed for node: {}",
            self.node.node_id
        );
        Ok(())
    }

    /// Get a handle to trigger shutdown
    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// Check if the service manager is running
    pub fn is_running(&self) -> bool {
        // A service manager is considered running if it has receivers
        self.shutdown_tx.receiver_count() > 0
    }

    /// Get node information
    pub fn node(&self) -> &ClusterNode {
        &self.node
    }

    /// Get cluster state
    pub fn cluster_state(&self) -> &Arc<ClusterState> {
        &self.cluster_state
    }

    /// Get metadata store
    pub fn metadata_store(&self) -> &Option<Arc<MetadataStore>> {
        &self.metadata_store
    }

    /// Get service discovery
    pub fn service_discovery(&self) -> &Option<Arc<ServiceDiscovery>> {
        &self.service_discovery
    }

    /// Add a node to the cluster dynamically
    pub async fn add_node_to_cluster(&self, node: ClusterNode) -> Result<()> {
        log_cluster_state_change!(
            "node_added",
            &self.node.node_id,
            "cluster_service_manager",
            "none",
            "active"
        );

        log_cluster_event!(
            info,
            &self.node.node_id,
            "cluster_service_manager",
            "add_node",
            "Adding node to cluster dynamically",
            target_node_id = %node.node_id,
            target_node_address = %node.address,
            target_node_role = ?node.role
        );

        // Add to cluster state (atomic operation)
        self.cluster_state.upsert_node(node.clone());

        // Add to metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            metadata_store
                .add_node(&node)
                .await
                .with_node_context(&node.node_id)
                .context("Failed to add node to metadata store")?;
        }

        info!("Successfully added node {} to cluster", node.node_id);
        Ok(())
    }

    /// Remove a node from the cluster dynamically
    pub async fn remove_node_from_cluster(&self, node_id: &str) -> Result<()> {
        info!("Removing node {} from cluster dynamically", node_id);

        // Remove from cluster state (atomic operation)
        self.cluster_state.remove_node(node_id);

        // Remove from metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            metadata_store
                .remove_node(node_id)
                .await
                .with_node_context(node_id)
                .context("Failed to remove node from metadata store")?;
        }

        info!("Successfully removed node {} from cluster", node_id);
        Ok(())
    }

    /// Update cluster topology based on discovered nodes
    pub async fn update_cluster_topology(&self) -> Result<()> {
        if let Some(ref service_discovery) = self.service_discovery {
            let discovered_nodes = service_discovery.get_discovered_nodes().await;
            let current_nodes = self.cluster_state.get_all_nodes();

            // Find new nodes to add
            for discovered_node in &discovered_nodes {
                if !current_nodes
                    .iter()
                    .any(|n| n.node_id == discovered_node.node_id)
                {
                    info!("Adding newly discovered node: {}", discovered_node.node_id);
                    self.add_node_to_cluster(discovered_node.clone()).await?;
                }
            }

            // Find nodes that are no longer discovered (but keep self)
            for current_node in &current_nodes {
                if current_node.node_id != self.node.node_id
                    && !discovered_nodes
                        .iter()
                        .any(|n| n.node_id == current_node.node_id)
                {
                    info!(
                        "Removing node that is no longer discovered: {}",
                        current_node.node_id
                    );
                    self.remove_node_from_cluster(&current_node.node_id).await?;
                }
            }
        }

        Ok(())
    }

    /// Get cluster topology information
    pub fn get_cluster_topology(&self) -> ClusterTopology {
        let all_nodes = self.cluster_state.get_all_nodes();
        let healthy_nodes = self.cluster_state.get_nodes_by_status(&NodeStatus::Healthy);
        let leader_node = all_nodes
            .iter()
            .find(|n| n.role == crate::models::NodeRole::Leader)
            .cloned();

        ClusterTopology {
            total_nodes: all_nodes.len(),
            healthy_nodes: healthy_nodes.len(),
            leader_node_id: leader_node.map(|n| n.node_id),
            nodes: all_nodes,
        }
    }
}

/// Cluster topology information
#[derive(Debug, Clone)]
pub struct ClusterTopology {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub leader_node_id: Option<String>,
    pub nodes: Vec<ClusterNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn test_service_manager_creation() {
        let config = Arc::new(Config::default());
        let node = ClusterNode::new("test-node".to_string(), "127.0.0.1".to_string(), 9090, 8080);
        let cluster_state = Arc::new(ClusterState::new());

        let service_manager = ClusterServiceManager::new(config, node.clone(), cluster_state, None);

        assert_eq!(service_manager.node().node_id, "test-node");

        // Create a receiver to make the service manager "running"
        let _receiver = service_manager.shutdown_handle().subscribe();
        assert!(service_manager.is_running());
    }

    #[tokio::test]
    async fn test_service_manager_with_metadata_store() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let config = Arc::new(Config::default());
        let node = ClusterNode::new("test-node".to_string(), "127.0.0.1".to_string(), 9090, 8080);
        let cluster_state = Arc::new(ClusterState::new());
        let metadata_store = Arc::new(MetadataStore::new(db_path).unwrap());

        let service_manager =
            ClusterServiceManager::new(config, node.clone(), cluster_state, Some(metadata_store));

        assert!(service_manager.metadata_store().is_some());
        assert!(service_manager.service_discovery().is_none());
    }

    #[tokio::test]
    async fn test_service_manager_with_service_discovery() {
        let config = Arc::new(Config::default());
        let node = ClusterNode::new("test-node".to_string(), "127.0.0.1".to_string(), 9090, 8080);
        let cluster_state = Arc::new(ClusterState::new());

        let service_manager =
            ClusterServiceManager::new(config, node.clone(), cluster_state.clone(), None);

        let service_discovery = service_manager.create_service_discovery();
        let service_manager = service_manager.with_service_discovery(service_discovery);

        assert!(service_manager.service_discovery().is_some());
    }

    #[tokio::test]
    async fn test_cluster_topology() {
        let config = Arc::new(Config::default());
        let node = ClusterNode::new("test-node".to_string(), "127.0.0.1".to_string(), 9090, 8080);
        let cluster_state = Arc::new(ClusterState::new());

        let service_manager =
            ClusterServiceManager::new(config, node.clone(), cluster_state.clone(), None);

        // Add the node to cluster state
        cluster_state.upsert_node(node);

        let topology = service_manager.get_cluster_topology();
        assert_eq!(topology.total_nodes, 1);
        assert_eq!(topology.nodes.len(), 1);
        assert_eq!(topology.nodes[0].node_id, "test-node");
    }

    #[tokio::test]
    async fn test_shutdown_handle() {
        let config = Arc::new(Config::default());
        let node = ClusterNode::new("test-node".to_string(), "127.0.0.1".to_string(), 9090, 8080);
        let cluster_state = Arc::new(ClusterState::new());

        let service_manager = ClusterServiceManager::new(config, node.clone(), cluster_state, None);

        let shutdown_handle = service_manager.shutdown_handle();

        // Create a receiver to test the channel
        let mut receiver = shutdown_handle.subscribe();

        // Test that we can send shutdown signal
        assert!(shutdown_handle.send(()).is_ok());

        // Test that we can receive the signal
        assert!(receiver.recv().await.is_ok());
    }
}
