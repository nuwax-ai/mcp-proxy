#![recursion_limit = "256"]

use utoipa::OpenApi;

pub mod app_state;
pub mod config;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod parsers;
pub mod performance;
pub mod processors;
pub mod production;
pub mod routes;
pub mod services;
pub mod utils;

#[cfg(test)]
mod tests;

pub use app_state::AppState;
pub use config::AppConfig;
pub use error::AppError;
pub use models::*;

/// 应用版本
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 应用名称
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// 应用描述
pub const APP_DESCRIPTION: &str =
    "多格式文档解析服务 - 支持PDF、Word、Excel、PowerPoint等格式转换为结构化Markdown";

/// 默认配置
pub fn get_default_config() -> AppConfig {
    serde_yaml::from_str(include_str!("../config.yml")).expect("默认配置应该有效")
}

/// OpenAPI 文档配置
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Document Parser API",
        version = "1.0.0",
        description = "多格式文档解析服务 - 支持PDF、Word、Excel、PowerPoint等格式转换为结构化Markdown",
        contact(
            name = "Document Parser Team",
            email = "support@example.com"
        ),
        license(
            name = "MIT",
            url = "https://opensource.org/licenses/MIT"
        )
    ),
    paths(
        // 文档处理接口
        handlers::document_handler::upload_document,
        handlers::document_handler::download_document_from_url,
        handlers::document_handler::generate_structured_document,
        handlers::document_handler::get_supported_formats,
        handlers::document_handler::get_parser_stats,
        handlers::document_handler::check_parser_health,

        // 任务管理接口
        handlers::task_handler::create_task,
        handlers::task_handler::get_task,
        handlers::task_handler::list_tasks,
        handlers::task_handler::cancel_task,
        handlers::task_handler::delete_task,
        handlers::task_handler::batch_operation_tasks,
        handlers::task_handler::retry_task,
        handlers::task_handler::get_task_stats,
        handlers::task_handler::cleanup_expired_tasks,
        handlers::task_handler::get_task_progress,
        handlers::task_handler::get_task_result,

        // Markdown处理接口
        handlers::markdown_handler::parse_markdown_sections,
        handlers::markdown_handler::download_markdown,
        handlers::markdown_handler::get_markdown_url,

        // 私有桶的OSS服务接口
        handlers::private_oss_handler::upload_file_to_oss,
        handlers::private_oss_handler::get_upload_sign_url,
        handlers::private_oss_handler::get_download_sign_url,
        handlers::private_oss_handler::delete_file_from_oss,

        // 健康检查接口
        handlers::health_handler::health_check,
        handlers::health_handler::ready_check,

        // TOC接口
        handlers::toc_handler::get_document_toc,
        handlers::toc_handler::get_section_content,
        handlers::toc_handler::get_all_sections,

    ),
    components(
        schemas(
            // 基础模型
            models::HttpResult<String>,
            models::HttpResult<serde_json::Value>,
            models::TaskStatus,
            models::ProcessingStage,
            models::DocumentTask,
            models::StructuredDocument,
            // models::StructuredSection, // 临时移除以避免递归问题
            models::DocumentFormat,
            models::ParserEngine,
            models::TestPostMineruRequest,
            models::TestPostMineruResponse,
            // models::TocItem, // 临时移除以避免递归问题
            models::DocumentStructure,
            models::DocumentStatistics,
            models::OssData,
            models::ImageInfo,

            // 文档处理相关
            handlers::document_handler::DocumentParseResponse,
            handlers::document_handler::StructuredDocumentResponse,
            handlers::document_handler::SupportedFormatsResponse,
            handlers::document_handler::ParserStatsResponse,


            // 任务管理相关
            handlers::task_handler::CreateTaskRequest,
            handlers::task_handler::TaskQueryParams,
            handlers::task_handler::BatchOperationRequest,
            handlers::task_handler::BatchOperation,
            handlers::task_handler::CancelTaskRequest,
            handlers::task_handler::TaskResponse,
            handlers::task_handler::TaskListResponse,
            handlers::task_handler::TaskStatsResponse,
            handlers::task_handler::TaskResultSummaryResponse,
            handlers::task_handler::TaskFileInfo,
            handlers::task_handler::TaskOssInfo,
            handlers::task_handler::TaskProcessingStats,

            // Markdown处理相关
            handlers::markdown_handler::MarkdownProcessRequest,
            handlers::markdown_handler::SectionsSyncResponse,

            // OSS服务相关
            handlers::private_oss_handler::FileUploadResponse,
            handlers::private_oss_handler::DownloadUrlResponse,
            handlers::private_oss_handler::GetDownloadUrlParams,
            handlers::private_oss_handler::GetUploadSignUrlParams,
            handlers::private_oss_handler::GetDownloadSignUrlParams,
            handlers::private_oss_handler::UploadSignUrlResponse,
            handlers::private_oss_handler::DownloadSignUrlResponse,

            // 响应类型
            // handlers::response::PaginatedResponse<models::DocumentTask>, // 移除以避免循环引用
            handlers::response::PaginationInfo,
            handlers::response::MessageResponse,
            handlers::response::StatsResponse,
            handlers::response::HealthResponse,
            handlers::response::ServiceHealth,
            handlers::response::UploadResponse,
            handlers::response::FileInfo,
            handlers::response::DownloadResponse,
            handlers::response::UrlInfo,
            handlers::response::TaskOperationResponse,
            handlers::response::BatchOperationResponse,
            handlers::response::BatchError,

            // 服务类型
            crate::services::TaskStats,

            // TOC和章节相关
            handlers::toc_handler::TocResponse,
            handlers::toc_handler::SectionResponse,
            handlers::toc_handler::SectionsResponse
        )
    ),
    tags(
        (name = "documents", description = "文档处理相关接口"),
        (name = "tasks", description = "任务管理相关接口"),
        (name = "markdown", description = "Markdown处理相关接口"),
        (name = "oss", description = "OSS服务相关接口"),
        (name = "health", description = "健康检查相关接口"),
        (name = "toc", description = "目录和章节相关接口"),
        (name = "test", description = "测试相关接口"),

    )
)]
pub struct ApiDoc;

/// 获取OpenAPI规范JSON
pub fn get_openapi_spec() -> String {
    ApiDoc::openapi().to_pretty_json().unwrap()
}
