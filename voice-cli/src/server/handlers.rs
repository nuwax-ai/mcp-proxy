use crate::models::{
    Config, HealthResponse, HttpResult, ModelInfo, ModelsResponse, TranscriptionResponse,
};
use crate::services::{ModelService, TranscriptionWorkerPool};
use crate::VoiceCliError;
use axum::{
    extract::{Multipart, State},
    response::Json,
};
use base64::{Engine as _, engine::general_purpose};
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
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
        let transcription_worker_pool =
            Arc::new(TranscriptionWorkerPool::new(config.clone()).await?);
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
pub async fn health_handler(
    State(state): State<AppState>,
) -> Result<Json<HealthResponse>, VoiceCliError> {
    let uptime = state.start_time.elapsed().unwrap_or_default().as_secs();

    let loaded_models = state.model_service.list_loaded_models().await?;

    let response = HealthResponse {
        status: "healthy".to_string(),
        models_loaded: loaded_models,
        uptime,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    Ok(Json(response))
}

/// Simple test endpoint for load balancer testing
/// GET /test
pub async fn test_handler(
    State(state): State<AppState>,
) -> Result<String, VoiceCliError> {
    // Try to determine the port from the config
    let port = state.config.server.port;
    Ok(format!("backend-{}", port))
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
        (status = 200, description = "Models information retrieved successfully", body = HttpResult<ModelsResponse>),
        (status = 500, description = "Failed to retrieve models information", body = HttpResult<String>)
    )
)]
pub async fn models_list_handler(
    State(state): State<AppState>,
) -> HttpResult<ModelsResponse> {
    let available_models = state.config.whisper.supported_models.clone();
    
    let loaded_models = match state.model_service.list_loaded_models().await {
        Ok(models) => models,
        Err(e) => return HttpResult::from(e),
    };
    
    let downloaded_models = match state.model_service.list_downloaded_models().await {
        Ok(models) => models,
        Err(e) => return HttpResult::from(e),
    };

    let mut model_info = HashMap::new();

    for model_name in &downloaded_models {
        match state.model_service.get_model_info(model_name).await {
            Ok(info) => {
                model_info.insert(model_name.clone(), info);
            }
            Err(e) => {
                warn!("Failed to get info for model {}: {}", model_name, e);
                model_info.insert(
                    model_name.clone(),
                    ModelInfo {
                        size: "Unknown".to_string(),
                        memory_usage: "Unknown".to_string(),
                        status: "Error".to_string(),
                    },
                );
            }
        }
    }

    let response = ModelsResponse {
        available_models,
        loaded_models,
        model_info,
    };

    HttpResult::success(response)
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
        (status = 200, description = "Transcription completed successfully", body = HttpResult<TranscriptionResponse>),
        (status = 400, description = "Invalid request - missing audio file, unsupported format, or invalid parameters", body = HttpResult<String>),
        (status = 413, description = "File too large - exceeds 200MB limit", body = HttpResult<String>),
        (status = 500, description = "Transcription failed due to server error", body = HttpResult<String>)
    ),
)]
pub async fn transcribe_handler(
    State(state): State<AppState>,
    multipart: Multipart,
) -> HttpResult<TranscriptionResponse> {
    let start_time = Instant::now();
    let task_id = format!(
        "task_{}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        std::process::id()
    );

    info!("Starting transcription request {}", task_id);

    // 1. Extract multipart form data
    let (audio_data, request) = match extract_transcription_request(multipart).await {
        Ok(result) => result,
        Err(e) => return HttpResult::from(e),
    };

    // 2. Validate audio file
    if let Err(e) = validate_audio_file(
        &audio_data,
        &request.filename,
        state.config.server.max_file_size,
    ) {
        return HttpResult::from(e);
    }

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
    if let Err(e) = state.transcription_worker_pool.submit_task(task).await {
        return HttpResult::from(e);
    }

    // 6. Wait for result from worker
    let worker_result = match result_receiver.await {
        Ok(result) => result,
        Err(_) => {
            return HttpResult::from(VoiceCliError::WorkerPoolError(
                "Worker result channel closed".to_string(),
            ));
        }
    };

    // 7. Handle worker result
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

/// Helper function to extract transcription request from multipart data
pub async fn extract_transcription_request(
    mut multipart: Multipart,
) -> Result<(Bytes, WorkerTranscriptionRequest), VoiceCliError> {
    let mut audio_data: Option<Bytes> = None;
    let mut filename: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut model: Option<String> = None;
    let mut response_format: Option<String> = None;

    // Extract multipart fields
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| VoiceCliError::MultipartError(format!("Multipart parsing error: {}", e)))?
    {
        let field_name = field.name().unwrap_or("unknown");

        match field_name {
            "audio" => {
                // Get filename and content-type from formdata
                filename = field.file_name().map(|s| s.to_string());
                content_type = field.content_type().map(|ct| ct.to_string());
                
                info!("Form data - filename: {:?}, content-type: {:?}", filename, content_type);

                // Read audio data
                let data = field.bytes().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("Failed to read audio field: {}", e))
                })?;

                // Check if the data is Base64 encoded
                let original_data_len = data.len();
                let decoded_data = if is_base64_encoded(&data) {
                    match general_purpose::STANDARD.decode(&data) {
                        Ok(decoded) => {
                            info!("Decoded Base64 audio data: {} bytes -> {} bytes", data.len(), decoded.len());
                            Bytes::from(decoded)
                        }
                        Err(e) => {
                            warn!("Failed to decode Base64 data: {}, using original data", e);
                            data
                        }
                    }
                } else {
                    data
                };

                audio_data = Some(decoded_data);
                info!(
                    "Received audio file: {} bytes (original: {} bytes), form_filename: {:?}, content_type: {:?}",
                    audio_data.as_ref().unwrap().len(),
                    original_data_len,
                    filename,
                    content_type
                );
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
    let audio_data = audio_data.ok_or_else(|| VoiceCliError::MissingField("audio".to_string()))?;

    // Generate filename based on frontend information
    let uid = uuid::Uuid::new_v4();
    
    // Generate filename using UUID + detected format for consistency
    let filename = {
        // Use Symphonia-based format detection to determine the correct extension
        let extension = match crate::services::AudioFormatDetector::detect_format(
            &audio_data, 
            filename.as_deref() // Use original filename as hint for detection
        ) {
            Ok(format_result) => {
                info!(
                    "Detected audio format: {:?} (method: {:?}, confidence: {:.2})", 
                    format_result.format, 
                    format_result.detection_method,
                    format_result.confidence
                );
                format_result.format.to_string()
            }
            Err(e) => {
                warn!(
                    "Format detection failed: {}, falling back to content-type or default", 
                    e
                );
                // Fallback to content-type based detection if Symphonia fails
                if let Some(content_type) = &content_type {
                    match content_type.as_str() {
                        "audio/mpeg" => "mp3",
                        "audio/wav" => "wav",
                        "audio/flac" => "flac",
                        "audio/ogg" => "ogg",
                        "audio/mp4" => "m4a",
                        "audio/aac" => "aac",
                        "audio/webm" => "webm",
                        _ => "webm" // Default to webm for unknown types
                    }
                } else {
                    "webm" // Default extension
                }
            }
        };
        
        // Always use UUID + detected extension for consistent naming
        format!("{}.{}", uid, extension)
    };
    
    info!("Generated filename with UUID + detected format: {} (content-type: {:?})", filename, content_type);

    let request = WorkerTranscriptionRequest {
        filename,
        model,
        response_format,
    };

    Ok((audio_data, request))
}

/// Helper function to validate audio file with enhanced format detection
pub fn validate_audio_file(
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
    info!("Audio file name: {}", filename);

    // Use enhanced format detection with Symphonia
    let format_result = crate::services::AudioFormatDetector::detect_format(
        audio_data, 
        Some(filename)
    )?;
    
    info!(
        "Detected audio format: {:?} (method: {:?}, confidence: {:.2})", 
        format_result.format, 
        format_result.detection_method,
        format_result.confidence
    );
    
    // Validate format support
    crate::services::AudioFormatDetector::validate_format_support(&format_result)?;
    
    // Log metadata if available
    if let Some(metadata) = &format_result.metadata {
        info!(
            "Audio metadata - Duration: {:?}, Sample rate: {:?}, Channels: {:?}, Codec: {}",
            metadata.duration,
            metadata.sample_rate,
            metadata.channels,
            metadata.codec_info
        );
    }

    Ok(())
}




/// Helper function to detect if data is Base64 encoded
fn is_base64_encoded(data: &Bytes) -> bool {
    // Base64 encoded data should only contain valid Base64 characters
    // and should have a length that's a multiple of 4 (with padding)
    if data.len() == 0 {
        return false;
    }
    
    // Check if all characters are valid Base64 characters
    let valid_chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
    let is_valid = data.iter().all(|&byte| valid_chars.contains(&byte));
    
    if !is_valid {
        return false;
    }
    
    // Check if length is reasonable for Base64 (should be multiple of 4)
    // But allow some flexibility for padding
    data.len() % 4 <= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Config;
    use crate::models::request::AudioFormat;

    #[tokio::test]
    async fn test_app_state_creation() {
        let config = Arc::new(Config::default());
        let state = AppState::new(config).await;
        assert!(state.is_ok());
    }

    #[test]
    fn test_audio_format_detection() {
        // Test core audio formats
        assert!(AudioFormat::from_filename("test.mp3").is_supported());
        assert!(AudioFormat::from_filename("test.wav").is_supported());
        assert!(AudioFormat::from_filename("test.flac").is_supported());
        assert!(AudioFormat::from_filename("test.m4a").is_supported());
        assert!(AudioFormat::from_filename("test.aac").is_supported());
        assert!(AudioFormat::from_filename("test.ogg").is_supported());
        assert!(AudioFormat::from_filename("test.webm").is_supported());
        assert!(AudioFormat::from_filename("test.opus").is_supported());
        
        // Test extended audio formats
        assert!(AudioFormat::from_filename("test.amr").is_supported());
        assert!(AudioFormat::from_filename("test.wma").is_supported());
        assert!(AudioFormat::from_filename("test.ra").is_supported());
        assert!(AudioFormat::from_filename("test.ram").is_supported());
        assert!(AudioFormat::from_filename("test.au").is_supported());
        assert!(AudioFormat::from_filename("test.aiff").is_supported());
        assert!(AudioFormat::from_filename("test.caf").is_supported());
        
        // Test video formats with audio
        assert!(AudioFormat::from_filename("test.3gp").is_supported());
        assert!(AudioFormat::from_filename("test.mp4").is_supported());
        assert!(AudioFormat::from_filename("test.mov").is_supported());
        assert!(AudioFormat::from_filename("test.avi").is_supported());
        assert!(AudioFormat::from_filename("test.mkv").is_supported());
        
        // Test unsupported format
        assert!(!AudioFormat::from_filename("test.xyz").is_supported());
    }
    
    #[test]
    fn test_audio_format_mime_types() {
        assert_eq!(AudioFormat::Mp3.get_mime_type(), "audio/mpeg");
        assert_eq!(AudioFormat::Wav.get_mime_type(), "audio/wav");
        assert_eq!(AudioFormat::Flac.get_mime_type(), "audio/flac");
        assert_eq!(AudioFormat::Amr.get_mime_type(), "audio/amr");
        assert_eq!(AudioFormat::Wma.get_mime_type(), "audio/x-ms-wma");
        assert_eq!(AudioFormat::Mp4.get_mime_type(), "video/mp4");
        assert_eq!(AudioFormat::Unknown.get_mime_type(), "application/octet-stream");
    }
    
    #[test]
    fn test_audio_format_ffmpeg_formats() {
        assert_eq!(AudioFormat::Mp3.get_ffmpeg_input_format(), Some("mp3"));
        assert_eq!(AudioFormat::Wav.get_ffmpeg_input_format(), Some("wav"));
        assert_eq!(AudioFormat::Wma.get_ffmpeg_input_format(), Some("asf"));
        assert_eq!(AudioFormat::Ra.get_ffmpeg_input_format(), Some("rm"));
        assert_eq!(AudioFormat::Mkv.get_ffmpeg_input_format(), Some("matroska"));
        assert_eq!(AudioFormat::Unknown.get_ffmpeg_input_format(), None);
    }
    
    #[test]
    fn test_audio_format_conversion_requirements() {
        assert!(!AudioFormat::Wav.requires_ffmpeg_conversion());
        assert!(AudioFormat::Mp3.requires_ffmpeg_conversion());
        assert!(AudioFormat::Amr.requires_ffmpeg_conversion());
        assert!(AudioFormat::Mp4.requires_ffmpeg_conversion());
    }
    #[test]
    fn test_base64_detection() {
        // Test Base64 encoded data
        let base64_data = Bytes::from(b"SGVsbG8gV29ybGQ=".to_vec()); // "Hello World" in Base64
        assert!(is_base64_encoded(&base64_data));
        
        // Test non-Base64 data
        let binary_data = Bytes::from(vec![0x1A, 0x45, 0xDF, 0xA3]);
        assert!(!is_base64_encoded(&binary_data));
        
        // Test empty data
        let empty_data = Bytes::from(vec![]);
        assert!(!is_base64_encoded(&empty_data));
    }
    
    #[test]
    fn test_symphonia_integration() {
        // Test that AudioFormatDetector methods are accessible and working
        use crate::services::AudioFormatDetector;
        use crate::models::request::{AudioFormat, DetectionMethod};
        
        // Test with dummy audio data and filename
        let test_data = Bytes::from(vec![0u8; 1024]); // Dummy data
        
        // Test filename-based fallback when Symphonia probe fails
        // Note: New behavior always generates UUID-based filename regardless of input filename
        let result = AudioFormatDetector::detect_format(&test_data, Some("test.mp3"));
        match result {
            Ok(format_result) => {
                // Should fallback to filename-based detection for dummy data
                assert_eq!(format_result.detection_method, DetectionMethod::FileExtension);
                assert_eq!(format_result.format, AudioFormat::Mp3);
                assert!(format_result.confidence >= 0.0);
            }
            Err(_) => {
                // This is also acceptable for dummy data
            }
        }
        
        // Test format validation
        let valid_result = crate::models::request::AudioFormatResult {
            format: AudioFormat::Mp3,
            confidence: 0.9,
            metadata: None,
            detection_method: DetectionMethod::SymphoniaProbe,
        };
        assert!(AudioFormatDetector::validate_format_support(&valid_result).is_ok());
        
        // Test format string conversion for UUID-based filename generation
        assert_eq!(AudioFormat::Mp3.to_string(), "mp3");
        assert_eq!(AudioFormat::Wav.to_string(), "wav");
        assert_eq!(AudioFormat::Flac.to_string(), "flac");
        
        // Test that UUID + extension format contains a dot and proper extension
        let example_filename = format!("{}.{}", uuid::Uuid::new_v4(), "mp3");
        assert!(example_filename.contains('.'));
        assert!(example_filename.ends_with(".mp3"));
        assert_eq!(example_filename.len(), 40); // UUID is 36 chars + ".mp3" = 40 chars exactly
    }
}
