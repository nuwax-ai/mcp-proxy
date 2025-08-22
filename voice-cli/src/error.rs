use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
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
}

impl IntoResponse for VoiceCliError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            VoiceCliError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::AudioProcessing(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::Transcription(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Model(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::FileIo(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Http(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Serialization(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Json(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::Daemon(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::UnsupportedFormat(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::FileTooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            VoiceCliError::ModelNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            VoiceCliError::InvalidModelName(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        };

        let body = Json(json!({
            "error": error_message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, VoiceCliError>;