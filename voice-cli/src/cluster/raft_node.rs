use crate::models::{ClusterError, MetadataStore, ClusterNode, NodeRole, NodeStatus};
use raft::prelude::*;
use raft::{Config, RawNode};
use raft::storage::MemStorage;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, info, warn, error};

/// Messages for Raft node communication
#[derive(Debug)]
pub enum RaftMessage {
    /// Propose a new entry to the Raft log
    Propose { data: Vec<u8> },
    /// Process received Raft message
    Raft { message: Message },
    /// Tick event for driving Raft state machine
    Tick,
    /// Shutdown the Raft node
    Shutdown,
}

/// Raft node events that can be handled externally
#[derive(Debug, Clone)]
pub enum RaftEvent {
    /// Node became leader
    BecameLeader,
    /// Node became follower
    BecameFollower,
    /// Node became candidate
    BecameCandidate,
    /// Leader committed new entries
    CommittedEntries { entries: Vec<Entry> },
    /// Configuration changed
    ConfigChange,
}

/// AudioClusterRaft manages Raft consensus for the audio cluster
pub struct AudioClusterRaft {
    /// Current node ID
    node_id: u64,
    /// Raft raw node instance
    raft_node: RawNode<MemStorage>,
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
    /// Node information for this cluster node
    node_info: ClusterNode,
    /// Receiver for Raft messages
    message_rx: mpsc::UnboundedReceiver<RaftMessage>,
    /// Sender for Raft messages (cloneable for external use)
    message_tx: mpsc::UnboundedSender<RaftMessage>,
    /// Event sender for notifying external components
    event_tx: mpsc::UnboundedSender<RaftEvent>,
    /// Map of peer nodes by ID
    peers: Arc<RwLock<HashMap<u64, String>>>, // peer_id -> address
    /// Last tick time for timeout handling
    last_tick: Instant,
    /// Whether the node is currently the leader
    is_leader: bool,
}

impl AudioClusterRaft {
    /// Create a new AudioClusterRaft instance
    pub fn new(
        node_id: u64,
        node_info: ClusterNode,
        metadata_store: Arc<MetadataStore>,
        peers: Vec<(u64, String)>, // (peer_id, address)
        event_tx: mpsc::UnboundedSender<RaftEvent>,
    ) -> Result<Self, ClusterError> {
        // Create Raft configuration
        let config = Config {
            id: node_id,
            election_tick: 10,
            heartbeat_tick: 3,
            max_size_per_msg: 1024 * 1024, // 1MB
            max_inflight_msgs: 256,
            check_quorum: true,
            ..Config::default()
        };

        // Create in-memory storage for Raft logs
        let storage = MemStorage::new();
        
        // Initialize peers for the Raft cluster
        let peer_ids: Vec<u64> = peers.iter().map(|(id, _)| *id).collect();
        let initial_peers = if peer_ids.is_empty() {
            // Single node cluster
            vec![node_id]
        } else {
            // Multi-node cluster
            let mut all_peers = peer_ids;
            if !all_peers.contains(&node_id) {
                all_peers.push(node_id);
            }
            all_peers
        };

        // Create Raft node
        let raft_node = RawNode::new(&config, storage, &initial_peers)
            .map_err(|e| ClusterError::Config(format!("Failed to create Raft node: {}", e)))?;

        // Create message channel
        let (message_tx, message_rx) = mpsc::unbounded_channel();

        // Create peers map
        let peers_map: HashMap<u64, String> = peers.into_iter().collect();

        Ok(Self {
            node_id,
            raft_node,
            metadata_store,
            node_info,
            message_rx,
            message_tx,
            event_tx,
            peers: Arc::new(RwLock::new(peers_map)),
            last_tick: Instant::now(),
            is_leader: false,
        })
    }

    /// Get a cloneable message sender for external communication
    pub fn message_sender(&self) -> mpsc::UnboundedSender<RaftMessage> {
        self.message_tx.clone()
    }

    /// Check if this node is currently the leader
    pub fn is_leader(&self) -> bool {
        self.is_leader
    }

    /// Get current Raft state
    pub fn state(&self) -> StateRole {
        self.raft_node.raft.state
    }

    /// Get current term
    pub fn term(&self) -> u64 {
        self.raft_node.raft.term
    }

    /// Get leader ID if known
    pub fn leader_id(&self) -> Option<u64> {
        if self.raft_node.raft.leader_id == INVALID_ID {
            None
        } else {
            Some(self.raft_node.raft.leader_id)
        }
    }

    /// Run the Raft node event loop
    pub async fn run(&mut self) -> Result<(), ClusterError> {
        info!("Starting Raft node {} with peers: {:?}", self.node_id, self.peers.read().await);

        // Start tick timer
        let mut tick_timer = interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                // Handle incoming messages
                msg = self.message_rx.recv() => {
                    match msg {
                        Some(RaftMessage::Propose { data }) => {
                            if let Err(e) = self.handle_propose(data) {
                                error!("Failed to propose: {}", e);
                            }
                        }
                        Some(RaftMessage::Raft { message }) => {
                            if let Err(e) = self.handle_raft_message(message) {
                                error!("Failed to handle Raft message: {}", e);
                            }
                        }
                        Some(RaftMessage::Tick) => {
                            self.handle_tick();
                        }
                        Some(RaftMessage::Shutdown) => {
                            info!("Shutting down Raft node {}", self.node_id);
                            break;
                        }
                        None => {
                            warn!("Message channel closed, shutting down Raft node");
                            break;
                        }
                    }
                }
                
                // Handle periodic ticks
                _ = tick_timer.tick() => {
                    self.handle_tick();
                }
            }

            // Process ready state
            if let Err(e) = self.process_ready().await {
                error!("Failed to process ready state: {}", e);
            }
        }

        Ok(())
    }

    /// Handle propose request
    fn handle_propose(&mut self, data: Vec<u8>) -> Result<(), ClusterError> {
        if !self.is_leader() {
            return Err(ClusterError::InvalidOperation("Only leader can propose entries".to_string()));
        }

        self.raft_node.propose(vec![], data)
            .map_err(|e| ClusterError::Config(format!("Failed to propose: {}", e)))?;

        Ok(())
    }

    /// Handle incoming Raft message
    fn handle_raft_message(&mut self, message: Message) -> Result<(), ClusterError> {
        self.raft_node.step(message)
            .map_err(|e| ClusterError::Network(format!("Failed to step Raft: {}", e)))?;

        Ok(())
    }

    /// Handle tick event
    fn handle_tick(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_tick) >= Duration::from_millis(100) {
            self.raft_node.tick();
            self.last_tick = now;
        }
    }

    /// Process Raft ready state
    async fn process_ready(&mut self) -> Result<(), ClusterError> {
        if !self.raft_node.has_ready() {
            return Ok(());
        }

        let mut ready = self.raft_node.ready();

        // Check for state changes
        let new_state = self.raft_node.raft.state;
        let was_leader = self.is_leader;
        self.is_leader = new_state == StateRole::Leader;

        // Send events for state changes
        if !was_leader && self.is_leader {
            info!("Node {} became leader in term {}", self.node_id, self.term());
            let _ = self.event_tx.send(RaftEvent::BecameLeader);
            
            // Update node role in metadata store
            if let Err(e) = self.metadata_store.update_node_role(
                &self.node_info.node_id, 
                NodeRole::Leader
            ).await {
                warn!("Failed to update node role to leader: {}", e);
            }
        } else if was_leader && !self.is_leader {
            info!("Node {} is no longer leader", self.node_id);
            match new_state {
                StateRole::Follower => {
                    let _ = self.event_tx.send(RaftEvent::BecameFollower);
                    if let Err(e) = self.metadata_store.update_node_role(
                        &self.node_info.node_id, 
                        NodeRole::Follower
                    ).await {
                        warn!("Failed to update node role to follower: {}", e);
                    }
                }
                StateRole::Candidate => {
                    let _ = self.event_tx.send(RaftEvent::BecameCandidate);
                    if let Err(e) = self.metadata_store.update_node_role(
                        &self.node_info.node_id, 
                        NodeRole::Candidate
                    ).await {
                        warn!("Failed to update node role to candidate: {}", e);
                    }
                }
                _ => {}
            }
        }

        // Send messages to peers
        if !ready.messages().is_empty() {
            self.send_messages(ready.take_messages()).await;
        }

        // Apply committed entries
        if !ready.committed_entries().is_empty() {
            let entries = ready.take_committed_entries();
            self.apply_entries(entries.clone()).await?;
            let _ = self.event_tx.send(RaftEvent::CommittedEntries { entries });
        }

        // Advance the Raft state machine
        let mut light_rd = self.raft_node.advance(ready);
        
        // Handle light ready if available
        if let Some(light_ready) = light_rd.take() {
            if !light_ready.messages().is_empty() {
                self.send_messages(light_ready.take_messages()).await;
            }
            self.raft_node.advance_apply();
        }

        Ok(())
    }

    /// Send messages to peer nodes
    async fn send_messages(&self, messages: Vec<Message>) {
        for message in messages {
            let peer_id = message.to;
            let peers = self.peers.read().await;
            
            if let Some(peer_addr) = peers.get(&peer_id) {
                debug!("Sending Raft message to peer {} at {}: {:?}", peer_id, peer_addr, message.msg_type());
                
                // Clone necessary data for async task
                let peer_addr = peer_addr.clone();
                let message_clone = message.clone();
                let node_id_clone = self.node_id.clone();
                
                // Send message asynchronously to avoid blocking
                tokio::spawn(async move {
                    if let Err(e) = Self::send_raft_message_to_peer(
                        &node_id_clone,
                        &peer_addr,
                        message_clone
                    ).await {
                        debug!("Failed to send Raft message to peer {}: {}", peer_id, e);
                    }
                });
            } else {
                debug!("No address found for peer {}", peer_id);
            }
        }
    }
    
    /// Send a single Raft message to a peer via gRPC
    async fn send_raft_message_to_peer(
        local_node_id: &str,
        peer_address: &str,
        message: Message,
    ) -> Result<(), ClusterError> {
        use crate::grpc::client::AudioClusterClient;
        use tonic::Request;
        
        // Construct full gRPC address
        let grpc_address = if peer_address.starts_with("http://") {
            peer_address.to_string()
        } else {
            format!("http://{}", peer_address)
        };
        
        // Create gRPC client with timeout
        let mut client = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            AudioClusterClient::connect(grpc_address.clone())
        ).await {
            Ok(Ok(client)) => client,
            Ok(Err(e)) => {
                debug!("Failed to connect to peer at {}: {}", grpc_address, e);
                return Err(ClusterError::Network(format!("Connection failed: {}", e)));
            }
            Err(_) => {
                debug!("Connection timeout to peer at {}", grpc_address);
                return Err(ClusterError::Network("Connection timeout".to_string()));
            }
        };
        
        // Convert Raft message to gRPC format
        let raft_request = crate::grpc::audio_cluster_service::RaftMessageRequest {
            from: message.from,
            to: message.to,
            msg_type: message.msg_type() as i32,
            term: message.term,
            log_term: message.log_term,
            index: message.index,
            entries: message.entries.iter().map(|entry| {
                crate::grpc::audio_cluster_service::RaftEntry {
                    index: entry.index,
                    term: entry.term,
                    data: entry.data.clone(),
                    entry_type: entry.entry_type() as i32,
                }
            }).collect(),
            commit: message.commit,
            commit_term: message.commit_term,
            snapshot: message.snapshot.as_ref().map(|snap| {
                crate::grpc::audio_cluster_service::RaftSnapshot {
                    data: snap.data.clone(),
                    metadata: Some(crate::grpc::audio_cluster_service::SnapshotMetadata {
                        conf_state: snap.metadata.conf_state.as_ref().map(|cs| {
                            crate::grpc::audio_cluster_service::ConfState {
                                voters: cs.voters.clone(),
                                learners: cs.learners.clone(),
                                voters_outgoing: cs.voters_outgoing.clone(),
                                learners_next: cs.learners_next.clone(),
                                auto_leave: cs.auto_leave,
                            }
                        }),
                        index: snap.metadata.index,
                        term: snap.metadata.term,
                    }),
                }
            }),
            request_snapshot: message.request_snapshot,
            reject: message.reject,
            reject_hint: message.reject_hint,
            context: message.context.clone(),
        };
        
        // Send Raft message via gRPC
        match client.send_raft_message(Request::new(raft_request)).await {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    debug!("Raft message sent successfully to {}", grpc_address);
                } else {
                    debug!("Raft message rejected by {}: {}", grpc_address, resp.message);
                }
                Ok(())
            }
            Err(e) => {
                debug!("gRPC Raft message failed to {}: {}", grpc_address, e);
                Err(ClusterError::Network(format!("gRPC call failed: {}", e)))
            }
        }
    }

    /// Apply committed entries to the state machine
    async fn apply_entries(&self, entries: Vec<Entry>) -> Result<(), ClusterError> {
        for entry in entries {
            if entry.data.is_empty() {
                // Empty entry (heartbeat or configuration entry)
                debug!("Skipping empty entry: index={}, term={}", entry.index, entry.term);
                continue;
            }

            debug!("Applying entry: index={}, term={}, data_len={}", 
                entry.index, entry.term, entry.data.len());

            // Parse and apply cluster operation
            match self.apply_cluster_operation(&entry.data).await {
                Ok(operation) => {
                    info!("Successfully applied cluster operation: {:?}", operation);
                }
                Err(e) => {
                    warn!("Failed to apply cluster operation from entry {}: {}", entry.index, e);
                    // Continue processing other entries even if one fails
                }
            }
        }

        Ok(())
    }
    
    /// Apply a single cluster operation from entry data
    async fn apply_cluster_operation(&self, data: &[u8]) -> Result<ClusterOperation, ClusterError> {
        // Deserialize the operation
        let operation: ClusterOperation = serde_json::from_slice(data)
            .map_err(|e| ClusterError::Serialization(e))?;
        
        // Apply the operation to cluster state
        match &operation {
            ClusterOperation::AddNode { node_id, address, grpc_port, http_port } => {
                let new_node = ClusterNode {
                    node_id: node_id.clone(),
                    address: address.clone(),
                    grpc_port: *grpc_port,
                    http_port: *http_port,
                    role: NodeRole::Follower, // New nodes start as followers
                    status: NodeStatus::Joining,
                    last_heartbeat: chrono::Utc::now().timestamp(),
                };
                
                // Add to metadata store
                self.metadata_store.add_node(&new_node).await
                    .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
                    
                info!("Added node {} to cluster", node_id);
            }
            
            ClusterOperation::RemoveNode { node_id } => {
                // Remove from metadata store
                self.metadata_store.remove_node(node_id).await
                    .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
                    
                info!("Removed node {} from cluster", node_id);
            }
            
            ClusterOperation::UpdateNodeStatus { node_id, status } => {
                // Update node status in metadata store
                self.metadata_store.update_node_status(node_id, *status).await
                    .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
                    
                debug!("Updated node {} status to {:?}", node_id, status);
            }
            
            ClusterOperation::AssignTask { task_id, node_id } => {
                // Assign task to node in metadata store
                self.metadata_store.assign_task(task_id, node_id).await
                    .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
                    
                debug!("Assigned task {} to node {}", task_id, node_id);
            }
            
            ClusterOperation::CompleteTask { task_id, processing_duration } => {
                // Mark task as completed
                self.metadata_store.complete_task(task_id).await
                    .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
                    
                debug!("Completed task {} in {:.2}s", task_id, processing_duration);
            }
            
            ClusterOperation::FailTask { task_id, error_message } => {
                // Mark task as failed
                self.metadata_store.fail_task(task_id, error_message).await
                    .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
                    
                debug!("Failed task {} with error: {}", task_id, error_message);
            }
        }
        
        Ok(operation)
    }

    /// Add a new peer to the cluster
    pub async fn add_peer(&mut self, peer_id: u64, address: String) -> Result<(), ClusterError> {
        let mut peers = self.peers.write().await;
        peers.insert(peer_id, address.clone());
        
        info!("Added peer {} at address {}", peer_id, address);
        
        // TODO: Implement configuration change
        // self.raft_node.propose_conf_change(...)
        
        Ok(())
    }

    /// Remove a peer from the cluster
    pub async fn remove_peer(&mut self, peer_id: u64) -> Result<(), ClusterError> {
        let mut peers = self.peers.write().await;
        if let Some(address) = peers.remove(&peer_id) {
            info!("Removed peer {} at address {}", peer_id, address);
            
            // TODO: Implement configuration change
            // self.raft_node.propose_conf_change(...)
        }
        
        Ok(())
    }

    /// Get current cluster configuration
    pub async fn get_cluster_config(&self) -> HashMap<u64, String> {
        self.peers.read().await.clone()
    }

    /// Propose a cluster operation
    pub fn propose_operation(&mut self, operation: ClusterOperation) -> Result<(), ClusterError> {
        let data = serde_json::to_vec(&operation)
            .map_err(|e| ClusterError::Serialization(e))?;
        
        self.handle_propose(data)
    }
}

/// Operations that can be proposed to the cluster
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum ClusterOperation {
    /// Add a new node to the cluster
    AddNode { node_id: String, address: String, grpc_port: u16, http_port: u16 },
    /// Remove a node from the cluster
    RemoveNode { node_id: String },
    /// Update node status
    UpdateNodeStatus { node_id: String, status: NodeStatus },
    /// Assign a task to a node
    AssignTask { task_id: String, node_id: String },
    /// Complete a task
    CompleteTask { task_id: String, processing_duration: f32 },
    /// Fail a task
    FailTask { task_id: String, error_message: String },
}

/// Helper function to convert cluster node ID to Raft node ID
pub fn cluster_node_id_to_raft_id(cluster_node_id: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    cluster_node_id.hash(&mut hasher);
    hasher.finish()
}

/// Helper function to convert Raft node ID to cluster node ID
pub fn raft_id_to_cluster_node_id(raft_id: u64) -> String {
    format!("raft-{}", raft_id)
}