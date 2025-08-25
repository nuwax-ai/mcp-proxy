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
            VoiceCliError::WorkerPoolError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::TranscriptionTimeout(_) => (StatusCode::REQUEST_TIMEOUT, self.to_string()),
            VoiceCliError::TranscriptionFailed(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::AudioConversionFailed(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::AudioProbeError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::TempFileError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            VoiceCliError::MultipartError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            VoiceCliError::MissingField(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        };

        let body = Json(json!({
            "error": error_message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}

// Conversion from ClusterError to VoiceCliError (for HTTP responses)
impl From<crate::models::metadata_store::ClusterError> for VoiceCliError {
    fn from(error: crate::models::metadata_store::ClusterError) -> Self {
        match error {
            crate::models::metadata_store::ClusterError::Config(msg) => VoiceCliError::Config(msg),
            crate::models::metadata_store::ClusterError::Network(msg) => VoiceCliError::Config(format!("Network error: {}", msg)),
            crate::models::metadata_store::ClusterError::Timeout(msg) => VoiceCliError::Config(format!("Timeout: {}", msg)),
            crate::models::metadata_store::ClusterError::NodeNotFound(msg) => VoiceCliError::Config(format!("Node not found: {}", msg)),
            crate::models::metadata_store::ClusterError::TaskNotFound(msg) => VoiceCliError::Config(format!("Task not found: {}", msg)),
            crate::models::metadata_store::ClusterError::NoAvailableNodes => VoiceCliError::Config("No available nodes".to_string()),
            crate::models::metadata_store::ClusterError::TranscriptionFailed(msg) => VoiceCliError::TranscriptionFailed(msg),
            crate::models::metadata_store::ClusterError::InvalidOperation(msg) => VoiceCliError::Config(format!("Invalid operation: {}", msg)),
            crate::models::metadata_store::ClusterError::Database(err) => VoiceCliError::Config(format!("Database error: {}", err)),
            crate::models::metadata_store::ClusterError::Serialization(err) => VoiceCliError::Json(err),
        }
    }
}

// Note: ClusterError automatically converts to anyhow::Error via the blanket impl
// since ClusterError implements std::error::Error through thiserror

/// Extension trait for adding context to cluster operation results
pub trait ClusterResultExt<T> {
    /// Add node context to the error
    fn with_node_context(self, node_id: &str) -> anyhow::Result<T>;
    
    /// Add task context to the error
    fn with_task_context(self, task_id: &str) -> anyhow::Result<T>;
    
    /// Add cluster operation context to the error
    fn with_cluster_context(self, operation: &str) -> anyhow::Result<T>;
}

impl<T> ClusterResultExt<T> for std::result::Result<T, crate::models::metadata_store::ClusterError> {
    fn with_node_context(self, node_id: &str) -> anyhow::Result<T> {
        self.map_err(|e| anyhow::Error::new(e).context(format!("Node operation failed for node '{}'", node_id)))
    }
    
    fn with_task_context(self, task_id: &str) -> anyhow::Result<T> {
        self.map_err(|e| anyhow::Error::new(e).context(format!("Task operation failed for task '{}'", task_id)))
    }
    
    fn with_cluster_context(self, operation: &str) -> anyhow::Result<T> {
        self.map_err(|e| anyhow::Error::new(e).context(format!("Cluster operation '{}' failed", operation)))
    }
}

pub type Result<T> = std::result::Result<T, VoiceCliError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::metadata_store::ClusterError;

    #[test]
    fn test_cluster_result_ext_node_context() {
        let error: std::result::Result<(), ClusterError> = Err(ClusterError::NodeNotFound("test-node".to_string()));
        let result = error.with_node_context("test-node");
        
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        println!("Error message: {}", error_msg);
        assert!(error_msg.contains("Node operation failed for node 'test-node'"));
        // The original error message is in the chain, not necessarily in the main message
        let error_chain = format!("{:?}", error);
        assert!(error_chain.contains("Node not found: test-node"));
    }

    #[test]
    fn test_cluster_result_ext_task_context() {
        let error: std::result::Result<(), ClusterError> = Err(ClusterError::TaskNotFound("task-123".to_string()));
        let result = error.with_task_context("task-123");
        
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        println!("Error message: {}", error_msg);
        assert!(error_msg.contains("Task operation failed for task 'task-123'"));
        // The original error message is in the chain, not necessarily in the main message
        let error_chain = format!("{:?}", error);
        assert!(error_chain.contains("Task not found: task-123"));
    }

    #[test]
    fn test_cluster_result_ext_cluster_context() {
        let error: std::result::Result<(), ClusterError> = Err(ClusterError::NoAvailableNodes);
        let result = error.with_cluster_context("task assignment");
        
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        println!("Error message: {}", error_msg);
        assert!(error_msg.contains("Cluster operation 'task assignment' failed"));
        // The original error message is in the chain, not necessarily in the main message
        let error_chain = format!("{:?}", error);
        assert!(error_chain.contains("No available nodes"));
    }

    #[test]
    fn test_cluster_result_ext_success() {
        let success: std::result::Result<String, ClusterError> = Ok("success".to_string());
        let result = success.with_node_context("test-node");
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }
}