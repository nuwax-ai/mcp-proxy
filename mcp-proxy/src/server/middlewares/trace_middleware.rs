use axum::{
    body::Body,
    extract::Request,
    http::HeaderMap,
    response::Response,
    middleware::Next,
};
use tracing::Span;
use uuid::Uuid;

/// 请求追踪上下文
#[derive(Clone, Debug)]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
}

impl TraceContext {
    pub fn new() -> Self {
        Self {
            trace_id: Uuid::new_v4().to_string(),
            span_id: format!("{:016x}", rand::random::<u64>()),
            parent_span_id: None,
        }
    }

    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        headers
            .get("x-trace-id")
            .and_then(|value| value.to_str().ok())
            .map(|trace_id| Self {
                trace_id: trace_id.to_string(),
                span_id: format!("{:016x}", rand::random::<u64>()),
                parent_span_id: headers
                    .get("x-span-id")
                    .and_then(|v| v.to_str().ok().map(|s| s.to_string())),
            })
    }

    pub fn to_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-trace-id", self.trace_id.parse().unwrap());
        headers.insert("x-span-id", self.span_id.parse().unwrap());
        if let Some(ref parent_id) = self.parent_span_id {
            headers.insert("x-parent-span-id", parent_id.parse().unwrap());
        }
        headers
    }
}

/// 追踪中间件
pub async fn trace_middleware(
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    // 1. 获取或创建追踪上下文
    let trace_context = TraceContext::from_headers(request.headers())
        .unwrap_or_else(TraceContext::new);

    // 2. 将 trace_id 添加到请求头中，供下游使用
    let trace_headers = trace_context.to_headers();
    for (name, value) in trace_headers.iter() {
        request.headers_mut().insert(name, value.clone());
    }

    // 3. 创建根 span 并记录 trace 信息
    let span = tracing::info_span!(
        "http_request",
        trace_id = %trace_context.trace_id,
        span_id = %trace_context.span_id,
        http.method = %request.method(),
        http.route = %request.uri().path(),
        http.url = %request.uri(),
        component = "http_middleware",
    );

    // 4. 如果请求中有 x-trace-id 头，也记录到日志中
    if let Some(trace_header) = request.headers().get("x-trace-id") {
        if let Ok(trace_id) = trace_header.to_str() {
            tracing::info!(
                parent: &span,
                "HTTP请求开始 - Method: {}, Path: {}, TraceId: {}",
                request.method(),
                request.uri().path(),
                trace_id
            );
        }
    }

    // 5. 执行下一个中间件/处理器
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    let response = async {
        let _guard = span.enter();
        next.run(request).await
    }.await;

    // 6. 记录响应信息
    let status = response.status();
    span.record("http.response.status_code", status.as_u16());

    tracing::info!(
        parent: &span,
        "HTTP请求完成 - Method: {}, Path: {}, Status: {}, TraceId: {}",
        method,
        path,
        status,
        trace_context.trace_id
    );

    Ok(response)
}

/// 从请求头中提取 trace_id
pub fn extract_trace_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-trace-id")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
}

/// 创建带有 trace_id 的 span
pub fn trace_span_with_context(_name: &str, headers: &HeaderMap) -> Span {
    if let Some(trace_id) = extract_trace_id(headers) {
        tracing::info_span!(
            "trace_span",
            trace_id = %trace_id,
            component = "dynamic_router"
        )
    } else {
        tracing::info_span!("trace_span", component = "dynamic_router")
    }
}

/// 为 tracing 订阅器添加 trace_id 过滤器
pub fn setup_trace_id_logging() {
    use tracing_subscriber::{filter::EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .init();
}