use crate::error::AppError;
use crate::models::HttpResult;
use axum::{
    Json,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::time::Instant;
use tracing::{error, info, warn};

/// 错误处理中间件
pub async fn error_handler_middleware(request: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();

    let response = next.run(request).await;
    let duration = start.elapsed();

    // 记录请求日志
    info!(
        "HTTP {} {} - {} - {:?}",
        method,
        uri,
        response.status(),
        duration
    );

    response
}

/// 全局错误处理器
pub async fn global_error_handler(err: AppError) -> impl IntoResponse {
    let (status, error_response) = match &err {
        AppError::Validation(_) => {
            warn!("Validation error: {}", err);
            (StatusCode::BAD_REQUEST, err.to_http_result::<()>())
        }
        AppError::File(_) | AppError::UnsupportedFormat(_) => {
            warn!("File error: {}", err);
            (StatusCode::BAD_REQUEST, err.to_http_result::<()>())
        }
        AppError::Task(_) => {
            warn!("Task error: {}", err);
            (StatusCode::NOT_FOUND, err.to_http_result::<()>())
        }
        AppError::Network(_) | AppError::Timeout(_) => {
            warn!("Network/Timeout error: {}", err);
            (StatusCode::REQUEST_TIMEOUT, err.to_http_result::<()>())
        }
        AppError::Parse(_) | AppError::MinerU(_) | AppError::MarkItDown(_) => {
            error!("Parser error: {}", err);
            (StatusCode::UNPROCESSABLE_ENTITY, err.to_http_result::<()>())
        }
        AppError::Database(_) | AppError::Internal(_) => {
            error!("Internal error: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err.to_http_result::<()>(),
            )
        }
        AppError::Oss(_) => {
            error!("OSS error: {}", err);
            (StatusCode::BAD_GATEWAY, err.to_http_result::<()>())
        }
        _ => {
            error!("Unknown error: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err.to_http_result::<()>(),
            )
        }
    };

    (status, Json(error_response))
}

/// 速率限制中间件（简单实现）
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<SystemTime>>>>,
    max_requests: usize,
    window_duration: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window_duration: Duration) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_duration,
        }
    }

    pub fn check_rate_limit(&self, client_ip: &str) -> bool {
        let now = SystemTime::now();
        let mut requests = self.requests.lock().unwrap();

        let client_requests = requests
            .entry(client_ip.to_string())
            .or_default();

        // 清理过期的请求记录
        client_requests.retain(|&time| {
            now.duration_since(time).unwrap_or(Duration::MAX) < self.window_duration
        });

        // 检查是否超过限制
        if client_requests.len() >= self.max_requests {
            false
        } else {
            client_requests.push(now);
            true
        }
    }
}

pub async fn rate_limit_middleware(request: Request, next: Next) -> Response {
    // 简单的IP提取（实际应用中应该考虑代理头）
    let client_ip = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    // 创建速率限制器（1秒最多100个请求）
    static RATE_LIMITER: std::sync::OnceLock<RateLimiter> = std::sync::OnceLock::new();
    let limiter = RATE_LIMITER.get_or_init(|| RateLimiter::new(100, Duration::from_secs(1)));

    if !limiter.check_rate_limit(&client_ip) {
        let error_response: HttpResult<()> =
            HttpResult::<()>::error("E017".to_string(), "请求频率过高，请稍后再试".to_string());
        return (StatusCode::TOO_MANY_REQUESTS, Json(error_response)).into_response();
    }

    next.run(request).await
}
