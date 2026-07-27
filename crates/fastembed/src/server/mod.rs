use anyhow::Result;
use axum::{
    Router,
    routing::{get, post},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::signal;
use tokio::sync::Semaphore;
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
            crate::handlers::embeddings::SparseEmbeddingDto,
            crate::handlers::embeddings::ErrorResponse,
            crate::handlers::models::ModelsResponse,
            crate::models::ModelInfo,
            crate::models::EmbeddingType,
        )
    ),
    tags(
        (name = "健康检查", description = "服务健康状态监控"),
        (name = "文本嵌入", description = "文本向量化接口"),
        (name = "模型管理", description = "模型列表与管理"),
    )
)]
struct ApiDoc;

/// Application state
#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub start_time: Instant,
    pub model_cache_ready: Arc<AtomicBool>,
    /// Limits concurrent embedding requests to avoid thread pool exhaustion
    pub embed_semaphore: Arc<Semaphore>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        // Default: 2x CPU cores, min 4, max 64
        let max_concurrent = std::env::var("FASTEMBED_MAX_CONCURRENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| {
                let cpus = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4);
                (cpus * 2).clamp(4, 64)
            });
        Self {
            config,
            start_time: Instant::now(),
            model_cache_ready: Arc::new(AtomicBool::new(false)),
            embed_semaphore: Arc::new(Semaphore::new(max_concurrent)),
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

    // 如果配置了 model_url，启动前先从 URL 下载模型包
    if let Some(ref url) = config.fastembed.model_url {
        let cache_dir = std::path::PathBuf::from(&config.fastembed.cache_dir);
        tracing::info!("配置了 model_url，启动前先拉取模型包...");
        match crate::models::download_model_from_url(url, &cache_dir).await {
            Ok(()) => {
                tracing::info!("模型包拉取成功，继续启动...");
            }
            Err(e) => {
                // 下载失败不阻止启动：可能缓存已存在，或可 fallback 到 HF
                tracing::warn!(
                    "模型包下载失败: {:?}，将继续启动（可能从 HuggingFace 按需下载或使用已有缓存）",
                    e
                );
            }
        }
    }

    let state = Arc::new(AppState::new(config.clone()));

    // 预热模型：init + 推理均同步阻塞，放 spawn_blocking 避免占用 async worker
    let warmup_state = state.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = warmup_model(warmup_state, &config) {
            tracing::warn!("Model warm-up failed: {:?}", e);
        }
    });

    let app = create_router(state);

    tracing::info!("FastEmbed service is starting...");
    tracing::info!("Listening address: {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("✅ FastEmbed service has been started: http://{}", addr);
    tracing::info!("Health check: http://{}/health", addr);
    tracing::info!("Text embedding: POST http://{}/api/embeddings", addr);
    tracing::info!("Available models: GET http://{}/api/models/available", addr);
    tracing::info!("📚 Swagger UI: http://{}/swagger-ui/", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("✅ FastEmbed service has been gracefully closed");

    Ok(())
}

/// Warm up models: init ONNX sessions for all configured model types.
/// Text model gets a test inference; image/sparse only load (no inference).
fn warmup_model(state: Arc<AppState>, config: &AppConfig) -> Result<()> {
    use crate::models::{EmbeddingType, get_or_init_model};

    let models: [(EmbeddingType, &str); 3] = [
        (EmbeddingType::Text, &config.fastembed.default_model),
        (EmbeddingType::Image, &config.fastembed.default_image_model),
        (
            EmbeddingType::Sparse,
            &config.fastembed.default_sparse_model,
        ),
    ];

    let start = Instant::now();

    for (model_type, model_name) in &models {
        tracing::info!("warming up {:?}: {}", model_type, model_name);
        match get_or_init_model(
            *model_type,
            model_name,
            Some(config.fastembed.cache_dir.clone()),
            None,
            config.fastembed.device,
            config.fastembed.pool_size,
            true,
        ) {
            Ok((pool, _)) => {
                // run a test inference for text models only
                if *model_type == EmbeddingType::Text {
                    let warmup_text = vec!["passage: warmup".to_string()];
                    let instance = pool.pick();
                    let mut guard = instance
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Err(e) = guard.embed(warmup_text, Some(1)) {
                        tracing::warn!(
                            "{:?} warmup inference failed (init ok): {:?}",
                            model_type,
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!("{:?} warmup failed: {:?}", model_type, e);
            }
        }
    }

    state.model_cache_ready.store(true, Ordering::Release);
    tracing::info!("model warmup completed, total time: {:?}", start.elapsed());

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
            tracing::info!("Receive Ctrl+C signal and start graceful shutdown...");
        },
        _ = terminate => {
            tracing::info!("Receive SIGTERM signal and start graceful shutdown...");
        },
    }
}
