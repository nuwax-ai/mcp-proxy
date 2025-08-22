use serde::{Deserialize, Serialize};
use bytes::Bytes;
use utoipa::ToSchema;
use serde_json::json;

/// Request structure for transcription (internal use after extracting from multipart)
#[derive(Debug)]
pub struct TranscriptionRequest {
    pub audio_data: Bytes,
    pub filename: Option<String>,
    pub model: Option<String>,
    pub response_format: Option<String>,
}

/// Response structure for transcription API
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TranscriptionResponse {
    #[schema(example = "Hello, this is a test transcription.")]
    pub text: String,
    #[schema(example = json!([{"start": 0.0, "end": 2.5, "text": "Hello world", "confidence": 0.95}]))]
    pub segments: Vec<Segment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "en")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 2.5)]
    pub duration: Option<f32>,
    #[schema(example = 0.8)]
    pub processing_time: f32,
}

/// Individual segment in transcription
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Segment {
    /// Start time of the segment in seconds
    #[schema(example = 0.0)]
    pub start: f32,
    /// End time of the segment in seconds
    #[schema(example = 2.5)]
    pub end: f32,
    /// Text content of this segment
    #[schema(example = "Hello, this is a test transcription.")]
    pub text: String,
    /// Confidence score for this segment (0.0-1.0)
    #[schema(example = 0.95)]
    pub confidence: f32,
}

/// Health check response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Current service status
    #[schema(example = "healthy")]
    pub status: String,
    /// List of currently loaded models
    #[schema(example = json!(["base", "small"]))]
    pub models_loaded: Vec<String>,
    /// Service uptime in seconds
    #[schema(example = 3600)]
    pub uptime: u64,
    /// Service version
    #[schema(example = "0.1.0")]
    pub version: String,
}

/// Models list response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelsResponse {
    /// All supported model names
    #[schema(example = json!(["tiny", "base", "small", "medium", "large"]))]
    pub available_models: Vec<String>,
    /// Currently loaded models in memory
    #[schema(example = json!(["base"]))]
    pub loaded_models: Vec<String>,
    /// Detailed information about each model
    pub model_info: std::collections::HashMap<String, ModelInfo>,
}

/// Information about a specific model
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelInfo {
    /// Model file size on disk
    #[schema(example = "142 MB")]
    pub size: String,
    /// Memory usage when loaded
    #[schema(example = "388 MB")]
    pub memory_usage: String,
    /// Current model status
    #[schema(example = "loaded")]
    pub status: String,
}

/// Processed audio data
#[derive(Debug)]
pub struct ProcessedAudio {
    pub data: Bytes,
    pub converted: bool,
    pub original_format: Option<String>,
}

/// Audio format detection result
#[derive(Debug, Clone, Copy)]
pub enum AudioFormat {
    Wav,
    Mp3,
    Flac,
    M4a,
    Aac,
    Ogg,
    Unknown,
}

impl AudioFormat {
    pub fn from_filename(filename: &str) -> Self {
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            "wav" => AudioFormat::Wav,
            "mp3" => AudioFormat::Mp3,
            "flac" => AudioFormat::Flac,
            "m4a" => AudioFormat::M4a,
            "aac" => AudioFormat::Aac,
            "ogg" => AudioFormat::Ogg,
            _ => AudioFormat::Unknown,
        }
    }

    pub fn is_supported(&self) -> bool {
        !matches!(self, AudioFormat::Unknown)
    }

    pub fn needs_conversion(&self) -> bool {
        !matches!(self, AudioFormat::Wav)
    }

    pub fn to_string(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::M4a => "m4a",
            AudioFormat::Aac => "aac",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Unknown => "unknown",
        }
    }
}

/// Model download status
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelDownloadStatus {
    pub model_name: String,
    pub status: DownloadStatus,
    pub progress: Option<f32>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DownloadStatus {
    NotStarted,
    Downloading,
    Completed,
    Failed,
    Exists,
}

/// Daemon status information
#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime: Option<u64>,
    pub memory_usage: Option<String>,
    pub cpu_usage: Option<f32>,
}