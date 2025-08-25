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
        let peers = self.peers.read().await;
        
        for message in messages {
            if let Some(peer_addr) = peers.get(&message.to) {
                debug!("Sending message to peer {} at {}: {:?}", message.to, peer_addr, message.msg_type());
                
                // TODO: Implement actual network communication to peers
                // For now, we'll just log the messages
                // In a real implementation, this would send the message via gRPC
            }
        }
    }

    /// Apply committed entries to the state machine
    async fn apply_entries(&self, entries: Vec<Entry>) -> Result<(), ClusterError> {
        for entry in entries {
            if entry.data.is_empty() {
                // Empty entry (heartbeat)
                continue;
            }

            debug!("Applying entry: index={}, term={}, data_len={}", 
                entry.index, entry.term, entry.data.len());

            // TODO: Implement actual state machine application
            // This would handle cluster metadata updates, task assignments, etc.
        }

        Ok(())
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