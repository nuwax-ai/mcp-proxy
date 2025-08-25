use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{RwLock, mpsc, Mutex};
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn, error};
use uuid::Uuid;
use serde::{Serialize, Deserialize};

use crate::models::{
    ClusterNode, MetadataStore, ClusterError, NodeStatus
};
use crate::load_balancer::HealthChecker;

/// Service registration request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRegistration {
    pub service_id: String,
    pub service_name: String,
    pub node_id: String,
    pub address: String,
    pub port: u16,
    pub health_check_path: String,
    pub metadata: HashMap<String, String>,
    pub tags: Vec<String>,
}

/// Service instance information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInstance {
    pub service_id: String,
    pub service_name: String,
    pub node_id: String,
    pub address: String,
    pub port: u16,
    pub health_check_path: String,
    pub metadata: HashMap<String, String>,
    pub tags: Vec<String>,
    pub status: ServiceStatus,
    pub registered_at: SystemTime,
    pub last_health_check: Option<SystemTime>,
    pub health_check_failures: u32,
}

/// Service status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    Healthy,
    Unhealthy,
    Unknown,
    Draining,
    Maintenance,
}

/// Service discovery query
#[derive(Debug, Clone)]
pub struct ServiceQuery {
    pub service_name: Option<String>,
    pub tags: Vec<String>,
    pub node_id: Option<String>,
    pub status: Option<ServiceStatus>,
    pub healthy_only: bool,
}

/// Service manager events
#[derive(Debug, Clone)]
pub enum ServiceEvent {
    ServiceRegistered { service_id: String, node_id: String },
    ServiceDeregistered { service_id: String, node_id: String },
    ServiceHealthy { service_id: String },
    ServiceUnhealthy { service_id: String, error: String },
    NodeJoined { node_id: String, address: String },
    NodeLeft { node_id: String, reason: String },
    NodeHealthChanged { node_id: String, status: NodeStatus },
}

/// Service manager configuration
#[derive(Debug, Clone)]
pub struct ServiceManagerConfig {
    /// Interval for service health checks
    pub health_check_interval: Duration,
    /// Timeout for service health checks
    pub health_check_timeout: Duration,
    /// Maximum consecutive failures before marking service unhealthy
    pub max_health_failures: u32,
    /// Interval for node status synchronization
    pub node_sync_interval: Duration,
    /// Enable automatic service deregistration on node failure
    pub auto_deregister_on_failure: bool,
    /// Service registration TTL (time to live)
    pub service_ttl: Duration,
}

impl Default for ServiceManagerConfig {
    fn default() -> Self {
        Self {
            health_check_interval: Duration::from_secs(10),
            health_check_timeout: Duration::from_secs(5),
            max_health_failures: 3,
            node_sync_interval: Duration::from_secs(30),
            auto_deregister_on_failure: true,
            service_ttl: Duration::from_secs(300), // 5 minutes
        }
    }
}

/// Comprehensive service manager for cluster coordination
pub struct ServiceManager {
    /// Configuration
    config: ServiceManagerConfig,
    /// Metadata store for persistent data
    metadata_store: Arc<MetadataStore>,
    /// Registered services
    services: Arc<RwLock<HashMap<String, ServiceInstance>>>,
    /// Active cluster nodes
    cluster_nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
    /// Health checker for services and nodes
    #[allow(dead_code)]
    health_checker: Option<Arc<HealthChecker>>,
    /// Event sender for service events
    event_sender: mpsc::UnboundedSender<ServiceEvent>,
    /// Event receiver (for internal use)
    event_receiver: Arc<Mutex<mpsc::UnboundedReceiver<ServiceEvent>>>,
    /// Manager instance ID
    manager_id: String,
    /// Service name to instances mapping
    service_index: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    /// Node to services mapping
    node_service_index: Arc<RwLock<HashMap<String, HashSet<String>>>>,
}

impl ServiceManager {
    /// Create a new service manager
    pub fn new(
        config: ServiceManagerConfig,
        metadata_store: Arc<MetadataStore>,
        health_checker: Option<Arc<HealthChecker>>,
    ) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        
        Self {
            config,
            metadata_store,
            services: Arc::new(RwLock::new(HashMap::new())),
            cluster_nodes: Arc::new(RwLock::new(HashMap::new())),
            health_checker,
            event_sender,
            event_receiver: Arc::new(Mutex::new(event_receiver)),
            manager_id: Uuid::new_v4().to_string(),
            service_index: Arc::new(RwLock::new(HashMap::new())),
            node_service_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start the service manager
    pub async fn start(&self) -> Result<(), ClusterError> {
        info!("Starting service manager {}", self.manager_id);

        // Initialize from metadata store
        self.load_cluster_state().await?;

        // Start background tasks
        let health_check_handle = self.start_health_monitoring();
        let node_sync_handle = self.start_node_synchronization();
        let event_handle = self.start_event_processor();

        // Run all tasks concurrently
        tokio::select! {
            result = health_check_handle => {
                error!("Health monitoring stopped: {:?}", result);
            }
            result = node_sync_handle => {
                error!("Node synchronization stopped: {:?}", result);
            }
            result = event_handle => {
                error!("Event processor stopped: {:?}", result);
            }
        }

        Ok(())
    }

    /// Register a service
    pub async fn register_service(&self, registration: ServiceRegistration) -> Result<(), ClusterError> {
        info!("Registering service {} on node {}", 
              registration.service_name, registration.node_id);

        // Validate the node exists
        let nodes = self.cluster_nodes.read().await;
        if !nodes.contains_key(&registration.node_id) {
            return Err(ClusterError::NodeNotFound(format!(
                "Node {} not found in cluster", registration.node_id
            )));
        }

        let service_instance = ServiceInstance {
            service_id: registration.service_id.clone(),
            service_name: registration.service_name.clone(),
            node_id: registration.node_id.clone(),
            address: registration.address.clone(),
            port: registration.port,
            health_check_path: registration.health_check_path.clone(),
            metadata: registration.metadata.clone(),
            tags: registration.tags.clone(),
            status: ServiceStatus::Unknown,
            registered_at: SystemTime::now(),
            last_health_check: None,
            health_check_failures: 0,
        };

        // Store service
        {
            let mut services = self.services.write().await;
            services.insert(registration.service_id.clone(), service_instance);
        }

        // Update indices
        self.update_service_indices(&registration.service_id, &registration.service_name, &registration.node_id).await;

        // Send registration event
        let _ = self.event_sender.send(ServiceEvent::ServiceRegistered {
            service_id: registration.service_id.clone(),
            node_id: registration.node_id.clone(),
        });

        info!("Successfully registered service {} (ID: {})", 
              registration.service_name, registration.service_id);
        
        Ok(())
    }

    /// Deregister a service
    pub async fn deregister_service(&self, service_id: &str) -> Result<(), ClusterError> {
        info!("Deregistering service {}", service_id);

        let removed_service = {
            let mut services = self.services.write().await;
            services.remove(service_id)
        };

        if let Some(service) = removed_service {
            // Update indices
            self.remove_from_service_indices(&service_id, &service.service_name, &service.node_id).await;

            // Send deregistration event
            let _ = self.event_sender.send(ServiceEvent::ServiceDeregistered {
                service_id: service_id.to_string(),
                node_id: service.node_id.clone(),
            });

            info!("Successfully deregistered service {}", service_id);
            Ok(())
        } else {
            Err(ClusterError::InvalidOperation(format!(
                "Service {} not found", service_id
            )))
        }
    }

    /// Discover services based on query
    pub async fn discover_services(&self, query: &ServiceQuery) -> Vec<ServiceInstance> {
        let services = self.services.read().await;
        
        services.values()
            .filter(|service| self.matches_query(service, query))
            .cloned()
            .collect()
    }

    /// Get all services for a specific node
    pub async fn get_node_services(&self, node_id: &str) -> Vec<ServiceInstance> {
        let services = self.services.read().await;
        
        services.values()
            .filter(|service| service.node_id == node_id)
            .cloned()
            .collect()
    }

    /// Register a new cluster node
    pub async fn register_node(&self, node: ClusterNode) -> Result<(), ClusterError> {
        info!("Registering cluster node {} at {}", node.node_id, node.address);

        // Add to metadata store
        self.metadata_store.add_node(&node).await?;

        // Update local cache
        {
            let mut nodes = self.cluster_nodes.write().await;
            nodes.insert(node.node_id.clone(), node.clone());
        }

        // Send node joined event
        let _ = self.event_sender.send(ServiceEvent::NodeJoined {
            node_id: node.node_id.clone(),
            address: format!("{}:{}", node.address, node.http_port),
        });

        info!("Successfully registered node {}", node.node_id);
        Ok(())
    }

    /// Deregister a cluster node
    pub async fn deregister_node(&self, node_id: &str, reason: String) -> Result<(), ClusterError> {
        info!("Deregistering cluster node {}: {}", node_id, reason);

        // Remove from metadata store
        self.metadata_store.remove_node(node_id).await?;

        // Remove from local cache
        {
            let mut nodes = self.cluster_nodes.write().await;
            nodes.remove(node_id);
        }

        // Deregister all services on this node if auto-deregistration is enabled
        if self.config.auto_deregister_on_failure {
            let node_services = self.get_node_services(node_id).await;
            for service in node_services {
                if let Err(e) = self.deregister_service(&service.service_id).await {
                    warn!("Failed to auto-deregister service {}: {}", service.service_id, e);
                }
            }
        }

        // Send node left event
        let _ = self.event_sender.send(ServiceEvent::NodeLeft {
            node_id: node_id.to_string(),
            reason,
        });

        info!("Successfully deregistered node {}", node_id);
        Ok(())
    }

    /// Update node health status
    pub async fn update_node_health(&self, node_id: &str, status: NodeStatus) -> Result<(), ClusterError> {
        debug!("Updating node {} health status to {:?}", node_id, status);

        // Update metadata store
        self.metadata_store.update_node_status(node_id, status).await?;

        // Update local cache
        {
            let mut nodes = self.cluster_nodes.write().await;
            if let Some(node) = nodes.get_mut(node_id) {
                node.status = status;
                node.update_heartbeat();
            }
        }

        // Send health change event
        let _ = self.event_sender.send(ServiceEvent::NodeHealthChanged {
            node_id: node_id.to_string(),
            status,
        });

        // Handle unhealthy nodes
        if status == NodeStatus::Unhealthy && self.config.auto_deregister_on_failure {
            warn!("Node {} became unhealthy, considering auto-deregistration", node_id);
            // Note: In a production system, you might want to wait for a grace period
            // before auto-deregistering to handle temporary network issues
        }

        Ok(())
    }

    /// Get cluster health summary
    pub async fn get_cluster_summary(&self) -> ClusterSummary {
        let nodes = self.cluster_nodes.read().await;
        let services = self.services.read().await;

        let healthy_nodes = nodes.values()
            .filter(|n| n.status == NodeStatus::Healthy)
            .count();

        let healthy_services = services.values()
            .filter(|s| s.status == ServiceStatus::Healthy)
            .count();

        ClusterSummary {
            total_nodes: nodes.len(),
            healthy_nodes,
            unhealthy_nodes: nodes.len() - healthy_nodes,
            total_services: services.len(),
            healthy_services,
            unhealthy_services: services.len() - healthy_services,
            last_updated: SystemTime::now(),
        }
    }

    /// Load cluster state from metadata store
    async fn load_cluster_state(&self) -> Result<(), ClusterError> {
        info!("Loading cluster state from metadata store");

        // Load cluster nodes
        let nodes = self.metadata_store.get_all_nodes().await?;
        {
            let mut cluster_nodes = self.cluster_nodes.write().await;
            for node in nodes {
                cluster_nodes.insert(node.node_id.clone(), node);
            }
        }

        info!("Loaded {} cluster nodes", self.cluster_nodes.read().await.len());
        Ok(())
    }

    /// Start health monitoring background task
    async fn start_health_monitoring(&self) -> Result<(), ClusterError> {
        let mut interval = interval(self.config.health_check_interval);
        let services_ref = Arc::clone(&self.services);
        let event_sender = self.event_sender.clone();
        let config = self.config.clone();

        loop {
            interval.tick().await;
            
            let services = services_ref.read().await.clone();
            for (service_id, service) in services {
                // Perform health check
                let health_result = Self::check_service_health(&service, config.health_check_timeout).await;
                
                let mut services_write = services_ref.write().await;
                if let Some(service_mut) = services_write.get_mut(&service_id) {
                    service_mut.last_health_check = Some(SystemTime::now());
                    
                    if health_result {
                        service_mut.status = ServiceStatus::Healthy;
                        service_mut.health_check_failures = 0;
                        let _ = event_sender.send(ServiceEvent::ServiceHealthy {
                            service_id: service_id.clone(),
                        });
                    } else {
                        service_mut.health_check_failures += 1;
                        if service_mut.health_check_failures >= config.max_health_failures {
                            service_mut.status = ServiceStatus::Unhealthy;
                        }
                        let _ = event_sender.send(ServiceEvent::ServiceUnhealthy {
                            service_id: service_id.clone(),
                            error: "Health check failed".to_string(),
                        });
                    }
                }
            }
        }
    }

    /// Start node synchronization background task
    async fn start_node_synchronization(&self) -> Result<(), ClusterError> {
        let mut interval = interval(self.config.node_sync_interval);
        let metadata_store = Arc::clone(&self.metadata_store);
        let cluster_nodes = Arc::clone(&self.cluster_nodes);

        loop {
            interval.tick().await;
            
            // Sync cluster nodes from metadata store
            if let Ok(nodes) = metadata_store.get_all_nodes().await {
                let mut cluster_nodes_write = cluster_nodes.write().await;
                cluster_nodes_write.clear();
                for node in nodes {
                    cluster_nodes_write.insert(node.node_id.clone(), node);
                }
            }
        }
    }

    /// Start event processor background task
    async fn start_event_processor(&self) -> Result<(), ClusterError> {
        let event_receiver = Arc::clone(&self.event_receiver);
        
        loop {
            if let Some(event) = event_receiver.lock().await.recv().await {
                self.handle_service_event(event).await;
            }
        }
    }

    /// Handle service manager events
    async fn handle_service_event(&self, event: ServiceEvent) {
        match event {
            ServiceEvent::ServiceRegistered { service_id, node_id } => {
                debug!("Handled service registration: {} on node {}", service_id, node_id);
            }
            ServiceEvent::ServiceDeregistered { service_id, node_id } => {
                debug!("Handled service deregistration: {} from node {}", service_id, node_id);
            }
            ServiceEvent::ServiceHealthy { service_id } => {
                debug!("Service {} is healthy", service_id);
            }
            ServiceEvent::ServiceUnhealthy { service_id, error } => {
                warn!("Service {} is unhealthy: {}", service_id, error);
            }
            ServiceEvent::NodeJoined { node_id, address } => {
                info!("Node {} joined at {}", node_id, address);
            }
            ServiceEvent::NodeLeft { node_id, reason } => {
                info!("Node {} left: {}", node_id, reason);
            }
            ServiceEvent::NodeHealthChanged { node_id, status } => {
                debug!("Node {} health changed to {:?}", node_id, status);
            }
        }
    }

    /// Check if service matches query criteria
    fn matches_query(&self, service: &ServiceInstance, query: &ServiceQuery) -> bool {
        // Filter by service name
        if let Some(ref name) = query.service_name {
            if service.service_name != *name {
                return false;
            }
        }

        // Filter by node ID
        if let Some(ref node_id) = query.node_id {
            if service.node_id != *node_id {
                return false;
            }
        }

        // Filter by status
        if let Some(status) = query.status {
            if service.status != status {
                return false;
            }
        }

        // Filter by healthy only
        if query.healthy_only && service.status != ServiceStatus::Healthy {
            return false;
        }

        // Filter by tags
        if !query.tags.is_empty() {
            let has_all_tags = query.tags.iter()
                .all(|tag| service.tags.contains(tag));
            if !has_all_tags {
                return false;
            }
        }

        true
    }

    /// Update service indices
    async fn update_service_indices(&self, service_id: &str, service_name: &str, node_id: &str) {
        // Update service name index
        {
            let mut service_index = self.service_index.write().await;
            service_index.entry(service_name.to_string())
                .or_default()
                .insert(service_id.to_string());
        }

        // Update node service index
        {
            let mut node_service_index = self.node_service_index.write().await;
            node_service_index.entry(node_id.to_string())
                .or_default()
                .insert(service_id.to_string());
        }
    }

    /// Remove from service indices
    async fn remove_from_service_indices(&self, service_id: &str, service_name: &str, node_id: &str) {
        // Remove from service name index
        {
            let mut service_index = self.service_index.write().await;
            if let Some(service_set) = service_index.get_mut(service_name) {
                service_set.remove(service_id);
                if service_set.is_empty() {
                    service_index.remove(service_name);
                }
            }
        }

        // Remove from node service index
        {
            let mut node_service_index = self.node_service_index.write().await;
            if let Some(service_set) = node_service_index.get_mut(node_id) {
                service_set.remove(service_id);
                if service_set.is_empty() {
                    node_service_index.remove(node_id);
                }
            }
        }
    }

    /// Perform health check for a service
    async fn check_service_health(service: &ServiceInstance, timeout_duration: Duration) -> bool {
        let health_url = format!("http://{}:{}{}", 
                                service.address, 
                                service.port, 
                                service.health_check_path);

        let client = reqwest::Client::new();
        
        match timeout(timeout_duration, client.get(&health_url).send()).await {
            Ok(Ok(response)) => response.status().is_success(),
            _ => false,
        }
    }
}

/// Cluster summary information
#[derive(Debug, Clone, Serialize)]
pub struct ClusterSummary {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub unhealthy_nodes: usize,
    pub total_services: usize,
    pub healthy_services: usize,
    pub unhealthy_services: usize,
    pub last_updated: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_service_manager_creation() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());
        let config = ServiceManagerConfig::default();
        
        let manager = ServiceManager::new(config, metadata_store, None);
        assert!(!manager.manager_id.is_empty());
    }

    #[test]
    fn test_service_query_matching() {
        let service = ServiceInstance {
            service_id: "svc-1".to_string(),
            service_name: "audio-processor".to_string(),
            node_id: "node-1".to_string(),
            address: "127.0.0.1".to_string(),
            port: 8080,
            health_check_path: "/health".to_string(),
            metadata: HashMap::new(),
            tags: vec!["audio".to_string(), "transcription".to_string()],
            status: ServiceStatus::Healthy,
            registered_at: SystemTime::now(),
            last_health_check: None,
            health_check_failures: 0,
        };

        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());
        let config = ServiceManagerConfig::default();
        let manager = ServiceManager::new(config, metadata_store, None);

        // Test service name matching
        let query = ServiceQuery {
            service_name: Some("audio-processor".to_string()),
            tags: vec![],
            node_id: None,
            status: None,
            healthy_only: false,
        };
        assert!(manager.matches_query(&service, &query));

        // Test tag matching
        let query = ServiceQuery {
            service_name: None,
            tags: vec!["audio".to_string()],
            node_id: None,
            status: None,
            healthy_only: false,
        };
        assert!(manager.matches_query(&service, &query));

        // Test healthy only filter
        let query = ServiceQuery {
            service_name: None,
            tags: vec![],
            node_id: None,
            status: None,
            healthy_only: true,
        };
        assert!(manager.matches_query(&service, &query));
    }
}