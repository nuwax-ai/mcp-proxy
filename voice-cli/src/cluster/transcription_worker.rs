use crate::models::{ClusterError, MetadataStore, TaskMetadata, ClusterTranscriptionResult};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};

/// Task assignment request from scheduler
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssignmentRequest {
    pub task_id: String,
    pub client_id: String,
    pub filename: String,
    pub audio_file_path: String,
    pub model: Option<String>,
    pub response_format: Option<String>,
    pub created_at: i64,
}

/// Task assignment response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssignmentResponse {
    pub success: bool,
    pub message: String,
    pub assigned_node_id: Option<String>,
}

/// Simple transcription worker that reuses existing voice-cli logic
pub struct SimpleTranscriptionWorker {
    /// Current node ID
    node_id: String,
    /// Metadata store for cluster information
    metadata_store: Arc<MetadataStore>,
    /// Worker configuration
    config: WorkerConfig,
    /// Channel for receiving worker events
    event_rx: mpsc::UnboundedReceiver<WorkerEvent>,
    /// Channel sender for worker events (cloneable)
    event_tx: mpsc::UnboundedSender<WorkerEvent>,
    /// Current processing statistics
    stats: WorkerStats,
}

/// Configuration for transcription worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Maximum concurrent tasks this worker can handle
    pub max_concurrent_tasks: usize,
    /// Timeout for transcription processing
    pub processing_timeout: Duration,
    /// Default model to use if none specified
    pub default_model: String,
    /// Default response format if none specified
    pub default_response_format: String,
    /// Whether to enable detailed logging
    pub enable_detailed_logging: bool,
    /// Cleanup temporary files after processing
    pub cleanup_temp_files: bool,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 3,
            processing_timeout: Duration::from_secs(300), // 5 minutes
            default_model: "base".to_string(),
            default_response_format: "json".to_string(),
            enable_detailed_logging: true,
            cleanup_temp_files: true,
        }
    }
}

/// Events for transcription worker
#[derive(Debug)]
pub enum WorkerEvent {
    /// Process a new task
    ProcessTask {
        task_request: TaskAssignmentRequest,
        response_tx: oneshot::Sender<Result<ClusterTranscriptionResult, ClusterError>>,
    },
    /// Get worker statistics
    GetStats {
        response_tx: oneshot::Sender<WorkerStats>,
    },
    /// Shutdown the worker
    Shutdown,
}

/// Worker processing statistics
#[derive(Debug, Clone, Default)]
pub struct WorkerStats {
    /// Total tasks processed
    pub total_processed: u64,
    /// Tasks completed successfully
    pub completed_tasks: u64,
    /// Tasks failed
    pub failed_tasks: u64,
    /// Currently processing tasks
    pub active_tasks: u64,
    /// Average processing time in seconds
    pub avg_processing_time: f32,
    /// Total processing time in seconds
    pub total_processing_time: f32,
    /// Worker uptime in seconds
    pub uptime_seconds: u64,
    /// Last task completion time
    pub last_completion_time: Option<i64>,
}

impl SimpleTranscriptionWorker {
    /// Create a new SimpleTranscriptionWorker
    pub fn new(
        node_id: String,
        metadata_store: Arc<MetadataStore>,
        config: WorkerConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            node_id,
            metadata_store,
            config,
            event_rx,
            event_tx,
            stats: WorkerStats::default(),
        }
    }

    /// Get a cloneable event sender for external use
    pub fn event_sender(&self) -> mpsc::UnboundedSender<WorkerEvent> {
        self.event_tx.clone()
    }

    /// Start the transcription worker
    pub async fn start(&mut self) -> Result<(), ClusterError> {
        info!("Starting transcription worker for node {}", self.node_id);

        let start_time = Instant::now();

        // Main event loop
        while let Some(event) = self.event_rx.recv().await {
            match event {
                WorkerEvent::ProcessTask { task_request, response_tx } => {
                    self.handle_process_task(task_request, response_tx).await;
                }
                WorkerEvent::GetStats { response_tx } => {
                    self.stats.uptime_seconds = start_time.elapsed().as_secs();
                    let _ = response_tx.send(self.stats.clone());
                }
                WorkerEvent::Shutdown => {
                    info!("Shutting down transcription worker for node {}", self.node_id);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handle task processing request
    async fn handle_process_task(
        &mut self,
        task_request: TaskAssignmentRequest,
        response_tx: oneshot::Sender<Result<ClusterTranscriptionResult, ClusterError>>,
    ) {
        let task_id = task_request.task_id.clone();
        let start_time = Instant::now();

        info!("Worker {} processing task {} for client {}", 
              self.node_id, task_id, task_request.client_id);

        // Update statistics
        self.stats.total_processed += 1;
        self.stats.active_tasks += 1;

        // Update task status to processing
        if let Err(e) = self.metadata_store.start_task_processing(&task_id).await {
            warn!("Failed to update task status to processing: {}", e);
        }

        // Process transcription with timeout
        let result = match timeout(
            self.config.processing_timeout,
            self.perform_transcription(&task_request)
        ).await {
            Ok(Ok(result)) => {
                let processing_duration = start_time.elapsed().as_secs_f32();
                
                // Update task as completed
                if let Err(e) = self.metadata_store.complete_task(&task_id, processing_duration).await {
                    error!("Failed to mark task as completed: {}", e);
                }

                // Update statistics
                self.stats.completed_tasks += 1;
                self.stats.total_processing_time += processing_duration;
                self.stats.avg_processing_time = self.stats.total_processing_time / self.stats.completed_tasks as f32;
                self.stats.last_completion_time = Some(chrono::Utc::now().timestamp());

                info!("Task {} completed successfully in {:.2}s", task_id, processing_duration);
                Ok(result)
            }
            Ok(Err(error)) => {
                // Processing failed
                if let Err(e) = self.metadata_store.fail_task(&task_id, &error.to_string()).await {
                    error!("Failed to mark task as failed: {}", e);
                }

                // Update statistics
                self.stats.failed_tasks += 1;

                error!("Task {} failed: {}", task_id, error);
                Err(error)
            }
            Err(_) => {
                // Timeout occurred
                let timeout_error = ClusterError::Timeout(format!("Task {} timed out after {:?}", task_id, self.config.processing_timeout));
                
                if let Err(e) = self.metadata_store.fail_task(&task_id, &timeout_error.to_string()).await {
                    error!("Failed to mark timed out task as failed: {}", e);
                }

                // Update statistics
                self.stats.failed_tasks += 1;

                error!("Task {} timed out", task_id);
                Err(timeout_error)
            }
        };

        // Update active tasks count
        self.stats.active_tasks -= 1;

        // Send response
        let _ = response_tx.send(result);
    }

    /// Perform the actual transcription using existing voice-cli logic
    async fn perform_transcription(
        &self,
        task_request: &TaskAssignmentRequest
    ) -> Result<ClusterTranscriptionResult, ClusterError> {
        
        // Determine model and response format
        let model = task_request.model.as_ref()
            .unwrap_or(&self.config.default_model);
        let response_format = task_request.response_format.as_ref()
            .unwrap_or(&self.config.default_response_format);

        if self.config.enable_detailed_logging {
            debug!("Transcribing file {} with model {} (format: {})", 
                   task_request.audio_file_path, model, response_format);
        }

        // Check if audio file exists
        if !std::path::Path::new(&task_request.audio_file_path).exists() {
            return Err(ClusterError::InvalidOperation(
                format!("Audio file not found: {}", task_request.audio_file_path)
            ));
        }

        // Use actual voice-toolkit for transcription
        let transcription_result = self.perform_real_transcription(task_request).await?;

        // Convert to cluster transcription result
        let cluster_result = ClusterTranscriptionResult {
            task_id: task_request.task_id.clone(),
            text: transcription_result,
            language: Some("en".to_string()),
            duration: None,
            processing_time: 0.0, // Will be calculated by caller
            processed_by: self.node_id.clone(),
            filename: task_request.filename.clone(),
        };

        // Cleanup temporary files if enabled
        if self.config.cleanup_temp_files {
            if let Err(e) = self.cleanup_temp_file(&task_request.audio_file_path).await {
                warn!("Failed to cleanup temporary file {}: {}", task_request.audio_file_path, e);
            }
        }

        Ok(cluster_result)
    }

    /// Perform real transcription using voice-toolkit
    async fn perform_real_transcription(
        &self,
        task_request: &TaskAssignmentRequest,
    ) -> Result<String, ClusterError> {
        use std::path::Path;
        
        let audio_path = Path::new(&task_request.audio_file_path);
        
        // Read audio file
        let audio_data = tokio::fs::read(audio_path).await
            .map_err(|e| ClusterError::InvalidOperation(
                format!("Failed to read audio file {}: {}", task_request.audio_file_path, e)
            ))?;
        
        // Create a temporary file for processing
        let temp_file = tempfile::NamedTempFile::new()
            .map_err(|e| ClusterError::InvalidOperation(
                format!("Failed to create temp file: {}", e)
            ))?;
        
        tokio::fs::write(temp_file.path(), &audio_data).await
            .map_err(|e| ClusterError::InvalidOperation(
                format!("Failed to write temp file: {}", e)
            ))?;
        
        // Ensure audio is Whisper-compatible
        let compatible_audio_path = voice_toolkit::audio::ensure_whisper_compatible(
            temp_file.path(),
            None
        ).map_err(|e| ClusterError::InvalidOperation(
            format!("Failed to convert audio to Whisper format: {}", e)
        ))?;
        
        // Get model path
        let model = task_request.model.as_ref()
            .unwrap_or(&self.config.default_model);
        
        // Construct model path (assuming models are stored in ~/.cache/whisper)
        let model_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".cache")
            .join("whisper")
            .join(format!("{}.bin", model));
        
        if !model_path.exists() {
            return Err(ClusterError::InvalidOperation(
                format!("Model file not found: {}. Please download the model first.", model_path.display())
            ));
        }
        
        // Perform transcription
        let transcription_result = tokio::task::spawn_blocking(move || {
            // Note: voice_toolkit::stt::transcribe_file is async, but spawn_blocking expects sync
            // We'll need to handle this differently
            tokio::runtime::Handle::current().block_on(async {
                voice_toolkit::stt::transcribe_file(
                    &compatible_audio_path.path,
                    &model_path
                ).await
            })
        }).await
        .map_err(|e| ClusterError::InvalidOperation(
            format!("Transcription task failed: {}", e)
        ))?
        .map_err(|e| ClusterError::InvalidOperation(
            format!("Transcription failed: {}", e)
        ))?;
        
        // Return the transcribed text
        Ok(transcription_result.text)
    }

    /// Cleanup temporary files after processing
    async fn cleanup_temp_file(&self, file_path: &str) -> Result<(), ClusterError> {
        // Only cleanup files in temp directories to avoid accidentally deleting user files
        let path = std::path::Path::new(file_path);
        
        if let Some(parent) = path.parent() {
            let parent_str = parent.to_string_lossy();
            if parent_str.contains("temp") || parent_str.contains("tmp") || parent_str.contains("cache") {
                if let Err(e) = tokio::fs::remove_file(file_path).await {
                    return Err(ClusterError::InvalidOperation(
                        format!("Failed to remove temp file {}: {}", file_path, e)
                    ));
                }
                debug!("Cleaned up temporary file: {}", file_path);
            }
        }

        Ok(())
    }

    /// Process a task (external API)
    pub async fn process_task(
        &self,
        task_request: TaskAssignmentRequest,
    ) -> Result<ClusterTranscriptionResult, ClusterError> {
        // Check if worker is at capacity
        if self.stats.active_tasks >= self.config.max_concurrent_tasks as u64 {
            return Err(ClusterError::InvalidOperation(
                format!("Worker {} is at capacity ({} active tasks)", 
                        self.node_id, self.stats.active_tasks)
            ));
        }

        let (response_tx, response_rx) = oneshot::channel();
        
        let event = WorkerEvent::ProcessTask {
            task_request,
            response_tx,
        };

        self.event_tx.send(event)
            .map_err(|_| ClusterError::InvalidOperation("Worker channel closed".to_string()))?;

        response_rx.await
            .map_err(|_| ClusterError::InvalidOperation("Failed to receive task response".to_string()))?
    }

    /// Get worker statistics (external API)
    pub async fn get_stats(&self) -> Result<WorkerStats, ClusterError> {
        let (response_tx, response_rx) = oneshot::channel();
        
        let event = WorkerEvent::GetStats { response_tx };

        self.event_tx.send(event)
            .map_err(|_| ClusterError::InvalidOperation("Worker channel closed".to_string()))?;

        response_rx.await
            .map_err(|_| ClusterError::InvalidOperation("Failed to get stats response".to_string()))
    }

    /// Get worker statistics directly (for testing - non-blocking)
    /// This method accesses stats directly without going through the event loop
    /// Use only for testing purposes when the background event loop is not running
    pub fn get_stats_direct(&self) -> WorkerStats {
        self.stats.clone()
    }

    /// Check if worker can accept more tasks
    pub fn can_accept_task(&self) -> bool {
        self.stats.active_tasks < self.config.max_concurrent_tasks as u64
    }

    /// Get current load as a percentage (0.0 to 1.0)
    pub fn get_load_percentage(&self) -> f32 {
        if self.config.max_concurrent_tasks == 0 {
            1.0
        } else {
            self.stats.active_tasks as f32 / self.config.max_concurrent_tasks as f32
        }
    }

    /// Shutdown the worker gracefully
    pub async fn shutdown(&self) -> Result<(), ClusterError> {
        self.event_tx.send(WorkerEvent::Shutdown)
            .map_err(|_| ClusterError::InvalidOperation("Worker channel closed".to_string()))?;

        Ok(())
    }
}

/// Helper function to create a task assignment request from task metadata
pub fn create_task_assignment_request(
    task: &TaskMetadata,
    audio_file_path: String,
) -> TaskAssignmentRequest {
    TaskAssignmentRequest {
        task_id: task.task_id.clone(),
        client_id: task.client_id.clone(),
        filename: task.filename.clone(),
        audio_file_path,
        model: task.model.clone(),
        response_format: task.response_format.clone(),
        created_at: task.created_at,
    }
}

/// Helper function to validate task assignment request
pub fn validate_task_assignment_request(
    request: &TaskAssignmentRequest,
) -> Result<(), ClusterError> {
    if request.task_id.is_empty() {
        return Err(ClusterError::InvalidOperation("Task ID cannot be empty".to_string()));
    }

    if request.client_id.is_empty() {
        return Err(ClusterError::InvalidOperation("Client ID cannot be empty".to_string()));
    }

    if request.filename.is_empty() {
        return Err(ClusterError::InvalidOperation("Filename cannot be empty".to_string()));
    }

    if request.audio_file_path.is_empty() {
        return Err(ClusterError::InvalidOperation("Audio file path cannot be empty".to_string()));
    }

    // Validate audio file exists
    if !std::path::Path::new(&request.audio_file_path).exists() {
        return Err(ClusterError::InvalidOperation(
            format!("Audio file does not exist: {}", request.audio_file_path)
        ));
    }

    Ok(())
}