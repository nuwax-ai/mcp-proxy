use axum::{
    extract::Request,
    http::{HeaderMap, HeaderValue},
    middleware::Next,
    response::Response,
};
use opentelemetry::{
    trace::TraceContextExt,
    Context,
};
use std::time::Instant;
use tracing::{info_span, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::{REQUEST_ID_HEADER, SERVER_TIME_HEADER};

/// OpenTelemetry 追踪中间件
/// 
/// 功能：
/// 1. 自动创建 OpenTelemetry span 和 trace
/// 2. 在响应头中添加 x-request-id (trace_id)
/// 3. 在响应头中添加 x-server-time (请求处理时间)
/// 4. 记录 HTTP 请求的语义化属性
pub async fn opentelemetry_tracing_middleware(
    request: Request,
    next: Next,
) -> Response {
    let start_time = Instant::now();
    
    // 提取请求信息
    let method = request.method().to_string();
    let uri = request.uri().to_string();
    let version = format!("{:?}", request.version());
    let user_agent = request
        .headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");

    // 创建 OpenTelemetry span
    let span = info_span!(
        "http_request",
        otel.name = format!("{} {}", method, uri).as_str(),
        otel.kind = "server",
        http.method = method.as_str(),
        http.url = uri.as_str(),
        http.scheme = "http",
        http.version = version.as_str(),
        http.user_agent = user_agent,
    );

    // 设置 OpenTelemetry 属性
    let otel_cx = Context::current();
    span.set_parent(otel_cx);

    // 获取 trace_id
    let trace_id = span
        .context()
        .span()
        .span_context()
        .trace_id()
        .to_string();
    
    // 如果 trace_id 全为0，生成一个随机的 trace_id
    let trace_id = if trace_id == "00000000000000000000000000000000" {
        use uuid::Uuid;
        Uuid::new_v4().simple().to_string()
    } else {
        trace_id
    };

    async move {
        // 执行请求处理
        let mut response = next.run(request).await;
        
        // 计算处理时间
        let duration = start_time.elapsed();
        let duration_micros = duration.as_micros();
        
        // 记录响应状态码
        let status_code = response.status().as_u16();
        
        // 添加响应头
        let headers = response.headers_mut();
        
        // 添加 trace_id 到响应头
        if let Ok(trace_header) = HeaderValue::from_str(&trace_id) {
            headers.insert(REQUEST_ID_HEADER, trace_header);
        }
        
        // 添加服务器处理时间到响应头 (微秒)
        if let Ok(time_header) = HeaderValue::from_str(&duration_micros.to_string()) {
            headers.insert(SERVER_TIME_HEADER, time_header);
        }
        
        // 记录请求完成日志
        tracing::info!(
            method = %method,
            uri = %uri,
            status = %status_code,
            duration_micros = %duration_micros,
            trace_id = %trace_id,
            "HTTP request completed"
        );
        
        response
    }
    .instrument(span)
    .await
}

/// 从当前 OpenTelemetry 上下文中提取 trace_id
/// 
/// 这个函数可以在任何地方调用来获取当前请求的 trace_id
pub fn extract_trace_id() -> String {
    let current_span = tracing::Span::current();
    let context = current_span.context();
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    
    let trace_id = if span_context.is_valid() {
        span_context.trace_id().to_string()
    } else {
        "00000000000000000000000000000000".to_string()
    };
    
    // 如果 trace_id 全为0，生成一个随机的 trace_id
    if trace_id == "00000000000000000000000000000000" {
        use uuid::Uuid;
        Uuid::new_v4().simple().to_string()
    } else {
        trace_id
    }
}

/// 从请求头中提取现有的 trace_id（如果有的话）
/// 
/// 支持标准的 OpenTelemetry 传播头：
/// - traceparent (W3C Trace Context)
/// - x-trace-id (自定义)
pub fn extract_trace_from_headers(headers: &HeaderMap) -> Option<String> {
    // 尝试从 W3C Trace Context 中提取
    if let Some(traceparent) = headers.get("traceparent") {
        if let Ok(traceparent_str) = traceparent.to_str() {
            // traceparent 格式: 00-{trace_id}-{span_id}-{flags}
            let parts: Vec<&str> = traceparent_str.split('-').collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
    }
    
    // 尝试从自定义头中提取
    if let Some(trace_id) = headers.get("x-trace-id") {
        if let Ok(trace_id_str) = trace_id.to_str() {
            return Some(trace_id_str.to_string());
        }
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn test_extract_trace_from_headers_traceparent() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            HeaderValue::from_static("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        );
        
        let trace_id = extract_trace_from_headers(&headers);
        assert_eq!(trace_id, Some("4bf92f3577b34da6a3ce929d0e0e4736".to_string()));
    }

    #[test]
    fn test_extract_trace_from_headers_custom() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-trace-id",
            HeaderValue::from_static("custom-trace-id-123"),
        );
        
        let trace_id = extract_trace_from_headers(&headers);
        assert_eq!(trace_id, Some("custom-trace-id-123".to_string()));
    }

    #[test]
    fn test_extract_trace_from_headers_none() {
        let headers = HeaderMap::new();
        let trace_id = extract_trace_from_headers(&headers);
        assert_eq!(trace_id, None);
    }
}