use axum::{
    routing::{delete, get, post},
    Router,
    extract::DefaultBodyLimit,
};
use tower::ServiceBuilder;
use tower_http::{
    cors::CorsLayer,
    trace::TraceLayer,
};
use crate::app_state::AppState;
use crate::config::get_global_file_size_config;
use crate::handlers::{
    document_handler,
    task_handler,
    health_handler,
    toc_handler,
    markdown_handler,
    oss_handler,
};

/// 创建应用路由
pub fn create_routes(state: AppState) -> Router {
    Router::new()
        // 健康检查路由
        .route("/health", get(health_handler::health_check))
        .route("/ready", get(health_handler::ready_check))
        
        // 文档处理路由
        .nest("/api/v1/documents", document_routes())
        
        // 任务管理路由
        .nest("/api/v1/tasks", task_routes())
        
        // OSS 服务路由
        .nest("/api/v1/oss", oss_routes())
        
        // 添加中间件
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(CorsLayer::permissive())
                .layer(DefaultBodyLimit::max(get_global_file_size_config().max_file_size.bytes() as usize))
        )
        .with_state(state)
}

/// 文档处理相关路由
fn document_routes() -> Router<AppState> {
    Router::new()
        // 文档上传和解析
        .route("/upload", post(document_handler::upload_document))
        .route("/download", post(document_handler::download_document_from_url))
        .route("/oss", post(document_handler::parse_oss_document))
        
        // 结构化文档生成
        .route("/structured", post(document_handler::generate_structured_document))

        // Markdown 结果
        .route("/{task_id}/markdown/download", get(markdown_handler::download_markdown))
        .route("/{task_id}/markdown/url", get(markdown_handler::get_markdown_url))
        
        // 解析器管理
        .route("/formats", get(document_handler::get_supported_formats))
        .route("/parser/stats", get(document_handler::get_parser_stats))
        .route("/parser/health", get(document_handler::check_parser_health))
        
        // 处理器缓存管理
        .route("/processor/cache", delete(document_handler::clear_processor_cache))
        .route("/processor/cache/stats", get(document_handler::get_processor_cache_stats))
        // 同步Markdown结构化接口
        .route("/markdown/sections", post(markdown_handler::parse_markdown_sections))
}

/// 任务管理相关路由
fn task_routes() -> Router<AppState> {
    Router::new()
        // 任务CRUD操作
        .route("/", post(task_handler::create_task))
        .route("/", get(task_handler::list_tasks))
        .route("/{task_id}", get(task_handler::get_task))
        .route("/{task_id}", delete(task_handler::delete_task))
        .route("/{task_id}/result", get(task_handler::get_task_result))
        
        // 任务统计和管理
        .route("/stats", get(task_handler::get_task_stats))
        .route("/cleanup", post(task_handler::cleanup_expired_tasks))
        // 目录和章节
        .route("/{task_id}/toc", get(toc_handler::get_document_toc))
        .route("/{task_id}/section/{section_id}", get(toc_handler::get_section_content))
        .route("/{task_id}/sections", get(toc_handler::get_all_sections))
}

/// OSS服务相关路由
fn oss_routes() -> Router<AppState> {
    Router::new()
        // 文件上传到OSS
        .route("/upload", post(oss_handler::upload_file_to_oss))
        // 根据文件名获取下载链接
        .route("/download-url", get(oss_handler::get_download_url))
        // 获取上传签名URL（4小时有效）
        .route("/upload-sign-url", get(oss_handler::get_upload_sign_url))
        // 获取下载签名URL（4小时有效）
        .route("/download-sign-url", get(oss_handler::get_download_sign_url))
}