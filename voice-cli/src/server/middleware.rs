use axum::{
    extract::Request,
    http::{HeaderMap, Method, Uri},
    middleware::Next,
    response::Response,
    body::Body,
};
use std::time::Instant;
use tracing::{error, info, warn};
use serde_json::Value;

/// HTTP请求日志中间件
/// 记录HTTP请求的详细信息，包括方法、路径、查询参数、headers等
/// 对于Multipart请求，不记录请求体内容以避免日志过大
/// 对于其他请求，记录请求参数（body和query params）
pub async fn request_logging_middleware(request: Request, next: Next) -> Response {
    let start_time = Instant::now();
    
    // 提取请求信息
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let version = request.version();
    
    // 检查是否为Multipart请求
    let is_multipart = is_multipart_request(&method, &uri, &headers);
    
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

    // 获取查询参数
    let query_params = extract_query_params(&uri);

    // 记录请求开始
    let (request, _body_params) = if is_multipart {
        info!(
            method = %method,
            uri = %uri,
            version = ?version,
            client_ip = %client_ip,
            user_agent = %user_agent,
            content_type = %content_type,
            content_length = content_length,
            query_params = ?query_params,
            is_multipart = true,
            "HTTP request started (Multipart - body not logged)"
        );
        (request, Value::Null)
    } else {
        // 对于非Multipart请求，提取请求体参数
        let (body_params, rebuilt_request) = extract_body_params(request).await;
        
        info!(
            method = %method,
            uri = %uri,
            version = ?version,
            client_ip = %client_ip,
            user_agent = %user_agent,
            content_type = %content_type,
            content_length = content_length,
            query_params = ?query_params,
            body_params = ?body_params,
            is_multipart = false,
            "HTTP request started (body params extracted)"
        );
        (rebuilt_request, body_params)
    };

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

/// 检查是否为Multipart请求
fn is_multipart_request(method: &Method, uri: &Uri, headers: &HeaderMap) -> bool {
    // 检查Content-Type是否包含multipart
    if let Some(content_type) = headers.get("content-type") {
        if let Ok(content_type_str) = content_type.to_str() {
            return content_type_str.contains("multipart/form-data");
        }
    }
    false
}

/// 提取请求体参数（仅适用于非Multipart请求）
/// 注意：此函数会消费请求，调用者需要重新构建请求
async fn extract_body_params(request: Request) -> (Value, Request) {
    // 只处理JSON请求体
    let content_type = request.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    if content_type.contains("application/json") {
        // 提取请求体
        let (parts, body) = request.into_parts();
        
        // 尝试读取请求体
        match axum::body::to_bytes(body, usize::MAX).await {
            Ok(bytes) => {
                // 克隆字节数据以便重新构建请求
                let bytes_clone = bytes.clone();
                
                // 尝试解析JSON
                match serde_json::from_slice::<Value>(&bytes) {
                    Ok(json_value) => {
                        // 重新构建请求体
                        let new_body = Body::from(bytes_clone);
                        let new_request = Request::from_parts(parts, new_body);
                        (json_value, new_request)
                    }
                    Err(_) => {
                        // JSON解析失败，重新构建请求并返回原始内容
                        let raw_content = Value::String(String::from_utf8_lossy(&bytes).to_string());
                        let new_body = Body::from(bytes);
                        let new_request = Request::from_parts(parts, new_body);
                        (raw_content, new_request)
                    }
                }
            }
            Err(_) => {
                // 无法读取请求体，重新构建空请求
                let new_body = Body::empty();
                let new_request = Request::from_parts(parts, new_body);
                (Value::Null, new_request)
            }
        }
    } else if content_type.contains("application/x-www-form-urlencoded") {
        // 处理表单数据
        let (parts, body) = request.into_parts();
        
        match axum::body::to_bytes(body, usize::MAX).await {
            Ok(bytes) => {
                let bytes_clone = bytes.clone();
                let form_data = String::from_utf8_lossy(&bytes);
                let mut params = serde_json::Map::new();
                
                for (key, value) in url::form_urlencoded::parse(form_data.as_bytes()) {
                    params.insert(key.to_string(), Value::String(value.to_string()));
                }
                
                // 重新构建请求体
                let new_body = Body::from(bytes_clone);
                let new_request = Request::from_parts(parts, new_body);
                (Value::Object(params), new_request)
            }
            Err(_) => {
                // 无法读取请求体，重新构建空请求
                let new_body = Body::empty();
                let new_request = Request::from_parts(parts, new_body);
                (Value::Null, new_request)
            }
        }
    } else {
        // 不支持的Content-Type，返回Null
        (Value::Null, request)
    }
}

/// 检查是否为文件上传请求（保留原有逻辑以兼容）
fn is_file_upload_request(method: &Method, uri: &Uri, headers: &HeaderMap) -> bool {
    // 检查是否为POST方法
    if method != Method::POST {
        return false;
    }
    
    // 检查路径是否为转录端点
    if uri.path() == "/transcribe" {
        return true;
    }
    
    // 检查Content-Type是否包含multipart或媒体文件
    if let Some(content_type) = headers.get("content-type") {
        if let Ok(content_type_str) = content_type.to_str() {
            return content_type_str.contains("multipart/form-data") 
                || content_type_str.contains("audio/")
                || content_type_str.contains("video/");
        }
    }
    
    false
}

/// 提取查询参数
fn extract_query_params(uri: &Uri) -> Value {
    if let Some(query) = uri.query() {
        let mut params = serde_json::Map::new();
        
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            params.insert(key.to_string(), Value::String(value.to_string()));
        }
        
        Value::Object(params)
    } else {
        Value::Null
    }
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
    fn test_is_multipart_request() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("multipart/form-data; boundary=----WebKitFormBoundary7MA4YWxkTrZu0gW"),
        );
        
        let method = Method::POST;
        let uri: Uri = "/api/v1/tasks/transcribe".parse().unwrap();
        
        assert!(is_multipart_request(&method, &uri, &headers));
        
        // Test with non-multipart content type
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        
        assert!(!is_multipart_request(&method, &uri, &headers));
        
        // Test with GET request
        let method = Method::GET;
        let uri: Uri = "/health".parse().unwrap();
        let headers = HeaderMap::new();
        
        assert!(!is_multipart_request(&method, &uri, &headers));
    }

    #[test]
    fn test_extract_query_params() {
        let uri: Uri = "http://localhost:8080/api/v1/tasks?status=completed&limit=10".parse().unwrap();
        let params = extract_query_params(&uri);
        
        if let Value::Object(map) = params {
            assert_eq!(map.get("status"), Some(&Value::String("completed".to_string())));
            assert_eq!(map.get("limit"), Some(&Value::String("10".to_string())));
        } else {
            panic!("Expected object but got: {:?}", params);
        }
    }

    #[test]
    fn test_extract_query_params_empty() {
        let uri: Uri = "http://localhost:8080/health".parse().unwrap();
        let params = extract_query_params(&uri);
        
        assert_eq!(params, Value::Null);
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
