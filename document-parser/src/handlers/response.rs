use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::models::HttpResult;
use crate::error::AppError;

/// 标准API响应构建器
pub struct ApiResponse;

impl ApiResponse {
    /// 成功响应
    pub fn success<T: Serialize>(data: T) ->  impl IntoResponse   {
        Json(HttpResult::success(data))
    }
    
    /// 成功响应（带状态码）
    pub fn success_with_status<T: Serialize>(
        data: T, 
        status: StatusCode
    ) -> impl IntoResponse {
        (status, HttpResult::success(data))
    }
    
    /// 错误响应
    pub fn error<T>(
        error_code: String, 
        message: String
    ) -> Json<HttpResult<T>> {
        Json(HttpResult::<T>::error(error_code, message))
    }
    
    /// 错误响应（带状态码）
    pub fn error_with_status<T>(
        error_code: String, 
        message: String, 
        status: StatusCode
    ) -> impl IntoResponse 
    where
        T: serde::Serialize,
    {
        (status, HttpResult::<T>::error::<T>(error_code, message))
    }
    
    /// 从AppError创建响应
    pub fn from_app_error<T>(error: AppError) -> impl IntoResponse 
    where
        T: serde::Serialize,
    {
        let status = match &error {
            AppError::Validation(_) => StatusCode::BAD_REQUEST,
            AppError::File(_) | AppError::UnsupportedFormat(_) => StatusCode::BAD_REQUEST,
            AppError::Task(_) => StatusCode::NOT_FOUND,
            AppError::Network(_) | AppError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
            AppError::Parse(_) | AppError::MinerU(_) | AppError::MarkItDown(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            AppError::Database(_) | AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Oss(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        
        (status, error.to_http_result::<T>())
    }
    
    /// 创建分页响应
    pub fn paginated<T: Serialize>(
        data: Vec<T>,
        total: usize,
        page: usize,
        page_size: usize,
    ) -> Json<HttpResult<PaginatedResponse<T>>> {
        let total_pages = (total + page_size - 1) / page_size;
        let response = PaginatedResponse {
            data,
            pagination: PaginationInfo {
                total,
                page,
                page_size,
                total_pages,
                has_next: page < total_pages,
                has_prev: page > 1,
            },
        };
        Json(HttpResult::success(response))
    }
    
    /// 创建空响应
    pub fn empty() -> Json<HttpResult<()>> {
        Json(HttpResult::success(()))
    }
    
    /// 创建消息响应
    pub fn message(message: String) -> Json<HttpResult<MessageResponse>> {
        Json(HttpResult::success(MessageResponse { message }))
    }
    
    /// 创建统计响应
    pub fn stats(stats: HashMap<String, serde_json::Value>) -> Json<HttpResult<StatsResponse>> {
        Json(HttpResult::success(StatsResponse { stats }))
    }
    
    /// 验证错误响应
    pub fn validation_error<T>(message: &str) -> Json<HttpResult<T>> {
        Json(HttpResult::<T>::error("VALIDATION_ERROR".to_string(), message.to_string()))
    }
    
    /// 内部错误响应
    pub fn internal_error<T>(message: &str) -> Json<HttpResult<T>> {
        Json(HttpResult::<T>::error("INTERNAL_ERROR".to_string(), message.to_string()))
    }
    
    /// 未找到错误响应
    pub fn not_found<T>(message: &str) -> Json<HttpResult<T>> {
        Json(HttpResult::<T>::error("NOT_FOUND".to_string(), message.to_string()))
    }
    
    /// 请求错误响应
    pub fn bad_request<T>(message: &str) -> Json<HttpResult<T>> {
        Json(HttpResult::<T>::error("BAD_REQUEST".to_string(), message.to_string()))
    }
}

/// 分页响应结构
#[derive(Debug, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub pagination: PaginationInfo,
}

/// 分页信息
#[derive(Debug, Serialize, Deserialize)]
pub struct PaginationInfo {
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
    pub has_next: bool,
    pub has_prev: bool,
}

/// 消息响应
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    pub message: String,
}

/// 统计响应
#[derive(Debug, Serialize, Deserialize)]
pub struct StatsResponse {
    pub stats: HashMap<String, serde_json::Value>,
}

/// 健康检查响应
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub timestamp: String,
    pub services: HashMap<String, ServiceHealth>,
}

/// 服务健康状态
#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceHealth {
    pub status: String,
    pub message: Option<String>,
    pub response_time_ms: Option<u64>,
}

/// 文件上传响应
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadResponse {
    pub task_id: String,
    pub message: String,
    pub file_info: FileInfo,
}

/// 文件信息
#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub filename: String,
    pub size: u64,
    pub format: String,
    pub mime_type: String,
}

/// URL下载响应
#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadResponse {
    pub task_id: String,
    pub message: String,
    pub url_info: UrlInfo,
}

/// URL信息
#[derive(Debug, Serialize, Deserialize)]
pub struct UrlInfo {
    pub url: String,
    pub format: String,
    pub estimated_size: Option<u64>,
}

/// 任务操作响应
#[derive(Debug, Serialize, Deserialize)]
pub struct TaskOperationResponse {
    pub task_id: String,
    pub operation: String,
    pub message: String,
    pub timestamp: String,
    pub task: Option<crate::models::DocumentTask>,
    /// 任务是否已完成（终态：完成、失败或取消）
    pub complete: bool,
}

/// 批量操作响应
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchOperationResponse {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub errors: Vec<BatchError>,
}

/// 批量操作错误
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchError {
    pub item_id: String,
    pub error_code: String,
    pub error_message: String,
}

/// 响应头工具
pub struct ResponseHeaders;

impl ResponseHeaders {
    /// 添加CORS头
    pub fn cors() -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
            "*".parse().unwrap(),
        );
        headers.insert(
            axum::http::header::ACCESS_CONTROL_ALLOW_METHODS,
            "GET, POST, PUT, DELETE, OPTIONS".parse().unwrap(),
        );
        headers.insert(
            axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Content-Type, Authorization, X-Requested-With".parse().unwrap(),
        );
        headers
    }
    
    /// 添加缓存头
    pub fn cache_control(max_age: u32) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            format!("public, max-age={}", max_age).parse().unwrap(),
        );
        headers
    }
    
    /// 添加内容类型头
    pub fn content_type(content_type: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            content_type.parse().unwrap(),
        );
        headers
    }
}

/// 响应时间中间件
use std::time::Instant;
use axum::{
    extract::Request,
    middleware::Next,
};

pub async fn response_time_middleware(
    request: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let mut response = next.run(request).await;
    let duration = start.elapsed();
    
    response.headers_mut().insert(
        "X-Response-Time",
        format!("{:.2}ms", duration.as_secs_f64() * 1000.0)
            .parse()
            .unwrap(),
    );
    
    response
}