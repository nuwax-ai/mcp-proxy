use crate::models::{ClusterError, ClusterNode, MetadataStore, NodeRole, NodeStatus};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, info, warn};

/// Heartbeat service for cluster health monitoring
pub struct HeartbeatService {
    /// Current node information
    local_node: ClusterNode,
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
    /// Known peers and their heartbeat status
    peer_status: Arc<RwLock<HashMap<String, PeerHeartbeatStatus>>>,
    /// Heartbeat configuration
    config: HeartbeatConfig,
    /// Channel for receiving heartbeat events
    event_rx: mpsc::UnboundedReceiver<HeartbeatEvent>,
    /// Channel sender for heartbeat events (cloneable)
    event_tx: mpsc::UnboundedSender<HeartbeatEvent>,
}

/// Configuration for heartbeat service
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Interval between sending heartbeats
    pub heartbeat_interval: Duration,
    /// Timeout for considering a node as failed
    pub failure_timeout: Duration,
    /// Maximum number of missed heartbeats before marking as unhealthy
    pub max_missed_heartbeats: u32,
    /// Interval for checking peer health
    pub health_check_interval: Duration,
    /// Whether to enable heartbeat cleanup of old data
    pub enable_cleanup: bool,
    /// Cleanup interval for old heartbeat data
    pub cleanup_interval: Duration,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(3),
            failure_timeout: Duration::from_secs(15),
            max_missed_heartbeats: 5,
            health_check_interval: Duration::from_secs(5),
            enable_cleanup: true,
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

/// Heartbeat events that can be processed
#[derive(Debug)]
pub enum HeartbeatEvent {
    /// Received heartbeat from a peer
    PeerHeartbeat {
        node_id: String,
        status: NodeStatus,
        role: NodeRole,
        timestamp: i64,
    },
    /// Send heartbeat to all peers
    SendHeartbeat,
    /// Check health of all peers
    CheckPeerHealth,
    /// Clean up old heartbeat data
    Cleanup,
    /// Shutdown the heartbeat service
    Shutdown,
}

/// Status tracking for peer heartbeats
#[derive(Debug, Clone)]
struct PeerHeartbeatStatus {
    /// Last received heartbeat timestamp
    last_heartbeat: Instant,
    /// Number of consecutive missed heartbeats
    missed_count: u32,
    /// Current node status
    status: NodeStatus,
    /// Current node role
    role: NodeRole,
    /// Last known timestamp from peer
    peer_timestamp: i64,
}

impl PeerHeartbeatStatus {
    fn new(status: NodeStatus, role: NodeRole, timestamp: i64) -> Self {
        Self {
            last_heartbeat: Instant::now(),
            missed_count: 0,
            status,
            role,
            peer_timestamp: timestamp,
        }
    }

    fn update(&mut self, status: NodeStatus, role: NodeRole, timestamp: i64) {
        self.last_heartbeat = Instant::now();
        self.missed_count = 0;
        self.status = status;
        self.role = role;
        self.peer_timestamp = timestamp;
    }

    fn mark_missed(&mut self) {
        self.missed_count += 1;
    }

    fn is_healthy(&self, failure_timeout: Duration, max_missed: u32) -> bool {
        self.last_heartbeat.elapsed() < failure_timeout && self.missed_count < max_missed
    }
}

impl HeartbeatService {
    /// Create a new HeartbeatService
    pub fn new(
        local_node: ClusterNode,
        metadata_store: Arc<MetadataStore>,
        config: HeartbeatConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            local_node,
            metadata_store,
            peer_status: Arc::new(RwLock::new(HashMap::new())),
            config,
            event_rx,
            event_tx,
        }
    }

    /// Get a cloneable event sender for external use
    pub fn event_sender(&self) -> mpsc::UnboundedSender<HeartbeatEvent> {
        self.event_tx.clone()
    }

    /// Start the heartbeat service
    pub async fn start(&mut self) -> Result<(), ClusterError> {
        info!(
            "Starting heartbeat service for node {}",
            self.local_node.node_id
        );

        // Initialize peer status from metadata store
        self.initialize_peer_status().await?;

        // Clone necessary data for async tasks
        let event_tx = self.event_tx.clone();
        let config = self.config.clone();

        // Start heartbeat timer
        let heartbeat_handle = {
            let event_tx = event_tx.clone();
            let heartbeat_interval = config.heartbeat_interval;
            tokio::spawn(async move {
                let mut interval = interval(heartbeat_interval);
                loop {
                    interval.tick().await;
                    if event_tx.send(HeartbeatEvent::SendHeartbeat).is_err() {
                        warn!("Failed to send heartbeat event - channel closed");
                        break;
                    }
                }
            })
        };

        // Start health check timer
        let health_check_handle = {
            let event_tx = event_tx.clone();
            let health_check_interval = config.health_check_interval;
            tokio::spawn(async move {
                let mut interval = interval(health_check_interval);
                loop {
                    interval.tick().await;
                    if event_tx.send(HeartbeatEvent::CheckPeerHealth).is_err() {
                        warn!("Failed to send health check event - channel closed");
                        break;
                    }
                }
            })
        };

        // Start cleanup timer if enabled
        let cleanup_handle = if config.enable_cleanup {
            let event_tx = event_tx.clone();
            let cleanup_interval = config.cleanup_interval;
            Some(tokio::spawn(async move {
                let mut interval = interval(cleanup_interval);
                loop {
                    interval.tick().await;
                    if event_tx.send(HeartbeatEvent::Cleanup).is_err() {
                        warn!("Failed to send cleanup event - channel closed");
                        break;
                    }
                }
            }))
        } else {
            None
        };

        // Main event loop
        // Run the actual event loop in the current task
        tokio::select! {
            _ = self.run_event_loop() => {
                info!("Event loop completed");
            }
            _ = heartbeat_handle => {
                warn!("Heartbeat timer stopped");
            }
            _ = health_check_handle => {
                warn!("Health check timer stopped");
            }
            _ = async {
                if let Some(handle) = cleanup_handle {
                    handle.await.unwrap_or_else(|_| warn!("Cleanup timer failed"));
                }
            } => {
                warn!("Cleanup timer stopped");
            }
        }

        Ok(())
    }

    /// Initialize peer status from metadata store
    async fn initialize_peer_status(&self) -> Result<(), ClusterError> {
        let nodes = self.metadata_store.get_all_nodes().await?;
        let mut peer_status = self.peer_status.write().await;

        for node in nodes {
            if node.node_id != self.local_node.node_id {
                let status = PeerHeartbeatStatus::new(node.status, node.role, node.last_heartbeat);
                peer_status.insert(node.node_id, status);
            }
        }

        info!(
            "Initialized heartbeat status for {} peers",
            peer_status.len()
        );
        Ok(())
    }

    /// Run the main event loop
    async fn run_event_loop(&mut self) -> Result<(), ClusterError> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                HeartbeatEvent::PeerHeartbeat {
                    node_id,
                    status,
                    role,
                    timestamp,
                } => {
                    self.handle_peer_heartbeat(node_id, status, role, timestamp)
                        .await?;
                }
                HeartbeatEvent::SendHeartbeat => {
                    self.send_heartbeat().await?;
                }
                HeartbeatEvent::CheckPeerHealth => {
                    self.check_peer_health().await?;
                }
                HeartbeatEvent::Cleanup => {
                    self.cleanup_old_data().await?;
                }
                HeartbeatEvent::Shutdown => {
                    info!("Shutting down heartbeat service");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handle received heartbeat from a peer
    async fn handle_peer_heartbeat(
        &self,
        node_id: String,
        status: NodeStatus,
        role: NodeRole,
        timestamp: i64,
    ) -> Result<(), ClusterError> {
        debug!("Received heartbeat from peer {}", node_id);

        // Update local peer status
        {
            let mut peer_status = self.peer_status.write().await;
            match peer_status.get_mut(&node_id) {
                Some(status_entry) => {
                    status_entry.update(status, role, timestamp);
                }
                None => {
                    // New peer discovered
                    let status_entry = PeerHeartbeatStatus::new(status, role, timestamp);
                    peer_status.insert(node_id.clone(), status_entry);
                    info!("Discovered new peer: {}", node_id);
                }
            }
        }

        // Update metadata store
        self.metadata_store.update_heartbeat(&node_id).await?;
        self.metadata_store
            .update_node_status(&node_id, status)
            .await?;
        self.metadata_store.update_node_role(&node_id, role).await?;

        Ok(())
    }

    /// Send heartbeat to all peers
    async fn send_heartbeat(&self) -> Result<(), ClusterError> {
        // Update our own heartbeat in metadata store
        self.metadata_store
            .update_heartbeat(&self.local_node.node_id)
            .await?;

        debug!("Sending heartbeat from node {}", self.local_node.node_id);

        // Get all known peers from metadata store
        let all_nodes = self
            .metadata_store
            .get_all_nodes()
            .await
            .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;

        // Send heartbeat to all peers (exclude self)
        for node in all_nodes {
            if node.node_id != self.local_node.node_id {
                // Spawn concurrent heartbeat sending to avoid blocking
                let node_clone = node.clone();
                let local_node_clone = self.local_node.clone();

                tokio::spawn(async move {
                    if let Err(e) =
                        Self::send_heartbeat_to_peer(&local_node_clone, &node_clone).await
                    {
                        debug!("Failed to send heartbeat to {}: {}", node_clone.node_id, e);
                    }
                });
            }
        }

        Ok(())
    }

    /// Send heartbeat to a specific peer via gRPC
    async fn send_heartbeat_to_peer(
        local_node: &ClusterNode,
        peer_node: &ClusterNode,
    ) -> Result<(), ClusterError> {
        use crate::grpc::client::AudioClusterClient;

        let peer_address = format!("http://{}:{}", peer_node.address, peer_node.grpc_port);

        // Create gRPC client with timeout
        let mut client = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            AudioClusterClient::connect(&peer_address, None),
        )
        .await
        {
            Ok(Ok(client)) => client,
            Ok(Err(e)) => {
                debug!("Failed to connect to peer {}: {}", peer_node.node_id, e);
                return Err(ClusterError::Network(format!("Connection failed: {}", e)));
            }
            Err(_) => {
                debug!("Connection timeout to peer {}", peer_node.node_id);
                return Err(ClusterError::Network("Connection timeout".to_string()));
            }
        };

        // Send heartbeat request
        // TODO: Fix type conversion between models::NodeStatus and proto::NodeStatus
        // For now, convert to proto type for gRPC call
        let proto_status = match local_node.status {
            NodeStatus::Healthy => crate::grpc::proto::NodeStatus::Healthy,
            NodeStatus::Unhealthy => crate::grpc::proto::NodeStatus::Unhealthy,
            NodeStatus::Leaving => crate::grpc::proto::NodeStatus::Leaving,
            NodeStatus::Joining => crate::grpc::proto::NodeStatus::Joining,
        };

        let result = client
            .send_heartbeat(
                &local_node.node_id,
                proto_status,
                chrono::Utc::now().timestamp(),
            )
            .await;

        match result {
            Ok(response) => {
                if response.success {
                    debug!("Heartbeat sent successfully to {}", peer_node.node_id);
                } else {
                    debug!(
                        "Heartbeat rejected by {}: {}",
                        peer_node.node_id, response.message
                    );
                }
                Ok(())
            }
            Err(e) => {
                debug!("gRPC heartbeat failed to {}: {}", peer_node.node_id, e);
                Err(e)
            }
        }
    }

    /// Check health of all peers
    async fn check_peer_health(&self) -> Result<(), ClusterError> {
        let mut unhealthy_peers = Vec::new();
        let mut healthy_peers = Vec::new();

        {
            let mut peer_status = self.peer_status.write().await;

            for (node_id, status) in peer_status.iter_mut() {
                if !status.is_healthy(
                    self.config.failure_timeout,
                    self.config.max_missed_heartbeats,
                ) {
                    if status.status != NodeStatus::Unhealthy {
                        unhealthy_peers.push(node_id.clone());
                        status.status = NodeStatus::Unhealthy;
                    }
                    status.mark_missed();
                } else if status.status == NodeStatus::Unhealthy {
                    // Peer recovered
                    healthy_peers.push(node_id.clone());
                    status.status = NodeStatus::Healthy;
                }
            }
        }

        // Update metadata store for unhealthy peers
        for node_id in unhealthy_peers {
            warn!(
                "Marking peer {} as unhealthy due to missed heartbeats",
                node_id
            );
            self.metadata_store
                .update_node_status(&node_id, NodeStatus::Unhealthy)
                .await?;
        }

        // Update metadata store for recovered peers
        for node_id in healthy_peers {
            info!("Peer {} recovered and is now healthy", node_id);
            self.metadata_store
                .update_node_status(&node_id, NodeStatus::Healthy)
                .await?;
        }

        Ok(())
    }

    /// Clean up old heartbeat data
    async fn cleanup_old_data(&self) -> Result<(), ClusterError> {
        debug!("Cleaning up old heartbeat data");

        let cutoff_time = Instant::now() - (self.config.failure_timeout * 3); // Keep data 3x failure timeout
        let mut removed_peers = Vec::new();

        {
            let mut peer_status = self.peer_status.write().await;
            peer_status.retain(|node_id, status| {
                if status.last_heartbeat < cutoff_time && status.status == NodeStatus::Unhealthy {
                    removed_peers.push(node_id.clone());
                    false
                } else {
                    true
                }
            });
        }

        // Remove old unhealthy peers from metadata store
        for node_id in removed_peers {
            info!("Removing old unhealthy peer: {}", node_id);
            if let Err(e) = self.metadata_store.remove_node(&node_id).await {
                warn!("Failed to remove old peer {}: {}", node_id, e);
            }
        }

        Ok(())
    }

    /// Get heartbeat status for all peers
    pub async fn get_peer_status(&self) -> HashMap<String, (NodeStatus, NodeRole, Duration)> {
        let peer_status = self.peer_status.read().await;
        let mut result = HashMap::new();

        for (node_id, status) in peer_status.iter() {
            result.insert(
                node_id.clone(),
                (status.status, status.role, status.last_heartbeat.elapsed()),
            );
        }

        result
    }

    /// Check if a specific peer is healthy
    pub async fn is_peer_healthy(&self, node_id: &str) -> bool {
        let peer_status = self.peer_status.read().await;

        peer_status
            .get(node_id)
            .map(|status| {
                status.is_healthy(
                    self.config.failure_timeout,
                    self.config.max_missed_heartbeats,
                )
            })
            .unwrap_or(false)
    }

    /// Get list of healthy peers
    pub async fn get_healthy_peers(&self) -> Vec<String> {
        let peer_status = self.peer_status.read().await;

        peer_status
            .iter()
            .filter(|(_, status)| {
                status.is_healthy(
                    self.config.failure_timeout,
                    self.config.max_missed_heartbeats,
                )
            })
            .map(|(node_id, _)| node_id.clone())
            .collect()
    }

    /// Get the current leader node ID if any
    pub async fn get_leader_node(&self) -> Option<String> {
        let peer_status = self.peer_status.read().await;

        // Check if local node is leader
        if self.local_node.role == NodeRole::Leader {
            return Some(self.local_node.node_id.clone());
        }

        // Check peers for leader
        peer_status
            .iter()
            .find(|(_, status)| {
                status.role == NodeRole::Leader
                    && status.is_healthy(
                        self.config.failure_timeout,
                        self.config.max_missed_heartbeats,
                    )
            })
            .map(|(node_id, _)| node_id.clone())
    }

    /// Force update a peer's status (for external integrations)
    pub async fn update_peer_status(
        &self,
        node_id: String,
        status: NodeStatus,
        role: NodeRole,
    ) -> Result<(), ClusterError> {
        let timestamp = Utc::now().timestamp();

        let event = HeartbeatEvent::PeerHeartbeat {
            node_id,
            status,
            role,
            timestamp,
        };

        self.event_tx.send(event).map_err(|_| {
            ClusterError::InvalidOperation("Failed to send heartbeat event".to_string())
        })?;

        Ok(())
    }

    /// Shutdown the heartbeat service gracefully
    pub async fn shutdown(&self) -> Result<(), ClusterError> {
        self.event_tx.send(HeartbeatEvent::Shutdown).map_err(|_| {
            ClusterError::InvalidOperation("Failed to send shutdown event".to_string())
        })?;

        Ok(())
    }
}
