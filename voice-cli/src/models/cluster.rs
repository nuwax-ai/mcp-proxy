use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Node roles in the cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRole {
    Leader,
    Follower,
    Candidate,
}

/// Node status states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Healthy,
    Unhealthy,
    Joining,
    Leaving,
}

/// Cluster health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterHealth {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Task state enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    Pending,
    Assigned,
    Processing,
    Completed,
    Failed,
}

/// Simplified cluster node tracking with essential fields
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClusterNode {
    /// Unique node identifier
    pub node_id: String,
    /// Node IP address or hostname
    pub address: String,
    /// gRPC port for cluster communication
    pub grpc_port: u16,
    /// HTTP port for client API
    pub http_port: u16,
    /// Current role in the cluster
    pub role: NodeRole,
    /// Current status of the node
    pub status: NodeStatus,
    /// Last heartbeat timestamp (Unix timestamp in seconds)
    pub last_heartbeat: i64,
}

impl ClusterNode {
    pub fn new(node_id: String, address: String, grpc_port: u16, http_port: u16) -> Self {
        Self {
            node_id,
            address,
            grpc_port,
            http_port,
            role: NodeRole::Follower,
            status: NodeStatus::Joining,
            last_heartbeat: Utc::now().timestamp(),
        }
    }

    /// Check if node is healthy based on heartbeat timeout
    pub fn is_healthy(&self, heartbeat_timeout_secs: i64) -> bool {
        let now = Utc::now().timestamp();
        (now - self.last_heartbeat) <= heartbeat_timeout_secs
    }

    /// Update heartbeat timestamp
    pub fn update_heartbeat(&mut self) {
        self.last_heartbeat = Utc::now().timestamp();
    }

    /// Get gRPC endpoint address
    pub fn grpc_address(&self) -> String {
        format!("{}:{}", self.address, self.grpc_port)
    }

    /// Get HTTP endpoint address
    pub fn http_address(&self) -> String {
        format!("{}:{}", self.address, self.http_port)
    }
}

/// Basic task tracking with essential business information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskMetadata {
    /// Unique task identifier
    pub task_id: String,
    /// Client identifier who submitted the task
    pub client_id: String,
    /// Original audio filename
    pub filename: String,
    /// Path to the audio file (for cluster processing)
    pub audio_file_path: Option<String>,
    /// Node assigned to process this task
    pub assigned_node: Option<String>,
    /// Current state of the task
    pub state: TaskState,
    /// Task creation timestamp (Unix timestamp in seconds)
    pub created_at: i64,
    /// Task completion timestamp (Unix timestamp in seconds)
    pub completed_at: Option<i64>,
    /// Error message for failed tasks
    pub error_message: Option<String>,
    /// Model used for transcription
    pub model: Option<String>,
    /// Response format requested
    pub response_format: Option<String>,
    /// Processing duration in seconds
    pub processing_duration: Option<f32>,
}

impl TaskMetadata {
    pub fn new(task_id: String, client_id: String, filename: String) -> Self {
        Self {
            task_id,
            client_id,
            filename,
            audio_file_path: None,
            assigned_node: None,
            state: TaskState::Pending,
            created_at: Utc::now().timestamp(),
            completed_at: None,
            error_message: None,
            model: None,
            response_format: None,
            processing_duration: None,
        }
    }

    /// Mark task as assigned to a node
    pub fn assign_to_node(&mut self, node_id: String) {
        self.assigned_node = Some(node_id);
        self.state = TaskState::Assigned;
    }

    /// Mark task as processing
    pub fn mark_processing(&mut self) {
        self.state = TaskState::Processing;
    }

    /// Mark task as completed
    pub fn mark_completed(&mut self, processing_duration: f32) {
        self.state = TaskState::Completed;
        self.completed_at = Some(Utc::now().timestamp());
        self.processing_duration = Some(processing_duration);
    }

    /// Mark task as failed with error message
    pub fn mark_failed(&mut self, error_message: String) {
        self.state = TaskState::Failed;
        self.completed_at = Some(Utc::now().timestamp());
        self.error_message = Some(error_message);
    }

    /// Check if task is in a terminal state (completed or failed)
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, TaskState::Completed | TaskState::Failed)
    }

    /// Get task age in seconds
    pub fn age_seconds(&self) -> i64 {
        Utc::now().timestamp() - self.created_at
    }
}

/// Cluster statistics and health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterStats {
    /// Total number of nodes in cluster
    pub total_nodes: usize,
    /// Number of healthy nodes
    pub healthy_nodes: usize,
    /// Current leader node ID
    pub leader_node_id: Option<String>,
    /// Overall cluster health
    pub cluster_health: ClusterHealth,
    /// Total tasks processed
    pub total_tasks: usize,
    /// Active tasks being processed
    pub active_tasks: usize,
    /// Failed tasks count
    pub failed_tasks: usize,
    /// Node statistics by node ID
    pub node_stats: HashMap<String, NodeStats>,
}

/// Statistics for individual nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStats {
    /// Node ID
    pub node_id: String,
    /// Tasks assigned to this node
    pub assigned_tasks: usize,
    /// Tasks completed by this node
    pub completed_tasks: usize,
    /// Tasks failed on this node
    pub failed_tasks: usize,
    /// Last heartbeat time
    pub last_heartbeat: i64,
    /// Node uptime in seconds
    pub uptime_seconds: i64,
}

impl ClusterStats {
    pub fn new() -> Self {
        Self {
            total_nodes: 0,
            healthy_nodes: 0,
            leader_node_id: None,
            cluster_health: ClusterHealth::Unhealthy,
            total_tasks: 0,
            active_tasks: 0,
            failed_tasks: 0,
            node_stats: HashMap::new(),
        }
    }

    /// Calculate cluster health based on node status
    pub fn calculate_health(&mut self) {
        if self.total_nodes == 0 {
            self.cluster_health = ClusterHealth::Unhealthy;
            return;
        }

        let healthy_ratio = self.healthy_nodes as f64 / self.total_nodes as f64;

        self.cluster_health = if healthy_ratio >= 0.8 {
            ClusterHealth::Healthy
        } else if healthy_ratio >= 0.5 {
            ClusterHealth::Degraded
        } else {
            ClusterHealth::Unhealthy
        };
    }
}

impl Default for ClusterStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Transcription result from cluster processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterTranscriptionResult {
    /// Task ID
    pub task_id: String,
    /// Transcribed text
    pub text: String,
    /// Detected language
    pub language: Option<String>,
    /// Audio duration in seconds
    pub duration: Option<f32>,
    /// Processing time in seconds
    pub processing_time: f32,
    /// Node that processed the task
    pub processed_by: String,
    /// Original filename
    pub filename: String,
}
