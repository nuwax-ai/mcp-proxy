use axum::body::Body;
use axum::{extract::Request, middleware::Next, response::Response};
use tracing::{Instrument, info_span};
use uuid::Uuid;

/// 基本追踪中间件
/// 为每个请求生成唯一的追踪ID，并在响应中添加tid字段
pub async fn basic_tracing_middleware(request: Request, next: Next) -> Response {
    // 获取或生成追踪ID
    let tid = get_or_generate_trace_id(&request);

    // 在移动 request 之前先获取 URI 信息
    let is_health_check = request.uri().path() == "/health";

    // 创建请求span
    let span = info_span!(
        "http_request",
        tid = %tid,
        method = %request.method(),
        uri = %request.uri(),
        version = ?request.version(),
    );

    // 在span中执行请求处理
    let response = next.run(request).instrument(span).await;

    // 健康检查不处理
    if is_health_check {
        return response;
    }

    // 仅当响应是 HttpResult（通过扩展标记判断）才注入 tid
    if response
        .extensions()
        .get::<crate::models::HttpResultMarker>()
        .is_some()
    {
        return add_tid_to_response(response, tid).await;
    }

    response
}

/// 获取或生成追踪ID
fn get_or_generate_trace_id(request: &Request) -> String {
    if let Some(traceparent) = request.headers().get("traceparent") {
        if let Ok(traceparent_str) = traceparent.to_str() {
            if let Some(trace_id) = traceparent_str.split('-').nth(1) {
                if trace_id.len() == 32 {
                    return trace_id.to_string();
                }
            }
        }
    }
    Uuid::new_v4().to_string()
}

/// 向响应中添加追踪ID
async fn add_tid_to_response(response: Response, tid: String) -> Response {
    // 分解响应以获取body和parts
    let (parts, body) = response.into_parts();

    // 尝试从body中提取JSON内容
    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => {
            // 如果无法读取body，直接返回原响应
            return Response::from_parts(parts, Body::from(Vec::<u8>::new()));
        }
    };

    // 尝试解析为JSON
    if let Ok(mut json_value) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        // 仅当是 HttpResult 结构（至少包含 code 与 data）时才注入 tid
        if let Some(obj) = json_value.as_object_mut() {
            let is_http_result_like = obj.contains_key("code") && obj.contains_key("data");
            if is_http_result_like {
                obj.insert("tid".to_string(), serde_json::Value::String(tid));
                if let Ok(new_body) = serde_json::to_vec(&json_value) {
                    return Response::from_parts(parts, Body::from(new_body));
                }
            }
        }
    }

    // 如果不是 HttpResult 或无法修改，返回原响应
    Response::from_parts(parts, Body::from(body_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};

    #[test]
    fn test_get_or_generate_trace_id() {
        let request = Request::builder()
            .header(
                "traceparent",
                "00-12345678901234567890123456789012-1234567890123456-01",
            )
            .uri("/some/path")
            .body(axum::body::Body::from("{}"))
            .unwrap();
        assert_eq!(
            get_or_generate_trace_id(&request),
            "12345678901234567890123456789012"
        );

        let request = Request::builder()
            .uri("/health")
            .body(axum::body::Body::from("{}"))
            .unwrap();
        assert_eq!(
            get_or_generate_trace_id(&request),
            Uuid::new_v4().to_string()
        );

        let request = Request::builder()
            .uri("/some/path")
            .body(axum::body::Body::from("{}"))
            .unwrap();
        assert_eq!(
            get_or_generate_trace_id(&request),
            Uuid::new_v4().to_string()
        );
    }
}
