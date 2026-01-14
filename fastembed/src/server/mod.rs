use anyhow::Result;
use axum::{
    Router,
    routing::{get, post},
};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::signal;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::config::AppConfig;
use crate::handlers::{
    embeddings::handle_embed, health::handle_health, models::handle_list_models,
};

/// OpenAPI 文档定义
#[derive(OpenApi)]
#[openapi(
    info(
        title = "FastEmbed API",
        version = "0.1.0",
        description = "基于 FastEmbed 的文本嵌入服务",
        contact(
            name = "API Support",
        )
    ),
    paths(
        crate::handlers::health::handle_health,
        crate::handlers::embeddings::handle_embed,
        crate::handlers::models::handle_list_models,
    ),
    components(
        schemas(
            crate::handlers::health::HealthResponse,
            crate::handlers::embeddings::EmbedRequest,
            crate::handlers::embeddings::EmbedResponse,
            crate::handlers::embeddings::ErrorResponse,
            crate::handlers::models::ModelsResponse,
            crate::models::ModelInfo,
        )
    ),
    tags(
        (name = "健康检查", description = "服务健康状态监控"),
        (name = "文本嵌入", description = "文本向量化接口"),
        (name = "模型管理", description = "模型列表与管理"),
    )
)]
struct ApiDoc;

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub start_time: Instant,
    pub model_cache_ready: Arc<Mutex<bool>>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            start_time: Instant::now(),
            model_cache_ready: Arc::new(Mutex::new(false)),
        }
    }
}

/// 创建路由
pub fn create_router(state: Arc<AppState>) -> Router {
    // CORS 中间件
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Body 限制：20MB
    let body_limit = RequestBodyLimitLayer::new(20 * 1024 * 1024);

    // 创建 Swagger UI（无状态路由）
    let swagger = SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi());

    // 创建 API 路由（有状态）
    Router::new()
        .merge(swagger)
        .route("/health", get(handle_health))
        .route("/api/embeddings", post(handle_embed))
        .route("/api/models/available", get(handle_list_models))
        .layer(cors)
        .layer(body_limit)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// 启动服务器
pub async fn start_server(config: AppConfig) -> Result<()> {
    let host = config.server.host.clone();
    let port = config.server.port;
    let addr = format!("{}:{}", host, port);

    let state = Arc::new(AppState::new(config.clone()));

    // 预热模型（异步执行）
    let warmup_state = state.clone();
    let warmup_config = config.clone();
    tokio::spawn(async move {
        if let Err(e) = warmup_model(warmup_state, warmup_config).await {
            tracing::warn!("模型预热失败: {}", e);
        }
    });

    let app = create_router(state);

    tracing::info!("FastEmbed 服务启动中...");
    tracing::info!("监听地址: {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("✅ FastEmbed 服务已启动: http://{}", addr);
    tracing::info!("健康检查: http://{}/health", addr);
    tracing::info!("文本嵌入: POST http://{}/api/embeddings", addr);
    tracing::info!("可用模型: GET http://{}/api/models/available", addr);
    tracing::info!("📚 Swagger UI: http://{}/swagger-ui/", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("✅ FastEmbed 服务已优雅关闭");

    Ok(())
}

/// 模型预热
async fn warmup_model(state: Arc<AppState>, config: AppConfig) -> Result<()> {
    use crate::models::{get_or_init_model, parse_model};

    tracing::info!("开始预热模型: {}", config.fastembed.default_model);
    let start = Instant::now();

    let model = parse_model(&config.fastembed.default_model)?;
    let model_arc = get_or_init_model(
        model,
        Some(config.fastembed.cache_dir.clone()),
        None, // 使用模型默认的 max_length
    )?;

    // 执行一次微型嵌入
    let warmup_text = vec!["passage: warmup".to_string()];
    let mut model_guard = model_arc.lock().unwrap();
    model_guard.embed(warmup_text, Some(1))?;

    let elapsed = start.elapsed();

    // 标记预热完成
    *state.model_cache_ready.lock().unwrap() = true;

    tracing::info!("✅ 模型预热完成，耗时: {:?}", elapsed);

    Ok(())
}

/// 优雅关闭信号
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("无法安装 Ctrl+C 信号处理器");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("无法安装 SIGTERM 信号处理器")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("收到 Ctrl+C 信号，开始优雅关闭...");
        },
        _ = terminate => {
            tracing::info!("收到 SIGTERM 信号，开始优雅关闭...");
        },
    }
}
