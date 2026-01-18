use axum::{
    Json,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// MCP 应用错误类型
///
/// 包含错误码和详细错误信息，便于程序化处理
#[derive(Error, Debug)]
pub enum AppError {
    /// 服务未找到 (0001)
    #[error("服务 {0} 未找到")]
    ServiceNotFound(String),

    /// 服务在重启冷却期内 (0002)
    #[error("服务 {0} 在重启冷却期内，请稍后再试")]
    ServiceRestartCooldown(String),

    /// 服务正在启动中 (0003)
    #[error("服务 {0} 正在启动中，请稍后再试")]
    ServiceStartupInProgress(String),

    /// 服务启动失败 (0004)
    #[error("服务启动失败: {mcp_id}: {reason}")]
    ServiceStartupFailed {
        mcp_id: String,
        reason: String,
    },

    /// 后端连接错误 (0005)
    #[error("后端连接错误: {0}")]
    BackendConnection(String),

    /// 配置解析错误 (0006)
    #[error("配置解析错误: {0}")]
    ConfigParse(String),

    /// MCP 服务器错误 (0007)
    #[error("MCP 服务器错误: {0}")]
    McpServerError(#[from] anyhow::Error),

    /// JSON 序列化错误 (0008)
    #[error("JSON 序列化错误: {0}")]
    SerdeJsonError(#[from] serde_json::Error),

    /// IO 错误 (0009)
    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),

    /// 路由未找到 (0010)
    #[error("路由未找到: {0}")]
    RouteNotFound(String),

    /// 无效的请求参数 (0011)
    #[error("无效的请求参数: {0}")]
    InvalidParameter(String),
}

impl AppError {
    /// 获取错误码
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::ServiceNotFound(_) => "0001",
            Self::ServiceRestartCooldown(_) => "0002",
            Self::ServiceStartupInProgress(_) => "0003",
            Self::ServiceStartupFailed { .. } => "0004",
            Self::BackendConnection(_) => "0005",
            Self::ConfigParse(_) => "0006",
            Self::McpServerError(_) => "0007",
            Self::SerdeJsonError(_) => "0008",
            Self::IoError(_) => "0009",
            Self::RouteNotFound(_) => "0010",
            Self::InvalidParameter(_) => "0011",
        }
    }

    /// 获取 HTTP 状态码
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::ServiceNotFound(_) => StatusCode::NOT_FOUND,
            Self::ServiceRestartCooldown(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::ServiceStartupInProgress(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::ServiceStartupFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BackendConnection(_) => StatusCode::BAD_GATEWAY,
            Self::ConfigParse(_) => StatusCode::BAD_REQUEST,
            Self::McpServerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::SerdeJsonError(_) => StatusCode::BAD_REQUEST,
            Self::IoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::RouteNotFound(_) => StatusCode::NOT_FOUND,
            Self::InvalidParameter(_) => StatusCode::BAD_REQUEST,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorOutput {
    pub code: String,
    pub error: String,
}

impl ErrorOutput {
    pub fn new(code: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            error: error.into(),
        }
    }

    pub fn with_code(code: &'static str, error: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            error: error.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response<axum::body::Body> {
        (
            self.status_code(),
            Json(ErrorOutput::with_code(self.error_code(), self.to_string())),
        )
            .into_response()
    }
}

/// HTTP 响应结果（用于成功响应）
#[derive(Debug, Serialize, Deserialize)]
pub struct HttpResult<T> {
    pub code: String,
    pub message: String,
    pub data: Option<T>,
}

impl<T> HttpResult<T> {
    pub fn ok(code: impl Into<String>, message: impl Into<String>, data: T) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>, data: Option<T>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data,
        }
    }
}

impl<T: Serialize> IntoResponse for HttpResult<T> {
    fn into_response(self) -> Response<axum::body::Body> {
        let status = if self.code.starts_with('2') {
            StatusCode::OK
        } else if self.code == "0002" {
            StatusCode::TOO_MANY_REQUESTS
        } else if self.code == "0003" {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };

        (status, Json(self)).into_response()
    }
}
