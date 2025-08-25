use crate::models::{ClusterError, ClusterNode, MetadataStore, NodeStatus};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::{interval, timeout};
use tracing::{debug, error, info, warn};

/// Node discovery and peer management service
pub struct NodeDiscovery {
    /// Current node information
    local_node: ClusterNode,
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
    /// HTTP client for health checks and communication
    client: Client,
    /// Known peers and their information
    peers: Arc<RwLock<HashMap<String, ClusterNode>>>,
    /// Discovery configuration
    config: DiscoveryConfig,
}

/// Configuration for node discovery
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Interval between peer discovery attempts
    pub discovery_interval: Duration,
    /// Timeout for peer health checks
    pub health_check_timeout: Duration,
    /// Maximum number of failed health checks before marking node as unhealthy
    pub max_failed_health_checks: u32,
    /// Interval between health checks
    pub health_check_interval: Duration,
    /// Bootstrap peers to connect to initially
    pub bootstrap_peers: Vec<String>, // addresses like "192.168.1.100:50051"
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            discovery_interval: Duration::from_secs(30),
            health_check_timeout: Duration::from_secs(5),
            max_failed_health_checks: 3,
            health_check_interval: Duration::from_secs(10),
            bootstrap_peers: Vec::new(),
        }
    }
}

impl NodeDiscovery {
    /// Create a new NodeDiscovery instance
    pub fn new(
        local_node: ClusterNode,
        metadata_store: Arc<MetadataStore>,
        config: DiscoveryConfig,
    ) -> Self {
        let client = Client::builder()
            .timeout(config.health_check_timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            local_node,
            metadata_store,
            client,
            peers: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Start the node discovery service
    pub async fn start(&self) -> Result<(), ClusterError> {
        info!(
            "Starting node discovery service for node {}",
            self.local_node.node_id
        );

        // Add local node to metadata store
        self.metadata_store.add_node(&self.local_node).await?;

        // Start discovery loop
        let discovery_handle = self.start_discovery_loop();

        // Start health check loop
        let health_check_handle = self.start_health_check_loop();

        // Wait for both tasks
        tokio::select! {
            result = discovery_handle => {
                if let Err(e) = result {
                    error!("Discovery loop failed: {}", e);
                }
            }
            result = health_check_handle => {
                if let Err(e) = result {
                    error!("Health check loop failed: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Start the peer discovery loop
    async fn start_discovery_loop(&self) -> Result<(), ClusterError> {
        let mut interval = interval(self.config.discovery_interval);

        loop {
            interval.tick().await;

            if let Err(e) = self.discover_peers().await {
                warn!("Peer discovery failed: {}", e);
            }
        }
    }

    /// Start the health check loop
    async fn start_health_check_loop(&self) -> Result<(), ClusterError> {
        let mut interval = interval(self.config.health_check_interval);

        loop {
            interval.tick().await;

            if let Err(e) = self.check_peer_health().await {
                warn!("Health check failed: {}", e);
            }
        }
    }

    /// Discover new peers in the cluster
    async fn discover_peers(&self) -> Result<(), ClusterError> {
        debug!("Starting peer discovery");

        // Get all known nodes from metadata store
        let known_nodes = self.metadata_store.get_all_nodes().await?;

        // Update local peer list
        let mut peers = self.peers.write().await;
        for node in known_nodes {
            if node.node_id != self.local_node.node_id {
                peers.insert(node.node_id.clone(), node);
            }
        }

        // Try to discover new peers from bootstrap addresses
        for bootstrap_addr in &self.config.bootstrap_peers {
            if let Err(e) = self.discover_from_bootstrap(bootstrap_addr).await {
                debug!(
                    "Failed to discover from bootstrap {}: {}",
                    bootstrap_addr, e
                );
            }
        }

        // Try to discover peers through existing peers (gossip-style discovery)
        let peer_list: Vec<ClusterNode> = peers.values().cloned().collect();
        drop(peers); // Release the lock

        for peer in &peer_list {
            if let Err(e) = self.discover_from_peer(peer).await {
                debug!("Failed to discover from peer {}: {}", peer.node_id, e);
            }
        }

        Ok(())
    }

    /// Discover peers from a bootstrap address
    async fn discover_from_bootstrap(&self, bootstrap_addr: &str) -> Result<(), ClusterError> {
        // Parse bootstrap address
        let parts: Vec<&str> = bootstrap_addr.split(':').collect();
        if parts.len() != 2 {
            return Err(ClusterError::Config(format!(
                "Invalid bootstrap address: {}",
                bootstrap_addr
            )));
        }

        let addr = parts[0].to_string();
        let grpc_port: u16 = parts[1].parse().map_err(|_| {
            ClusterError::Config(format!(
                "Invalid port in bootstrap address: {}",
                bootstrap_addr
            ))
        })?;

        // Try to get cluster status from bootstrap node
        let http_port = grpc_port + 1; // Assume HTTP port is gRPC port + 1
        let health_url = format!("http://{}:{}/health", addr, http_port);

        match timeout(
            self.config.health_check_timeout,
            self.client.get(&health_url).send(),
        )
        .await
        {
            Ok(Ok(response)) if response.status().is_success() => {
                // Create a temporary node entry for the bootstrap peer
                let bootstrap_node = ClusterNode::new(
                    format!("bootstrap-{}", bootstrap_addr),
                    addr,
                    grpc_port,
                    http_port,
                );

                // Try to get more peers from this node
                self.discover_from_peer(&bootstrap_node).await?;
            }
            Ok(Ok(response)) => {
                debug!(
                    "Bootstrap node {} returned status: {}",
                    bootstrap_addr,
                    response.status()
                );
            }
            Ok(Err(e)) => {
                debug!(
                    "Failed to connect to bootstrap node {}: {}",
                    bootstrap_addr, e
                );
            }
            Err(_) => {
                debug!("Timeout connecting to bootstrap node {}", bootstrap_addr);
            }
        }

        Ok(())
    }

    /// Discover peers from an existing peer
    async fn discover_from_peer(&self, peer: &ClusterNode) -> Result<(), ClusterError> {
        // Try to get cluster status from peer
        let status_url = format!("http://{}:{}/cluster/status", peer.address, peer.http_port);

        match timeout(
            self.config.health_check_timeout,
            self.client.get(&status_url).send(),
        )
        .await
        {
            Ok(Ok(response)) if response.status().is_success() => {
                // Parse cluster status response
                if let Ok(text) = response.text().await {
                    // TODO: Parse the actual cluster status response
                    // For now, just log that we got a response
                    debug!(
                        "Got cluster status from peer {}: {} bytes",
                        peer.node_id,
                        text.len()
                    );
                }
            }
            Ok(Ok(response)) => {
                debug!(
                    "Peer {} returned status: {}",
                    peer.node_id,
                    response.status()
                );
            }
            Ok(Err(e)) => {
                debug!("Failed to get status from peer {}: {}", peer.node_id, e);
            }
            Err(_) => {
                debug!("Timeout getting status from peer {}", peer.node_id);
            }
        }

        Ok(())
    }

    /// Check health of all known peers
    async fn check_peer_health(&self) -> Result<(), ClusterError> {
        let peers = self.peers.read().await.clone();

        for (node_id, peer) in peers {
            if let Err(e) = self.check_single_peer_health(&peer).await {
                debug!("Health check failed for peer {}: {}", node_id, e);

                // Update peer health status
                if let Err(e) = self
                    .metadata_store
                    .update_node_status(&node_id, NodeStatus::Unhealthy)
                    .await
                {
                    warn!(
                        "Failed to update peer {} status to unhealthy: {}",
                        node_id, e
                    );
                }
            } else {
                // Update heartbeat for healthy peer
                if let Err(e) = self.metadata_store.update_heartbeat(&node_id).await {
                    warn!("Failed to update heartbeat for peer {}: {}", node_id, e);
                }

                // Update status to healthy if it was unhealthy
                if peer.status == NodeStatus::Unhealthy {
                    if let Err(e) = self
                        .metadata_store
                        .update_node_status(&node_id, NodeStatus::Healthy)
                        .await
                    {
                        warn!("Failed to update peer {} status to healthy: {}", node_id, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Check health of a single peer
    async fn check_single_peer_health(&self, peer: &ClusterNode) -> Result<(), ClusterError> {
        let health_url = format!("http://{}:{}/health", peer.address, peer.http_port);

        match timeout(
            self.config.health_check_timeout,
            self.client.get(&health_url).send(),
        )
        .await
        {
            Ok(Ok(response)) if response.status().is_success() => {
                debug!("Peer {} is healthy", peer.node_id);
                Ok(())
            }
            Ok(Ok(response)) => Err(ClusterError::Network(format!(
                "Peer {} health check failed with status: {}",
                peer.node_id,
                response.status()
            ))),
            Ok(Err(e)) => Err(ClusterError::Network(format!(
                "Failed to connect to peer {}: {}",
                peer.node_id, e
            ))),
            Err(_) => Err(ClusterError::Timeout(format!(
                "Health check timeout for peer {}",
                peer.node_id
            ))),
        }
    }

    /// Add a new peer to the cluster
    pub async fn add_peer(&self, node: ClusterNode) -> Result<(), ClusterError> {
        info!("Adding new peer: {}", node.node_id);

        // Add to metadata store
        self.metadata_store.add_node(&node).await?;

        // Add to local peer list
        let mut peers = self.peers.write().await;
        peers.insert(node.node_id.clone(), node);

        Ok(())
    }

    /// Remove a peer from the cluster
    pub async fn remove_peer(&self, node_id: &str) -> Result<(), ClusterError> {
        info!("Removing peer: {}", node_id);

        // Remove from metadata store
        self.metadata_store.remove_node(node_id).await?;

        // Remove from local peer list
        let mut peers = self.peers.write().await;
        peers.remove(node_id);

        Ok(())
    }

    /// Get list of all known peers
    pub async fn get_peers(&self) -> HashMap<String, ClusterNode> {
        self.peers.read().await.clone()
    }

    /// Get list of healthy peers
    pub async fn get_healthy_peers(&self) -> Vec<ClusterNode> {
        self.peers
            .read()
            .await
            .values()
            .filter(|node| node.status == NodeStatus::Healthy)
            .cloned()
            .collect()
    }

    /// Join an existing cluster by connecting to a known peer
    pub async fn join_cluster(&self, peer_address: &str) -> Result<(), ClusterError> {
        info!("Attempting to join cluster via peer: {}", peer_address);

        // Parse peer address
        let parts: Vec<&str> = peer_address.split(':').collect();
        if parts.len() != 2 {
            return Err(ClusterError::Config(format!(
                "Invalid peer address: {}",
                peer_address
            )));
        }

        let addr = parts[0].to_string();
        let grpc_port: u16 = parts[1].parse().map_err(|_| {
            ClusterError::Config(format!("Invalid port in peer address: {}", peer_address))
        })?;

        // Try to contact the peer and request to join
        let http_port = grpc_port + 1; // Assume HTTP port is gRPC port + 1
        let join_url = format!("http://{}:{}/cluster/join", addr, http_port);

        // Prepare join request
        let join_request = serde_json::json!({
            "node_id": self.local_node.node_id,
            "address": self.local_node.address,
            "grpc_port": self.local_node.grpc_port,
            "http_port": self.local_node.http_port
        });

        match timeout(
            self.config.health_check_timeout,
            self.client.post(&join_url).json(&join_request).send(),
        )
        .await
        {
            Ok(Ok(response)) if response.status().is_success() => {
                info!("Successfully joined cluster via peer: {}", peer_address);

                // Add the peer to our known peers
                let peer_node =
                    ClusterNode::new(format!("peer-{}", peer_address), addr, grpc_port, http_port);

                self.add_peer(peer_node).await?;

                // Start discovery to find other peers
                self.discover_peers().await?;

                Ok(())
            }
            Ok(Ok(response)) => Err(ClusterError::Network(format!(
                "Join request failed with status: {}",
                response.status()
            ))),
            Ok(Err(e)) => Err(ClusterError::Network(format!(
                "Failed to connect to peer {}: {}",
                peer_address, e
            ))),
            Err(_) => Err(ClusterError::Timeout(format!(
                "Join request timeout for peer {}",
                peer_address
            ))),
        }
    }

    /// Leave the cluster gracefully
    pub async fn leave_cluster(&self) -> Result<(), ClusterError> {
        info!("Leaving cluster gracefully");

        // Update local node status to leaving
        self.metadata_store
            .update_node_status(&self.local_node.node_id, NodeStatus::Leaving)
            .await?;

        // Notify all peers that we're leaving
        let peers = self.get_peers().await;
        for (_, peer) in peers {
            let leave_url = format!("http://{}:{}/cluster/leave", peer.address, peer.http_port);
            let leave_request = serde_json::json!({
                "node_id": self.local_node.node_id
            });

            if let Err(e) = timeout(
                self.config.health_check_timeout,
                self.client.post(&leave_url).json(&leave_request).send(),
            )
            .await
            {
                warn!("Failed to notify peer {} of leave: {:?}", peer.node_id, e);
            }
        }

        // Remove ourselves from metadata store
        self.metadata_store
            .remove_node(&self.local_node.node_id)
            .await?;

        info!("Successfully left cluster");
        Ok(())
    }

    /// Get cluster statistics
    pub async fn get_cluster_stats(&self) -> Result<crate::models::ClusterStats, ClusterError> {
        self.metadata_store.get_cluster_stats().await
    }
}
