use anyhow::{Context, Result};
use serde;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use tonic::transport::Channel;

use crate::load_balancer::{
    HealthCheckConfig, HealthChecker, HealthEvent, LoadBalancerService,
    ServiceManager, ServiceManagerConfig,
};
use crate::models::{LoadBalancerConfig, MetadataStore, ClusterNode, NodeRole, NodeStatus};
use crate::grpc::proto::audio_cluster_service_client::AudioClusterServiceClient;
use crate::grpc::proto::{ClusterStatusRequest, NodeInfo};

/// Main VoiceCliLoadBalancer that coordinates all load balancing functionality
pub struct VoiceCliLoadBalancer {
    /// Load balancer configuration
    #[allow(dead_code)]
    config: LoadBalancerConfig,
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
    /// Core load balancer service (HTTP proxy)
    load_balancer_service: Arc<LoadBalancerService>,
    /// Health checker for monitoring node health
    health_checker: Arc<HealthChecker>,
    /// Service manager for service discovery and registration
    service_manager: Arc<ServiceManager>,
    /// Load balancer instance ID
    instance_id: String,
    /// Health event receiver
    health_event_receiver: Option<mpsc::UnboundedReceiver<HealthEvent>>,
    /// Circuit breaker state for failed nodes
    circuit_breakers: Arc<RwLock<HashMap<String, CircuitBreakerState>>>,
    /// Request routing statistics
    routing_stats: Arc<RwLock<RoutingStats>>,
}

/// Circuit breaker state for a node
#[derive(Debug, Clone, serde::Serialize)]
pub struct CircuitBreakerState {
    /// When the circuit breaker was activated (seconds since epoch)
    #[serde(serialize_with = "serialize_instant")]
    pub activated_at: Instant,
    /// Number of consecutive failures
    pub failure_count: u32,
    /// Last failure time (seconds since epoch)
    #[serde(serialize_with = "serialize_instant")]
    pub last_failure: Instant,
    /// Circuit breaker timeout duration (in seconds)
    #[serde(serialize_with = "serialize_duration")]
    pub timeout_duration: Duration,
}

/// Serialize Instant as seconds since program start
fn serialize_instant<S>(instant: &Instant, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // Convert to elapsed seconds since program start for serialization
    let elapsed = instant.elapsed().as_secs_f64();
    serializer.serialize_f64(elapsed)
}

/// Serialize Duration as seconds
fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_f64(duration.as_secs_f64())
}

/// Routing statistics for the load balancer
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RoutingStats {
    /// Total requests routed
    pub total_requests: u64,
    /// Successful requests
    pub successful_requests: u64,
    /// Failed requests
    pub failed_requests: u64,
    /// Requests per node
    pub requests_per_node: HashMap<String, u64>,
    /// Average response time per node
    pub avg_response_time_per_node: HashMap<String, f32>,
    /// Circuit breaker activations
    pub circuit_breaker_activations: u64,
    /// Last routing decision timestamp
    pub last_routing_time: Option<SystemTime>,
}

impl VoiceCliLoadBalancer {
    /// Create a new VoiceCliLoadBalancer
    pub async fn new(
        config: LoadBalancerConfig,
        metadata_store: Arc<MetadataStore>,
    ) -> Result<Self> {
        info!(
            "Initializing VoiceCliLoadBalancer with config: {:?}",
            config
        );

        // Create health event channel
        let (health_event_sender, health_event_receiver) = mpsc::unbounded_channel();

        // Create health checker configuration
        let health_config = HealthCheckConfig {
            check_interval: Duration::from_secs(config.health_check_interval),
            request_timeout: Duration::from_secs(config.health_check_timeout),
            failure_threshold: 3,
            recovery_threshold: 2,
            health_endpoint: "/health".to_string(),
            circuit_breaker_timeout: Duration::from_secs(30),
        };

        // Create health checker
        let health_checker = Arc::new(
            HealthChecker::new(
                health_config,
                Arc::clone(&metadata_store),
                Some(health_event_sender),
            )
            .context("Failed to create health checker")?,
        );

        // Create service manager configuration
        let service_config = ServiceManagerConfig {
            health_check_interval: Duration::from_secs(config.health_check_interval),
            health_check_timeout: Duration::from_secs(config.health_check_timeout),
            max_health_failures: 3,
            node_sync_interval: Duration::from_secs(30),
            auto_deregister_on_failure: true,
            service_ttl: Duration::from_secs(300),
        };

        // Create service manager (it creates its own event channel internally)
        let service_manager = Arc::new(ServiceManager::new(
            service_config,
            Arc::clone(&metadata_store),
            Some(Arc::clone(&health_checker)),
        ));

        // ServiceManager handles its own events internally, no need for event channel

        // Create load balancer service
        let load_balancer_service = Arc::new(
            LoadBalancerService::new(config.clone(), Arc::clone(&metadata_store))
                .context("Failed to create load balancer service")?,
        );

        let instance_id = Uuid::new_v4().to_string();

        Ok(Self {
            config,
            metadata_store,
            load_balancer_service,
            health_checker,
            service_manager,
            instance_id,
            health_event_receiver: Some(health_event_receiver),
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            routing_stats: Arc::new(RwLock::new(RoutingStats::default())),
        })
    }

    /// Start the load balancer with all its components
    pub async fn start(&mut self) -> Result<()> {
        info!(
            "Starting VoiceCliLoadBalancer instance {}",
            self.instance_id
        );

        // Initialize cluster nodes from metadata store
        self.initialize_cluster_nodes()
            .await
            .context("Failed to initialize cluster nodes")?;

        // Start health checker
        let health_checker_handle = {
            let health_checker = Arc::clone(&self.health_checker);
            tokio::spawn(async move {
                if let Err(e) = health_checker.start().await {
                    error!("Health checker failed: {}", e);
                }
            })
        };

        // Start service manager
        let service_manager_handle = {
            let service_manager = Arc::clone(&self.service_manager);
            tokio::spawn(async move {
                if let Err(e) = service_manager.start().await {
                    error!("Service manager failed: {}", e);
                }
            })
        };

        // Start event processors
        let health_event_handle = self.start_health_event_processor();
        let circuit_breaker_handle = self.start_circuit_breaker_manager();

        // Start the main load balancer service
        let load_balancer_handle = {
            let load_balancer_service = Arc::clone(&self.load_balancer_service);
            tokio::spawn(async move {
                if let Err(e) = load_balancer_service.start().await {
                    error!("Load balancer service failed: {}", e);
                }
            })
        };

        info!("All VoiceCliLoadBalancer components started successfully");

        // Run all components concurrently
        tokio::select! {
            result = health_checker_handle => {
                error!("Health checker stopped: {:?}", result);
            }
            result = service_manager_handle => {
                error!("Service manager stopped: {:?}", result);
            }
            result = health_event_handle => {
                error!("Health event processor stopped: {:?}", result);
            }
            result = circuit_breaker_handle => {
                error!("Circuit breaker manager stopped: {:?}", result);
            }
            result = load_balancer_handle => {
                error!("Load balancer service stopped: {:?}", result);
            }
        }

        Ok(())
    }

    /// Initialize cluster nodes from metadata store and seed nodes
    async fn initialize_cluster_nodes(&self) -> Result<()> {
        info!("Initializing cluster nodes from metadata store and seed nodes");

        // First, try to load existing nodes from metadata store
        let mut nodes = self
            .metadata_store
            .get_all_nodes()
            .await
            .context("Failed to get cluster nodes from metadata store")?;

        info!("Found {} existing cluster nodes in metadata store", nodes.len());

        // If no nodes found in metadata store, try to discover from seed nodes
        if nodes.is_empty() && !self.config.seed_nodes.is_empty() {
            info!("No nodes in metadata store, attempting cluster discovery from {} seed nodes", self.config.seed_nodes.len());
            
            let discovered_nodes = self.discover_cluster_from_seed_nodes().await?;
            
            if !discovered_nodes.is_empty() {
                info!("Discovered {} nodes from seed nodes, adding to metadata store", discovered_nodes.len());
                
                // Add discovered nodes to metadata store
                for node in &discovered_nodes {
                    if let Err(e) = self.metadata_store.add_node(node).await {
                        warn!("Failed to add discovered node {} to metadata store: {}", node.node_id, e);
                    }
                }
                
                nodes = discovered_nodes;
            } else {
                warn!("No nodes discovered from seed nodes");
            }
        } else if !self.config.seed_nodes.is_empty() {
            info!("Found existing nodes in metadata store, skipping seed node discovery");
        }

        info!("Total available cluster nodes: {}", nodes.len());

        for node in &nodes {
            info!(
                "Available node: {} at {}:{} (role: {:?}, status: {:?})",
                node.node_id, node.address, node.http_port, node.role, node.status
            );
        }

        Ok(())
    }

    /// Discover cluster nodes from configured seed nodes
    async fn discover_cluster_from_seed_nodes(&self) -> Result<Vec<ClusterNode>> {
        let mut discovered_nodes = Vec::new();
        
        for seed_node in &self.config.seed_nodes {
            info!("Attempting to discover cluster from seed node: {}", seed_node);
            
            match self.query_cluster_status_from_seed(seed_node).await {
                Ok(nodes) => {
                    info!("Successfully discovered {} nodes from seed node {}", nodes.len(), seed_node);
                    
                    for node in nodes {
                        // Avoid duplicates
                        if !discovered_nodes.iter().any(|n: &ClusterNode| n.node_id == node.node_id) {
                            discovered_nodes.push(node);
                        }
                    }
                    
                    // If we got nodes from this seed, we can stop trying others
                    if !discovered_nodes.is_empty() {
                        break;
                    }
                }
                Err(e) => {
                    warn!("Failed to discover cluster from seed node {}: {}", seed_node, e);
                    continue;
                }
            }
        }
        
        Ok(discovered_nodes)
    }

    /// Query cluster status from a single seed node
    async fn query_cluster_status_from_seed(&self, seed_node: &str) -> Result<Vec<ClusterNode>> {
        // Parse seed node address (format: host:port)
        let parts: Vec<&str> = seed_node.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid seed node format '{}', expected 'host:port'", seed_node));
        }
        
        let host = parts[0];
        let port: u16 = parts[1].parse()
            .with_context(|| format!("Invalid port in seed node '{}'", seed_node))?;
        
        // Build gRPC endpoint
        let endpoint = format!("http://{}:{}", host, port);
        
        // Create gRPC client with timeout
        let channel = Channel::from_shared(endpoint.clone())
            .context("Failed to create gRPC channel")?
            .timeout(Duration::from_secs(5))
            .connect()
            .await
            .with_context(|| format!("Failed to connect to seed node {}", endpoint))?;
        
        let mut client = AudioClusterServiceClient::new(channel);
        
        // Create cluster status request
        let request = tonic::Request::new(ClusterStatusRequest {
            node_id: self.instance_id.clone(),
        });
        
        // Query cluster status
        let response = client
            .get_cluster_status(request)
            .await
            .with_context(|| format!("Failed to get cluster status from {}", endpoint))?;
        
        let cluster_response = response.into_inner();
        
        // Convert protobuf nodes to ClusterNode
        let mut nodes = Vec::new();
        for proto_node in cluster_response.nodes {
            match self.proto_to_cluster_node(&proto_node) {
                Ok(node) => nodes.push(node),
                Err(e) => warn!("Failed to parse node from seed {}: {}", seed_node, e),
            }
        }
        
        Ok(nodes)
    }

    /// Convert protobuf NodeInfo to ClusterNode
    fn proto_to_cluster_node(&self, proto: &NodeInfo) -> Result<ClusterNode> {
        use crate::grpc::proto::{NodeRole as ProtoRole, NodeStatus as ProtoStatus};
        
        let role = match ProtoRole::try_from(proto.role) {
            Ok(ProtoRole::Leader) => NodeRole::Leader,
            Ok(ProtoRole::Follower) => NodeRole::Follower,
            Ok(ProtoRole::Candidate) => NodeRole::Candidate,
            Err(_) => return Err(anyhow::anyhow!("Invalid node role: {}", proto.role)),
        };
        
        let status = match ProtoStatus::try_from(proto.status) {
            Ok(ProtoStatus::Healthy) => NodeStatus::Healthy,
            Ok(ProtoStatus::Unhealthy) => NodeStatus::Unhealthy,
            Ok(ProtoStatus::Joining) => NodeStatus::Joining,
            Ok(ProtoStatus::Leaving) => NodeStatus::Leaving,
            Err(_) => return Err(anyhow::anyhow!("Invalid node status: {}", proto.status)),
        };
        
        let mut node = ClusterNode::new(
            proto.node_id.clone(),
            proto.address.clone(),
            proto.grpc_port as u16,
            proto.http_port as u16,
        );
        
        node.role = role;
        node.status = status;
        node.last_heartbeat = proto.last_heartbeat;
        
        Ok(node)
    }

    /// Start health event processor
    fn start_health_event_processor(&mut self) -> tokio::task::JoinHandle<()> {
        let mut receiver = self
            .health_event_receiver
            .take()
            .expect("Health event receiver should be available");
        let circuit_breakers = Arc::clone(&self.circuit_breakers);
        let routing_stats = Arc::clone(&self.routing_stats);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                Self::handle_health_event(event, &circuit_breakers, &routing_stats).await;
            }
        })
    }

    /// Start circuit breaker manager
    fn start_circuit_breaker_manager(&self) -> tokio::task::JoinHandle<()> {
        let mut interval = interval(Duration::from_secs(10)); // Check every 10 seconds
        let circuit_breakers = Arc::clone(&self.circuit_breakers);

        tokio::spawn(async move {
            loop {
                interval.tick().await;
                Self::manage_circuit_breakers(&circuit_breakers).await;
            }
        })
    }

    /// Handle health events from the health checker
    async fn handle_health_event(
        event: HealthEvent,
        circuit_breakers: &Arc<RwLock<HashMap<String, CircuitBreakerState>>>,
        routing_stats: &Arc<RwLock<RoutingStats>>,
    ) {
        match event {
            HealthEvent::NodeHealthy {
                node_id,
                response_time,
            } => {
                debug!(
                    "Node {} is healthy (response time: {:?})",
                    node_id, response_time
                );

                // Remove circuit breaker if it exists
                {
                    let mut breakers = circuit_breakers.write().await;
                    if breakers.remove(&node_id).is_some() {
                        info!("Removed circuit breaker for recovered node {}", node_id);
                    }
                }

                // Update routing stats
                {
                    let mut stats = routing_stats.write().await;
                    let current_avg = stats
                        .avg_response_time_per_node
                        .get(&node_id)
                        .copied()
                        .unwrap_or(0.0);
                    let request_count = stats.requests_per_node.get(&node_id).copied().unwrap_or(0);

                    if request_count > 0 {
                        let total_time = current_avg * request_count as f32;
                        let new_avg =
                            (total_time + response_time.as_secs_f32()) / (request_count + 1) as f32;
                        stats.avg_response_time_per_node.insert(node_id, new_avg);
                    } else {
                        stats
                            .avg_response_time_per_node
                            .insert(node_id, response_time.as_secs_f32());
                    }
                }
            }
            HealthEvent::NodeUnhealthy { node_id, error } => {
                warn!("Node {} is unhealthy: {}", node_id, error);
            }
            HealthEvent::NodeRecovered { node_id } => {
                info!("Node {} has recovered", node_id);
            }
            HealthEvent::NodeFailed {
                node_id,
                consecutive_failures,
            } => {
                warn!(
                    "Node {} failed (consecutive failures: {})",
                    node_id, consecutive_failures
                );

                // Activate circuit breaker if threshold reached
                if consecutive_failures >= 3 {
                    let mut breakers = circuit_breakers.write().await;
                    breakers.insert(
                        node_id.clone(),
                        CircuitBreakerState {
                            activated_at: Instant::now(),
                            failure_count: consecutive_failures,
                            last_failure: Instant::now(),
                            timeout_duration: Duration::from_secs(30),
                        },
                    );

                    // Update stats
                    {
                        let mut stats = routing_stats.write().await;
                        stats.circuit_breaker_activations += 1;
                    }

                    warn!(
                        "Activated circuit breaker for node {} after {} failures",
                        node_id, consecutive_failures
                    );
                }
            }
        }
    }

    /// Manage circuit breakers (check for recovery)
    async fn manage_circuit_breakers(
        circuit_breakers: &Arc<RwLock<HashMap<String, CircuitBreakerState>>>,
    ) {
        let mut breakers = circuit_breakers.write().await;
        let mut to_remove = Vec::new();

        for (node_id, state) in breakers.iter() {
            if state.activated_at.elapsed() >= state.timeout_duration {
                debug!(
                    "Circuit breaker timeout expired for node {}, allowing retry",
                    node_id
                );
                to_remove.push(node_id.clone());
            }
        }

        for node_id in to_remove {
            breakers.remove(&node_id);
            info!(
                "Circuit breaker removed for node {} (timeout expired)",
                node_id
            );
        }
    }

    /// Get current routing statistics
    pub async fn get_routing_stats(&self) -> RoutingStats {
        self.routing_stats.read().await.clone()
    }

    /// Get circuit breaker status for all nodes
    pub async fn get_circuit_breaker_status(&self) -> HashMap<String, CircuitBreakerState> {
        self.circuit_breakers.read().await.clone()
    }

    /// Get comprehensive load balancer status
    pub async fn get_status(&self) -> LoadBalancerStatus {
        let cluster_status = self.load_balancer_service.get_cluster_status().await;
        let routing_stats = self.get_routing_stats().await;
        let circuit_breakers = self.get_circuit_breaker_status().await;
        let health_summary = self.health_checker.get_cluster_health_summary().await;
        let service_summary = self.service_manager.get_cluster_summary().await;

        LoadBalancerStatus {
            instance_id: self.instance_id.clone(),
            cluster_status,
            routing_stats,
            circuit_breakers,
            health_summary,
            service_summary,
            uptime: SystemTime::now(),
        }
    }

    /// Gracefully shutdown the load balancer
    pub async fn shutdown(&self) -> Result<()> {
        info!(
            "Shutting down VoiceCliLoadBalancer instance {}",
            self.instance_id
        );

        // In a real implementation, you would:
        // 1. Stop accepting new requests
        // 2. Wait for existing requests to complete
        // 3. Shutdown background tasks
        // 4. Clean up resources

        info!("VoiceCliLoadBalancer shutdown completed");
        Ok(())
    }
}

/// Comprehensive load balancer status
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadBalancerStatus {
    pub instance_id: String,
    pub cluster_status: crate::load_balancer::ClusterStatus,
    pub routing_stats: RoutingStats,
    pub circuit_breakers: HashMap<String, CircuitBreakerState>,
    pub health_summary: crate::load_balancer::ClusterHealthSummary,
    pub service_summary: crate::load_balancer::ClusterSummary,
    pub uptime: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_voice_cli_load_balancer_creation() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());
        let config = LoadBalancerConfig::default();

        let load_balancer = VoiceCliLoadBalancer::new(config, metadata_store).await;
        assert!(load_balancer.is_ok());
    }

    #[tokio::test]
    async fn test_routing_stats_initialization() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());
        let config = LoadBalancerConfig::default();

        let load_balancer = VoiceCliLoadBalancer::new(config, metadata_store)
            .await
            .unwrap();
        let stats = load_balancer.get_routing_stats().await;

        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.successful_requests, 0);
        assert_eq!(stats.failed_requests, 0);
    }

    #[tokio::test]
    async fn test_circuit_breaker_initialization() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());
        let config = LoadBalancerConfig::default();

        let load_balancer = VoiceCliLoadBalancer::new(config, metadata_store)
            .await
            .unwrap();
        let circuit_breakers = load_balancer.get_circuit_breaker_status().await;

        assert!(circuit_breakers.is_empty());
    }
}
