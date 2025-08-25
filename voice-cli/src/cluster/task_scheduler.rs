use crate::cluster::ClusterState;
use crate::error::ClusterResultExt;
use crate::models::{ClusterError, ClusterNode, MetadataStore, NodeRole, NodeStatus, TaskMetadata};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Simple round-robin scheduler with configurable leader processing
pub struct SimpleTaskScheduler {
    /// Cluster state for atomic operations
    cluster_state: Arc<ClusterState>,
    /// Metadata store for persistence (optional, for backwards compatibility)
    metadata_store: Option<Arc<MetadataStore>>,
    /// Current node index for round-robin selection
    current_node_index: AtomicUsize,
    /// Whether leader can process tasks
    leader_can_process: bool,
    /// Current leader node ID
    leader_node_id: String,
    /// Scheduler configuration
    config: SchedulerConfig,
    /// Channel for receiving scheduling events
    event_rx: mpsc::UnboundedReceiver<SchedulerEvent>,
    /// Channel sender for scheduling events (cloneable)
    event_tx: mpsc::UnboundedSender<SchedulerEvent>,
    /// Statistics for scheduler performance
    stats: Arc<RwLock<SchedulerStats>>,
}

/// Configuration for task scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum number of concurrent tasks per node
    pub max_tasks_per_node: usize,
    /// Timeout for task assignment
    pub assignment_timeout: Duration,
    /// Interval for refreshing available nodes cache
    pub cache_refresh_interval: Duration,
    /// Maximum queue size for pending tasks
    pub max_queue_size: usize,
    /// Task assignment retry attempts
    pub max_retry_attempts: u32,
    /// Delay between retry attempts
    pub retry_delay: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_tasks_per_node: 5,
            assignment_timeout: Duration::from_secs(30),
            cache_refresh_interval: Duration::from_secs(10),
            max_queue_size: 1000,
            max_retry_attempts: 3,
            retry_delay: Duration::from_secs(2),
        }
    }
}

/// Events for task scheduling
#[derive(Debug)]
pub enum SchedulerEvent {
    /// Schedule a new task
    ScheduleTask {
        task_id: String,
        client_id: String,
        filename: String,
        audio_file_path: String,
        model: Option<String>,
        response_format: Option<String>,
    },
    /// Task completed successfully
    TaskCompleted {
        task_id: String,
        node_id: String,
        processing_duration: f32,
    },
    /// Task failed
    TaskFailed {
        task_id: String,
        node_id: String,
        error_message: String,
    },
    /// Refresh available nodes cache
    RefreshNodes,
    /// Rebalance tasks across nodes
    RebalanceTasks,
    /// Get scheduler statistics
    GetStats {
        response_tx: oneshot::Sender<SchedulerStats>,
    },
    /// Shutdown the scheduler
    Shutdown,
}

/// Scheduler statistics
#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    /// Total tasks scheduled
    pub total_scheduled: u64,
    /// Tasks completed successfully
    pub completed_tasks: u64,
    /// Tasks failed
    pub failed_tasks: u64,
    /// Current pending tasks
    pub pending_tasks: u64,
    /// Current active tasks (assigned/processing)
    pub active_tasks: u64,
    /// Average task completion time
    pub avg_completion_time: f32,
    /// Tasks per node distribution
    pub tasks_per_node: HashMap<String, NodeTaskStats>,
}

/// Task statistics per node
#[derive(Debug, Clone, Default)]
pub struct NodeTaskStats {
    /// Currently assigned tasks
    pub assigned: u32,
    /// Completed tasks
    pub completed: u32,
    /// Failed tasks
    pub failed: u32,
    /// Average completion time
    pub avg_time: f32,
}

impl SimpleTaskScheduler {
    /// Create a new SimpleTaskScheduler with ClusterState
    pub fn new_with_cluster_state(
        cluster_state: Arc<ClusterState>,
        leader_can_process: bool,
        leader_node_id: String,
        config: SchedulerConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            cluster_state,
            metadata_store: None,
            current_node_index: AtomicUsize::new(0),
            leader_can_process,
            leader_node_id,
            config,
            event_rx,
            event_tx,
            stats: Arc::new(RwLock::new(SchedulerStats::default())),
        }
    }

    /// Create a new SimpleTaskScheduler (backwards compatibility)
    pub fn new(
        metadata_store: Arc<MetadataStore>,
        leader_can_process: bool,
        leader_node_id: String,
        config: SchedulerConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            cluster_state: Arc::new(ClusterState::new()),
            metadata_store: Some(metadata_store),
            current_node_index: AtomicUsize::new(0),
            leader_can_process,
            leader_node_id,
            config,
            event_rx,
            event_tx,
            stats: Arc::new(RwLock::new(SchedulerStats::default())),
        }
    }

    /// Get a cloneable event sender for external use
    pub fn event_sender(&self) -> mpsc::UnboundedSender<SchedulerEvent> {
        self.event_tx.clone()
    }

    /// Start the task scheduler
    pub async fn start(&mut self) -> Result<(), ClusterError> {
        info!(
            "Starting task scheduler (leader_can_process: {})",
            self.leader_can_process
        );

        // Initial sync with metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            self.sync_with_metadata_store(metadata_store).await?;
        }

        // Clone necessary data for async tasks
        let event_tx = self.event_tx.clone();
        let config = self.config.clone();

        // Start cache refresh timer (less frequent since ClusterState is more efficient)
        let cache_refresh_handle = {
            let event_tx = event_tx.clone();
            let cache_refresh_interval = config.cache_refresh_interval;
            tokio::spawn(async move {
                let mut interval = interval(cache_refresh_interval);
                loop {
                    interval.tick().await;
                    if event_tx.send(SchedulerEvent::RefreshNodes).is_err() {
                        warn!("Failed to send cache refresh event - channel closed");
                        break;
                    }
                }
            })
        };

        // Run the main event loop
        tokio::select! {
            _ = self.run_event_loop() => {
                info!("Scheduler event loop completed");
            }
            _ = cache_refresh_handle => {
                warn!("Cache refresh timer stopped");
            }
        }

        Ok(())
    }

    /// Run the main event loop
    async fn run_event_loop(&mut self) -> Result<(), ClusterError> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                SchedulerEvent::ScheduleTask {
                    task_id,
                    client_id,
                    filename,
                    audio_file_path,
                    model,
                    response_format,
                } => {
                    if let Err(e) = self
                        .handle_schedule_task(
                            task_id,
                            client_id,
                            filename,
                            audio_file_path,
                            model,
                            response_format,
                        )
                        .await
                    {
                        error!("Failed to schedule task: {}", e);
                    }
                }
                SchedulerEvent::TaskCompleted {
                    task_id,
                    node_id,
                    processing_duration,
                } => {
                    if let Err(e) = self
                        .handle_task_completed(task_id, node_id, processing_duration)
                        .await
                    {
                        error!("Failed to handle task completion: {}", e);
                    }
                }
                SchedulerEvent::TaskFailed {
                    task_id,
                    node_id,
                    error_message,
                } => {
                    if let Err(e) = self
                        .handle_task_failed(task_id, node_id, error_message)
                        .await
                    {
                        error!("Failed to handle task failure: {}", e);
                    }
                }
                SchedulerEvent::RefreshNodes => {
                    if let Err(e) = self.refresh_available_nodes().await {
                        warn!("Failed to refresh available nodes: {}", e);
                    }
                }
                SchedulerEvent::RebalanceTasks => {
                    if let Err(e) = self.rebalance_tasks().await {
                        warn!("Failed to rebalance tasks: {}", e);
                    }
                }
                SchedulerEvent::GetStats { response_tx } => {
                    let stats = self.stats.read().await.clone();
                    let _ = response_tx.send(stats);
                }
                SchedulerEvent::Shutdown => {
                    info!("Shutting down task scheduler");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handle new task scheduling
    async fn handle_schedule_task(
        &self,
        task_id: String,
        client_id: String,
        filename: String,
        _audio_file_path: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> Result<(), ClusterError> {
        info!("Scheduling task {} for client {}", task_id, client_id);

        // Create task metadata
        let mut task = TaskMetadata::new(task_id.clone(), client_id, filename);
        task.model = model;
        task.response_format = response_format;

        // Store task in cluster state
        self.cluster_state.upsert_task(task.clone());

        // Also store in metadata store if available for persistence
        if let Some(ref metadata_store) = self.metadata_store {
            metadata_store
                .create_task(&task)
                .await
                .with_task_context(&task_id)
                .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
        }

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.total_scheduled += 1;
            stats.pending_tasks += 1;
        }

        // Assign task to a node
        match self.assign_next_task(task_id.clone()).await {
            Ok(assigned_node_id) => {
                info!("Task {} assigned to node {}", task_id, assigned_node_id);

                // Update statistics
                {
                    let mut stats = self.stats.write().await;
                    stats.pending_tasks -= 1;
                    stats.active_tasks += 1;

                    let node_stats = stats
                        .tasks_per_node
                        .entry(assigned_node_id.clone())
                        .or_default();
                    node_stats.assigned += 1;
                }
            }
            Err(e) => {
                error!("Failed to assign task {}: {}", task_id, e);

                // Mark task as failed in cluster state
                self.cluster_state.fail_task(&task_id, &e.to_string())?;

                // Also update metadata store if available
                if let Some(ref metadata_store) = self.metadata_store {
                    metadata_store
                        .fail_task(&task_id, &e.to_string())
                        .await
                        .with_task_context(&task_id)
                        .map_err(|e| {
                            ClusterError::Database(sled::Error::Unsupported(e.to_string()))
                        })?;
                }

                // Update statistics
                {
                    let mut stats = self.stats.write().await;
                    stats.pending_tasks -= 1;
                    stats.failed_tasks += 1;
                }
            }
        }

        Ok(())
    }

    /// Assign next task to an available node
    pub async fn assign_next_task(&self, task_id: String) -> Result<String, ClusterError> {
        let available_nodes = self.get_available_nodes_for_tasks().await?;

        if available_nodes.is_empty() {
            return Err(ClusterError::NoAvailableNodes);
        }

        // Simple round-robin selection
        let index = self.current_node_index.fetch_add(1, Ordering::Relaxed) % available_nodes.len();
        let selected_node = &available_nodes[index];

        // Assign task to selected node in cluster state
        self.cluster_state
            .assign_task(&task_id, &selected_node.node_id)?;

        // Also assign in metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            metadata_store
                .assign_task(&task_id, &selected_node.node_id)
                .await
                .with_task_context(&task_id)
                .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
        }

        Ok(selected_node.node_id.clone())
    }

    /// Get available nodes for task processing based on configuration
    async fn get_available_nodes_for_tasks(&self) -> Result<Vec<ClusterNode>, ClusterError> {
        // Get healthy nodes from cluster state
        let healthy_nodes = self.cluster_state.get_nodes_by_status(&NodeStatus::Healthy);

        let mut available_nodes: Vec<ClusterNode> = healthy_nodes;

        // Filter nodes based on leader processing configuration
        if self.leader_can_process {
            // Leader can process: include all healthy nodes (leader + followers)
            available_nodes
                .retain(|node| node.role == NodeRole::Leader || node.role == NodeRole::Follower);
        } else {
            // Leader only coordinates: include only healthy followers
            available_nodes.retain(|node| node.role == NodeRole::Follower);
        }

        // Filter out overloaded nodes using cluster state
        let mut filtered_nodes = Vec::new();
        for node in available_nodes {
            let current_tasks = self.cluster_state.get_node_active_task_count(&node.node_id);
            if current_tasks < self.config.max_tasks_per_node {
                filtered_nodes.push(node);
            }
        }

        Ok(filtered_nodes)
    }

    /// Check if current node (leader) should process this task
    pub fn should_leader_process(&self, assigned_node_id: &str) -> bool {
        self.leader_can_process && assigned_node_id == self.leader_node_id
    }

    /// Handle task completion
    async fn handle_task_completed(
        &self,
        task_id: String,
        node_id: String,
        processing_duration: f32,
    ) -> Result<(), ClusterError> {
        info!(
            "Task {} completed by node {} in {:.2}s",
            task_id, node_id, processing_duration
        );

        // Update task in cluster state
        self.cluster_state
            .complete_task(&task_id, processing_duration)?;

        // Also update metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            metadata_store
                .complete_task(&task_id, processing_duration)
                .await
                .with_task_context(&task_id)
                .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
        }

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.active_tasks -= 1;
            stats.completed_tasks += 1;

            // Update average completion time
            let total_completed = stats.completed_tasks as f32;
            stats.avg_completion_time = (stats.avg_completion_time * (total_completed - 1.0)
                + processing_duration)
                / total_completed;

            let node_stats = stats.tasks_per_node.entry(node_id).or_default();
            node_stats.assigned = node_stats.assigned.saturating_sub(1);
            node_stats.completed += 1;

            // Update node average time
            let node_completed = node_stats.completed as f32;
            node_stats.avg_time = (node_stats.avg_time * (node_completed - 1.0)
                + processing_duration)
                / node_completed;
        }

        Ok(())
    }

    /// Handle task failure
    async fn handle_task_failed(
        &self,
        task_id: String,
        node_id: String,
        error_message: String,
    ) -> Result<(), ClusterError> {
        warn!(
            "Task {} failed on node {}: {}",
            task_id, node_id, error_message
        );

        // Update task in cluster state
        self.cluster_state.fail_task(&task_id, &error_message)?;

        // Also update metadata store if available
        if let Some(ref metadata_store) = self.metadata_store {
            metadata_store
                .fail_task(&task_id, &error_message)
                .await
                .with_task_context(&task_id)
                .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
        }

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.active_tasks -= 1;
            stats.failed_tasks += 1;

            let node_stats = stats.tasks_per_node.entry(node_id).or_default();
            node_stats.assigned = node_stats.assigned.saturating_sub(1);
            node_stats.failed += 1;
        }

        Ok(())
    }

    /// Sync with metadata store (if available)
    async fn sync_with_metadata_store(
        &self,
        metadata_store: &MetadataStore,
    ) -> Result<(), ClusterError> {
        // Load nodes from metadata store into cluster state
        let nodes = metadata_store
            .get_all_nodes()
            .await
            .with_cluster_context("sync nodes from metadata store")
            .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
        for node in nodes {
            self.cluster_state.upsert_node(node);
        }

        // Load tasks from metadata store into cluster state
        let all_tasks = metadata_store
            .get_all_tasks()
            .await
            .with_cluster_context("sync tasks from metadata store")
            .map_err(|e| ClusterError::Database(sled::Error::Unsupported(e.to_string())))?;
        for task in all_tasks {
            self.cluster_state.upsert_task(task);
        }

        debug!(
            "Synced cluster state with metadata store: {} nodes, {} tasks",
            self.cluster_state.get_all_nodes().len(),
            self.cluster_state.get_all_tasks().len()
        );

        Ok(())
    }

    /// Refresh nodes from metadata store (backwards compatibility)
    async fn refresh_available_nodes(&self) -> Result<(), ClusterError> {
        if let Some(ref metadata_store) = self.metadata_store {
            self.sync_with_metadata_store(metadata_store).await?;
        }
        Ok(())
    }

    /// Rebalance tasks across nodes (intelligent load balancing)
    async fn rebalance_tasks(&self) -> Result<(), ClusterError> {
        debug!("Starting task rebalancing across cluster nodes");

        // Get all healthy nodes
        let healthy_nodes = self.cluster_state.get_nodes_by_status(&NodeStatus::Healthy);
        if healthy_nodes.len() < 2 {
            debug!(
                "Not enough healthy nodes for rebalancing (need >= 2, have {})",
                healthy_nodes.len()
            );
            return Ok(());
        }

        // Calculate load distribution
        let mut node_loads = Vec::new();
        let mut total_tasks = 0;

        for node in &healthy_nodes {
            let active_tasks = self.cluster_state.get_node_active_task_count(&node.node_id);
            node_loads.push((node.clone(), active_tasks));
            total_tasks += active_tasks;
        }

        if total_tasks == 0 {
            debug!("No active tasks to rebalance");
            return Ok(());
        }

        // Sort nodes by current load (ascending)
        node_loads.sort_by_key(|(_, load)| *load);

        let ideal_load_per_node = total_tasks / healthy_nodes.len();
        let remainder = total_tasks % healthy_nodes.len();

        debug!(
            "Rebalancing {} tasks across {} nodes (ideal: {} per node, remainder: {})",
            total_tasks,
            healthy_nodes.len(),
            ideal_load_per_node,
            remainder
        );

        // Find overloaded and underloaded nodes
        let mut overloaded_nodes = Vec::new();
        let mut underloaded_nodes = Vec::new();

        for (i, (node, current_load)) in node_loads.iter().enumerate() {
            let target_load = ideal_load_per_node + if i < remainder { 1 } else { 0 };

            if *current_load > target_load {
                let excess = *current_load - target_load;
                overloaded_nodes.push((node.clone(), excess));
            } else if *current_load < target_load {
                let deficit = target_load - *current_load;
                underloaded_nodes.push((node.clone(), deficit));
            }
        }

        if overloaded_nodes.is_empty() || underloaded_nodes.is_empty() {
            debug!("No rebalancing needed - load is already well distributed");
            return Ok(());
        }

        // Perform task redistribution
        let mut rebalanced_count = 0;

        for (overloaded_node, mut excess) in overloaded_nodes {
            if excess == 0 {
                continue;
            }

            // Get tasks from this overloaded node (prefer pending/assigned tasks)
            let tasks = self
                .cluster_state
                .get_tasks_by_node(&overloaded_node.node_id);
            let mut reassignable_tasks: Vec<_> = tasks
                .into_iter()
                .filter(|task| {
                    matches!(
                        task.state,
                        crate::models::TaskState::Pending | crate::models::TaskState::Assigned
                    )
                })
                .collect();

            // Sort by creation time (newest first for better load distribution)
            reassignable_tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));

            for task in reassignable_tasks {
                if excess == 0 {
                    break;
                }

                // Find an underloaded node to move this task to
                for (underloaded_node, deficit) in &mut underloaded_nodes {
                    if *deficit > 0 {
                        // Move task from overloaded to underloaded node
                        match self
                            .cluster_state
                            .assign_task(&task.task_id, &underloaded_node.node_id)
                        {
                            Ok(()) => {
                                info!(
                                    "Rebalanced task {} from {} to {}",
                                    task.task_id, overloaded_node.node_id, underloaded_node.node_id
                                );

                                // Update metadata store if available
                                if let Some(ref metadata_store) = self.metadata_store {
                                    if let Err(e) = metadata_store
                                        .assign_task(&task.task_id, &underloaded_node.node_id)
                                        .await
                                    {
                                        warn!("Failed to update metadata store during rebalancing: {}", e);
                                    }
                                }

                                excess -= 1;
                                *deficit -= 1;
                                rebalanced_count += 1;
                                break;
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to reassign task {} during rebalancing: {}",
                                    task.task_id, e
                                );
                            }
                        }
                    }
                }
            }
        }

        if rebalanced_count > 0 {
            info!(
                "Task rebalancing completed: {} tasks redistributed",
                rebalanced_count
            );

            // Update scheduler statistics
            let mut stats = self.stats.write().await;
            // We could add rebalancing stats here if needed
        } else {
            debug!("No tasks were rebalanced - all eligible tasks may be in processing state");
        }

        Ok(())
    }

    /// Schedule a new task (external API)
    pub async fn schedule_task(
        &self,
        client_id: String,
        filename: String,
        audio_file_path: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> Result<String, ClusterError> {
        let task_id = Uuid::new_v4().to_string();

        let event = SchedulerEvent::ScheduleTask {
            task_id: task_id.clone(),
            client_id,
            filename,
            audio_file_path,
            model,
            response_format,
        };

        self.event_tx
            .send(event)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        Ok(task_id)
    }

    /// Report task completion (external API)
    pub async fn report_task_completed(
        &self,
        task_id: String,
        node_id: String,
        processing_duration: f32,
    ) -> Result<(), ClusterError> {
        let event = SchedulerEvent::TaskCompleted {
            task_id,
            node_id,
            processing_duration,
        };

        self.event_tx
            .send(event)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        Ok(())
    }

    /// Report task failure (external API)
    pub async fn report_task_failed(
        &self,
        task_id: String,
        node_id: String,
        error_message: String,
    ) -> Result<(), ClusterError> {
        let event = SchedulerEvent::TaskFailed {
            task_id,
            node_id,
            error_message,
        };

        self.event_tx
            .send(event)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        Ok(())
    }

    /// Trigger manual task rebalancing (external API)
    pub async fn trigger_rebalancing(&self) -> Result<(), ClusterError> {
        let event = SchedulerEvent::RebalanceTasks;
        self.event_tx
            .send(event)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;
        Ok(())
    }

    /// Get scheduler statistics (external API)
    pub async fn get_stats(&self) -> Result<SchedulerStats, ClusterError> {
        let (response_tx, response_rx) = oneshot::channel();

        let event = SchedulerEvent::GetStats { response_tx };

        self.event_tx
            .send(event)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        response_rx
            .await
            .map_err(|_| ClusterError::InvalidOperation("Failed to get stats response".to_string()))
    }

    /// Get scheduler statistics directly (for testing - non-blocking)
    /// This method accesses stats directly without going through the event loop
    /// Use only for testing purposes when the background event loop is not running
    pub async fn get_stats_direct(&self) -> SchedulerStats {
        self.stats.read().await.clone()
    }

    /// Get cluster state (for external access)
    pub fn cluster_state(&self) -> Arc<ClusterState> {
        Arc::clone(&self.cluster_state)
    }

    /// Add a node to the cluster
    pub fn add_node(&self, node: ClusterNode) {
        self.cluster_state.upsert_node(node);
    }

    /// Remove a node from the cluster
    pub fn remove_node(&self, node_id: &str) -> Option<ClusterNode> {
        self.cluster_state.remove_node(node_id)
    }

    /// Update node status
    pub fn update_node_status(
        &self,
        node_id: &str,
        status: NodeStatus,
    ) -> Result<(), ClusterError> {
        self.cluster_state.update_node_status(node_id, status)
    }

    /// Shutdown the scheduler gracefully
    pub async fn shutdown(&self) -> Result<(), ClusterError> {
        self.event_tx
            .send(SchedulerEvent::Shutdown)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        Ok(())
    }
}
