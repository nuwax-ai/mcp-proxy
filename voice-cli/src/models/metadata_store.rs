use crate::cluster::ClusterState;
use crate::models::cluster::{
    ClusterNode, ClusterStats, NodeRole, NodeStats, NodeStatus, TaskMetadata, TaskState,
};
use chrono::Utc;
use std::path::Path;
use std::sync::Arc;

/// Error type for MetadataStore operations
#[derive(Debug, thiserror::Error)]
pub enum ClusterError {
    #[error("Database error: {0}")]
    Database(#[from] sled::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("No available nodes for task assignment")]
    NoAvailableNodes,

    #[error("Transcription failed: {0}")]
    TranscriptionFailed(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

/// Database key prefixes for different data types
const NODES_PREFIX: &str = "nodes:";
const TASKS_PREFIX: &str = "tasks:";
const CLIENT_TASKS_PREFIX: &str = "client_tasks:";
const NODE_TASKS_PREFIX: &str = "node_tasks:";
const CLUSTER_META_PREFIX: &str = "cluster_meta:";

/// Metadata store using Sled embedded database with ClusterState integration
pub struct MetadataStore {
    db: Arc<sled::Db>,
    /// Optional cluster state for atomic operations
    cluster_state: Option<Arc<ClusterState>>,
}

impl MetadataStore {
    /// Create a new MetadataStore with the given database path
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, ClusterError> {
        let db = sled::open(db_path)?;
        Ok(Self {
            db: Arc::new(db),
            cluster_state: None,
        })
    }

    /// Create a new MetadataStore with ClusterState integration
    pub fn new_with_cluster_state<P: AsRef<Path>>(
        db_path: P,
        cluster_state: Arc<ClusterState>,
    ) -> Result<Self, ClusterError> {
        let db = sled::open(db_path)?;
        Ok(Self {
            db: Arc::new(db),
            cluster_state: Some(cluster_state),
        })
    }

    /// Create an in-memory MetadataStore for testing
    pub fn new_temp() -> Result<Self, ClusterError> {
        let db = sled::Config::new().temporary(true).open()?;
        Ok(Self {
            db: Arc::new(db),
            cluster_state: None,
        })
    }

    /// Create an in-memory MetadataStore with ClusterState for testing
    pub fn new_temp_with_cluster_state(
        cluster_state: Arc<ClusterState>,
    ) -> Result<Self, ClusterError> {
        let db = sled::Config::new().temporary(true).open()?;
        Ok(Self {
            db: Arc::new(db),
            cluster_state: Some(cluster_state),
        })
    }

    /// Set cluster state for atomic operations (can be called after creation)
    pub fn set_cluster_state(&mut self, cluster_state: Arc<ClusterState>) {
        self.cluster_state = Some(cluster_state);
    }

    /// Get cluster state reference
    pub fn cluster_state(&self) -> Option<&Arc<ClusterState>> {
        self.cluster_state.as_ref()
    }

    // === Node Operations ===

    /// Add a new node to the cluster
    pub async fn add_node(&self, node: &ClusterNode) -> Result<(), ClusterError> {
        // Update cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.upsert_node(node.clone());
        }

        // Persist to database
        let key = format!("{}{}", NODES_PREFIX, node.node_id);
        let value = serde_json::to_vec(node)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Remove a node from the cluster
    pub async fn remove_node(&self, node_id: &str) -> Result<(), ClusterError> {
        // Remove from cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.remove_node(node_id);
        }

        let key = format!("{}{}", NODES_PREFIX, node_id);

        if self.db.remove(&key)?.is_none() {
            return Err(ClusterError::NodeNotFound(node_id.to_string()));
        }

        // Clean up node tasks reference
        let node_tasks_key = format!("{}{}", NODE_TASKS_PREFIX, node_id);
        self.db.remove(node_tasks_key)?;

        Ok(())
    }

    /// Get all nodes in the cluster
    pub async fn get_all_nodes(&self) -> Result<Vec<ClusterNode>, ClusterError> {
        let mut nodes = Vec::new();

        for result in self.db.scan_prefix(NODES_PREFIX.as_bytes()) {
            let (_, value) = result?;
            let node: ClusterNode = serde_json::from_slice(&value)?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    /// Get a specific node by ID
    pub async fn get_node(&self, node_id: &str) -> Result<Option<ClusterNode>, ClusterError> {
        let key = format!("{}{}", NODES_PREFIX, node_id);

        if let Some(value) = self.db.get(&key)? {
            let node: ClusterNode = serde_json::from_slice(&value)?;
            Ok(Some(node))
        } else {
            Ok(None)
        }
    }

    /// Update node status
    pub async fn update_node_status(
        &self,
        node_id: &str,
        status: NodeStatus,
    ) -> Result<(), ClusterError> {
        // Update cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.update_node_status(node_id, status)?;
        }

        let key = format!("{}{}", NODES_PREFIX, node_id);

        if let Some(value) = self.db.get(&key)? {
            let mut node: ClusterNode = serde_json::from_slice(&value)?;
            node.status = status;
            let updated_value = serde_json::to_vec(&node)?;
            self.db.insert(key, updated_value)?;
            Ok(())
        } else {
            Err(ClusterError::NodeNotFound(node_id.to_string()))
        }
    }

    /// Update node role
    pub async fn update_node_role(
        &self,
        node_id: &str,
        role: NodeRole,
    ) -> Result<(), ClusterError> {
        let key = format!("{}{}", NODES_PREFIX, node_id);

        if let Some(value) = self.db.get(&key)? {
            let mut node: ClusterNode = serde_json::from_slice(&value)?;
            node.role = role;
            let updated_value = serde_json::to_vec(&node)?;
            self.db.insert(key, updated_value)?;
            Ok(())
        } else {
            Err(ClusterError::NodeNotFound(node_id.to_string()))
        }
    }

    /// Update node heartbeat timestamp
    pub async fn update_heartbeat(&self, node_id: &str) -> Result<(), ClusterError> {
        // Update cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            if let Some(mut node) = cluster_state.get_node(node_id) {
                node.update_heartbeat();
                cluster_state.upsert_node(node);
            }
        }

        let key = format!("{}{}", NODES_PREFIX, node_id);

        if let Some(value) = self.db.get(&key)? {
            let mut node: ClusterNode = serde_json::from_slice(&value)?;
            node.update_heartbeat();
            let updated_value = serde_json::to_vec(&node)?;
            self.db.insert(key, updated_value)?;
            Ok(())
        } else {
            Err(ClusterError::NodeNotFound(node_id.to_string()))
        }
    }

    // === Task Operations ===

    /// Create a new task
    pub async fn create_task(&self, task: &TaskMetadata) -> Result<(), ClusterError> {
        // Update cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.upsert_task(task.clone());
        }

        let task_key = format!("{}{}", TASKS_PREFIX, task.task_id);
        let task_value = serde_json::to_vec(task)?;
        self.db.insert(task_key, task_value)?;

        // Index by client ID for efficient lookup
        let client_tasks_key = format!("{}{}", CLIENT_TASKS_PREFIX, task.client_id);
        let mut client_tasks: Vec<String> = if let Some(value) = self.db.get(&client_tasks_key)? {
            serde_json::from_slice(&value)?
        } else {
            Vec::new()
        };

        if !client_tasks.contains(&task.task_id) {
            client_tasks.push(task.task_id.clone());
            let client_tasks_value = serde_json::to_vec(&client_tasks)?;
            self.db.insert(client_tasks_key, client_tasks_value)?;
        }

        Ok(())
    }

    /// Assign a task to a node
    pub async fn assign_task(&self, task_id: &str, node_id: &str) -> Result<(), ClusterError> {
        // Update cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.assign_task(task_id, node_id)?;
        }

        let task_key = format!("{}{}", TASKS_PREFIX, task_id);

        if let Some(value) = self.db.get(&task_key)? {
            let mut task: TaskMetadata = serde_json::from_slice(&value)?;
            task.assign_to_node(node_id.to_string());
            let updated_value = serde_json::to_vec(&task)?;
            self.db.insert(task_key, updated_value)?;

            // Index by node ID for efficient lookup
            let node_tasks_key = format!("{}{}", NODE_TASKS_PREFIX, node_id);
            let mut node_tasks: Vec<String> = if let Some(value) = self.db.get(&node_tasks_key)? {
                serde_json::from_slice(&value)?
            } else {
                Vec::new()
            };

            if !node_tasks.contains(&task.task_id) {
                node_tasks.push(task.task_id.clone());
                let node_tasks_value = serde_json::to_vec(&node_tasks)?;
                self.db.insert(node_tasks_key, node_tasks_value)?;
            }

            Ok(())
        } else {
            Err(ClusterError::TaskNotFound(task_id.to_string()))
        }
    }

    /// Mark task as processing
    pub async fn start_task_processing(&self, task_id: &str) -> Result<(), ClusterError> {
        let key = format!("{}{}", TASKS_PREFIX, task_id);

        if let Some(value) = self.db.get(&key)? {
            let mut task: TaskMetadata = serde_json::from_slice(&value)?;
            task.mark_processing();
            let updated_value = serde_json::to_vec(&task)?;
            self.db.insert(key, updated_value)?;
            Ok(())
        } else {
            Err(ClusterError::TaskNotFound(task_id.to_string()))
        }
    }

    /// Mark task as completed
    pub async fn complete_task(
        &self,
        task_id: &str,
        processing_duration: f32,
    ) -> Result<(), ClusterError> {
        // Update cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.complete_task(task_id, processing_duration)?;
        }

        let key = format!("{}{}", TASKS_PREFIX, task_id);

        if let Some(value) = self.db.get(&key)? {
            let mut task: TaskMetadata = serde_json::from_slice(&value)?;
            task.mark_completed(processing_duration);
            let updated_value = serde_json::to_vec(&task)?;
            self.db.insert(key, updated_value)?;
            Ok(())
        } else {
            Err(ClusterError::TaskNotFound(task_id.to_string()))
        }
    }

    /// Mark task as failed with error message
    pub async fn fail_task(&self, task_id: &str, error_message: &str) -> Result<(), ClusterError> {
        // Update cluster state first if available (atomic operation)
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.fail_task(task_id, error_message)?;
        }

        let key = format!("{}{}", TASKS_PREFIX, task_id);

        if let Some(value) = self.db.get(&key)? {
            let mut task: TaskMetadata = serde_json::from_slice(&value)?;
            task.mark_failed(error_message.to_string());
            let updated_value = serde_json::to_vec(&task)?;
            self.db.insert(key, updated_value)?;
            Ok(())
        } else {
            Err(ClusterError::TaskNotFound(task_id.to_string()))
        }
    }

    /// Get a specific task by ID
    pub async fn get_task(&self, task_id: &str) -> Result<Option<TaskMetadata>, ClusterError> {
        let key = format!("{}{}", TASKS_PREFIX, task_id);

        if let Some(value) = self.db.get(&key)? {
            let task: TaskMetadata = serde_json::from_slice(&value)?;
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    /// Get all tasks for a specific client
    pub async fn get_tasks_by_client(
        &self,
        client_id: &str,
    ) -> Result<Vec<TaskMetadata>, ClusterError> {
        let client_tasks_key = format!("{}{}", CLIENT_TASKS_PREFIX, client_id);

        if let Some(value) = self.db.get(&client_tasks_key)? {
            let task_ids: Vec<String> = serde_json::from_slice(&value)?;
            let mut tasks = Vec::new();

            for task_id in task_ids {
                if let Some(task) = self.get_task(&task_id).await? {
                    tasks.push(task);
                }
            }

            Ok(tasks)
        } else {
            Ok(Vec::new())
        }
    }

    /// Get all tasks assigned to a specific node
    pub async fn get_tasks_by_node(
        &self,
        node_id: &str,
    ) -> Result<Vec<TaskMetadata>, ClusterError> {
        let node_tasks_key = format!("{}{}", NODE_TASKS_PREFIX, node_id);

        if let Some(value) = self.db.get(&node_tasks_key)? {
            let task_ids: Vec<String> = serde_json::from_slice(&value)?;
            let mut tasks = Vec::new();

            for task_id in task_ids {
                if let Some(task) = self.get_task(&task_id).await? {
                    tasks.push(task);
                }
            }

            Ok(tasks)
        } else {
            Ok(Vec::new())
        }
    }

    /// Get tasks by state
    pub async fn get_tasks_by_state(
        &self,
        state: TaskState,
    ) -> Result<Vec<TaskMetadata>, ClusterError> {
        let mut tasks = Vec::new();

        for result in self.db.scan_prefix(TASKS_PREFIX.as_bytes()) {
            let (_, value) = result?;
            let task: TaskMetadata = serde_json::from_slice(&value)?;
            if task.state == state {
                tasks.push(task);
            }
        }

        Ok(tasks)
    }

    // === Cluster Statistics ===

    /// Get cluster statistics
    pub async fn get_cluster_stats(&self) -> Result<ClusterStats, ClusterError> {
        let nodes = self.get_all_nodes().await?;
        let mut stats = ClusterStats::new();

        stats.total_nodes = nodes.len();
        stats.healthy_nodes = nodes
            .iter()
            .filter(|n| n.status == NodeStatus::Healthy)
            .count();
        stats.leader_node_id = nodes
            .iter()
            .find(|n| n.role == NodeRole::Leader)
            .map(|n| n.node_id.clone());

        // Calculate cluster health
        stats.calculate_health();

        // Count tasks by state
        let all_tasks = self.get_all_tasks().await?;
        stats.total_tasks = all_tasks.len();
        stats.active_tasks = all_tasks
            .iter()
            .filter(|t| matches!(t.state, TaskState::Assigned | TaskState::Processing))
            .count();
        stats.failed_tasks = all_tasks
            .iter()
            .filter(|t| t.state == TaskState::Failed)
            .count();

        // Generate node statistics
        for node in &nodes {
            let node_tasks = self.get_tasks_by_node(&node.node_id).await?;
            let completed_tasks = node_tasks
                .iter()
                .filter(|t| t.state == TaskState::Completed)
                .count();
            let failed_tasks = node_tasks
                .iter()
                .filter(|t| t.state == TaskState::Failed)
                .count();

            let node_stat = NodeStats {
                node_id: node.node_id.clone(),
                assigned_tasks: node_tasks.len(),
                completed_tasks,
                failed_tasks,
                last_heartbeat: node.last_heartbeat,
                uptime_seconds: Utc::now().timestamp() - node.last_heartbeat, // Simplified uptime calculation
            };

            stats.node_stats.insert(node.node_id.clone(), node_stat);
        }

        Ok(stats)
    }

    /// Get all tasks (public method)
    pub async fn get_all_tasks(&self) -> Result<Vec<TaskMetadata>, ClusterError> {
        let mut tasks = Vec::new();

        for result in self.db.scan_prefix(TASKS_PREFIX.as_bytes()) {
            let (_, value) = result?;
            let task: TaskMetadata = serde_json::from_slice(&value)?;
            tasks.push(task);
        }

        Ok(tasks)
    }

    // === Cluster Metadata Operations ===

    /// Set cluster metadata
    pub async fn set_cluster_meta(&self, key: &str, value: &str) -> Result<(), ClusterError> {
        let meta_key = format!("{}{}", CLUSTER_META_PREFIX, key);
        self.db.insert(meta_key, value.as_bytes())?;
        Ok(())
    }

    /// Get cluster metadata
    pub async fn get_cluster_meta(&self, key: &str) -> Result<Option<String>, ClusterError> {
        let meta_key = format!("{}{}", CLUSTER_META_PREFIX, key);

        if let Some(value) = self.db.get(&meta_key)? {
            Ok(Some(String::from_utf8_lossy(&value).to_string()))
        } else {
            Ok(None)
        }
    }

    /// Clean up old completed/failed tasks (maintenance)
    pub async fn cleanup_old_tasks(&self, max_age_hours: i64) -> Result<usize, ClusterError> {
        let cutoff_time = Utc::now().timestamp() - (max_age_hours * 3600);
        let mut cleaned_count = 0;

        for result in self.db.scan_prefix(TASKS_PREFIX.as_bytes()) {
            let (key, value) = result?;
            let task: TaskMetadata = serde_json::from_slice(&value)?;

            // Clean up old terminal tasks
            if task.is_terminal() && task.created_at < cutoff_time {
                self.db.remove(&key)?;
                cleaned_count += 1;

                // Also clean up from indexes
                if let Some(ref assigned_node) = task.assigned_node {
                    let node_tasks_key = format!("{}{}", NODE_TASKS_PREFIX, assigned_node);
                    if let Some(value) = self.db.get(&node_tasks_key)? {
                        let mut node_tasks: Vec<String> = serde_json::from_slice(&value)?;
                        node_tasks.retain(|id| id != &task.task_id);
                        let updated_value = serde_json::to_vec(&node_tasks)?;
                        self.db.insert(node_tasks_key, updated_value)?;
                    }
                }

                let client_tasks_key = format!("{}{}", CLIENT_TASKS_PREFIX, task.client_id);
                if let Some(value) = self.db.get(&client_tasks_key)? {
                    let mut client_tasks: Vec<String> = serde_json::from_slice(&value)?;
                    client_tasks.retain(|id| id != &task.task_id);
                    let updated_value = serde_json::to_vec(&client_tasks)?;
                    self.db.insert(client_tasks_key, updated_value)?;
                }
            }
        }

        Ok(cleaned_count)
    }

    /// Sync data from database to cluster state (for initialization)
    pub async fn sync_to_cluster_state(&self) -> Result<(), ClusterError> {
        if let Some(ref cluster_state) = self.cluster_state {
            // Load all nodes from database into cluster state
            let nodes = self.get_all_nodes().await?;
            for node in nodes {
                cluster_state.upsert_node(node);
            }

            // Load all tasks from database into cluster state
            let tasks = self.get_all_tasks().await?;
            for task in tasks {
                cluster_state.upsert_task(task);
            }

            tracing::info!(
                "Synced {} nodes and {} tasks from database to cluster state",
                cluster_state.get_all_nodes().len(),
                cluster_state.get_all_tasks().len()
            );
        }
        Ok(())
    }

    /// Perform atomic node health monitoring update
    pub async fn update_node_health_atomic(
        &self,
        node_id: &str,
        is_healthy: bool,
    ) -> Result<(), ClusterError> {
        let new_status = if is_healthy {
            NodeStatus::Healthy
        } else {
            NodeStatus::Unhealthy
        };

        // Update both cluster state and database atomically
        if let Some(ref cluster_state) = self.cluster_state {
            // First update cluster state (atomic)
            cluster_state.update_node_status(node_id, new_status)?;

            // Then persist to database
            self.update_node_status(node_id, new_status).await?;
        } else {
            // Fallback to database-only update
            self.update_node_status(node_id, new_status).await?;
        }

        Ok(())
    }

    /// Get node health status from cluster state (atomic read)
    pub fn get_node_health_atomic(&self, node_id: &str) -> Option<NodeStatus> {
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.get_node(node_id).map(|node| node.status)
        } else {
            None
        }
    }

    /// Get all healthy nodes atomically
    pub fn get_healthy_nodes_atomic(&self) -> Vec<ClusterNode> {
        if let Some(ref cluster_state) = self.cluster_state {
            cluster_state.get_nodes_by_status(&NodeStatus::Healthy)
        } else {
            Vec::new()
        }
    }

    /// Get cluster statistics from cluster state (atomic read)
    pub fn get_cluster_stats_atomic(&self) -> Option<crate::cluster::state::ClusterStats> {
        if let Some(ref cluster_state) = self.cluster_state {
            Some(cluster_state.get_stats())
        } else {
            None
        }
    }

    /// Check if cluster state is available
    pub fn has_cluster_state(&self) -> bool {
        self.cluster_state.is_some()
    }

    /// Flush all data to disk
    pub async fn flush(&self) -> Result<(), ClusterError> {
        self.db.flush()?;
        Ok(())
    }
}

impl Clone for MetadataStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            cluster_state: self.cluster_state.clone(),
        }
    }
}
