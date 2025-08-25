use crate::models::{ClusterNode, TaskMetadata, ClusterError};
use dashmap::DashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;
use chrono;

/// Cluster state management using DashMap for atomic operations
#[derive(Debug, Clone)]
pub struct ClusterState {
    /// Map of node_id -> ClusterNode for atomic node operations
    nodes: Arc<DashMap<String, ClusterNode>>,
    /// Map of task_id -> TaskMetadata for atomic task operations
    tasks: Arc<DashMap<String, TaskMetadata>>,
    /// Map of node_id -> Vec<task_id> for efficient task-by-node queries
    node_tasks: Arc<DashMap<String, Vec<String>>>,
}

impl ClusterState {
    /// Create a new ClusterState instance
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(DashMap::new()),
            tasks: Arc::new(DashMap::new()),
            node_tasks: Arc::new(DashMap::new()),
        }
    }

    /// Add or update a cluster node atomically
    pub fn upsert_node(&self, node: ClusterNode) {
        let node_id = node.node_id.clone();
        info!("Upserting node: {} (role: {:?}, status: {:?})", 
              node_id, node.role, node.status);
        
        self.nodes.insert(node_id.clone(), node);
        
        // Initialize empty task list for new nodes if not exists
        self.node_tasks.entry(node_id).or_insert_with(Vec::new);
    }

    /// Remove a cluster node atomically
    pub fn remove_node(&self, node_id: &str) -> Option<ClusterNode> {
        info!("Removing node: {}", node_id);
        
        // Remove all tasks assigned to this node
        if let Some((_, task_ids)) = self.node_tasks.remove(node_id) {
            for task_id in task_ids {
                if let Some(mut task) = self.tasks.get_mut(&task_id) {
                    task.assigned_node = None;
                    warn!("Task {} was assigned to removed node {}, clearing assignment", 
                          task_id, node_id);
                }
            }
        }
        
        self.nodes.remove(node_id).map(|(_, node)| node)
    }

    /// Get a cluster node by ID
    pub fn get_node(&self, node_id: &str) -> Option<ClusterNode> {
        self.nodes.get(node_id).map(|entry| entry.value().clone())
    }

    /// Get all cluster nodes
    pub fn get_all_nodes(&self) -> Vec<ClusterNode> {
        self.nodes.iter().map(|entry| entry.value().clone()).collect()
    }

    /// Get nodes by role
    pub fn get_nodes_by_role(&self, role: &crate::models::NodeRole) -> Vec<ClusterNode> {
        self.nodes
            .iter()
            .filter(|entry| &entry.value().role == role)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get nodes by status
    pub fn get_nodes_by_status(&self, status: &crate::models::NodeStatus) -> Vec<ClusterNode> {
        self.nodes
            .iter()
            .filter(|entry| &entry.value().status == status)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Update node status atomically
    pub fn update_node_status(&self, node_id: &str, status: crate::models::NodeStatus) -> Result<(), ClusterError> {
        match self.nodes.get_mut(node_id) {
            Some(mut node) => {
                debug!("Updating node {} status from {:?} to {:?}", 
                       node_id, node.status, status);
                node.status = status;
                Ok(())
            }
            None => Err(ClusterError::NodeNotFound(node_id.to_string())),
        }
    }

    /// Add or update a task atomically
    pub fn upsert_task(&self, task: TaskMetadata) {
        let task_id = task.task_id.clone();
        debug!("Upserting task: {} (state: {:?})", task_id, task.state);
        
        self.tasks.insert(task_id, task);
    }

    /// Remove a task atomically
    pub fn remove_task(&self, task_id: &str) -> Option<TaskMetadata> {
        debug!("Removing task: {}", task_id);
        
        if let Some((_, task)) = self.tasks.remove(task_id) {
            // Remove from node_tasks mapping if assigned
            if let Some(node_id) = &task.assigned_node {
                if let Some(mut task_list) = self.node_tasks.get_mut(node_id) {
                    task_list.retain(|id| id != task_id);
                }
            }
            Some(task)
        } else {
            None
        }
    }

    /// Get a task by ID
    pub fn get_task(&self, task_id: &str) -> Option<TaskMetadata> {
        self.tasks.get(task_id).map(|entry| entry.value().clone())
    }

    /// Get all tasks
    pub fn get_all_tasks(&self) -> Vec<TaskMetadata> {
        self.tasks.iter().map(|entry| entry.value().clone()).collect()
    }

    /// Get tasks by state
    pub fn get_tasks_by_state(&self, state: &crate::models::TaskState) -> Vec<TaskMetadata> {
        self.tasks
            .iter()
            .filter(|entry| &entry.value().state == state)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get tasks assigned to a specific node
    pub fn get_tasks_by_node(&self, node_id: &str) -> Vec<TaskMetadata> {
        if let Some(task_ids) = self.node_tasks.get(node_id) {
            task_ids
                .iter()
                .filter_map(|task_id| self.get_task(task_id))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Assign a task to a node atomically
    pub fn assign_task(&self, task_id: &str, node_id: &str) -> Result<(), ClusterError> {
        // Check if node exists
        if !self.nodes.contains_key(node_id) {
            return Err(ClusterError::NodeNotFound(node_id.to_string()));
        }

        // Update task assignment
        match self.tasks.get_mut(task_id) {
            Some(mut task) => {
                // Remove from previous node if reassigning
                if let Some(old_node_id) = &task.assigned_node {
                    if let Some(mut old_task_list) = self.node_tasks.get_mut(old_node_id) {
                        old_task_list.retain(|id| id != task_id);
                    }
                }

                // Assign to new node
                task.assigned_node = Some(node_id.to_string());
                task.state = crate::models::TaskState::Assigned;
                
                info!("Assigned task {} to node {}", task_id, node_id);

                // Add to new node's task list
                self.node_tasks
                    .entry(node_id.to_string())
                    .or_insert_with(Vec::new)
                    .push(task_id.to_string());

                Ok(())
            }
            None => Err(ClusterError::TaskNotFound(task_id.to_string())),
        }
    }

    /// Update task state atomically
    pub fn update_task_state(&self, task_id: &str, state: crate::models::TaskState) -> Result<(), ClusterError> {
        match self.tasks.get_mut(task_id) {
            Some(mut task) => {
                debug!("Updating task {} state from {:?} to {:?}", 
                       task_id, task.state, state);
                task.state = state;
                Ok(())
            }
            None => Err(ClusterError::TaskNotFound(task_id.to_string())),
        }
    }

    /// Complete a task atomically
    pub fn complete_task(&self, task_id: &str, processing_duration: f32) -> Result<(), ClusterError> {
        match self.tasks.get_mut(task_id) {
            Some(mut task) => {
                task.state = crate::models::TaskState::Completed;
                task.completed_at = Some(chrono::Utc::now().timestamp());
                task.processing_duration = Some(processing_duration);
                
                info!("Completed task {} in {:.2}s", task_id, processing_duration);

                // Remove from node_tasks mapping
                if let Some(node_id) = &task.assigned_node {
                    if let Some(mut task_list) = self.node_tasks.get_mut(node_id) {
                        task_list.retain(|id| id != task_id);
                    }
                }

                Ok(())
            }
            None => Err(ClusterError::TaskNotFound(task_id.to_string())),
        }
    }

    /// Fail a task atomically
    pub fn fail_task(&self, task_id: &str, error_message: &str) -> Result<(), ClusterError> {
        match self.tasks.get_mut(task_id) {
            Some(mut task) => {
                task.state = crate::models::TaskState::Failed;
                task.completed_at = Some(chrono::Utc::now().timestamp());
                task.error_message = Some(error_message.to_string());
                
                warn!("Failed task {}: {}", task_id, error_message);

                // Remove from node_tasks mapping
                if let Some(node_id) = &task.assigned_node {
                    if let Some(mut task_list) = self.node_tasks.get_mut(node_id) {
                        task_list.retain(|id| id != task_id);
                    }
                }

                Ok(())
            }
            None => Err(ClusterError::TaskNotFound(task_id.to_string())),
        }
    }

    /// Get cluster statistics
    pub fn get_stats(&self) -> ClusterStats {
        let total_nodes = self.nodes.len();
        let healthy_nodes = self.get_nodes_by_status(&crate::models::NodeStatus::Healthy).len();
        let total_tasks = self.tasks.len();
        
        let pending_tasks = self.get_tasks_by_state(&crate::models::TaskState::Pending).len();
        let assigned_tasks = self.get_tasks_by_state(&crate::models::TaskState::Assigned).len();
        let processing_tasks = self.get_tasks_by_state(&crate::models::TaskState::Processing).len();
        let completed_tasks = self.get_tasks_by_state(&crate::models::TaskState::Completed).len();
        let failed_tasks = self.get_tasks_by_state(&crate::models::TaskState::Failed).len();

        ClusterStats {
            total_nodes,
            healthy_nodes,
            total_tasks,
            pending_tasks,
            assigned_tasks,
            processing_tasks,
            completed_tasks,
            failed_tasks,
        }
    }

    /// Create a new task with generated ID
    pub fn create_task(
        &self,
        client_id: String,
        filename: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> String {
        let task_id = Uuid::new_v4().to_string();
        let mut task = TaskMetadata::new(task_id.clone(), client_id, filename);
        task.model = model;
        task.response_format = response_format;
        
        self.upsert_task(task);
        task_id
    }

    /// Get the number of active tasks for a node (assigned + processing)
    pub fn get_node_active_task_count(&self, node_id: &str) -> usize {
        if let Some(task_ids) = self.node_tasks.get(node_id) {
            task_ids
                .iter()
                .filter_map(|task_id| self.get_task(task_id))
                .filter(|task| matches!(task.state, 
                    crate::models::TaskState::Assigned | 
                    crate::models::TaskState::Processing))
                .count()
        } else {
            0
        }
    }

    /// Check if a node exists
    pub fn node_exists(&self, node_id: &str) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Check if a task exists
    pub fn task_exists(&self, task_id: &str) -> bool {
        self.tasks.contains_key(task_id)
    }
}

impl Default for ClusterState {
    fn default() -> Self {
        Self::new()
    }
}

/// Cluster statistics
#[derive(Debug, Clone)]
pub struct ClusterStats {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub total_tasks: usize,
    pub pending_tasks: usize,
    pub assigned_tasks: usize,
    pub processing_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NodeRole, NodeStatus, TaskState};

    #[test]
    fn test_cluster_state_node_operations() {
        let state = ClusterState::new();
        
        // Create test node
        let node = ClusterNode {
            node_id: "node1".to_string(),
            address: "127.0.0.1".to_string(),
            grpc_port: 9090,
            http_port: 8080,
            role: NodeRole::Leader,
            status: NodeStatus::Healthy,
            last_heartbeat: chrono::Utc::now().timestamp(),
        };

        // Test upsert
        state.upsert_node(node.clone());
        assert!(state.node_exists("node1"));
        
        // Test get
        let retrieved = state.get_node("node1").unwrap();
        assert_eq!(retrieved.node_id, "node1");
        assert_eq!(retrieved.role, NodeRole::Leader);

        // Test update status
        state.update_node_status("node1", NodeStatus::Unhealthy).unwrap();
        let updated = state.get_node("node1").unwrap();
        assert_eq!(updated.status, NodeStatus::Unhealthy);

        // Test remove
        let removed = state.remove_node("node1").unwrap();
        assert_eq!(removed.node_id, "node1");
        assert!(!state.node_exists("node1"));
    }

    #[test]
    fn test_cluster_state_task_operations() {
        let state = ClusterState::new();
        
        // Create test task
        let task_id = state.create_task(
            "client1".to_string(),
            "test.wav".to_string(),
            Some("whisper-1".to_string()),
            None,
        );

        assert!(state.task_exists(&task_id));
        
        // Test get task
        let task = state.get_task(&task_id).unwrap();
        assert_eq!(task.client_id, "client1");
        assert_eq!(task.filename, "test.wav");
        assert_eq!(task.state, TaskState::Pending);

        // Test update state
        state.update_task_state(&task_id, TaskState::Processing).unwrap();
        let updated = state.get_task(&task_id).unwrap();
        assert_eq!(updated.state, TaskState::Processing);

        // Test complete task
        state.complete_task(&task_id, 2.5).unwrap();
        let completed = state.get_task(&task_id).unwrap();
        assert_eq!(completed.state, TaskState::Completed);
        assert_eq!(completed.processing_duration, Some(2.5));
    }

    #[test]
    fn test_cluster_state_task_assignment() {
        let state = ClusterState::new();
        
        // Create node and task
        let node = ClusterNode {
            node_id: "node1".to_string(),
            address: "127.0.0.1".to_string(),
            grpc_port: 9090,
            http_port: 8080,
            role: NodeRole::Follower,
            status: NodeStatus::Healthy,
            last_heartbeat: chrono::Utc::now().timestamp(),
        };
        state.upsert_node(node);

        let task_id = state.create_task(
            "client1".to_string(),
            "test.wav".to_string(),
            None,
            None,
        );

        // Test assignment
        state.assign_task(&task_id, "node1").unwrap();
        
        let task = state.get_task(&task_id).unwrap();
        assert_eq!(task.assigned_node, Some("node1".to_string()));
        assert_eq!(task.state, TaskState::Assigned);

        // Test get tasks by node
        let node_tasks = state.get_tasks_by_node("node1");
        assert_eq!(node_tasks.len(), 1);
        assert_eq!(node_tasks[0].task_id, task_id);

        // Test active task count
        assert_eq!(state.get_node_active_task_count("node1"), 1);
    }
}