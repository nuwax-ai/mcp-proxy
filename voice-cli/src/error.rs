use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use config::ConfigError;
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VoiceCliError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Audio processing error: {0}")]
    AudioProcessing(String),

    #[error("Transcription error: {0}")]
    Transcription(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("File I/O error: {0}")]
    FileIo(#[from] std::io::Error),

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Config-rs error: {0}")]
    ConfigRs(#[from] ConfigError),

    #[error("Daemon error: {0}")]
    Daemon(String),

    #[error("Audio format not supported: {0}")]
    UnsupportedFormat(String),

    #[error("File too large: {size} bytes (max: {max} bytes)")]
    FileTooLarge { size: usize, max: usize },

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Invalid model name: {0}")]
    InvalidModelName(String),

    // Worker pool related errors
    #[error("Worker pool error: {0}")]
    WorkerPoolError(String),

    #[error("Transcription timeout after {0} seconds")]
    TranscriptionTimeout(u64),

    #[error("Transcription failed: {0}")]
    TranscriptionFailed(String),

    // Audio processing specific errors
    #[error("Audio conversion failed: {0}")]
    AudioConversionFailed(String),

    #[error("Audio probe error: {0}")]
    AudioProbeError(String),

    #[error("Temporary file error: {0}")]
    TempFileError(String),

    // Multipart form handling errors
    #[error("Multipart form error: {0}")]
    MultipartError(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    // Storage related errors
    #[error("Storage error: {0}")]
    Storage(String),

    // Task management errors
    #[error("Task management is disabled")]
    TaskManagementDisabled,

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Initialization error: {0}")]
    Initialization(String),
}

impl IntoResponse for VoiceCliError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            VoiceCliError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::AudioProcessing(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::Transcription(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            VoiceCliError::Model(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::FileIo(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Http(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Serialization(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            VoiceCliError::Json(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Daemon(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::UnsupportedFormat(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::FileTooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            VoiceCliError::ModelNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            VoiceCliError::InvalidModelName(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::WorkerPoolError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            VoiceCliError::TranscriptionTimeout(_) => {
                (StatusCode::REQUEST_TIMEOUT, self.to_string())
            }
            VoiceCliError::TranscriptionFailed(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            VoiceCliError::AudioConversionFailed(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::AudioProbeError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::TempFileError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            VoiceCliError::MultipartError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::MissingField(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::Storage(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::ConfigRs(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::TaskManagementDisabled => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            VoiceCliError::Initialization(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = Json(json!({
            "error": error_message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}


// Conversion from ServiceError to VoiceCliError (for background services)
impl From<crate::daemon::background_service::ServiceError> for VoiceCliError {
    fn from(error: crate::daemon::background_service::ServiceError) -> Self {
        match error {
            crate::daemon::background_service::ServiceError::AlreadyRunning(msg) => {
                VoiceCliError::Daemon(format!("Service already running: {}", msg))
            }
            crate::daemon::background_service::ServiceError::ConfigurationError(msg) => {
                VoiceCliError::Config(msg)
            }
            crate::daemon::background_service::ServiceError::InitializationFailed(msg) => {
                VoiceCliError::Daemon(format!("Service initialization failed: {}", msg))
            }
            crate::daemon::background_service::ServiceError::ShutdownTimeout => {
                VoiceCliError::Daemon("Service shutdown timeout".to_string())
            }
            crate::daemon::background_service::ServiceError::ServiceError(msg) => {
                VoiceCliError::Daemon(msg)
            }
            crate::daemon::background_service::ServiceError::DaemonError(msg) => {
                VoiceCliError::Daemon(format!("Daemonization error: {}", msg))
            }
        }
    }
}


pub type Result<T> = std::result::Result<T, VoiceCliError>;

