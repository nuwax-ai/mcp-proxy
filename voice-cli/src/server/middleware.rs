use axum::{
    extract::Request,
    http::{HeaderMap, Method, Uri},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::{error, info, warn};

/// HTTP请求日志中间件
/// 记录HTTP请求的详细信息，包括方法、路径、查询参数、headers等
/// 对于文件上传请求，不记录请求体内容以避免日志过大
pub async fn request_logging_middleware(request: Request, next: Next) -> Response {
    let start_time = Instant::now();
    
    // 提取请求信息
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let version = request.version();
    
    // 检查是否为文件上传请求
    let is_file_upload = is_file_upload_request(&method, &uri, &headers);
    
    // 获取用户IP (从headers中提取)
    let client_ip = extract_client_ip(&headers);
    
    // 获取用户代理
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    
    // 获取内容类型
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    
    // 获取内容长度
    let content_length = headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    // 记录请求开始
    info!(
        method = %method,
        uri = %uri,
        version = ?version,
        client_ip = %client_ip,
        user_agent = %user_agent,
        content_type = %content_type,
        content_length = content_length,
        is_file_upload = is_file_upload,
        "HTTP request started"
    );

    // 处理请求
    let response = next.run(request).await;
    
    // 计算处理时间
    let duration = start_time.elapsed();
    let duration_ms = duration.as_millis() as u64;
    
    // 获取响应信息
    let status = response.status();
    let response_headers = response.headers();
    
    // 获取响应内容长度
    let response_content_length = response_headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    // 记录请求完成
    match status.as_u16() {
        200..=299 => {
            info!(
                method = %method,
                uri = %uri,
                status = %status,
                duration_ms = duration_ms,
                response_content_length = response_content_length,
                client_ip = %client_ip,
                "HTTP request completed successfully"
            );
        }
        400..=499 => {
            warn!(
                method = %method,
                uri = %uri,
                status = %status,
                duration_ms = duration_ms,
                response_content_length = response_content_length,
                client_ip = %client_ip,
                "HTTP request completed with client error"
            );
        }
        500..=599 => {
            error!(
                method = %method,
                uri = %uri,
                status = %status,
                duration_ms = duration_ms,
                response_content_length = response_content_length,
                client_ip = %client_ip,
                "HTTP request completed with server error"
            );
        }
        _ => {
            warn!(
                method = %method,
                uri = %uri,
                status = %status,
                duration_ms = duration_ms,
                response_content_length = response_content_length,
                client_ip = %client_ip,
                "HTTP request completed with unexpected status"
            );
        }
    }

    response
}

/// 检查是否为文件上传请求
fn is_file_upload_request(method: &Method, uri: &Uri, headers: &HeaderMap) -> bool {
    // 检查是否为POST方法
    if method != Method::POST {
        return false;
    }
    
    // 检查路径是否为转录端点
    if uri.path() == "/transcribe" {
        return true;
    }
    
    // 检查Content-Type是否包含multipart
    if let Some(content_type) = headers.get("content-type") {
        if let Ok(content_type_str) = content_type.to_str() {
            return content_type_str.contains("multipart/form-data") 
                || content_type_str.contains("audio/")
                || content_type_str.contains("video/");
        }
    }
    
    false
}

/// 从请求头中提取客户端IP地址
fn extract_client_ip(headers: &HeaderMap) -> String {
    // 按优先级检查不同的IP头
    let ip_headers = [
        "x-forwarded-for",
        "x-real-ip", 
        "x-client-ip",
        "cf-connecting-ip",
        "true-client-ip",
    ];
    
    for header_name in &ip_headers {
        if let Some(header_value) = headers.get(*header_name) {
            if let Ok(ip_str) = header_value.to_str() {
                // x-forwarded-for 可能包含多个IP，取第一个
                let ip = ip_str.split(',').next().unwrap_or(ip_str).trim();
                if !ip.is_empty() && ip != "unknown" {
                    return ip.to_string();
                }
            }
        }
    }
    
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};
    
    #[test]
    fn test_middleware_module_exists() {
        // Simple test to verify the module compiles
        // Actual middleware testing would require more complex setup
        assert!(true);
    }
    
    #[test]
    fn test_is_file_upload_request() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("multipart/form-data"),
        );
        
        let method = Method::POST;
        let uri: Uri = "/transcribe".parse().unwrap();
        
        assert!(is_file_upload_request(&method, &uri, &headers));
        
        // Test with audio content type
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("audio/wav"),
        );
        
        assert!(is_file_upload_request(&method, &uri, &headers));
        
        // Test with non-upload request
        let method = Method::GET;
        let uri: Uri = "/health".parse().unwrap();
        let headers = HeaderMap::new();
        
        assert!(!is_file_upload_request(&method, &uri, &headers));
    }
    
    #[test]
    fn test_extract_client_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("192.168.1.1, 10.0.0.1"),
        );
        
        assert_eq!(extract_client_ip(&headers), "192.168.1.1");
        
        // Test with x-real-ip
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-real-ip"),
            HeaderValue::from_static("203.0.113.1"),
        );
        
        assert_eq!(extract_client_ip(&headers), "203.0.113.1");
        
        // Test with no IP headers
        let headers = HeaderMap::new();
        assert_eq!(extract_client_ip(&headers), "unknown");
    }
}
