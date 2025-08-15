use serde::{Deserialize, Serialize};
use axum::{
    response::{IntoResponse, Response},
    Json,
};

/// 统一HTTP响应格式
#[derive(Debug, Serialize, Deserialize)]
pub struct HttpResult<T> {
    pub code: String,
    pub message: String,
    pub data: Option<T>,
}

impl<T> HttpResult<T> {
    /// 成功响应
    pub fn success(data: T) -> Self {
        Self {
            code: "0000".to_string(),
            message: "操作成功".to_string(),
            data: Some(data),
        }
    }

    /// 成功响应（自定义消息）
    pub fn success_with_message(data: T, message: String) -> Self {
        Self {
            code: "0000".to_string(),
            message,
            data: Some(data),
        }
    }

    /// 错误响应（与泛型保持一致，data为空）
    pub fn error<E>(code: String, message: String) -> HttpResult<E> {
        HttpResult {
            code,
            message,
            data: None,
        }
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
        Json(self).into_response()
    }
}
