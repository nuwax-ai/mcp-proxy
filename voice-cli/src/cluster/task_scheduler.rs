use crate::models::{
    ClusterError, MetadataStore, ClusterNode, TaskMetadata, NodeRole, NodeStatus, TaskState
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::sync::oneshot;
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn, error};
use uuid::Uuid;
use chrono::Utc;

/// Simple round-robin scheduler with configurable leader processing
pub struct SimpleTaskScheduler {
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
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
    /// Cache of available nodes for performance
    available_nodes_cache: Arc<RwLock<Vec<ClusterNode>>>,
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
    /// Create a new SimpleTaskScheduler
    pub fn new(
        metadata_store: Arc<MetadataStore>,
        leader_can_process: bool,
        leader_node_id: String,
        config: SchedulerConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            metadata_store,
            current_node_index: AtomicUsize::new(0),
            leader_can_process,
            leader_node_id,
            config,
            event_rx,
            event_tx,
            available_nodes_cache: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(SchedulerStats::default())),
        }
    }

    /// Get a cloneable event sender for external use
    pub fn event_sender(&self) -> mpsc::UnboundedSender<SchedulerEvent> {
        self.event_tx.clone()
    }

    /// Start the task scheduler
    pub async fn start(&mut self) -> Result<(), ClusterError> {
        info!("Starting task scheduler (leader_can_process: {})", self.leader_can_process);

        // Initial cache refresh
        self.refresh_available_nodes().await?;

        // Clone necessary data for async tasks
        let event_tx = self.event_tx.clone();
        let config = self.config.clone();

        // Start cache refresh timer
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
                    task_id, client_id, filename, audio_file_path, model, response_format 
                } => {
                    if let Err(e) = self.handle_schedule_task(
                        task_id, client_id, filename, audio_file_path, model, response_format
                    ).await {
                        error!("Failed to schedule task: {}", e);
                    }
                }
                SchedulerEvent::TaskCompleted { task_id, node_id, processing_duration } => {
                    if let Err(e) = self.handle_task_completed(task_id, node_id, processing_duration).await {
                        error!("Failed to handle task completion: {}", e);
                    }
                }
                SchedulerEvent::TaskFailed { task_id, node_id, error_message } => {
                    if let Err(e) = self.handle_task_failed(task_id, node_id, error_message).await {
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
        audio_file_path: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> Result<(), ClusterError> {
        info!("Scheduling task {} for client {}", task_id, client_id);

        // Create task metadata
        let mut task = TaskMetadata::new(task_id.clone(), client_id, filename);
        task.model = model;
        task.response_format = response_format;

        // Store task in metadata store
        self.metadata_store.create_task(&task).await?;

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
                    
                    let node_stats = stats.tasks_per_node.entry(assigned_node_id.clone()).or_default();
                    node_stats.assigned += 1;
                }
            }
            Err(e) => {
                error!("Failed to assign task {}: {}", task_id, e);
                
                // Mark task as failed
                self.metadata_store.fail_task(&task_id, &e.to_string()).await?;
                
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

        // Assign task to selected node
        self.metadata_store
            .assign_task(&task_id, &selected_node.node_id)
            .await?;

        Ok(selected_node.node_id.clone())
    }

    /// Get available nodes for task processing based on configuration
    async fn get_available_nodes_for_tasks(&self) -> Result<Vec<ClusterNode>, ClusterError> {
        let all_nodes = self.available_nodes_cache.read().await.clone();
        
        let mut available_nodes: Vec<ClusterNode> = all_nodes
            .into_iter()
            .filter(|node| node.status == NodeStatus::Healthy)
            .collect();

        // Filter nodes based on leader processing configuration
        if self.leader_can_process {
            // Leader can process: include all healthy nodes (leader + followers)
            available_nodes.retain(|node| 
                node.role == NodeRole::Leader || node.role == NodeRole::Follower
            );
        } else {
            // Leader only coordinates: include only healthy followers
            available_nodes.retain(|node| node.role == NodeRole::Follower);
        }

        // Filter out overloaded nodes
        let mut filtered_nodes = Vec::new();
        for node in available_nodes {
            let current_tasks = self.get_node_current_tasks(&node.node_id).await?;
            if current_tasks < self.config.max_tasks_per_node {
                filtered_nodes.push(node);
            }
        }

        Ok(filtered_nodes)
    }

    /// Get current number of tasks assigned to a node
    async fn get_node_current_tasks(&self, node_id: &str) -> Result<usize, ClusterError> {
        let tasks = self.metadata_store.get_tasks_by_node(node_id).await?;
        
        // Count only active tasks (assigned or processing)
        let active_count = tasks.iter()
            .filter(|task| matches!(task.state, TaskState::Assigned | TaskState::Processing))
            .count();

        Ok(active_count)
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
        info!("Task {} completed by node {} in {:.2}s", task_id, node_id, processing_duration);

        // Update task in metadata store
        self.metadata_store.complete_task(&task_id, processing_duration).await?;

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.active_tasks -= 1;
            stats.completed_tasks += 1;
            
            // Update average completion time
            let total_completed = stats.completed_tasks as f32;
            stats.avg_completion_time = (stats.avg_completion_time * (total_completed - 1.0) + processing_duration) / total_completed;
            
            let node_stats = stats.tasks_per_node.entry(node_id).or_default();
            node_stats.assigned = node_stats.assigned.saturating_sub(1);
            node_stats.completed += 1;
            
            // Update node average time
            let node_completed = node_stats.completed as f32;
            node_stats.avg_time = (node_stats.avg_time * (node_completed - 1.0) + processing_duration) / node_completed;
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
        warn!("Task {} failed on node {}: {}", task_id, node_id, error_message);

        // Update task in metadata store
        self.metadata_store.fail_task(&task_id, &error_message).await?;

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

    /// Refresh available nodes cache
    async fn refresh_available_nodes(&self) -> Result<(), ClusterError> {
        let nodes = self.metadata_store.get_all_nodes().await?;
        
        {
            let mut cache = self.available_nodes_cache.write().await;
            *cache = nodes;
        }

        debug!("Refreshed available nodes cache with {} nodes", 
               self.available_nodes_cache.read().await.len());

        Ok(())
    }

    /// Rebalance tasks across nodes (future implementation)
    async fn rebalance_tasks(&self) -> Result<(), ClusterError> {
        // TODO: Implement task rebalancing logic
        // This would move tasks from overloaded nodes to underloaded ones
        debug!("Task rebalancing not yet implemented");
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

        self.event_tx.send(event)
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

        self.event_tx.send(event)
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

        self.event_tx.send(event)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        Ok(())
    }

    /// Get scheduler statistics (external API)
    pub async fn get_stats(&self) -> Result<SchedulerStats, ClusterError> {
        let (response_tx, response_rx) = oneshot::channel();
        
        let event = SchedulerEvent::GetStats { response_tx };

        self.event_tx.send(event)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        response_rx.await
            .map_err(|_| ClusterError::InvalidOperation("Failed to get stats response".to_string()))
    }

    /// Get scheduler statistics directly (for testing - non-blocking)
    /// This method accesses stats directly without going through the event loop
    /// Use only for testing purposes when the background event loop is not running
    pub async fn get_stats_direct(&self) -> SchedulerStats {
        self.stats.read().await.clone()
    }

    /// Shutdown the scheduler gracefully
    pub async fn shutdown(&self) -> Result<(), ClusterError> {
        self.event_tx.send(SchedulerEvent::Shutdown)
            .map_err(|_| ClusterError::InvalidOperation("Scheduler channel closed".to_string()))?;

        Ok(())
    }
}