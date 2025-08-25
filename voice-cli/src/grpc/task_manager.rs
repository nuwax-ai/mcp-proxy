use crate::cluster::{SimpleTaskScheduler, SimpleTranscriptionWorker};
use crate::grpc::proto::{NodeStatus, TaskState};
use crate::grpc::{connect_to_cluster_node, AudioClusterClient};
use crate::models::{ClusterError, ClusterNode, MetadataStore, NodeRole, TaskMetadata};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

/// Manages task assignment and completion reporting across the cluster via gRPC
pub struct ClusterTaskManager {
    /// Current node information
    node_info: ClusterNode,
    /// Metadata store for persistence
    metadata_store: Arc<MetadataStore>,
    /// Task scheduler (for leaders)
    task_scheduler: Option<Arc<SimpleTaskScheduler>>,
    /// Transcription worker (for processing nodes)
    transcription_worker: Option<Arc<SimpleTranscriptionWorker>>,
    /// Connected gRPC clients to other cluster nodes
    cluster_clients: Arc<RwLock<HashMap<String, AudioClusterClient>>>,
    /// Configuration for task management
    config: TaskManagerConfig,
}

#[derive(Debug, Clone)]
pub struct TaskManagerConfig {
    /// Interval for checking pending tasks
    pub task_check_interval: Duration,
    /// Interval for reporting heartbeats
    pub heartbeat_interval: Duration,
    /// Maximum retries for failed operations
    pub max_retries: u32,
    /// Timeout for gRPC operations
    pub operation_timeout: Duration,
}

impl Default for TaskManagerConfig {
    fn default() -> Self {
        Self {
            task_check_interval: Duration::from_secs(5),
            heartbeat_interval: Duration::from_secs(10),
            max_retries: 3,
            operation_timeout: Duration::from_secs(30),
        }
    }
}

impl ClusterTaskManager {
    /// Create a new task manager
    pub fn new(
        node_info: ClusterNode,
        metadata_store: Arc<MetadataStore>,
        task_scheduler: Option<Arc<SimpleTaskScheduler>>,
        transcription_worker: Option<Arc<SimpleTranscriptionWorker>>,
        config: TaskManagerConfig,
    ) -> Self {
        Self {
            node_info,
            metadata_store,
            task_scheduler,
            transcription_worker,
            cluster_clients: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Start the task manager background processes
    pub async fn start(&self) -> Result<(), ClusterError> {
        info!(
            "Starting cluster task manager for node {}",
            self.node_info.node_id
        );

        // Start task processing loop
        let task_processor_handle = self.start_task_processor();

        // Start heartbeat sender (if not a leader)
        let heartbeat_handle = if self.node_info.role != NodeRole::Leader {
            Some(self.start_heartbeat_sender())
        } else {
            None
        };

        // Wait for either process to complete (they should run indefinitely)
        tokio::select! {
            result = task_processor_handle => {
                error!("Task processor stopped unexpectedly: {:?}", result);
            }
            result = async {
                match heartbeat_handle {
                    Some(handle) => handle.await,
                    None => std::future::pending().await, // Never completes for leaders
                }
            } => {
                error!("Heartbeat sender stopped unexpectedly: {:?}", result);
            }
        }

        Ok(())
    }

    /// Connect to a cluster node
    pub async fn connect_to_node(&self, node: &ClusterNode) -> Result<(), ClusterError> {
        let address = format!("{}:{}", node.address, node.grpc_port);

        info!("Connecting to cluster node {} at {}", node.node_id, address);

        match connect_to_cluster_node(&address).await {
            Ok(client) => {
                let mut clients = self.cluster_clients.write().await;
                clients.insert(node.node_id.clone(), client);
                info!("Successfully connected to node {}", node.node_id);
                Ok(())
            }
            Err(e) => {
                warn!("Failed to connect to node {}: {}", node.node_id, e);
                Err(e)
            }
        }
    }

    /// Disconnect from a cluster node
    pub async fn disconnect_from_node(&self, node_id: &str) {
        let mut clients = self.cluster_clients.write().await;
        if clients.remove(node_id).is_some() {
            info!("Disconnected from node {}", node_id);
        }
    }

    /// Submit a task for processing to the cluster leader
    pub async fn submit_task(
        &self,
        task: &TaskMetadata,
        audio_file_path: &str,
    ) -> Result<String, ClusterError> {
        // Find the leader node
        let leader_node = self.find_leader_node().await?;

        // Get or create connection to leader
        let mut client = self.get_or_create_client(&leader_node).await?;

        // Submit task assignment request
        let response = client
            .assign_task(
                &task.task_id,
                &task.client_id,
                &task.filename,
                audio_file_path,
                task.model.clone(),
                task.response_format.clone(),
                task.created_at,
            )
            .await?;

        if response.success {
            info!(
                "Task {} submitted successfully, assigned to node {}",
                task.task_id, response.assigned_node_id
            );
            Ok(response.assigned_node_id)
        } else {
            Err(ClusterError::InvalidOperation(format!(
                "Failed to submit task: {}",
                response.message
            )))
        }
    }

    /// Report task completion to the cluster leader
    pub async fn report_completion(
        &self,
        task_id: &str,
        success: bool,
        result_data: Option<String>,
        error_message: Option<String>,
    ) -> Result<(), ClusterError> {
        let leader_node = self.find_leader_node().await?;
        let mut client = self.get_or_create_client(&leader_node).await?;

        let final_state = if success {
            TaskState::Completed
        } else {
            TaskState::Failed
        };

        let response = client
            .report_task_completion(
                task_id,
                final_state,
                error_message,
                result_data,
                Utc::now().timestamp(),
            )
            .await?;

        if response.success {
            info!("Task {} completion reported successfully", task_id);
            Ok(())
        } else {
            Err(ClusterError::InvalidOperation(format!(
                "Failed to report completion: {}",
                response.message
            )))
        }
    }

    /// Process assigned tasks (for worker nodes)
    async fn start_task_processor(&self) -> Result<(), ClusterError> {
        let mut interval = interval(self.config.task_check_interval);

        loop {
            interval.tick().await;

            if let Err(e) = self.process_assigned_tasks().await {
                error!("Error processing assigned tasks: {}", e);
            }
        }
    }

    /// Check for and process assigned tasks
    async fn process_assigned_tasks(&self) -> Result<(), ClusterError> {
        // Only process tasks if we have a transcription worker
        let worker = match &self.transcription_worker {
            Some(worker) => worker,
            None => return Ok(()),
        };

        // Get tasks assigned to this node
        let assigned_tasks = self
            .metadata_store
            .get_tasks_by_node(&self.node_info.node_id)
            .await?;

        for task in assigned_tasks {
            if task.state == crate::models::TaskState::Assigned {
                debug!("Processing assigned task: {}", task.task_id);

                // Process the task
                match self.process_single_task(&task, worker).await {
                    Ok(result) => {
                        // Report successful completion
                        if let Err(e) = self
                            .report_completion(&task.task_id, true, Some(result), None)
                            .await
                        {
                            error!("Failed to report task completion: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Task {} failed: {}", task.task_id, e);

                        // Report failure
                        if let Err(e) = self
                            .report_completion(&task.task_id, false, None, Some(e.to_string()))
                            .await
                        {
                            error!("Failed to report task failure: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Process a single task using the transcription worker
    async fn process_single_task(
        &self,
        task: &TaskMetadata,
        worker: &SimpleTranscriptionWorker,
    ) -> Result<String, ClusterError> {
        use crate::cluster::transcription_worker::{TaskAssignmentRequest, WorkerEvent};
        use tokio::sync::oneshot;

        // Create task assignment request from task metadata
        let task_request = TaskAssignmentRequest {
            task_id: task.task_id.clone(),
            client_id: task.client_id.clone(),
            filename: task.filename.clone(),
            audio_file_path: task.audio_file_path.clone().unwrap_or_else(|| {
                // If no audio_file_path in metadata, construct from filename
                format!("/tmp/audio/{}", task.filename)
            }),
            model: task.model.clone(),
            response_format: task.response_format.clone(),
            created_at: task.created_at,
        };

        // Create a oneshot channel for the response
        let (response_tx, response_rx) = oneshot::channel();

        // Send the task to the transcription worker
        let event_sender = worker.event_sender();
        event_sender
            .send(WorkerEvent::ProcessTask {
                task_request,
                response_tx,
            })
            .map_err(|_| {
                ClusterError::InvalidOperation(
                    "Failed to send task to transcription worker".to_string(),
                )
            })?;

        // Wait for the transcription result
        let transcription_result = response_rx.await.map_err(|_| {
            ClusterError::InvalidOperation(
                "Failed to receive response from transcription worker".to_string(),
            )
        })??;

        // Convert the result to JSON format expected by the cluster
        let result_json = serde_json::json!({
            "text": transcription_result.text,
            "language": transcription_result.language,
            "duration": transcription_result.duration,
            "processing_time": transcription_result.processing_time,
            "processed_by": transcription_result.processed_by,
            "filename": transcription_result.filename,
            "task_id": transcription_result.task_id
        });

        Ok(result_json.to_string())
    }

    /// Send periodic heartbeats to the leader
    async fn start_heartbeat_sender(&self) -> Result<(), ClusterError> {
        let mut interval = interval(self.config.heartbeat_interval);

        loop {
            interval.tick().await;

            if let Err(e) = self.send_heartbeat().await {
                warn!("Failed to send heartbeat: {}", e);
            }
        }
    }

    /// Send a heartbeat to the leader
    async fn send_heartbeat(&self) -> Result<(), ClusterError> {
        let leader_node = self.find_leader_node().await?;
        let mut client = self.get_or_create_client(&leader_node).await?;

        let response = client
            .send_heartbeat(
                &self.node_info.node_id,
                NodeStatus::Healthy,
                Utc::now().timestamp(),
            )
            .await?;

        if response.success {
            debug!("Heartbeat sent successfully to leader");
        } else {
            warn!("Heartbeat rejected: {}", response.message);
        }

        Ok(())
    }

    /// Find the current leader node
    async fn find_leader_node(&self) -> Result<ClusterNode, ClusterError> {
        let nodes = self.metadata_store.get_all_nodes().await?;

        nodes
            .into_iter()
            .find(|node| node.role == NodeRole::Leader)
            .ok_or_else(|| ClusterError::NoAvailableNodes)
    }

    /// Get or create a gRPC client for the specified node
    async fn get_or_create_client(
        &self,
        node: &ClusterNode,
    ) -> Result<AudioClusterClient, ClusterError> {
        let address = format!("{}:{}", node.address, node.grpc_port);

        // Check if we already have a healthy client
        {
            let clients = self.cluster_clients.read().await;
            if let Some(existing_client) = clients.get(&node.node_id) {
                // For now, return the existing client
                // In a production system, we would check client health here
                return Ok(existing_client.clone());
            }
        }

        // Create a new client with connection pooling
        info!(
            "Creating new gRPC client for node {} at {}",
            node.node_id, address
        );
        let client = connect_to_cluster_node(&address).await?;

        // Store the client for reuse
        {
            let mut clients = self.cluster_clients.write().await;
            clients.insert(node.node_id.clone(), client.clone());
        }

        Ok(client)
    }

    /// Get task manager statistics
    pub async fn get_stats(&self) -> TaskManagerStats {
        let total_connections = self.cluster_clients.read().await.len();

        TaskManagerStats {
            node_id: self.node_info.node_id.clone(),
            total_connections,
            is_leader: self.node_info.role == NodeRole::Leader,
            has_scheduler: self.task_scheduler.is_some(),
            has_worker: self.transcription_worker.is_some(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TaskManagerStats {
    pub node_id: String,
    pub total_connections: usize,
    pub is_leader: bool,
    pub has_scheduler: bool,
    pub has_worker: bool,
}

/// Helper function to create a task manager with default configuration
pub fn create_task_manager(
    node_info: ClusterNode,
    metadata_store: Arc<MetadataStore>,
    task_scheduler: Option<Arc<SimpleTaskScheduler>>,
    transcription_worker: Option<Arc<SimpleTranscriptionWorker>>,
) -> ClusterTaskManager {
    ClusterTaskManager::new(
        node_info,
        metadata_store,
        task_scheduler,
        transcription_worker,
        TaskManagerConfig::default(),
    )
}
