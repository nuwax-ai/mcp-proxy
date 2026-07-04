use axum::{
    Json,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use mcp_common::t;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// MCP 应用错误类型
///
/// 包含错误码和详细错误信息，便于程序化处理
#[derive(Error, Debug)]
pub enum AppError {
    /// 服务未找到 (0001)
    #[error("{0}")]
    ServiceNotFound(String),

    /// 服务在重启冷却期内 (0002)
    #[error("{0}")]
    ServiceRestartCooldown(String),

    /// 服务正在启动中 (0003)
    #[error("{0}")]
    ServiceStartupInProgress(String),

    /// 服务启动失败 (0004)
    #[error("{0}")]
    ServiceStartupFailed(String),

    /// 后端连接错误 (0005)
    #[error("{0}")]
    BackendConnection(String),

    /// 配置解析错误 (0006)
    #[error("{0}")]
    ConfigParse(String),

    /// MCP 服务器错误 (0007)
    #[error("{0}")]
    McpServerError(String),

    /// JSON 序列化错误 (0008)
    #[error("{0}")]
    SerdeJsonError(String),

    /// IO 错误 (0009)
    #[error("{0}")]
    IoError(String),

    /// 路由未找到 (0010)
    #[error("{0}")]
    RouteNotFound(String),

    /// 无效的请求参数 (0011)
    #[error("{0}")]
    InvalidParameter(String),
}

impl AppError {
    // ===========================================
    // 工厂方法 - 创建带国际化消息的错误
    // ===========================================

    /// 创建服务未找到错误
    pub fn service_not_found(service: impl Into<String>) -> Self {
        Self::ServiceNotFound(
            t!(
                "errors.mcp_proxy.service_not_found",
                service = service.into()
            )
            .to_string(),
        )
    }

    /// 创建服务重启冷却期错误
    pub fn service_restart_cooldown(service: impl Into<String>) -> Self {
        Self::ServiceRestartCooldown(
            t!(
                "errors.mcp_proxy.service_restart_cooldown",
                service = service.into()
            )
            .to_string(),
        )
    }

    /// 创建服务启动中错误
    pub fn service_startup_in_progress(service: impl Into<String>) -> Self {
        Self::ServiceStartupInProgress(
            t!(
                "errors.mcp_proxy.service_startup_in_progress",
                service = service.into()
            )
            .to_string(),
        )
    }

    /// 创建服务启动失败错误
    pub fn service_startup_failed(mcp_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ServiceStartupFailed(
            t!(
                "errors.mcp_proxy.service_startup_failed",
                mcp_id = mcp_id.into(),
                reason = reason.into()
            )
            .to_string(),
        )
    }

    /// 创建后端连接错误
    pub fn backend_connection(detail: impl Into<String>) -> Self {
        Self::BackendConnection(
            t!(
                "errors.mcp_proxy.backend_connection",
                detail = detail.into()
            )
            .to_string(),
        )
    }

    /// 创建配置解析错误
    pub fn config_parse(detail: impl Into<String>) -> Self {
        Self::ConfigParse(t!("errors.mcp_proxy.config_parse", detail = detail.into()).to_string())
    }

    /// 创建 MCP 服务器错误
    pub fn mcp_server_error(detail: impl Into<String>) -> Self {
        Self::McpServerError(
            t!("errors.mcp_proxy.mcp_server_error", detail = detail.into()).to_string(),
        )
    }

    /// 创建 JSON 序列化错误
    pub fn json_serialization(detail: impl Into<String>) -> Self {
        Self::SerdeJsonError(
            t!(
                "errors.mcp_proxy.json_serialization",
                detail = detail.into()
            )
            .to_string(),
        )
    }

    /// 创建 IO 错误
    pub fn io_error(detail: impl Into<String>) -> Self {
        Self::IoError(t!("errors.mcp_proxy.io_error", detail = detail.into()).to_string())
    }

    /// 创建路由未找到错误
    pub fn route_not_found(path: impl Into<String>) -> Self {
        Self::RouteNotFound(t!("errors.mcp_proxy.route_not_found", path = path.into()).to_string())
    }

    /// 创建无效参数错误
    pub fn invalid_parameter(detail: impl Into<String>) -> Self {
        Self::InvalidParameter(
            t!("errors.mcp_proxy.invalid_parameter", detail = detail.into()).to_string(),
        )
    }

    // ===========================================
    // 错误码和状态码
    // ===========================================

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

// ===========================================
// 从外部错误类型转换
// ===========================================

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::mcp_server_error(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        Self::json_serialization(err.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::io_error(err.to_string())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorOutput {
    pub code: String,
    pub error: String,
}

impl ErrorOutput {
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
