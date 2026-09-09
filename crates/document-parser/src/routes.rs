use crate::ApiDoc;
use crate::app_state::AppState;
use crate::config::get_global_file_size_config;
use crate::handlers::{
    document_handler, health_handler, markdown_handler, private_oss_handler, task_handler,
    toc_handler,
};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
};
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// 创建应用路由
pub fn create_routes(state: AppState) -> Router {
    Router::new()
        // 健康检查路由
        .route("/health", get(health_handler::health_check))
        .route("/ready", get(health_handler::ready_check))
        // OpenAPI 文档路由 - 使用 utoipa-swagger-ui 内置支持
        .merge(SwaggerUi::new("/api/docs").url("/api/docs/openapi.json", ApiDoc::openapi()))
        // 文档处理路由
        .nest(
            "/api/v1/documents",
            document_routes(
                state
                    .config
                    .document_parser
                    .sync_parse_max_file_size
                    .bytes() as usize,
            ),
        )
        // 任务管理路由
        .nest("/api/v1/tasks", task_routes())
        // OSS 服务路由
        .nest("/api/v1/oss", oss_routes())
        // 添加中间件
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(CorsLayer::permissive())
                .layer(DefaultBodyLimit::max(
                    get_global_file_size_config().max_file_size.bytes() as usize,
                ))
                // handler panic 转 500 响应而非硬重置连接（纵深防御——配合
                // 生产代码零 unwrap/expect 的规范兜底）
                .layer(tower_http::catch_panic::CatchPanicLayer::new()),
        )
        .with_state(state)
}

/// 文档处理相关路由
///
/// `sync_parse_max_file_size`：同步解析接口（/parse-sync）专属请求体大小上限，
/// 由路由级 [`DefaultBodyLimit`] 承载，覆盖全局限制（middleware 统一管理文件大小）。
fn document_routes(sync_parse_max_file_size: usize) -> Router<AppState> {
    // 同步解析接口单独挂载路由级 body 限制：仅该路由生效，不影响其他文档路由
    let parse_sync_router = Router::new()
        .route("/parse-sync", post(document_handler::parse_document_sync))
        .route_layer(DefaultBodyLimit::max(sync_parse_max_file_size));

    Router::new()
        // 文档上传和解析
        .route("/upload", post(document_handler::upload_document))
        .route(
            "/uploadFromUrl",
            post(document_handler::download_document_from_url),
        )
        // 同步解析（仅供测试验证，同一请求内同步返回 Markdown）
        .merge(parse_sync_router)
        // 结构化文档生成
        .route(
            "/structured",
            post(document_handler::generate_structured_document),
        )
        // Markdown处理接口
        .route(
            "/markdown/parse",
            post(markdown_handler::parse_markdown_sections),
        )
        .route(
            "/markdown/sections",
            post(markdown_handler::parse_markdown_sections),
        )
        // 解析器管理和状态
        .route("/formats", get(document_handler::get_supported_formats))
        .route("/parser/stats", get(document_handler::get_parser_stats))
        .route("/parser/health", get(document_handler::check_parser_health))
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
        // 任务操作
        .route("/{task_id}/cancel", post(task_handler::cancel_task))
        .route("/{task_id}/retry", post(task_handler::retry_task))
        .route("/{task_id}/progress", get(task_handler::get_task_progress))
        // 批量操作
        .route("/batch", post(task_handler::batch_operation_tasks))
        // 任务统计和管理
        .route("/stats", get(task_handler::get_task_stats))
        .route("/cleanup", post(task_handler::cleanup_expired_tasks))
        // Markdown 结果
        .route(
            "/{task_id}/markdown/download",
            get(markdown_handler::download_markdown),
        )
        .route(
            "/{task_id}/markdown/url",
            get(markdown_handler::get_markdown_url),
        )
        // 目录和章节
        .route("/{task_id}/toc", get(toc_handler::get_document_toc))
        .route(
            "/{task_id}/section/{section_id}",
            get(toc_handler::get_section_content),
        )
        .route("/{task_id}/sections", get(toc_handler::get_all_sections))
}

/// 私有桶OSS服务相关路由
fn oss_routes() -> Router<AppState> {
    Router::new()
        // 文件上传到OSS
        .route("/upload", post(private_oss_handler::upload_file_to_oss))
        // 获取上传签名URL（4小时有效）
        .route(
            "/upload-sign-url",
            get(private_oss_handler::get_upload_sign_url),
        )
        // 获取下载签名URL（4小时有效）
        .route(
            "/download-sign-url",
            get(private_oss_handler::get_download_sign_url),
        )
        // 删除OSS文件
        .route("/delete", get(private_oss_handler::delete_file_from_oss))
}
