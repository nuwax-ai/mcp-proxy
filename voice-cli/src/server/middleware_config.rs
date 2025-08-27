use axum::Router;
use axum::middleware::from_fn;
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::server::http_tracing::basic_tracing_middleware;
use crate::server::middleware::request_logging_middleware;

/// 与 mcp-proxy 风格一致的统一挂载接口
/// 建议路由构建完成后统一调用该函数挂载层
pub fn set_layer<T>(
    app: Router,
    _state: T,
    max_file_size: usize,
    cors_enabled: bool,
) -> Router
where
    T: Clone + Send + Sync + 'static,
{
    let app = app.layer(RequestBodyLimitLayer::new(max_file_size));

    let app = if cors_enabled {
        use tower_http::cors::CorsLayer;
        app.layer(CorsLayer::permissive())
    } else {
        app
    };

    app.layer(
        ServiceBuilder::new()
            .layer(from_fn(basic_tracing_middleware))
            .layer(from_fn(request_logging_middleware))
            .layer(TraceLayer::new_for_http()),
    )
}
