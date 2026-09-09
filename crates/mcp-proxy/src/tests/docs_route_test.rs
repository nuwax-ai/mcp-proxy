//! OpenAPI 文档路由测试：Swagger UI 与 Scalar 双风格并存、spec 完整性。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::AppConfig;
use crate::model::AppState;
use crate::server::get_router;

async fn build_router() -> axum::Router {
    let config = AppConfig::load_config().expect("加载默认配置失败");
    let state = AppState::new(config).await;
    get_router(state).await.expect("构建路由失败")
}

#[tokio::test]
async fn scalar_docs_page_returns_html_with_embedded_spec() {
    let app = build_router().await;
    let response = app
        .oneshot(
            Request::get("/api/docs/scalar")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/html"),
        "Scalar 页面应为 HTML，实际 content-type: {content_type}"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    // utoipa-scalar 将完整 OpenAPI spec 内嵌进 HTML
    assert!(
        html.contains("\"openapi\""),
        "Scalar HTML 应内嵌 OpenAPI spec"
    );
    assert!(html.contains("MCP Proxy API"));
}

#[tokio::test]
async fn swagger_openapi_json_covers_all_documented_paths() {
    let app = build_router().await;
    let response = app
        .oneshot(
            Request::get("/api/docs/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let paths = spec["paths"].as_object().expect("paths 应为对象");
    for expected in [
        "/health",
        "/ready",
        "/mcp/sse/add",
        "/mcp/config/delete/{mcp_id}",
        "/mcp/check/status/{mcp_id}",
        "/mcp/sse/check_status",
        "/mcp/stream/check_status",
        "/api/run_code_with_log",
    ] {
        assert!(paths.contains_key(expected), "文档缺少路径 {expected}");
    }
}

#[tokio::test]
async fn swagger_ui_still_served_alongside_scalar() {
    let app = build_router().await;
    let response = app
        .oneshot(Request::get("/api/docs/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
