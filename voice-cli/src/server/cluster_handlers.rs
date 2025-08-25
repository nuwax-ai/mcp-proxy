use crate::cluster::{SimpleTaskScheduler, SimpleTranscriptionWorker};
use crate::grpc::{ClusterTaskManager, TaskManagerConfig, TaskManagerStats};
use crate::models::MetadataStore;
use crate::models::{
    ClusterNode, Config, HttpResult, ModelsResponse, NodeRole, NodeStatus, TaskMetadata,
    TranscriptionResponse,
};
use crate::services::{ModelService, TranscriptionWorkerPool};
use crate::VoiceCliError;
use axum::{
    extract::{Multipart, State},
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use uuid::Uuid;

use utoipa;

/// Cluster-aware application state that extends the original AppState
#[derive(Clone)]
pub struct ClusterAppState {
    pub config: Arc<Config>,
    pub transcription_worker_pool: Arc<TranscriptionWorkerPool>,
    pub model_service: Arc<ModelService>,
    pub start_time: SystemTime,

    // Cluster-specific components
    pub cluster_enabled: bool,
    pub cluster_node: Option<ClusterNode>,
    pub metadata_store: Option<Arc<MetadataStore>>,
    pub task_scheduler: Option<Arc<SimpleTaskScheduler>>,
    pub transcription_worker: Option<Arc<SimpleTranscriptionWorker>>,
    pub task_manager: Option<Arc<ClusterTaskManager>>,
}

impl ClusterAppState {
    /// Create new cluster-aware app state
    pub async fn new(config: Arc<Config>) -> crate::Result<Self> {
        let transcription_worker_pool =
            Arc::new(TranscriptionWorkerPool::new(config.clone()).await?);
        let model_service = Arc::new(ModelService::new((*config).clone()));

        // Initialize cluster components if enabled
        let (
            cluster_enabled,
            cluster_node,
            metadata_store,
            task_scheduler,
            transcription_worker,
            task_manager,
        ) = if config.cluster.enabled {
            info!("Initializing cluster components");

            // Create cluster node
            let cluster_node = ClusterNode::new(
                config.cluster.node_id.clone(),
                config.cluster.bind_address.clone(),
                config.cluster.grpc_port,
                config.cluster.http_port,
            );

            // Initialize metadata store
            let metadata_store = Arc::new(
                MetadataStore::new(&config.cluster.metadata_db_path).map_err(|e| {
                    VoiceCliError::Config(format!("Failed to initialize metadata store: {}", e))
                })?,
            );

            // Create task scheduler (simplified for now)
            let task_scheduler = Arc::new(SimpleTaskScheduler::new(
                metadata_store.clone(),
                config.cluster.leader_can_process_tasks,
                cluster_node.node_id.clone(),
                crate::cluster::SchedulerConfig::default(),
            ));

            // Create transcription worker if enabled
            let transcription_worker = if config.cluster.leader_can_process_tasks
                || cluster_node.role != NodeRole::Leader
            {
                Some(Arc::new(SimpleTranscriptionWorker::new(
                    cluster_node.node_id.clone(),
                    metadata_store.clone(),
                    crate::cluster::WorkerConfig::default(),
                )))
            } else {
                None
            };

            // Create task manager
            let task_manager = Arc::new(ClusterTaskManager::new(
                cluster_node.clone(),
                metadata_store.clone(),
                Some(task_scheduler.clone()),
                transcription_worker.clone(),
                TaskManagerConfig::default(),
            ));

            (
                true,
                Some(cluster_node),
                Some(metadata_store),
                Some(task_scheduler),
                transcription_worker,
                Some(task_manager),
            )
        } else {
            info!("Cluster mode disabled, running in single-node mode");
            (false, None, None, None, None, None)
        };

        Ok(Self {
            config,
            transcription_worker_pool,
            model_service,
            start_time: SystemTime::now(),
            cluster_enabled,
            cluster_node,
            metadata_store,
            task_scheduler,
            transcription_worker,
            task_manager,
        })
    }

    /// Check if this node is a cluster leader
    pub fn is_cluster_leader(&self) -> bool {
        self.cluster_node
            .as_ref()
            .map(|node| node.role == NodeRole::Leader)
            .unwrap_or(false)
    }

    /// Check if this node can process tasks
    pub fn can_process_tasks(&self) -> bool {
        if !self.cluster_enabled {
            return true; // Single-node mode always processes tasks
        }

        self.config.cluster.leader_can_process_tasks || !self.is_cluster_leader()
    }

    /// Get cluster statistics
    pub async fn get_cluster_stats(&self) -> Option<ClusterStats> {
        if !self.cluster_enabled {
            return None;
        }

        let task_manager_stats = if let Some(ref task_manager) = self.task_manager {
            Some(task_manager.get_stats().await)
        } else {
            None
        };

        let cluster_nodes = if let Some(ref metadata_store) = self.metadata_store {
            metadata_store.get_all_nodes().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        Some(ClusterStats {
            enabled: true,
            node_id: self
                .cluster_node
                .as_ref()
                .map(|n| n.node_id.clone())
                .unwrap_or_default(),
            is_leader: self.is_cluster_leader(),
            can_process_tasks: self.can_process_tasks(),
            total_nodes: cluster_nodes.len(),
            healthy_nodes: cluster_nodes
                .iter()
                .filter(|n| n.status == NodeStatus::Healthy)
                .count(),
            task_manager_stats,
        })
    }

    /// Gracefully shutdown the cluster-aware app state
    pub async fn shutdown(self) {
        info!("Shutting down cluster-aware application state");

        // Shutdown cluster components
        if self.cluster_enabled {
            // TODO: Implement graceful cluster shutdown
            info!("Shutting down cluster components");
        }

        // Shutdown worker pool
        if let Ok(worker_pool) = Arc::try_unwrap(self.transcription_worker_pool) {
            worker_pool.shutdown().await;
        } else {
            warn!("Could not shutdown worker pool - multiple references exist");
        }

        info!("Cluster-aware application state shutdown complete");
    }

    /// Get the current cluster leader node
    pub async fn get_cluster_leader(&self) -> Option<ClusterNode> {
        if !self.cluster_enabled {
            return None;
        }

        if let Some(ref metadata_store) = self.metadata_store {
            match metadata_store.get_all_nodes().await {
                Ok(nodes) => nodes.into_iter().find(|node| node.role == NodeRole::Leader),
                Err(e) => {
                    warn!("Failed to get cluster nodes: {}", e);
                    None
                }
            }
        } else {
            None
        }
    }

    /// Get healthy worker nodes
    pub async fn get_healthy_workers(&self) -> Vec<ClusterNode> {
        if !self.cluster_enabled {
            return Vec::new();
        }

        if let Some(ref metadata_store) = self.metadata_store {
            match metadata_store.get_all_nodes().await {
                Ok(nodes) => nodes
                    .into_iter()
                    .filter(|node| {
                        node.role != NodeRole::Leader && node.status == NodeStatus::Healthy
                    })
                    .collect(),
                Err(e) => {
                    warn!("Failed to get cluster nodes: {}", e);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    }

    /// Check if cluster has capacity for new tasks
    pub async fn has_cluster_capacity(&self) -> bool {
        if !self.cluster_enabled {
            return true; // Single-node always has capacity
        }

        let healthy_workers = self.get_healthy_workers().await;
        let total_capacity = healthy_workers.len();

        // Simple capacity check - can be enhanced with actual load metrics
        total_capacity > 0
    }
}

/// Cluster statistics for health endpoint
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ClusterStats {
    pub enabled: bool,
    pub node_id: String,
    pub is_leader: bool,
    pub can_process_tasks: bool,
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub task_manager_stats: Option<TaskManagerStats>,
}

/// Enhanced health check endpoint with cluster information
/// GET /health
#[utoipa::path(
    get,
    path = "/health",
    tag = "Health",
    summary = "Get service health status with cluster information",
    description = "Returns the current health status of the voice-cli service, including uptime, loaded models, version information, and cluster status when in cluster mode.",
    responses(
        (status = 200, description = "Service is healthy", body = EnhancedHealthResponse),
        (status = 500, description = "Service error", body = String)
    )
)]
pub async fn cluster_health_handler(
    State(state): State<ClusterAppState>,
) -> Result<Json<EnhancedHealthResponse>, VoiceCliError> {
    let uptime = state.start_time.elapsed().unwrap_or_default().as_secs();
    let loaded_models = state.model_service.list_loaded_models().await?;
    let cluster_stats = state.get_cluster_stats().await;

    let response = EnhancedHealthResponse {
        status: "healthy".to_string(),
        models_loaded: loaded_models,
        uptime,
        version: env!("CARGO_PKG_VERSION").to_string(),
        cluster: cluster_stats,
    };

    Ok(Json(response))
}

/// Enhanced health response with cluster information
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EnhancedHealthResponse {
    pub status: String,
    pub models_loaded: Vec<String>,
    pub uptime: u64,
    pub version: String,
    pub cluster: Option<ClusterStats>,
}

/// Cluster-aware transcription handler that can distribute tasks
/// POST /transcribe
#[utoipa::path(
    post,
    path = "/transcribe",
    tag = "Transcription",
    summary = "Transcribe audio to text with cluster support",
    description = "Upload an audio file and get the transcribed text with automatic language detection. In cluster mode, tasks may be distributed to worker nodes for processing. Supports multiple audio formats (MP3, WAV, FLAC, M4A, AAC, OGG) with automatic format conversion. Maximum file size is 200MB.",
    request_body(
        content = String,
        description = "Multipart form data with audio file and optional parameters",
        content_type = "multipart/form-data"
    ),
    responses(
        (status = 200, description = "Transcription completed successfully", body = crate::models::HttpResult<TranscriptionResponse>),
        (status = 400, description = "Invalid request - missing audio file, unsupported format, or invalid parameters", body = crate::models::HttpResult<String>),
        (status = 413, description = "File too large - exceeds 200MB limit", body = crate::models::HttpResult<String>),
        (status = 500, description = "Transcription failed due to server error", body = crate::models::HttpResult<String>)
    ),
)]
pub async fn cluster_transcribe_handler(
    State(state): State<ClusterAppState>,
    multipart: Multipart,
) -> crate::models::HttpResult<TranscriptionResponse> {
    let start_time = Instant::now();
    let task_id = format!(
        "task_{}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        std::process::id()
    );

    info!("Starting cluster-aware transcription request {}", task_id);

    // Extract multipart form data (reuse existing logic)
    let (audio_data, request) = match extract_transcription_request(multipart).await {
        Ok(result) => result,
        Err(e) => return HttpResult::from(e),
    };

    // Validate audio file (reuse existing logic)
    if let Err(e) = validate_audio_file(
        &audio_data,
        &request.filename,
        state.config.server.max_file_size,
    ) {
        return HttpResult::from(e);
    }

    // Decide how to process the task based on cluster configuration and load
    match determine_task_processing_strategy(&state).await {
        TaskProcessingStrategy::DistributeToCluster => {
            info!("Distributing task {} to cluster workers", task_id);
            distribute_task_to_cluster(&state, task_id, audio_data, request, start_time).await
        }
        TaskProcessingStrategy::ProcessLocallyCluster => {
            info!("Processing task {} locally in cluster mode", task_id);
            process_task_locally_cluster(&state, task_id, audio_data, request, start_time).await
        }
        TaskProcessingStrategy::ProcessLocallySingle => {
            info!("Processing task {} in single-node mode", task_id);
            process_task_locally_single(&state, task_id, audio_data, request, start_time).await
        }
    }
}

/// Strategy for processing transcription tasks
#[derive(Debug, Clone)]
enum TaskProcessingStrategy {
    /// Distribute task to cluster workers (leader coordination mode)
    DistributeToCluster,
    /// Process locally in cluster mode (cluster-aware but local processing)
    ProcessLocallyCluster,
    /// Process locally in single-node mode (no cluster)
    ProcessLocallySingle,
}

/// Determine the optimal task processing strategy based on cluster state
async fn determine_task_processing_strategy(state: &ClusterAppState) -> TaskProcessingStrategy {
    if !state.cluster_enabled {
        return TaskProcessingStrategy::ProcessLocallySingle;
    }

    // Check if this is a leader that doesn't process tasks directly
    if state.is_cluster_leader() && !state.can_process_tasks() {
        // Check if we have healthy workers to distribute to
        let healthy_workers = state.get_healthy_workers().await;
        if healthy_workers.is_empty() {
            warn!("No healthy workers available, leader will process task locally despite configuration");
            return TaskProcessingStrategy::ProcessLocallyCluster;
        }

        // Leader should distribute tasks to workers
        info!(
            "Leader distributing task to {} healthy workers",
            healthy_workers.len()
        );
        return TaskProcessingStrategy::DistributeToCluster;
    }

    // Check cluster capacity and health
    if let Some(cluster_stats) = state.get_cluster_stats().await {
        let healthy_workers = cluster_stats.healthy_nodes.saturating_sub(1); // Exclude leader

        // If we're a leader that can process tasks, decide based on cluster load
        if state.is_cluster_leader() && state.can_process_tasks() {
            if healthy_workers > 0 && state.has_cluster_capacity().await {
                info!(
                    "Leader can process tasks, {} workers available, processing locally",
                    healthy_workers
                );
                TaskProcessingStrategy::ProcessLocallyCluster
            } else {
                info!("Limited cluster capacity, leader processing locally");
                TaskProcessingStrategy::ProcessLocallyCluster
            }
        } else {
            // Worker node or single-node cluster
            TaskProcessingStrategy::ProcessLocallyCluster
        }
    } else {
        // Fallback to cluster-aware local processing
        TaskProcessingStrategy::ProcessLocallyCluster
    }
}

/// Distribute task to cluster workers
async fn distribute_task_to_cluster(
    state: &ClusterAppState,
    task_id: String,
    audio_data: bytes::Bytes,
    request: crate::models::worker::TranscriptionRequest,
    start_time: Instant,
) -> crate::models::HttpResult<TranscriptionResponse> {
    // Create task metadata
    let mut task_metadata = TaskMetadata::new(
        task_id.clone(),
        format!("http_client_{}", Uuid::new_v4().simple()),
        request.filename.clone(),
    );
    task_metadata.model = request.model;
    task_metadata.response_format = request.response_format;

    // Submit task via task manager with real file sharing
    if let Some(ref task_manager) = state.task_manager {
        info!(
            "Distributing task {} to cluster with real file sharing",
            task_id
        );

        // Create file share instance for this task
        use crate::cluster::file_share::{ClusterFileShare, FileShareConfig, TaskDistributor};
        use std::sync::Arc;

        // Configure file sharing strategy
        let file_share_config = FileShareConfig {
            storage_base_dir: std::path::PathBuf::from("./cluster-storage"),
            max_file_size: state.config.server.max_file_size as u64,
            ..Default::default()
        };

        // Get node ID from cluster configuration
        let node_id = state
            .cluster_node
            .as_ref()
            .map(|n| n.node_id.clone())
            .unwrap_or_else(|| "unknown-node".to_string());

        // Create file share manager
        match ClusterFileShare::new(file_share_config, node_id.clone()).await {
            Ok(file_share) => {
                let file_share = Arc::new(file_share);
                let distributor = TaskDistributor::new(file_share, node_id);

                // Distribute task with real file sharing
                match distributor
                    .distribute_task_with_file_sharing(&mut task_metadata, audio_data)
                    .await
                {
                    Ok(file_id) => {
                        info!(
                            "Task {} audio file stored successfully with ID {}",
                            task_id, file_id
                        );

                        // Store task metadata in the metadata store
                        if let Some(ref metadata_store) = state.metadata_store {
                            match metadata_store.create_task(&task_metadata).await {
                                Ok(_) => {
                                    info!("Task {} metadata stored successfully", task_id);

                                    // Submit task to cluster for processing
                                    let audio_file_path =
                                        task_metadata.audio_file_path.clone().unwrap_or_default();
                                    match task_manager
                                        .submit_task(&task_metadata, &audio_file_path)
                                        .await
                                    {
                                        Ok(assigned_node) => {
                                            info!(
                                                "Task {} successfully assigned to node {}",
                                                task_id, assigned_node
                                            );

                                            let response = TranscriptionResponse {
                                                text: format!("Task {} successfully distributed to cluster node {}", task_id, assigned_node),
                                                language: Some("en".to_string()),
                                                duration: None,
                                                segments: Vec::new(),
                                                processing_time: start_time.elapsed().as_secs_f32(),
                                            };

                                            HttpResult::success(response)
                                        }
                                        Err(e) => {
                                            error!(
                                                "Failed to assign task {} to cluster: {}",
                                                task_id, e
                                            );
                                            HttpResult::from(VoiceCliError::Config(format!(
                                                "Task assignment failed: {}",
                                                e
                                            )))
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to store task metadata: {}", e);
                                    HttpResult::from(VoiceCliError::Config(format!(
                                        "Failed to store task: {}",
                                        e
                                    )))
                                }
                            }
                        } else {
                            HttpResult::from(VoiceCliError::Config(
                                "Metadata store not available".to_string(),
                            ))
                        }
                    }
                    Err(e) => {
                        error!("Failed to store audio file for task {}: {}", task_id, e);
                        HttpResult::from(VoiceCliError::Config(format!(
                            "File sharing failed: {}",
                            e
                        )))
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to initialize file sharing for task {}: {}",
                    task_id, e
                );
                HttpResult::from(VoiceCliError::Config(format!(
                    "File sharing initialization failed: {}",
                    e
                )))
            }
        }
    } else {
        HttpResult::from(VoiceCliError::Config(
            "Task manager not available".to_string(),
        ))
    }
}

/// Process task locally in cluster mode
async fn process_task_locally_cluster(
    state: &ClusterAppState,
    task_id: String,
    audio_data: bytes::Bytes,
    request: crate::models::worker::TranscriptionRequest,
    start_time: Instant,
) -> crate::models::HttpResult<TranscriptionResponse> {
    info!("Processing task {} locally in cluster mode", task_id);

    // Create task metadata for cluster tracking
    if let Some(ref metadata_store) = state.metadata_store {
        let mut task_metadata = TaskMetadata::new(
            task_id.clone(),
            format!("local_client_{}", Uuid::new_v4().simple()),
            request.filename.clone(),
        );
        task_metadata.model = request.model.clone();
        task_metadata.response_format = request.response_format.clone();
        task_metadata.assigned_node = state.cluster_node.as_ref().map(|n| n.node_id.clone());

        // Store the task metadata
        if let Err(e) = metadata_store.create_task(&task_metadata).await {
            warn!("Failed to store task metadata in cluster mode: {}", e);
        }
    }

    // Use cluster transcription worker if available
    if let Some(ref _cluster_worker) = state.transcription_worker {
        info!("Using cluster transcription worker for task {}", task_id);

        // For now, fall back to the single-node processing until cluster worker integration is complete
        // TODO: Implement direct cluster worker processing
        warn!("Cluster worker direct integration not yet implemented - falling back to single-node processing");
    }

    // Fall back to single-node processing
    process_task_locally_single(state, task_id, audio_data, request, start_time).await
}

/// Process task locally in single-node mode (existing logic)
async fn process_task_locally_single(
    state: &ClusterAppState,
    task_id: String,
    audio_data: bytes::Bytes,
    request: crate::models::worker::TranscriptionRequest,
    start_time: Instant,
) -> crate::models::HttpResult<TranscriptionResponse> {
    // Create result channel for receiving worker response
    let (result_sender, result_receiver) = tokio::sync::oneshot::channel();

    // Create transcription task (reuse existing structure)
    let task = crate::models::TranscriptionTask {
        task_id: task_id.clone(),
        audio_data,
        filename: request.filename,
        model: request.model,
        response_format: request.response_format,
        result_sender,
    };

    // Submit task to worker pool
    if let Err(e) = state.transcription_worker_pool.submit_task(task).await {
        return HttpResult::from(e);
    }

    // Wait for result from worker
    let worker_result = match result_receiver.await {
        Ok(result) => result,
        Err(_) => {
            return HttpResult::from(VoiceCliError::WorkerPoolError(
                "Worker result channel closed".to_string(),
            ));
        }
    };

    // Handle worker result (reuse existing logic)
    match worker_result {
        crate::models::TranscriptionResult {
            success: true,
            response: Some(mut response),
            ..
        } => {
            response.processing_time = start_time.elapsed().as_secs_f32();

            info!(
                "Transcription {} completed successfully in {:.2}s, text length: {} chars",
                task_id,
                response.processing_time,
                response.text.len()
            );

            HttpResult::success(response)
        }
        crate::models::TranscriptionResult {
            success: false,
            error: Some(error),
            ..
        } => {
            error!("Transcription {} failed: {}", task_id, error);
            HttpResult::from(error)
        }
        _ => {
            error!("Invalid worker result for task {}", task_id);
            HttpResult::from(VoiceCliError::TranscriptionFailed(
                "Invalid worker result".to_string(),
            ))
        }
    }
}

/// Cluster-aware models list handler that delegates to the original handler
/// GET /models
#[utoipa::path(
    get,
    path = "/models",
    tag = "Models",
    summary = "List available Whisper models (cluster-aware)",
    description = "Returns information about all available Whisper models, currently loaded models, and detailed model information including file sizes and memory usage. In cluster mode, this provides consistent model information across the cluster.",
    responses(
        (status = 200, description = "Models information retrieved successfully", body = crate::models::HttpResult<ModelsResponse>),
        (status = 500, description = "Failed to retrieve models information", body = crate::models::HttpResult<String>)
    )
)]
pub async fn cluster_models_list_handler(
    State(state): State<ClusterAppState>,
) -> crate::models::HttpResult<ModelsResponse> {
    // Create a compatible AppState for delegation
    let app_state = crate::server::handlers::AppState {
        config: state.config.clone(),
        transcription_worker_pool: state.transcription_worker_pool.clone(),
        model_service: state.model_service.clone(),
        start_time: state.start_time,
    };

    // Delegate to the original models handler to ensure identical behavior
    crate::server::handlers::models_list_handler(axum::extract::State(app_state)).await
}

// Re-export the existing helper functions to ensure API compatibility
pub use crate::server::handlers::{extract_transcription_request, validate_audio_file};
