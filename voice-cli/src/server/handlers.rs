use crate::models::{
    Config, TranscriptionRequest, TranscriptionResponse, HealthResponse, 
    ModelsResponse, ModelInfo, AudioFormat
};
use crate::services::{TranscriptionService, ModelService};
use crate::VoiceCliError;
use axum::{
    extract::{Multipart, State},
    response::Json,
};
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tracing::{info, warn};
use utoipa;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub transcription_service: Arc<TranscriptionService>,
    pub model_service: Arc<ModelService>,
    pub start_time: SystemTime,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> crate::Result<Self> {
        let transcription_service = Arc::new(TranscriptionService::new(config.clone()).await?);
        let model_service = Arc::new(ModelService::new((*config).clone()));
        
        Ok(Self {
            config,
            transcription_service,
            model_service,
            start_time: SystemTime::now(),
        })
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
/// - language (text, optional): Language hint
/// - response_format (text, optional): Output format (json, text, verbose_json)
#[utoipa::path(
    post,
    path = "/transcribe",
    tag = "Transcription",
    summary = "Transcribe audio to text",
    description = "Upload an audio file and get the transcribed text. Supports multiple audio formats (MP3, WAV, FLAC, M4A, AAC, OGG) with automatic format conversion. Maximum file size is 200MB.",
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
    mut multipart: Multipart,
) -> Result<Json<TranscriptionResponse>, VoiceCliError> {
    let start_time = Instant::now();
    
    let mut audio_data: Option<Bytes> = None;
    let mut filename: Option<String> = None;
    let mut model: Option<String> = None;
    let mut language: Option<String> = None;
    let mut response_format: Option<String> = None;

    // Extract multipart fields
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        VoiceCliError::AudioProcessing(format!("Multipart parsing error: {}", e))
    })? {
        let field_name = field.name().unwrap_or("unknown");
        
        match field_name {
            "audio" => {
                // Get filename if available
                filename = field.file_name().map(|s| s.to_string());
                
                // Read audio data
                let data = field.bytes().await.map_err(|e| {
                    VoiceCliError::AudioProcessing(format!("Failed to read audio field: {}", e))
                })?;
                
                // Check file size (additional check beyond middleware)
                if data.len() > state.config.server.max_file_size {
                    return Err(VoiceCliError::FileTooLarge {
                        size: data.len(),
                        max: state.config.server.max_file_size,
                    });
                }
                
                audio_data = Some(data);
                info!("Received audio file: {} bytes, filename: {:?}", 
                     audio_data.as_ref().unwrap().len(), filename);
            }
            "model" => {
                model = Some(field.text().await.map_err(|e| {
                    VoiceCliError::AudioProcessing(format!("Invalid model field: {}", e))
                })?);
            }
            "language" => {
                language = Some(field.text().await.map_err(|e| {
                    VoiceCliError::AudioProcessing(format!("Invalid language field: {}", e))
                })?);
            }
            "response_format" => {
                response_format = Some(field.text().await.map_err(|e| {
                    VoiceCliError::AudioProcessing(format!("Invalid response_format field: {}", e))
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
        VoiceCliError::AudioProcessing("Missing required 'audio' field in multipart data".to_string())
    })?;

    // Validate audio format if filename is available
    if let Some(ref filename) = filename {
        let format = AudioFormat::from_filename(filename);
        if !format.is_supported() {

            // audio::ensure_whisper_compatible()
            return Err(VoiceCliError::UnsupportedFormat(
                format!("Unsupported audio format: {} (from filename: {})", format.to_string(), filename)
            ));
        }
    }

    // Validate model name if provided
    if let Some(ref model_name) = model {
        if !state.config.whisper.supported_models.contains(model_name) {
            return Err(VoiceCliError::InvalidModelName(
                format!("Unsupported model: {}", model_name)
            ));
        }
    }

    // Create transcription request
    let request = TranscriptionRequest {
        audio_data,
        filename,
        model,
        language,
        response_format,
    };

    // Process transcription
    info!("Starting transcription with model: {:?}", request.model);
    let mut response = state.transcription_service.transcribe(request).await?;
    
    // Add processing time
    response.processing_time = start_time.elapsed().as_secs_f32();
    
    info!("Transcription completed in {:.2}s, text length: {} chars", 
         response.processing_time, response.text.len());

    Ok(Json(response))
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
}