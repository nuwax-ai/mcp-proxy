use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{RwLock, mpsc};
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn, error};
use uuid::Uuid;

use crate::models::{
    ClusterNode, MetadataStore, ClusterError, NodeStatus
};

/// Health check result for a single node
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    pub node_id: String,
    pub address: String,
    pub is_healthy: bool,
    pub response_time: Option<Duration>,
    pub error_message: Option<String>,
    pub last_checked: SystemTime,
}

/// Health statistics for a node
#[derive(Debug, Clone)]
pub struct NodeHealthStats {
    pub node_id: String,
    pub total_checks: u64,
    pub successful_checks: u64,
    pub failed_checks: u64,
    pub average_response_time: f32,
    pub last_success: Option<SystemTime>,
    pub last_failure: Option<SystemTime>,
    pub consecutive_failures: u32,
    pub uptime_percentage: f32,
}

/// Health check configuration
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Interval between health checks
    pub check_interval: Duration,
    /// Timeout for each health check request
    pub request_timeout: Duration,
    /// Number of consecutive failures before marking node as unhealthy
    pub failure_threshold: u32,
    /// Number of consecutive successes to mark node as healthy again
    pub recovery_threshold: u32,
    /// Health check endpoint path
    pub health_endpoint: String,
    /// Circuit breaker timeout (how long to wait before retrying failed nodes)
    pub circuit_breaker_timeout: Duration,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(5),
            request_timeout: Duration::from_secs(3),
            failure_threshold: 3,
            recovery_threshold: 2,
            health_endpoint: "/health".to_string(),
            circuit_breaker_timeout: Duration::from_secs(30),
        }
    }
}

/// Health check events
#[derive(Debug, Clone)]
pub enum HealthEvent {
    NodeHealthy { node_id: String, response_time: Duration },
    NodeUnhealthy { node_id: String, error: String },
    NodeRecovered { node_id: String },
    NodeFailed { node_id: String, consecutive_failures: u32 },
}

/// Advanced health checker for cluster nodes
pub struct HealthChecker {
    /// Configuration for health checking
    config: HealthCheckConfig,
    /// HTTP client for health checks
    client: Client,
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
    /// Health statistics for each node
    node_stats: Arc<RwLock<HashMap<String, NodeHealthStats>>>,
    /// Recent health check results
    recent_results: Arc<RwLock<HashMap<String, HealthCheckResult>>>,
    /// Event sender for health status changes
    event_sender: Option<mpsc::UnboundedSender<HealthEvent>>,
    /// Circuit breaker state for failed nodes
    circuit_breakers: Arc<RwLock<HashMap<String, Instant>>>,
    /// Health checker instance ID
    checker_id: String,
}

impl HealthChecker {
    /// Create a new health checker
    pub fn new(
        config: HealthCheckConfig,
        metadata_store: Arc<MetadataStore>,
        event_sender: Option<mpsc::UnboundedSender<HealthEvent>>,
    ) -> Result<Self, ClusterError> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|e| ClusterError::Config(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            config,
            client,
            metadata_store,
            node_stats: Arc::new(RwLock::new(HashMap::new())),
            recent_results: Arc::new(RwLock::new(HashMap::new())),
            event_sender,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            checker_id: Uuid::new_v4().to_string(),
        })
    }

    /// Start the health checking process
    pub async fn start(&self) -> Result<(), ClusterError> {
        info!("Starting health checker {} with interval: {:?}", 
              self.checker_id, self.config.check_interval);

        let mut interval = interval(self.config.check_interval);

        loop {
            interval.tick().await;
            
            if let Err(e) = self.check_all_nodes().await {
                error!("Health check cycle failed: {}", e);
            }
        }
    }

    /// Check health of all cluster nodes
    pub async fn check_all_nodes(&self) -> Result<(), ClusterError> {
        debug!("Starting health check cycle");

        // Get all cluster nodes
        let nodes = self.metadata_store.get_all_nodes().await?;
        
        if nodes.is_empty() {
            debug!("No nodes to check");
            return Ok(());
        }

        // Perform health checks concurrently
        let mut tasks = Vec::new();
        
        for node in nodes {
            // Skip circuit-broken nodes if they're still in timeout
            if self.is_circuit_breaker_active(&node.node_id).await {
                debug!("Skipping health check for {} (circuit breaker active)", node.node_id);
                continue;
            }

            let checker = self.clone_for_task();
            let node_clone = node.clone();
            
            let task = tokio::spawn(async move {
                checker.check_single_node(&node_clone).await
            });
            
            tasks.push(task);
        }

        // Wait for all health checks to complete
        for task in tasks {
            if let Err(e) = task.await {
                warn!("Health check task failed: {}", e);
            }
        }

        debug!("Health check cycle completed");
        Ok(())
    }

    /// Check health of a single node
    pub async fn check_single_node(&self, node: &ClusterNode) -> Result<HealthCheckResult, ClusterError> {
        let start_time = Instant::now();
        let health_url = format!("http://{}:{}{}", 
                                node.address, 
                                node.http_port, 
                                self.config.health_endpoint);

        debug!("Checking health of node {} at {}", node.node_id, health_url);

        let result = match timeout(
            self.config.request_timeout,
            self.client.get(&health_url).send()
        ).await {
            Ok(Ok(response)) => {
                let response_time = start_time.elapsed();
                
                if response.status().is_success() {
                    debug!("Node {} is healthy ({}ms)", node.node_id, response_time.as_millis());
                    
                    HealthCheckResult {
                        node_id: node.node_id.clone(),
                        address: health_url,
                        is_healthy: true,
                        response_time: Some(response_time),
                        error_message: None,
                        last_checked: SystemTime::now(),
                    }
                } else {
                    let error_msg = format!("HTTP {}", response.status());
                    warn!("Node {} health check failed: {}", node.node_id, error_msg);
                    
                    HealthCheckResult {
                        node_id: node.node_id.clone(),
                        address: health_url,
                        is_healthy: false,
                        response_time: Some(response_time),
                        error_message: Some(error_msg),
                        last_checked: SystemTime::now(),
                    }
                }
            }
            Ok(Err(e)) => {
                let error_msg = format!("Request failed: {}", e);
                warn!("Node {} health check failed: {}", node.node_id, error_msg);
                
                HealthCheckResult {
                    node_id: node.node_id.clone(),
                    address: health_url,
                    is_healthy: false,
                    response_time: None,
                    error_message: Some(error_msg),
                    last_checked: SystemTime::now(),
                }
            }
            Err(_) => {
                let error_msg = "Request timeout".to_string();
                warn!("Node {} health check timed out", node.node_id);
                
                HealthCheckResult {
                    node_id: node.node_id.clone(),
                    address: health_url,
                    is_healthy: false,
                    response_time: None,
                    error_message: Some(error_msg),
                    last_checked: SystemTime::now(),
                }
            }
        };

        // Update statistics and status
        self.update_node_stats(&result).await;
        self.update_node_status(&result).await?;
        
        // Store the result
        {
            let mut recent_results = self.recent_results.write().await;
            recent_results.insert(node.node_id.clone(), result.clone());
        }

        Ok(result)
    }

    /// Update health statistics for a node
    async fn update_node_stats(&self, result: &HealthCheckResult) {
        let mut stats_map = self.node_stats.write().await;
        
        let stats = stats_map.entry(result.node_id.clone()).or_insert_with(|| {
            NodeHealthStats {
                node_id: result.node_id.clone(),
                total_checks: 0,
                successful_checks: 0,
                failed_checks: 0,
                average_response_time: 0.0,
                last_success: None,
                last_failure: None,
                consecutive_failures: 0,
                uptime_percentage: 100.0,
            }
        });

        stats.total_checks += 1;

        if result.is_healthy {
            stats.successful_checks += 1;
            stats.last_success = Some(result.last_checked);
            stats.consecutive_failures = 0;
            
            // Update average response time
            if let Some(response_time) = result.response_time {
                let total_time = stats.average_response_time * (stats.successful_checks - 1) as f32;
                stats.average_response_time = (total_time + response_time.as_secs_f32()) / stats.successful_checks as f32;
            }

            // Send health event
            if let Some(ref sender) = self.event_sender {
                let event = HealthEvent::NodeHealthy {
                    node_id: result.node_id.clone(),
                    response_time: result.response_time.unwrap_or(Duration::from_secs(0)),
                };
                let _ = sender.send(event);
            }
        } else {
            stats.failed_checks += 1;
            stats.last_failure = Some(result.last_checked);
            stats.consecutive_failures += 1;

            // Activate circuit breaker if threshold reached
            if stats.consecutive_failures >= self.config.failure_threshold {
                self.activate_circuit_breaker(&result.node_id).await;
            }

            // Send failure event
            if let Some(ref sender) = self.event_sender {
                let event = HealthEvent::NodeFailed {
                    node_id: result.node_id.clone(),
                    consecutive_failures: stats.consecutive_failures,
                };
                let _ = sender.send(event);
            }
        }

        // Update uptime percentage
        stats.uptime_percentage = (stats.successful_checks as f32 / stats.total_checks as f32) * 100.0;
    }

    /// Update node status in metadata store
    async fn update_node_status(&self, result: &HealthCheckResult) -> Result<(), ClusterError> {
        let current_status = if result.is_healthy {
            NodeStatus::Healthy
        } else {
            NodeStatus::Unhealthy
        };

        if let Err(e) = self.metadata_store.update_node_status(&result.node_id, current_status).await {
            warn!("Failed to update node {} status: {}", result.node_id, e);
        }

        Ok(())
    }

    /// Activate circuit breaker for a node
    async fn activate_circuit_breaker(&self, node_id: &str) {
        let mut circuit_breakers = self.circuit_breakers.write().await;
        circuit_breakers.insert(node_id.to_string(), Instant::now());
        warn!("Circuit breaker activated for node {}", node_id);
    }

    /// Check if circuit breaker is active for a node
    async fn is_circuit_breaker_active(&self, node_id: &str) -> bool {
        let circuit_breakers = self.circuit_breakers.read().await;
        
        if let Some(activation_time) = circuit_breakers.get(node_id) {
            activation_time.elapsed() < self.config.circuit_breaker_timeout
        } else {
            false
        }
    }

    /// Get health statistics for all nodes
    pub async fn get_all_stats(&self) -> HashMap<String, NodeHealthStats> {
        self.node_stats.read().await.clone()
    }

    /// Get health statistics for a specific node
    pub async fn get_node_stats(&self, node_id: &str) -> Option<NodeHealthStats> {
        self.node_stats.read().await.get(node_id).cloned()
    }

    /// Get recent health check results
    pub async fn get_recent_results(&self) -> HashMap<String, HealthCheckResult> {
        self.recent_results.read().await.clone()
    }

    /// Get health summary for the cluster
    pub async fn get_cluster_health_summary(&self) -> ClusterHealthSummary {
        let stats = self.node_stats.read().await;
        let total_nodes = stats.len();
        let healthy_nodes = stats.values()
            .filter(|s| s.consecutive_failures == 0)
            .count();
        
        let average_uptime = if total_nodes > 0 {
            stats.values().map(|s| s.uptime_percentage).sum::<f32>() / total_nodes as f32
        } else {
            100.0
        };

        ClusterHealthSummary {
            total_nodes,
            healthy_nodes,
            unhealthy_nodes: total_nodes - healthy_nodes,
            average_uptime_percentage: average_uptime,
            last_check_time: SystemTime::now(),
        }
    }

    /// Clone for use in async tasks
    fn clone_for_task(&self) -> Self {
        Self {
            config: self.config.clone(),
            client: self.client.clone(),
            metadata_store: Arc::clone(&self.metadata_store),
            node_stats: Arc::clone(&self.node_stats),
            recent_results: Arc::clone(&self.recent_results),
            event_sender: self.event_sender.clone(),
            circuit_breakers: Arc::clone(&self.circuit_breakers),
            checker_id: self.checker_id.clone(),
        }
    }
}

/// Cluster health summary
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClusterHealthSummary {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub unhealthy_nodes: usize,
    pub average_uptime_percentage: f32,
    pub last_check_time: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NodeRole;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_health_checker_creation() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());
        let config = HealthCheckConfig::default();
        
        let checker = HealthChecker::new(config, metadata_store, None);
        assert!(checker.is_ok());
    }

    #[tokio::test]
    async fn test_health_check_config_defaults() {
        let config = HealthCheckConfig::default();
        
        assert_eq!(config.check_interval, Duration::from_secs(5));
        assert_eq!(config.request_timeout, Duration::from_secs(3));
        assert_eq!(config.failure_threshold, 3);
        assert_eq!(config.recovery_threshold, 2);
        assert_eq!(config.health_endpoint, "/health");
    }

    #[tokio::test]
    async fn test_health_stats_initialization() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());
        let config = HealthCheckConfig::default();
        
        let checker = HealthChecker::new(config, metadata_store, None).unwrap();
        let stats = checker.get_all_stats().await;
        
        assert!(stats.is_empty());
    }
}