use crate::models::{TranscriptionTask, TranscriptionResult, WorkerProcessedAudio, Config, TranscriptionResponse, Segment};
use crate::services::ModelService;
use crate::VoiceCliError;
use bytes::Bytes;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tracing::{info, warn, debug, error};

/// Worker pool for handling transcription tasks with SPMC architecture
pub struct TranscriptionWorkerPool {
    /// Channel sender for submitting tasks to workers
    task_sender: mpsc::Sender<TranscriptionTask>,
    /// Worker handles for cleanup
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
    /// Configuration
    config: Arc<Config>,
}

impl TranscriptionWorkerPool {
    /// Create a new transcription worker pool
    pub async fn new(config: Arc<Config>) -> Result<Self, VoiceCliError> {
        let worker_count = config.whisper.workers.transcription_workers;
        let buffer_size = config.whisper.workers.channel_buffer_size;
        
        info!("Initializing transcription worker pool with {} workers", worker_count);
        
        let (task_sender, task_receiver) = mpsc::channel::<TranscriptionTask>(buffer_size);
        let mut worker_handles = Vec::new();
        
        // Create a shared receiver that can be cloned for multiple workers
        let task_receiver = Arc::new(tokio::sync::Mutex::new(task_receiver));
        
        // Create multiple workers for SPMC pattern
        for worker_id in 0..worker_count {
            let config = Arc::clone(&config);
            let receiver = Arc::clone(&task_receiver);
            
            let handle = tokio::spawn(async move {
                let worker = TranscriptionWorker::new(worker_id, config).await;
                worker.run(receiver).await;
            });
            
            worker_handles.push(handle);
        }
        
        info!("Transcription worker pool initialized successfully");
        
        Ok(Self {
            task_sender,
            worker_handles,
            config,
        })
    }
    
    /// Submit a transcription task to the worker pool
    pub async fn submit_task(&self, task: TranscriptionTask) -> Result<(), VoiceCliError> {
        self.task_sender
            .send(task)
            .await
            .map_err(|_| VoiceCliError::WorkerPoolError("Failed to submit task to worker pool".to_string()))
    }
    
    /// Shutdown the worker pool gracefully
    pub async fn shutdown(self) {
        info!("Shutting down transcription worker pool");
        
        // Close the sender to signal workers to stop
        drop(self.task_sender);
        
        // Wait for all workers to finish
        for (index, handle) in self.worker_handles.into_iter().enumerate() {
            if let Err(e) = handle.await {
                warn!("Worker {} failed to shutdown cleanly: {}", index, e);
            }
        }
        
        info!("Transcription worker pool shutdown complete");
    }
}

/// Individual transcription worker
pub struct TranscriptionWorker {
    /// Worker identifier
    worker_id: usize,
    /// Configuration
    config: Arc<Config>,
    /// Model service for accessing Whisper models
    model_service: Arc<ModelService>,
}

impl TranscriptionWorker {
    /// Create a new transcription worker
    pub async fn new(worker_id: usize, config: Arc<Config>) -> Self {
        let model_service = Arc::new(ModelService::new((*config).clone()));
        
        Self {
            worker_id,
            config,
            model_service,
        }
    }
    
    /// Run the worker, processing tasks from the shared receiver
    pub async fn run(&self, task_receiver: Arc<tokio::sync::Mutex<mpsc::Receiver<TranscriptionTask>>>) {
        info!("Transcription worker {} started", self.worker_id);
        
        loop {
            // Acquire lock and try to receive a task
            let task = {
                let mut receiver = task_receiver.lock().await;
                receiver.recv().await
            };
            
            match task {
                Some(task) => {
                    let start_time = std::time::Instant::now();
                    let TranscriptionTask {
                        task_id,
                        audio_data,
                        filename,
                        model,
                        response_format,
                        result_sender,
                    } = task;
                    
                    debug!(
                        "Worker {} processing task {}", 
                        self.worker_id, 
                        task_id
                    );
                    
                    let task_for_processing = TranscriptionTask {
                        task_id: task_id.clone(),
                        audio_data,
                        filename,
                        model,
                        response_format,
                        result_sender: tokio::sync::oneshot::channel().0, // dummy sender
                    };
                    
                    let result = self.process_transcription_task(task_for_processing).await;
                    let processing_time = start_time.elapsed().as_secs_f32();
                    
                    let (success, response, error) = match result {
                        Ok(resp) => (true, Some(resp), None),
                        Err(err) => (false, None, Some(err)),
                    };
                    
                    let transcription_result = TranscriptionResult {
                        task_id: task_id.clone(),
                        success,
                        response,
                        error,
                        processing_time,
                    };
                    
                    // Send result back through oneshot channel
                    if let Err(_) = result_sender.send(transcription_result) {
                        warn!("Failed to send result for task {}", task_id);
                    }
                }
                None => {
                    // Channel closed, exit worker loop
                    break;
                }
            }
        }
        
        info!("Transcription worker {} stopped", self.worker_id);
    }
    
    /// Process a single transcription task
    async fn process_transcription_task(
        &self,
        task: TranscriptionTask,
    ) -> Result<TranscriptionResponse, VoiceCliError> {
        let audio_data = task.audio_data;
        let filename = task.filename;
        let model = task.model;
        let response_format = task.response_format;
        
        // 1. Process audio format and convert if needed
        let processed_audio = self.process_audio_format(
            audio_data,
            &filename
        ).await?;
        
        // 2. Perform transcription using voice-toolkit
        let transcription_result = self.perform_transcription(
            &processed_audio,
            &model,
            &response_format
        ).await?;
        
        // 3. Build response (WorkerProcessedAudio handles cleanup automatically via Drop)
        Ok(self.build_transcription_response(transcription_result))
    }
    
    /// Process audio format and convert to Whisper-compatible format if needed
    async fn process_audio_format(
        &self,
        audio_data: Bytes,
        filename: &str,
    ) -> Result<WorkerProcessedAudio, VoiceCliError> {
        use crate::models::AudioFormat;
        
        // Detect format from file extension
        let format = AudioFormat::from_filename(filename);
        if !format.is_supported() {
            return Err(VoiceCliError::UnsupportedFormat(filename.to_string()));
        }
        
        // Create temporary file for the audio data
        let temp_file = self.create_temp_audio_file(&audio_data, &format).await?;
        
        // Convert to Whisper-compatible format if needed
        let compatible_file = if matches!(format, AudioFormat::Wav) {
            temp_file.clone()
        } else {
            self.convert_to_whisper_format(&temp_file).await?
        };
        
        Ok(WorkerProcessedAudio {
            file_path: compatible_file,
            original_format: format,
            cleanup_files: vec![temp_file],
        })
    }
    
    /// Create a temporary file with the audio data
    async fn create_temp_audio_file(
        &self,
        audio_data: &Bytes,
        format: &crate::models::AudioFormat,
    ) -> Result<PathBuf, VoiceCliError> {
        let temp_file = NamedTempFile::with_suffix(&format!(".{}", format.to_string()))
            .map_err(|e| VoiceCliError::TempFileError(e.to_string()))?;
        
        tokio::fs::write(temp_file.path(), audio_data)
            .await
            .map_err(|e| VoiceCliError::TempFileError(e.to_string()))?;
        
        let path = temp_file.into_temp_path().keep()
            .map_err(|e| VoiceCliError::TempFileError(e.to_string()))?;
        
        Ok(path)
    }
    
    /// Convert audio to Whisper-compatible format using voice-toolkit
    async fn convert_to_whisper_format(
        &self,
        input_path: &Path,
    ) -> Result<PathBuf, VoiceCliError> {
        // Generate output path
        let output_path = input_path.with_extension("wav");
        
        // Use voice-toolkit's audio conversion
        // Note: This is a blocking operation, so we run it in a separate thread
        let input_path = input_path.to_path_buf();
        let output_path_clone = output_path.clone();
        
        let compatible_wav = tokio::task::spawn_blocking(move || {
            voice_toolkit::audio::ensure_whisper_compatible(&input_path, Some(output_path_clone))
        })
        .await
        .map_err(|e| VoiceCliError::AudioConversionFailed(format!("Task join error: {}", e)))?
        .map_err(|e| VoiceCliError::AudioConversionFailed(e.to_string()))?;
        
        Ok(compatible_wav.path)
    }
    
    /// Perform transcription using voice-toolkit
    async fn perform_transcription(
        &self,
        processed_audio: &WorkerProcessedAudio,
        model: &Option<String>,
        _response_format: &Option<String>,
    ) -> Result<voice_toolkit::stt::TranscriptionResult, VoiceCliError> {
        // Get model path
        let model_name = model
            .as_ref()
            .unwrap_or(&self.config.whisper.default_model);
        
        let model_path = self.model_service
            .get_model_path(model_name)?;
            
        if !model_path.exists() {
            return Err(VoiceCliError::ModelNotFound(model_name.clone()));
        }
        
        // Perform transcription with automatic language detection and timeout
        let timeout_duration = std::time::Duration::from_secs(
            self.config.whisper.workers.worker_timeout as u64
        );
        
        let model_path = model_path.clone();
        let audio_path = processed_audio.file_path.clone();
        
        let result = tokio::time::timeout(
            timeout_duration,
            tokio::task::spawn_blocking(move || {
                // Use voice-toolkit's transcribe_file function for automatic language detection
                tokio::runtime::Handle::current().block_on(async {
                    voice_toolkit::stt::transcribe_file(&model_path, &audio_path).await
                })
            })
        )
        .await
        .map_err(|_| VoiceCliError::TranscriptionTimeout(timeout_duration.as_secs()))?
        .map_err(|e| VoiceCliError::TranscriptionFailed(format!("Task join error: {}", e)))?
        .map_err(|e| VoiceCliError::TranscriptionFailed(e.to_string()))?;
        
        info!(
            "Worker {} completed transcription, language: {}", 
            self.worker_id,
            result.language.as_ref().unwrap_or(&"unknown".to_string())
        );
        
        Ok(result)
    }
    
    /// Build the final transcription response
    fn build_transcription_response(
        &self,
        result: voice_toolkit::stt::TranscriptionResult,
    ) -> TranscriptionResponse {
        let segments = result.segments.into_iter().map(|seg| Segment {
            start: seg.start_time as f32 / 1000.0, // Convert from ms to seconds
            end: seg.end_time as f32 / 1000.0,
            text: seg.text,
            confidence: seg.confidence,
        }).collect();
        
        TranscriptionResponse {
            text: result.text,
            segments,
            language: result.language,
            duration: Some(result.audio_duration as f32 / 1000.0), // Convert from ms to seconds
            processing_time: 0.0, // Will be set by the handler
        }
    }
}