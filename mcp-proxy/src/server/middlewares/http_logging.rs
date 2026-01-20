//! HTTP request/response logging middleware
//!
//! This middleware logs all HTTP requests and responses at TRACE level.
//! Using TRACE level avoids excessive log output for frequent MCP API calls.
//!
//! To enable HTTP logging, set:
//! - `RUST_LOG=trace` (global)
//! - `RUST_LOG=mcp_proxy=trace` (module-specific)

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use tracing::trace;

/// HTTP request/response logging middleware
///
/// Logs incoming requests with details (method, uri, route, headers)
/// and outgoing responses (status, duration, content length).
///
/// # Log Level
/// Uses TRACE level because MCP API calls are very frequent.
/// Enable with `RUST_LOG=trace` or `RUST_LOG=mcp_proxy=trace`.
///
/// # Middleware Order
/// Should be placed before `opentelemetry_tracing_middleware` to log
/// request entry first, while avoiding duplication of completion logs.
pub async fn http_logging_middleware(request: Request, next: Next) -> Response {
    // Extract all needed data before moving the request
    let method = request.method().clone();
    let uri = request.uri().clone();
    let version = request.version();

    // Get matched route path (extract to owned String to avoid borrow)
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| "<unknown>".to_owned());

    // Get request headers of interest (extract to owned Strings to avoid borrow)
    let headers = request.headers();
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>")
        .to_owned();
    let content_length = headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>")
        .to_owned();
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>")
        .to_owned();

    // Log incoming request at TRACE level (for frequent MCP API calls)
    trace!(
        method = %method,
        uri = %uri,
        route = %route,
        version = ?version,
        content_type = %content_type,
        content_length = %content_length,
        user_agent = %user_agent,
        "HTTP request received"
    );

    let start = std::time::Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed();

    // Get response details
    let status = response.status();
    let response_content_length = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");

    // Log response at TRACE level
    trace!(
        method = %method,
        uri = %uri,
        route = %route,
        status = %status,
        duration_ms = %duration.as_millis(),
        response_content_length = %response_content_length,
        "HTTP response sent"
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::StatusCode, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_http_logging_middleware() {
        // Create a test handler
        async fn test_handler() -> &'static str {
            "OK"
        }

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(axum::middleware::from_fn(http_logging_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_http_logging_with_headers() {
        async fn test_handler() -> &'static str {
            "OK"
        }

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(axum::middleware::from_fn(http_logging_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("content-type", "application/json")
                    .header("user-agent", "test-agent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
