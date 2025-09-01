use crate::VoiceCliError;
use axum::{
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// HttpResult 响应标记（仅用于内部中间件识别）
#[derive(Clone)]
pub struct HttpResultMarker;

/// 统一HTTP响应格式
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HttpResult<T> {
    pub code: String,
    pub message: String,
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tid: Option<String>,
}

impl<T> HttpResult<T> {
    /// 成功响应
    pub fn success(data: T) -> Self {
        Self {
            code: "0000".to_string(),
            message: "操作成功".to_string(),
            data: Some(data),
            tid: None,
        }
    }

    /// 成功响应（自定义消息）
    pub fn success_with_message(data: T, message: String) -> Self {
        Self {
            code: "0000".to_string(),
            message,
            data: Some(data),
            tid: None,
        }
    }

    /// 错误响应（与泛型保持一致，data为空）
    pub fn error<E>(code: String, message: String) -> HttpResult<E> {
        HttpResult {
            code,
            message,
            data: None,
            tid: None,
        }
    }

    /// 设置追踪ID
    pub fn with_tid(mut self, tid: String) -> Self {
        self.tid = Some(tid);
        self
    }

    /// 系统错误
    pub fn system_error<E>(message: String) -> HttpResult<E> {
        Self::error("E001".to_string(), message)
    }

    /// 格式不支持错误
    pub fn unsupported_format<E>(message: String) -> HttpResult<E> {
        Self::error("E002".to_string(), message)
    }

    /// 任务不存在错误
    pub fn task_not_found<E>(message: String) -> HttpResult<E> {
        Self::error("E003".to_string(), message)
    }

    /// 处理失败错误
    pub fn processing_failed<E>(message: String) -> HttpResult<E> {
        Self::error("E004".to_string(), message)
    }
}

impl<T> IntoResponse for HttpResult<T>
where
    T: serde::Serialize,
{
    fn into_response(self) -> Response {
        let mut res = Json(self).into_response();
        // 标记该响应为 HttpResult，供中间件识别
        res.extensions_mut().insert(HttpResultMarker);
        res
    }
}

/// Convert VoiceCliError to HttpResult for consistent error responses
impl<T> From<VoiceCliError> for HttpResult<T> {
    fn from(error: VoiceCliError) -> Self {
        match error {
            VoiceCliError::Config(msg) => {
                Self::system_error(format!("Configuration error: {}", msg))
            }
            VoiceCliError::FileTooLarge { size, max } => Self::unsupported_format(format!(
                "File size {} exceeds maximum {} bytes",
                size, max
            )),
            VoiceCliError::UnsupportedFormat(msg) => Self::unsupported_format(msg),
            VoiceCliError::ModelNotFound(msg) => {
                Self::task_not_found(format!("Model not found: {}", msg))
            }
            VoiceCliError::InvalidModelName(msg) => {
                Self::unsupported_format(format!("Invalid model name: {}", msg))
            }
            VoiceCliError::TranscriptionFailed(msg) => {
                Self::processing_failed(format!("Transcription failed: {}", msg))
            }
            VoiceCliError::TranscriptionTimeout(timeout) => {
                Self::processing_failed(format!("Transcription timeout after {} seconds", timeout))
            }
            VoiceCliError::WorkerPoolError(msg) => {
                Self::processing_failed(format!("Worker pool error: {}", msg))
            }
            VoiceCliError::AudioProcessing(msg) => {
                Self::unsupported_format(format!("Audio processing error: {}", msg))
            }
            VoiceCliError::AudioConversionFailed(msg) => {
                Self::unsupported_format(format!("Audio conversion failed: {}", msg))
            }
            VoiceCliError::AudioProbeError(msg) => {
                Self::unsupported_format(format!("Audio probe error: {}", msg))
            }
            VoiceCliError::MultipartError(msg) => {
                Self::unsupported_format(format!("Multipart form error: {}", msg))
            }
            VoiceCliError::MissingField(field) => {
                Self::unsupported_format(format!("Missing required field: {}", field))
            }
            VoiceCliError::TempFileError(msg) => {
                Self::system_error(format!("Temporary file error: {}", msg))
            }
            VoiceCliError::FileIo(err) => Self::system_error(format!("File I/O error: {}", err)),
            VoiceCliError::Http(err) => Self::system_error(format!("HTTP request error: {}", err)),
            VoiceCliError::Serialization(err) => {
                Self::system_error(format!("Serialization error: {}", err))
            }
            VoiceCliError::Json(err) => Self::system_error(format!("JSON error: {}", err)),
            VoiceCliError::Transcription(msg) => {
                Self::processing_failed(format!("Transcription error: {}", msg))
            }
            VoiceCliError::Model(msg) => Self::system_error(format!("Model error: {}", msg)),
            VoiceCliError::Daemon(msg) => Self::system_error(format!("Daemon error: {}", msg)),
            VoiceCliError::ConfigRs(err) => Self::system_error(format!("Configuration error: {}", err)),
            VoiceCliError::Storage(msg) => Self::system_error(format!("Storage error: {}", msg)),
            VoiceCliError::TaskManagementDisabled => Self::system_error("Task management is disabled".to_string()),
            VoiceCliError::NotFound(msg) => Self::task_not_found(msg),
            VoiceCliError::Network(msg) => Self::system_error(format!("Network error: {}", msg)),
            VoiceCliError::Initialization(msg) => Self::system_error(format!("Initialization error: {}", msg)),
        }
    }
}
