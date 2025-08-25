use crate::models::{ClusterNode, ClusterError, NodeRole, NodeStatus};
use crate::cluster::ClusterState;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, info, warn};
use serde::{Deserialize, Serialize};
use chrono::Utc;

/// Service discovery configuration
#[derive(Debug, Clone)]
pub struct ServiceDiscoveryConfig {
    /// Multicast address for discovery
    pub multicast_addr: IpAddr,
    /// Port for discovery messages
    pub discovery_port: u16,
    /// Interval between discovery broadcasts
    pub broadcast_interval: Duration,
    /// Timeout for discovery responses
    pub response_timeout: Duration,
    /// Maximum number of discovery attempts
    pub max_discovery_attempts: u32,
    /// Enable automatic node discovery
    pub enable_discovery: bool,
    /// Discovery message TTL
    pub message_ttl: u8,
}

impl Default for ServiceDiscoveryConfig {
    fn default() -> Self {
        Self {
            multicast_addr: IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), // mDNS multicast
            discovery_port: 5353,
            broadcast_interval: Duration::from_secs(30),
            response_timeout: Duration::from_secs(5),
            max_discovery_attempts: 3,
            enable_discovery: true,
            message_ttl: 64,
        }
    }
}

/// Discovery message types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiscoveryMessage {
    /// Announce node presence
    Announce {
        node_info: DiscoveryNodeInfo,
        timestamp: i64,
    },
    /// Query for available nodes
    Query {
        requester_id: String,
        timestamp: i64,
    },
    /// Response to query with node information
    Response {
        node_info: DiscoveryNodeInfo,
        timestamp: i64,
    },
    /// Node leaving notification
    Leave {
        node_id: String,
        timestamp: i64,
    },
}

/// Node information for discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryNodeInfo {
    pub node_id: String,
    pub address: String,
    pub grpc_port: u16,
    pub http_port: u16,
    pub role: String,
    pub status: String,
    pub capabilities: Vec<String>,
}

impl From<&ClusterNode> for DiscoveryNodeInfo {
    fn from(node: &ClusterNode) -> Self {
        Self {
            node_id: node.node_id.clone(),
            address: node.address.clone(),
            grpc_port: node.grpc_port,
            http_port: node.http_port,
            role: match node.role {
                NodeRole::Leader => "leader".to_string(),
                NodeRole::Follower => "follower".to_string(),
                NodeRole::Candidate => "candidate".to_string(),
            },
            status: match node.status {
                NodeStatus::Healthy => "healthy".to_string(),
                NodeStatus::Unhealthy => "unhealthy".to_string(),
                NodeStatus::Joining => "joining".to_string(),
                NodeStatus::Leaving => "leaving".to_string(),
            },
            capabilities: vec!["transcription".to_string(), "cluster".to_string()],
        }
    }
}

impl TryFrom<DiscoveryNodeInfo> for ClusterNode {
    type Error = ClusterError;

    fn try_from(info: DiscoveryNodeInfo) -> Result<Self, Self::Error> {
        let role = match info.role.as_str() {
            "leader" => NodeRole::Leader,
            "follower" => NodeRole::Follower,
            "candidate" => NodeRole::Candidate,
            _ => return Err(ClusterError::InvalidOperation("Invalid node role".to_string())),
        };

        let status = match info.status.as_str() {
            "healthy" => NodeStatus::Healthy,
            "unhealthy" => NodeStatus::Unhealthy,
            "joining" => NodeStatus::Joining,
            "leaving" => NodeStatus::Leaving,
            _ => return Err(ClusterError::InvalidOperation("Invalid node status".to_string())),
        };

        let mut node = ClusterNode::new(
            info.node_id,
            info.address,
            info.grpc_port,
            info.http_port,
        );
        node.role = role;
        node.status = status;

        Ok(node)
    }
}

/// Service discovery events
#[derive(Debug)]
pub enum DiscoveryEvent {
    /// Node discovered
    NodeDiscovered { node: ClusterNode },
    /// Node left
    NodeLeft { node_id: String },
    /// Discovery query received
    QueryReceived { requester_id: String },
    /// Start discovery process
    StartDiscovery,
    /// Stop discovery process
    StopDiscovery,
    /// Shutdown service discovery
    Shutdown,
}

/// Service discovery service for automatic node detection
pub struct ServiceDiscovery {
    /// Local node information
    local_node: ClusterNode,
    /// Cluster state for node management
    cluster_state: Arc<ClusterState>,
    /// Service discovery configuration
    config: ServiceDiscoveryConfig,
    /// UDP socket for discovery messages
    socket: Option<Arc<UdpSocket>>,
    /// Discovered nodes cache
    discovered_nodes: Arc<RwLock<HashMap<String, (ClusterNode, i64)>>>,
    /// Event channel receiver
    event_rx: mpsc::UnboundedReceiver<DiscoveryEvent>,
    /// Event channel sender (cloneable)
    event_tx: mpsc::UnboundedSender<DiscoveryEvent>,
    /// Discovery active flag
    discovery_active: Arc<RwLock<bool>>,
}

impl ServiceDiscovery {
    /// Create a new ServiceDiscovery instance
    pub fn new(
        local_node: ClusterNode,
        cluster_state: Arc<ClusterState>,
        config: ServiceDiscoveryConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            local_node,
            cluster_state,
            config,
            socket: None,
            discovered_nodes: Arc::new(RwLock::new(HashMap::new())),
            event_rx,
            event_tx,
            discovery_active: Arc::new(RwLock::new(false)),
        }
    }

    /// Get event sender for external use
    pub fn event_sender(&self) -> mpsc::UnboundedSender<DiscoveryEvent> {
        self.event_tx.clone()
    }

    /// Start the service discovery
    pub async fn start(&mut self) -> Result<(), ClusterError> {
        if !self.config.enable_discovery {
            info!("Service discovery is disabled");
            return Ok(());
        }

        info!("Starting service discovery for node: {}", self.local_node.node_id);

        // Initialize UDP socket
        self.initialize_socket().await?;

        // Set discovery as active
        *self.discovery_active.write().await = true;

        // Clone necessary data for async tasks
        let socket = self.socket.clone();
        let config = self.config.clone();
        let discovered_nodes = self.discovered_nodes.clone();
        let event_tx = self.event_tx.clone();
        let discovery_active = self.discovery_active.clone();
        let local_node = self.local_node.clone();

        // Start discovery processes
        tokio::select! {
            _ = self.run_event_loop() => {
                info!("Service discovery event loop completed");
            }
            _ = Self::run_discovery_broadcast_static(socket.clone(), config.clone(), discovery_active.clone(), local_node.clone()) => {
                info!("Service discovery broadcast completed");
            }
            _ = Self::run_discovery_listener_static(socket.clone(), event_tx.clone()) => {
                info!("Service discovery listener completed");
            }
            _ = Self::run_cleanup_task_static(discovered_nodes.clone(), event_tx.clone()) => {
                info!("Service discovery cleanup completed");
            }
        }

        Ok(())
    }

    /// Initialize UDP socket for discovery
    async fn initialize_socket(&mut self) -> Result<(), ClusterError> {
        let bind_addr = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            self.config.discovery_port,
        );

        let socket = UdpSocket::bind(bind_addr).await
            .map_err(|e| ClusterError::Network(format!("Failed to bind discovery socket: {}", e)))?;

        // Enable broadcast
        socket.set_broadcast(true)
            .map_err(|e| ClusterError::Network(format!("Failed to enable broadcast: {}", e)))?;

        self.socket = Some(Arc::new(socket));
        info!("Service discovery socket initialized on {}", bind_addr);

        Ok(())
    }

    /// Run the main event loop
    async fn run_event_loop(&mut self) -> Result<(), ClusterError> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                DiscoveryEvent::NodeDiscovered { node } => {
                    self.handle_node_discovered(node).await?;
                }
                DiscoveryEvent::NodeLeft { node_id } => {
                    self.handle_node_left(node_id).await?;
                }
                DiscoveryEvent::QueryReceived { requester_id } => {
                    self.handle_query_received(requester_id).await?;
                }
                DiscoveryEvent::StartDiscovery => {
                    *self.discovery_active.write().await = true;
                    info!("Service discovery started");
                }
                DiscoveryEvent::StopDiscovery => {
                    *self.discovery_active.write().await = false;
                    info!("Service discovery stopped");
                }
                DiscoveryEvent::Shutdown => {
                    info!("Shutting down service discovery");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run discovery broadcast loop (static version)
    async fn run_discovery_broadcast_static(
        socket: Option<Arc<UdpSocket>>,
        config: ServiceDiscoveryConfig,
        discovery_active: Arc<RwLock<bool>>,
        local_node: ClusterNode,
    ) -> Result<(), ClusterError> {
        let mut interval = interval(config.broadcast_interval);
        let socket = socket.ok_or_else(|| {
            ClusterError::InvalidOperation("Discovery socket not initialized".to_string())
        })?;

        loop {
            interval.tick().await;

            if !*discovery_active.read().await {
                continue;
            }

            // Broadcast node announcement
            if let Err(e) = Self::broadcast_announcement_static(&socket, &local_node, &config).await {
                warn!("Failed to broadcast announcement: {}", e);
            }

            // Periodically send discovery queries
            if let Err(e) = Self::broadcast_query_static(&socket, &local_node, &config).await {
                warn!("Failed to broadcast query: {}", e);
            }
        }
    }

    /// Run discovery broadcast loop
    #[allow(dead_code)]
    async fn run_discovery_broadcast(&self) -> Result<(), ClusterError> {
        let mut interval = interval(self.config.broadcast_interval);
        let socket = self.socket.as_ref().ok_or_else(|| {
            ClusterError::InvalidOperation("Discovery socket not initialized".to_string())
        })?;

        loop {
            interval.tick().await;

            if !*self.discovery_active.read().await {
                continue;
            }

            // Broadcast node announcement
            if let Err(e) = self.broadcast_announcement(socket).await {
                warn!("Failed to broadcast announcement: {}", e);
            }

            // Periodically send discovery queries
            if let Err(e) = self.broadcast_query(socket).await {
                warn!("Failed to broadcast query: {}", e);
            }
        }
    }

    /// Run discovery message listener (static version)
    async fn run_discovery_listener_static(
        socket: Option<Arc<UdpSocket>>,
        event_tx: mpsc::UnboundedSender<DiscoveryEvent>,
    ) -> Result<(), ClusterError> {
        let socket = socket.ok_or_else(|| {
            ClusterError::InvalidOperation("Discovery socket not initialized".to_string())
        })?;

        let mut buffer = [0u8; 1024];

        loop {
            match socket.recv_from(&mut buffer).await {
                Ok((len, addr)) => {
                    if let Err(e) = Self::handle_discovery_message_static(&buffer[..len], addr, &event_tx).await {
                        debug!("Failed to handle discovery message from {}: {}", addr, e);
                    }
                }
                Err(e) => {
                    warn!("Failed to receive discovery message: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Run discovery message listener
    #[allow(dead_code)]
    async fn run_discovery_listener(&self) -> Result<(), ClusterError> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            ClusterError::InvalidOperation("Discovery socket not initialized".to_string())
        })?;

        let mut buffer = [0u8; 1024];

        loop {
            match socket.recv_from(&mut buffer).await {
                Ok((len, addr)) => {
                    if let Err(e) = self.handle_discovery_message(&buffer[..len], addr).await {
                        debug!("Failed to handle discovery message from {}: {}", addr, e);
                    }
                }
                Err(e) => {
                    warn!("Failed to receive discovery message: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Run cleanup task for stale nodes (static version)
    async fn run_cleanup_task_static(
        discovered_nodes: Arc<RwLock<HashMap<String, (ClusterNode, i64)>>>,
        event_tx: mpsc::UnboundedSender<DiscoveryEvent>,
    ) -> Result<(), ClusterError> {
        let mut interval = interval(Duration::from_secs(60)); // Cleanup every minute
        let stale_timeout = 300; // 5 minutes

        loop {
            interval.tick().await;

            let current_time = Utc::now().timestamp();
            let mut nodes_to_remove = Vec::new();

            // Find stale nodes
            {
                let discovered_nodes_guard = discovered_nodes.read().await;
                for (node_id, (_, last_seen)) in discovered_nodes_guard.iter() {
                    if current_time - last_seen > stale_timeout {
                        nodes_to_remove.push(node_id.clone());
                    }
                }
            }

            // Remove stale nodes
            for node_id in nodes_to_remove {
                info!("Removing stale discovered node: {}", node_id);
                
                {
                    let mut discovered_nodes_guard = discovered_nodes.write().await;
                    discovered_nodes_guard.remove(&node_id);
                }

                // Send node left event
                if let Err(_) = event_tx.send(DiscoveryEvent::NodeLeft { node_id }) {
                    warn!("Failed to send node left event");
                }
            }
        }
    }

    /// Run cleanup task for stale nodes
    #[allow(dead_code)]
    async fn run_cleanup_task(&self) -> Result<(), ClusterError> {
        let mut interval = interval(Duration::from_secs(60)); // Cleanup every minute
        let stale_timeout = 300; // 5 minutes

        loop {
            interval.tick().await;

            let current_time = Utc::now().timestamp();
            let mut nodes_to_remove = Vec::new();

            // Find stale nodes
            {
                let discovered_nodes = self.discovered_nodes.read().await;
                for (node_id, (_, last_seen)) in discovered_nodes.iter() {
                    if current_time - last_seen > stale_timeout {
                        nodes_to_remove.push(node_id.clone());
                    }
                }
            }

            // Remove stale nodes
            for node_id in nodes_to_remove {
                info!("Removing stale discovered node: {}", node_id);
                
                {
                    let mut discovered_nodes = self.discovered_nodes.write().await;
                    discovered_nodes.remove(&node_id);
                }

                // Send node left event
                if let Err(_) = self.event_tx.send(DiscoveryEvent::NodeLeft { node_id }) {
                    warn!("Failed to send node left event");
                }
            }
        }
    }

    /// Broadcast node announcement (static version)
    async fn broadcast_announcement_static(
        socket: &UdpSocket,
        local_node: &ClusterNode,
        config: &ServiceDiscoveryConfig,
    ) -> Result<(), ClusterError> {
        let message = DiscoveryMessage::Announce {
            node_info: DiscoveryNodeInfo::from(local_node),
            timestamp: Utc::now().timestamp(),
        };

        let data = serde_json::to_vec(&message)
            .map_err(|e| ClusterError::Serialization(e))?;

        let broadcast_addr = SocketAddr::new(
            config.multicast_addr,
            config.discovery_port,
        );

        socket.send_to(&data, broadcast_addr).await
            .map_err(|e| ClusterError::Network(format!("Failed to send announcement: {}", e)))?;

        debug!("Broadcasted node announcement");
        Ok(())
    }

    /// Broadcast node announcement
    #[allow(dead_code)]
    async fn broadcast_announcement(&self, socket: &UdpSocket) -> Result<(), ClusterError> {
        let message = DiscoveryMessage::Announce {
            node_info: DiscoveryNodeInfo::from(&self.local_node),
            timestamp: Utc::now().timestamp(),
        };

        let data = serde_json::to_vec(&message)
            .map_err(|e| ClusterError::Serialization(e))?;

        let broadcast_addr = SocketAddr::new(
            self.config.multicast_addr,
            self.config.discovery_port,
        );

        socket.send_to(&data, broadcast_addr).await
            .map_err(|e| ClusterError::Network(format!("Failed to send announcement: {}", e)))?;

        debug!("Broadcasted node announcement");
        Ok(())
    }

    /// Broadcast discovery query (static version)
    async fn broadcast_query_static(
        socket: &UdpSocket,
        local_node: &ClusterNode,
        config: &ServiceDiscoveryConfig,
    ) -> Result<(), ClusterError> {
        let message = DiscoveryMessage::Query {
            requester_id: local_node.node_id.clone(),
            timestamp: Utc::now().timestamp(),
        };

        let data = serde_json::to_vec(&message)
            .map_err(|e| ClusterError::Serialization(e))?;

        let broadcast_addr = SocketAddr::new(
            config.multicast_addr,
            config.discovery_port,
        );

        socket.send_to(&data, broadcast_addr).await
            .map_err(|e| ClusterError::Network(format!("Failed to send query: {}", e)))?;

        debug!("Broadcasted discovery query");
        Ok(())
    }

    /// Broadcast discovery query
    async fn broadcast_query(&self, socket: &UdpSocket) -> Result<(), ClusterError> {
        let message = DiscoveryMessage::Query {
            requester_id: self.local_node.node_id.clone(),
            timestamp: Utc::now().timestamp(),
        };

        let data = serde_json::to_vec(&message)
            .map_err(|e| ClusterError::Serialization(e))?;

        let broadcast_addr = SocketAddr::new(
            self.config.multicast_addr,
            self.config.discovery_port,
        );

        socket.send_to(&data, broadcast_addr).await
            .map_err(|e| ClusterError::Network(format!("Failed to send query: {}", e)))?;

        debug!("Broadcasted discovery query");
        Ok(())
    }

    /// Handle incoming discovery message (static version)
    async fn handle_discovery_message_static(
        data: &[u8],
        _addr: SocketAddr,
        event_tx: &mpsc::UnboundedSender<DiscoveryEvent>,
    ) -> Result<(), ClusterError> {
        let message: DiscoveryMessage = serde_json::from_slice(data)
            .map_err(|e| ClusterError::Serialization(e))?;

        match message {
            DiscoveryMessage::Announce { node_info, timestamp: _ } => {
                let node = ClusterNode::try_from(node_info)?;
                if let Err(_) = event_tx.send(DiscoveryEvent::NodeDiscovered { node }) {
                    warn!("Failed to send node discovered event");
                }
            }
            DiscoveryMessage::Query { requester_id, .. } => {
                if let Err(_) = event_tx.send(DiscoveryEvent::QueryReceived { requester_id }) {
                    warn!("Failed to send query received event");
                }
            }
            DiscoveryMessage::Response { node_info, timestamp: _ } => {
                let node = ClusterNode::try_from(node_info)?;
                if let Err(_) = event_tx.send(DiscoveryEvent::NodeDiscovered { node }) {
                    warn!("Failed to send node discovered event");
                }
            }
            DiscoveryMessage::Leave { node_id, .. } => {
                if let Err(_) = event_tx.send(DiscoveryEvent::NodeLeft { node_id }) {
                    warn!("Failed to send node left event");
                }
            }
        }

        Ok(())
    }

    /// Handle incoming discovery message
    #[allow(dead_code)]
    async fn handle_discovery_message(
        &self,
        data: &[u8],
        _addr: SocketAddr,
    ) -> Result<(), ClusterError> {
        let message: DiscoveryMessage = serde_json::from_slice(data)
            .map_err(|e| ClusterError::Serialization(e))?;

        match message {
            DiscoveryMessage::Announce { node_info, timestamp } => {
                if node_info.node_id != self.local_node.node_id {
                    self.handle_node_announcement(node_info, timestamp).await?;
                }
            }
            DiscoveryMessage::Query { requester_id, .. } => {
                if requester_id != self.local_node.node_id {
                    self.handle_discovery_query(requester_id).await?;
                }
            }
            DiscoveryMessage::Response { node_info, timestamp } => {
                if node_info.node_id != self.local_node.node_id {
                    self.handle_node_announcement(node_info, timestamp).await?;
                }
            }
            DiscoveryMessage::Leave { node_id, .. } => {
                if node_id != self.local_node.node_id {
                    self.handle_node_leave_message(node_id).await?;
                }
            }
        }

        Ok(())
    }

    /// Handle node announcement
    #[allow(dead_code)]
    async fn handle_node_announcement(
        &self,
        node_info: DiscoveryNodeInfo,
        timestamp: i64,
    ) -> Result<(), ClusterError> {
        let node = ClusterNode::try_from(node_info)?;
        
        // Update discovered nodes cache
        {
            let mut discovered_nodes = self.discovered_nodes.write().await;
            discovered_nodes.insert(node.node_id.clone(), (node.clone(), timestamp));
        }

        // Send node discovered event
        if let Err(_) = self.event_tx.send(DiscoveryEvent::NodeDiscovered { node }) {
            warn!("Failed to send node discovered event");
        }

        Ok(())
    }

    /// Handle discovery query
    #[allow(dead_code)]
    async fn handle_discovery_query(&self, requester_id: String) -> Result<(), ClusterError> {
        // Send query received event
        if let Err(_) = self.event_tx.send(DiscoveryEvent::QueryReceived { requester_id }) {
            warn!("Failed to send query received event");
        }

        // Send response with our node information
        if let Some(socket) = &self.socket {
            let message = DiscoveryMessage::Response {
                node_info: DiscoveryNodeInfo::from(&self.local_node),
                timestamp: Utc::now().timestamp(),
            };

            let data = serde_json::to_vec(&message)
                .map_err(|e| ClusterError::Serialization(e))?;

            let broadcast_addr = SocketAddr::new(
                self.config.multicast_addr,
                self.config.discovery_port,
            );

            if let Err(e) = socket.send_to(&data, broadcast_addr).await {
                warn!("Failed to send discovery response: {}", e);
            }
        }

        Ok(())
    }

    /// Handle node leave message
    #[allow(dead_code)]
    async fn handle_node_leave_message(&self, node_id: String) -> Result<(), ClusterError> {
        // Remove from discovered nodes cache
        {
            let mut discovered_nodes = self.discovered_nodes.write().await;
            discovered_nodes.remove(&node_id);
        }

        // Send node left event
        if let Err(_) = self.event_tx.send(DiscoveryEvent::NodeLeft { node_id }) {
            warn!("Failed to send node left event");
        }

        Ok(())
    }

    /// Handle node discovered event
    async fn handle_node_discovered(&self, node: ClusterNode) -> Result<(), ClusterError> {
        info!("Discovered new node: {} at {}:{}", 
              node.node_id, node.address, node.grpc_port);

        // Add to cluster state
        self.cluster_state.upsert_node(node);

        Ok(())
    }

    /// Handle node left event
    async fn handle_node_left(&self, node_id: String) -> Result<(), ClusterError> {
        info!("Node left: {}", node_id);

        // Remove from cluster state
        self.cluster_state.remove_node(&node_id);

        Ok(())
    }

    /// Handle query received event
    async fn handle_query_received(&self, requester_id: String) -> Result<(), ClusterError> {
        debug!("Received discovery query from: {}", requester_id);
        Ok(())
    }

    /// Perform initial cluster discovery
    pub async fn discover_cluster(&self) -> Result<Vec<ClusterNode>, ClusterError> {
        if !self.config.enable_discovery {
            return Ok(Vec::new());
        }

        info!("Starting initial cluster discovery");

        let socket = self.socket.as_ref().ok_or_else(|| {
            ClusterError::InvalidOperation("Discovery socket not initialized".to_string())
        })?;

        // Send discovery query
        self.broadcast_query(socket).await?;

        // Wait for responses
        tokio::time::sleep(self.config.response_timeout).await;

        // Return discovered nodes
        let discovered_nodes = self.discovered_nodes.read().await;
        let nodes: Vec<ClusterNode> = discovered_nodes
            .values()
            .map(|(node, _)| node.clone())
            .collect();

        info!("Discovered {} nodes during initial discovery", nodes.len());
        Ok(nodes)
    }

    /// Announce node leaving
    pub async fn announce_leaving(&self) -> Result<(), ClusterError> {
        if let Some(socket) = &self.socket {
            let message = DiscoveryMessage::Leave {
                node_id: self.local_node.node_id.clone(),
                timestamp: Utc::now().timestamp(),
            };

            let data = serde_json::to_vec(&message)
                .map_err(|e| ClusterError::Serialization(e))?;

            let broadcast_addr = SocketAddr::new(
                self.config.multicast_addr,
                self.config.discovery_port,
            );

            socket.send_to(&data, broadcast_addr).await
                .map_err(|e| ClusterError::Network(format!("Failed to announce leaving: {}", e)))?;

            info!("Announced node leaving");
        }

        Ok(())
    }

    /// Get discovered nodes
    pub async fn get_discovered_nodes(&self) -> Vec<ClusterNode> {
        let discovered_nodes = self.discovered_nodes.read().await;
        discovered_nodes.values().map(|(node, _)| node.clone()).collect()
    }

    /// Check if discovery is active
    pub async fn is_active(&self) -> bool {
        *self.discovery_active.read().await
    }

    /// Shutdown service discovery
    pub async fn shutdown(&self) -> Result<(), ClusterError> {
        // Announce leaving
        self.announce_leaving().await?;

        // Send shutdown event
        self.event_tx.send(DiscoveryEvent::Shutdown)
            .map_err(|_| ClusterError::InvalidOperation("Failed to send shutdown event".to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_node_info_conversion() {
        let node = ClusterNode::new(
            "test-node".to_string(),
            "127.0.0.1".to_string(),
            9090,
            8080,
        );

        let discovery_info = DiscoveryNodeInfo::from(&node);
        assert_eq!(discovery_info.node_id, "test-node");
        assert_eq!(discovery_info.address, "127.0.0.1");
        assert_eq!(discovery_info.grpc_port, 9090);
        assert_eq!(discovery_info.http_port, 8080);

        let converted_node = ClusterNode::try_from(discovery_info).unwrap();
        assert_eq!(converted_node.node_id, node.node_id);
        assert_eq!(converted_node.address, node.address);
        assert_eq!(converted_node.grpc_port, node.grpc_port);
        assert_eq!(converted_node.http_port, node.http_port);
    }

    #[tokio::test]
    async fn test_service_discovery_creation() {
        let node = ClusterNode::new(
            "test-node".to_string(),
            "127.0.0.1".to_string(),
            9090,
            8080,
        );
        let cluster_state = Arc::new(ClusterState::new());
        let config = ServiceDiscoveryConfig::default();

        let discovery = ServiceDiscovery::new(node, cluster_state, config);
        assert!(!discovery.is_active().await);
    }
}