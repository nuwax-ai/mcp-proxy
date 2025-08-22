use crate::models::{
    Config, TranscriptionResponse, HealthResponse, 
    ModelsResponse, ModelInfo, AudioFormat
};
use crate::services::{ModelService, TranscriptionWorkerPool};
use crate::VoiceCliError;
use axum::{
    extract::{Multipart, State},
    response::Json,
};
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn, error};
use utoipa;

// Explicitly import the types we need to avoid ambiguity
use crate::models::worker::TranscriptionRequest as WorkerTranscriptionRequest;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub transcription_worker_pool: Arc<TranscriptionWorkerPool>,
    pub model_service: Arc<ModelService>,
    pub start_time: SystemTime,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> crate::Result<Self> {
        let transcription_worker_pool = Arc::new(TranscriptionWorkerPool::new(config.clone()).await?);
        let model_service = Arc::new(ModelService::new((*config).clone()));
        
        Ok(Self {
            config,
            transcription_worker_pool,
            model_service,
            start_time: SystemTime::now(),
        })
    }
    
    /// Gracefully shutdown the app state
    pub async fn shutdown(self) {
        info!("Shutting down application state");
        
        // Convert Arc to owned value for shutdown
        if let Ok(worker_pool) = Arc::try_unwrap(self.transcription_worker_pool) {
            worker_pool.shutdown().await;
        } else {
            warn!("Could not shutdown worker pool - multiple references exist");
        }
        
        info!("Application state shutdown complete");
    }
}

/// Health check endpoint
/// GET /health
#[utoipa::path(
    get,
    path = "/health",
    tag = "Health",
    summary = "Get service health status",
    description = "Returns the current health status of the voice-cli service, including uptime, loaded models, and version information.",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
        (status = 500, description = "Service error", body = String)
    )
)]
pub async fn health_handler(State(state): State<AppState>) -> Result<Json<HealthResponse>, VoiceCliError> {
    let uptime = state.start_time
        .elapsed()
        .unwrap_or_default()
        .as_secs();
    
    let loaded_models = state.model_service.list_loaded_models().await?;
    
    let response = HealthResponse {
        status: "healthy".to_string(),
        models_loaded: loaded_models,
        uptime,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    
    Ok(Json(response))
}

/// List available and loaded models
/// GET /models
#[utoipa::path(
    get,
    path = "/models",
    tag = "Models",
    summary = "List available Whisper models",
    description = "Returns information about all available Whisper models, currently loaded models, and detailed model information including file sizes and memory usage.",
    responses(
        (status = 200, description = "Models information retrieved successfully", body = ModelsResponse),
        (status = 500, description = "Failed to retrieve models information", body = String)
    )
)]
pub async fn models_list_handler(State(state): State<AppState>) -> Result<Json<ModelsResponse>, VoiceCliError> {
    let available_models = state.config.whisper.supported_models.clone();
    let loaded_models = state.model_service.list_loaded_models().await?;
    let downloaded_models = state.model_service.list_downloaded_models().await?;
    
    let mut model_info = HashMap::new();
    
    for model_name in &downloaded_models {
        match state.model_service.get_model_info(model_name).await {
            Ok(info) => {
                model_info.insert(model_name.clone(), info);
            }
            Err(e) => {
                warn!("Failed to get info for model {}: {}", model_name, e);
                model_info.insert(model_name.clone(), ModelInfo {
                    size: "Unknown".to_string(),
                    memory_usage: "Unknown".to_string(),
                    status: "Error".to_string(),
                });
            }
        }
    }
    
    let response = ModelsResponse {
        available_models,
        loaded_models,
        model_info,
    };
    
    Ok(Json(response))
}

/// Main transcription endpoint with multipart file handling
/// POST /transcribe
/// Content-Type: multipart/form-data
/// Fields:
/// - audio (file, required): Audio file to transcribe
/// - model (text, optional): Whisper model to use
/// - response_format (text, optional): Output format (json, text, verbose_json)
#[utoipa::path(
    post,
    path = "/transcribe",
    tag = "Transcription",
    summary = "Transcribe audio to text",
    description = "Upload an audio file and get the transcribed text with automatic language detection. Supports multiple audio formats (MP3, WAV, FLAC, M4A, AAC, OGG) with automatic format conversion. Maximum file size is 200MB.",
    request_body(
        content = String,
        description = "Multipart form data with audio file and optional parameters",
        content_type = "multipart/form-data"
    ),
    responses(
        (status = 200, description = "Transcription completed successfully", body = TranscriptionResponse),
        (status = 400, description = "Invalid request - missing audio file, unsupported format, or invalid parameters", body = String),
        (status = 413, description = "File too large - exceeds 200MB limit", body = String),
        (status = 500, description = "Transcription failed due to server error", body = String)
    ),
)]
pub async fn transcribe_handler(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<TranscriptionResponse>, VoiceCliError> {
    let start_time = Instant::now();
    let task_id = format!("task_{}_{}", 
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
        std::process::id()
    );
    
    info!("Starting transcription request {}", task_id);
    
    // 1. Extract multipart form data
    let (audio_data, request) = extract_transcription_request(multipart).await?;
    
    // 2. Validate audio file
    validate_audio_file(&audio_data, &request.filename, state.config.server.max_file_size)?;
    
    // 3. Create result channel for receiving worker response
    let (result_sender, result_receiver) = tokio::sync::oneshot::channel();
    
    // 4. Create transcription task
    let task = crate::models::TranscriptionTask {
        task_id: task_id.clone(),
        audio_data,
        filename: request.filename,
        model: request.model,
        response_format: request.response_format,
        result_sender,
    };
    
    // 5. Submit task to worker pool
    state.transcription_worker_pool.submit_task(task).await?;
    
    // 6. Wait for result from worker
    let worker_result = result_receiver
        .await
        .map_err(|_| VoiceCliError::WorkerPoolError("Worker result channel closed".to_string()))?;
    
    // 7. Handle worker result
    match worker_result {
        crate::models::TranscriptionResult { success: true, response: Some(mut response), .. } => {
            response.processing_time = start_time.elapsed().as_secs_f32();
            
            info!(
                "Transcription {} completed successfully in {:.2}s, text length: {} chars",
                task_id,
                response.processing_time,
                response.text.len()
            );
            
            Ok(Json(response))
        }
        crate::models::TranscriptionResult { success: false, error: Some(error), .. } => {
            error!(
                "Transcription {} failed: {}",
                task_id,
                error
            );
            Err(error)
        }
        _ => {
            error!("Invalid worker result for task {}", task_id);
            Err(VoiceCliError::TranscriptionFailed("Invalid worker result".to_string()))
        }
    }
}

/// Helper function to extract transcription request from multipart data
async fn extract_transcription_request(
    mut multipart: Multipart,
) -> Result<(Bytes, WorkerTranscriptionRequest), VoiceCliError> {
    let mut audio_data: Option<Bytes> = None;
    let mut filename: Option<String> = None;
    let mut model: Option<String> = None;
    let mut response_format: Option<String> = None;
    
    // Extract multipart fields
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        VoiceCliError::MultipartError(format!("Multipart parsing error: {}", e))
    })? {
        let field_name = field.name().unwrap_or("unknown");
        
        match field_name {
            "audio" => {
                // Get filename if available
                filename = field.file_name().map(|s| s.to_string());
                
                // Read audio data
                let data = field.bytes().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("Failed to read audio field: {}", e))
                })?;
                
                audio_data = Some(data);
                info!("Received audio file: {} bytes, filename: {:?}", 
                     audio_data.as_ref().unwrap().len(), filename);
            }
            "model" => {
                model = Some(field.text().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("Invalid model field: {}", e))
                })?);
            }
            "response_format" => {
                response_format = Some(field.text().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("Invalid response_format field: {}", e))
                })?);
            }
            _ => {
                // Ignore unknown fields
                warn!("Ignoring unknown multipart field: {}", field_name);
            }
        }
    }
    
    // Validate required fields
    let audio_data = audio_data.ok_or_else(|| {
        VoiceCliError::MissingField("audio".to_string())
    })?;
    
    // Generate filename based on audio data if not provided
    let filename = if let Some(provided_filename) = filename {
        provided_filename
    } else {
        // Detect format from magic bytes and generate random filename
        let detected_format = detect_audio_format_from_magic_bytes(&audio_data)?;
        let uid = uuid::Uuid::new_v4();
        format!("{}.{}", uid, detected_format.to_string())
    };
    
    let request = WorkerTranscriptionRequest {
        filename,
        model,
        response_format,
    };
    
    Ok((audio_data, request))
}

/// Helper function to validate audio file
fn validate_audio_file(
    audio_data: &Bytes,
    filename: &str,
    max_file_size: usize,
) -> Result<(), VoiceCliError> {
    // Check file size
    if audio_data.len() > max_file_size {
        return Err(VoiceCliError::FileTooLarge {
            size: audio_data.len(),
            max: max_file_size,
        });
    }
    
    // Validate audio format
    let format = AudioFormat::from_filename(filename);
    if !format.is_supported() {
        return Err(VoiceCliError::UnsupportedFormat(
            format!("Unsupported audio format: {} (from filename: {})", format.to_string(), filename)
        ));
    }
    
    Ok(())
}

/// Helper function to detect audio format from magic bytes
fn detect_audio_format_from_magic_bytes(audio_data: &Bytes) -> Result<AudioFormat, VoiceCliError> {
    if audio_data.len() < 4 {
        return Err(VoiceCliError::UnsupportedFormat(
            "Audio data too short to detect format".to_string()
        ));
    }
    
    let header = &audio_data[0..4];
    
    // WAV file signature
    if header == b"RIFF" && audio_data.len() >= 12 {
        let wave_header = &audio_data[8..12];
        if wave_header == b"WAVE" {
            return Ok(AudioFormat::Wav);
        }
    }
    
    // MP3 file signatures - comprehensive detection for audio/mpeg
    if detect_mp3_format(audio_data) {
        return Ok(AudioFormat::Mp3);
    }
    
    // FLAC file signature
    if header == b"fLaC" {
        return Ok(AudioFormat::Flac);
    }
    
    // OGG file signature
    if header == b"OggS" {
        return Ok(AudioFormat::Ogg);
    }
    
    // Check for M4A/AAC (more complex detection)
    if audio_data.len() >= 8 {
        let ftyp_check = &audio_data[4..8];
        if ftyp_check == b"ftyp" {
            return Ok(AudioFormat::M4a);
        }
    }
    
    // Try to detect AAC by checking for ADTS header
    if audio_data.len() >= 2 {
        let adts_header = &audio_data[0..2];
        if (adts_header[0] & 0xFF) == 0xFF && (adts_header[1] & 0xF0) == 0xF0 {
            return Ok(AudioFormat::Aac);
        }
    }
    
    // Check for WebM format (EBML header)
    if audio_data.len() >= 4 {
        // WebM files start with EBML header: 0x1A 0x45 0xDF 0xA3
        if header[0] == 0x1A && header[1] == 0x45 && header[2] == 0xDF && header[3] == 0xA3 {
            return Ok(AudioFormat::Webm);
        }
    }
    
    Err(VoiceCliError::UnsupportedFormat(
        "Unable to detect audio format from magic bytes".to_string()
    ))
}

/// Helper function to detect MP3 format with comprehensive magic byte checking
/// This handles various MP3 file structures including ID3 tags and MPEG frame headers
fn detect_mp3_format(audio_data: &Bytes) -> bool {
    if audio_data.len() < 3 {
        return false;
    }
    
    // Check for ID3v2 tag at the beginning (most common)
    if audio_data.len() >= 3 && &audio_data[0..3] == b"ID3" {
        return true;
    }
    
    // Check for ID3v1 tag at the end (if file is long enough)
    if audio_data.len() >= 128 {
        let id3v1_start = audio_data.len() - 128;
        if &audio_data[id3v1_start..id3v1_start + 3] == b"TAG" {
            return true;
        }
    }
    
    // Check for MPEG frame headers (various sync patterns)
    // MPEG-1 Layer 3: 0xFF 0xFB (0x90-0x93)
    // MPEG-2 Layer 3: 0xFF 0xF3 (0x90-0x93)
    // MPEG-2.5 Layer 3: 0xFF 0xF2 (0x90-0x93)
    if audio_data.len() >= 2 {
        let first_byte = audio_data[0];
        let second_byte = audio_data[1];
        
        // Check for MPEG sync byte (0xFF)
        if first_byte == 0xFF {
            // Check for valid MPEG frame header patterns
            match second_byte {
                0xFB | 0xF3 | 0xF2 => {
                    // These are valid MPEG frame header patterns
                    if audio_data.len() >= 4 {
                        let third_byte = audio_data[2];
                        // Check if the third byte is in valid range (0x90-0x93)
                        if third_byte >= 0x90 && third_byte <= 0x93 {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    
    // Check for MPEG-1 Layer 1/2 patterns
    if audio_data.len() >= 2 {
        let first_byte = audio_data[0];
        let second_byte = audio_data[1];
        
        if first_byte == 0xFF {
            match second_byte {
                0xF1 | 0xF5 | 0xF9 | 0xFD => {
                    // MPEG-1 Layer 1/2 patterns
                    if audio_data.len() >= 4 {
                        let third_byte = audio_data[2];
                        if third_byte >= 0x90 && third_byte <= 0x93 {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    
    // Check for MPEG-2 Layer 1/2 patterns
    if audio_data.len() >= 2 {
        let first_byte = audio_data[0];
        let second_byte = audio_data[1];
        
        if first_byte == 0xFF {
            match second_byte {
                0xF0 | 0xF4 | 0xF8 | 0xFC => {
                    // MPEG-2 Layer 1/2 patterns
                    if audio_data.len() >= 4 {
                        let third_byte = audio_data[2];
                        if third_byte >= 0x90 && third_byte <= 0x93 {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Config;

    #[tokio::test]
    async fn test_app_state_creation() {
        let config = Arc::new(Config::default());
        let state = AppState::new(config).await;
        assert!(state.is_ok());
    }

    #[test]
    fn test_audio_format_detection() {
        assert!(AudioFormat::from_filename("test.mp3").is_supported());
        assert!(AudioFormat::from_filename("test.wav").is_supported());
        assert!(!AudioFormat::from_filename("test.xyz").is_supported());
    }
    
    #[test]
    fn test_mp3_magic_bytes_detection() {
        // Test ID3v2 tag
        let id3v2_data = Bytes::from(b"ID3\x03\x00\x00\x00\x00\x00\x00".to_vec());
        assert!(detect_mp3_format(&id3v2_data));
        
        // Test MPEG frame header (MPEG-1 Layer 3)
        let mpeg_frame_data = Bytes::from(vec![0xFF, 0xFB, 0x90, 0x00]);
        assert!(detect_mp3_format(&mpeg_frame_data));
        
        // Test MPEG frame header (MPEG-2 Layer 3)
        let mpeg2_frame_data = Bytes::from(vec![0xFF, 0xF3, 0x92, 0x00]);
        assert!(detect_mp3_format(&mpeg2_frame_data));
        
        // Test invalid data
        let invalid_data = Bytes::from(b"RIFF".to_vec());
        assert!(!detect_mp3_format(&invalid_data));
        
        // Test short data
        let short_data = Bytes::from(vec![0xFF]);
        assert!(!detect_mp3_format(&short_data));
    }
    
    #[test]
    fn test_webm_format_detection() {
        // Test WebM EBML header
        let webm_data = Bytes::from(vec![0x1A, 0x45, 0xDF, 0xA3]);
        let format = detect_audio_format_from_magic_bytes(&webm_data);
        assert!(matches!(format, Ok(AudioFormat::Webm)));
        
        // Test WAV format
        let wav_data = Bytes::from(b"RIFF\x00\x00\x00\x00WAVE".to_vec());
        let format = detect_audio_format_from_magic_bytes(&wav_data);
        assert!(matches!(format, Ok(AudioFormat::Wav)));
    }
}