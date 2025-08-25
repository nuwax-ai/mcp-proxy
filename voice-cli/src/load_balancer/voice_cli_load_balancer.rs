use anyhow::{Context, Result};
use serde;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::load_balancer::{
    HealthCheckConfig, HealthChecker, HealthEvent, LoadBalancerService, ServiceEvent,
    ServiceManager, ServiceManagerConfig,
};
use crate::models::{LoadBalancerConfig, MetadataStore};

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
    /// Service event receiver
    service_event_receiver: Option<mpsc::UnboundedReceiver<ServiceEvent>>,
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

        // Create a dummy service event receiver since ServiceManager handles its own events
        let (_dummy_sender, service_event_receiver) = mpsc::unbounded_channel();

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
            service_event_receiver: Some(service_event_receiver),
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
        let service_event_handle = self.start_service_event_processor();
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
            result = service_event_handle => {
                error!("Service event processor stopped: {:?}", result);
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

    /// Initialize cluster nodes from metadata store
    async fn initialize_cluster_nodes(&self) -> Result<()> {
        info!("Initializing cluster nodes from metadata store");

        let nodes = self
            .metadata_store
            .get_all_nodes()
            .await
            .context("Failed to get cluster nodes from metadata store")?;

        info!("Found {} cluster nodes", nodes.len());

        for node in &nodes {
            info!(
                "Discovered node: {} at {}:{} (role: {:?}, status: {:?})",
                node.node_id, node.address, node.http_port, node.role, node.status
            );
        }

        Ok(())
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

    /// Start service event processor
    fn start_service_event_processor(&mut self) -> tokio::task::JoinHandle<()> {
        let mut receiver = self
            .service_event_receiver
            .take()
            .expect("Service event receiver should be available");

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                Self::handle_service_event(event).await;
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

    /// Handle service events from the service manager
    async fn handle_service_event(event: ServiceEvent) {
        match event {
            ServiceEvent::ServiceRegistered {
                service_id,
                node_id,
            } => {
                info!("Service {} registered on node {}", service_id, node_id);
            }
            ServiceEvent::ServiceDeregistered {
                service_id,
                node_id,
            } => {
                info!("Service {} deregistered from node {}", service_id, node_id);
            }
            ServiceEvent::ServiceHealthy { service_id } => {
                debug!("Service {} is healthy", service_id);
            }
            ServiceEvent::ServiceUnhealthy { service_id, error } => {
                warn!("Service {} is unhealthy: {}", service_id, error);
            }
            ServiceEvent::NodeJoined { node_id, address } => {
                info!("Node {} joined cluster at {}", node_id, address);
            }
            ServiceEvent::NodeLeft { node_id, reason } => {
                info!("Node {} left cluster: {}", node_id, reason);
            }
            ServiceEvent::NodeHealthChanged { node_id, status } => {
                debug!("Node {} health changed to {:?}", node_id, status);
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
