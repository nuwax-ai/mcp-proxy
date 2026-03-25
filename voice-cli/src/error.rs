use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use config::ConfigError;
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VoiceCliError {
    #[error("{0}")]
    Config(String),

    #[error("{0}")]
    AudioProcessing(String),

    #[error("{0}")]
    Transcription(String),

    #[error("{0}")]
    Model(String),

    #[error("{0}")]
    FileIo(String),

    #[error("{0}")]
    Http(String),

    #[error("{0}")]
    Serialization(String),

    #[error("{0}")]
    Json(String),

    #[error("{0}")]
    ConfigRs(String),

    #[error("{0}")]
    Daemon(String),

    #[error("{0}")]
    UnsupportedFormat(String),

    #[error("{0}")]
    FileTooLarge(String),

    #[error("{0}")]
    ModelNotFound(String),

    #[error("{0}")]
    InvalidModelName(String),

    // Worker pool related errors
    #[error("{0}")]
    WorkerPoolError(String),

    #[error("{0}")]
    TranscriptionTimeout(String),

    #[error("{0}")]
    TranscriptionFailed(String),

    // Audio processing specific errors
    #[error("{0}")]
    AudioConversionFailed(String),

    #[error("{0}")]
    AudioProbeError(String),

    #[error("{0}")]
    TempFileError(String),

    // Multipart form handling errors
    #[error("{0}")]
    MultipartError(String),

    #[error("{0}")]
    MissingField(String),

    // Network related errors
    #[error("{0}")]
    Network(String),

    // Storage related errors
    #[error("{0}")]
    Storage(String),

    // Task management errors
    #[error("{0}")]
    TaskManagementDisabled(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    Initialization(String),

    // TTS related errors
    #[error("{0}")]
    TtsError(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("{0}")]
    Io(String),
}

impl VoiceCliError {
    // ===========================================
    // 工厂方法 - 创建带国际化消息的错误
    // ===========================================

    pub fn config_error(detail: impl Into<String>) -> Self {
        Self::Config(t!("errors.voice.config", detail = detail.into()).to_string())
    }

    pub fn audio_processing_error(detail: impl Into<String>) -> Self {
        Self::AudioProcessing(t!("errors.voice.audio_processing", detail = detail.into()).to_string())
    }

    pub fn transcription_error(detail: impl Into<String>) -> Self {
        Self::Transcription(t!("errors.voice.transcription", detail = detail.into()).to_string())
    }

    pub fn model_error(detail: impl Into<String>) -> Self {
        Self::Model(t!("errors.voice.model", detail = detail.into()).to_string())
    }

    pub fn file_io_error(detail: impl Into<String>) -> Self {
        Self::FileIo(t!("errors.voice.file_io", detail = detail.into()).to_string())
    }

    pub fn http_error(detail: impl Into<String>) -> Self {
        Self::Http(t!("errors.voice.http", detail = detail.into()).to_string())
    }

    pub fn serialization_error(detail: impl Into<String>) -> Self {
        Self::Serialization(t!("errors.voice.serialization", detail = detail.into()).to_string())
    }

    pub fn json_error(detail: impl Into<String>) -> Self {
        Self::Json(t!("errors.voice.json", detail = detail.into()).to_string())
    }

    pub fn config_rs_error(detail: impl Into<String>) -> Self {
        Self::ConfigRs(t!("errors.voice.config_rs", detail = detail.into()).to_string())
    }

    pub fn daemon_error(detail: impl Into<String>) -> Self {
        Self::Daemon(t!("errors.voice.daemon", detail = detail.into()).to_string())
    }

    pub fn unsupported_format(detail: impl Into<String>) -> Self {
        Self::UnsupportedFormat(t!("errors.voice.unsupported_format", detail = detail.into()).to_string())
    }

    pub fn file_too_large(size: usize, max: usize) -> Self {
        Self::FileTooLarge(t!("errors.voice.file_too_large", size = size, max = max).to_string())
    }

    pub fn model_not_found(model: impl Into<String>) -> Self {
        Self::ModelNotFound(t!("errors.voice.model_not_found", model = model.into()).to_string())
    }

    pub fn invalid_model_name(model: impl Into<String>) -> Self {
        Self::InvalidModelName(t!("errors.voice.invalid_model_name", model = model.into()).to_string())
    }

    pub fn worker_pool_error(detail: impl Into<String>) -> Self {
        Self::WorkerPoolError(t!("errors.voice.worker_pool", detail = detail.into()).to_string())
    }

    pub fn transcription_timeout(seconds: u64) -> Self {
        Self::TranscriptionTimeout(t!("errors.voice.transcription_timeout", seconds = seconds).to_string())
    }

    pub fn transcription_failed(detail: impl Into<String>) -> Self {
        Self::TranscriptionFailed(t!("errors.voice.transcription_failed", detail = detail.into()).to_string())
    }

    pub fn audio_conversion_failed(detail: impl Into<String>) -> Self {
        Self::AudioConversionFailed(t!("errors.voice.audio_conversion_failed", detail = detail.into()).to_string())
    }

    pub fn audio_probe_error(detail: impl Into<String>) -> Self {
        Self::AudioProbeError(t!("errors.voice.audio_probe_error", detail = detail.into()).to_string())
    }

    pub fn temp_file_error(detail: impl Into<String>) -> Self {
        Self::TempFileError(t!("errors.voice.temp_file_error", detail = detail.into()).to_string())
    }

    pub fn multipart_error(detail: impl Into<String>) -> Self {
        Self::MultipartError(t!("errors.voice.multipart_error", detail = detail.into()).to_string())
    }

    pub fn missing_field(field: impl Into<String>) -> Self {
        Self::MissingField(t!("errors.voice.missing_field", field = field.into()).to_string())
    }

    pub fn network_error(detail: impl Into<String>) -> Self {
        Self::Network(t!("errors.voice.network", detail = detail.into()).to_string())
    }

    pub fn storage_error(detail: impl Into<String>) -> Self {
        Self::Storage(t!("errors.voice.storage", detail = detail.into()).to_string())
    }

    pub fn task_management_disabled() -> Self {
        Self::TaskManagementDisabled(t!("errors.voice.task_management_disabled").to_string())
    }

    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::NotFound(t!("errors.voice.not_found", resource = resource.into()).to_string())
    }

    pub fn initialization_error(detail: impl Into<String>) -> Self {
        Self::Initialization(t!("errors.voice.initialization", detail = detail.into()).to_string())
    }

    pub fn tts_error(detail: impl Into<String>) -> Self {
        Self::TtsError(t!("errors.voice.tts", detail = detail.into()).to_string())
    }

    pub fn invalid_input(detail: impl Into<String>) -> Self {
        Self::InvalidInput(t!("errors.voice.invalid_input", detail = detail.into()).to_string())
    }

    pub fn io_error(detail: impl Into<String>) -> Self {
        Self::Io(t!("errors.voice.io", detail = detail.into()).to_string())
    }
}

// ===========================================
// 从外部错误类型转换
// ===========================================

impl From<std::io::Error> for VoiceCliError {
    fn from(err: std::io::Error) -> Self {
        Self::file_io_error(err.to_string())
    }
}

impl From<reqwest::Error> for VoiceCliError {
    fn from(err: reqwest::Error) -> Self {
        Self::http_error(err.to_string())
    }
}

impl From<serde_yaml::Error> for VoiceCliError {
    fn from(err: serde_yaml::Error) -> Self {
        Self::serialization_error(err.to_string())
    }
}

impl From<serde_json::Error> for VoiceCliError {
    fn from(err: serde_json::Error) -> Self {
        Self::json_error(err.to_string())
    }
}

impl From<ConfigError> for VoiceCliError {
    fn from(err: ConfigError) -> Self {
        Self::config_rs_error(err.to_string())
    }
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
            VoiceCliError::FileTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
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
            VoiceCliError::Network(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            VoiceCliError::Storage(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::ConfigRs(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::TaskManagementDisabled(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            VoiceCliError::Initialization(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            VoiceCliError::TtsError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::InvalidInput(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = Json(json!({
            "error": error_message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, VoiceCliError>;
