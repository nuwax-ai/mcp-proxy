//! 同步解析接口（仅供测试验证）
//!
//! 上传文档并在同一请求内同步返回 Markdown 结果，不创建任务。
//! 可选通过 Query 参数指定自定义上传后端（`upload_*`）：此时解析产物中的图片
//! 会上传到该后端并把 Markdown 内路径替换为远程 URL（Markdown 本身不上传，
//! 内容直接在响应体返回）。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::upload::{UploadConfig, process_multipart_upload_streaming_with_task_id};
use crate::app_state::AppState;
use crate::error::AppError;
use crate::handlers::response::ApiResponse;
use crate::models::{DocumentFormat, ParserEngine};
use axum::{
    extract::{Multipart, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Serialize;
use tracing::{error, info, warn};
use utoipa::ToSchema;

/// 同步解析文档响应
///
/// 同步解析接口（`/parse-sync`）的响应结构，返回解析得到的 Markdown 内容及元信息。
/// 不创建任务。未携带 `upload_*` 参数时：不进行任何上传，markdown 中的图片路径为
/// 解析时的本地临时路径，请求结束后随临时文件一并清理（仅供内容验证）。
/// 携带 `upload_*` 参数时：图片上传到自定义后端，markdown 内图片路径为后端
/// 返回的远程 URL（store 类型为永久地址），响应自包含、请求结束后依然可用。
/// 生产/大文件场景请使用异步任务接口 `POST /api/v1/documents/upload`。
#[derive(Debug, Serialize, ToSchema)]
pub struct SyncParseResponse {
    /// 解析得到的 Markdown 内容
    pub markdown_content: String,
    /// 文档格式（自动检测）
    pub format: DocumentFormat,
    /// 实际使用的解析引擎（MinerU / MarkItDown）
    pub engine: ParserEngine,
    /// 解析耗时（毫秒）
    pub processing_time_ms: u64,
    /// 字数统计
    pub word_count: Option<usize>,
    /// 原始文件名
    pub filename: String,
    /// 文件大小（字节）
    pub file_size: u64,
}

/// 临时文件清理守卫（RAII）
///
/// 注册需要清理的临时文件或目录路径，在守卫被 Drop 时统一清理（同步删除）。
/// 用于同步解析接口，确保成功、失败、超时等所有路径下都不会泄漏临时文件。
/// 目录按目录整体递归删除（MinerU 输出的图片目录），文件按单文件删除。
pub(crate) struct TempCleanupGuard {
    paths: Vec<PathBuf>,
}

impl TempCleanupGuard {
    /// 创建空的清理守卫
    pub(crate) fn new() -> Self {
        Self { paths: Vec::new() }
    }

    /// 注册一个需要清理的路径（文件或目录）
    pub(crate) fn register(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    /// 注册解析结果的中间产物目录（output_dir/work_dir）
    ///
    /// 把"注册必须先于任何可失败的后续步骤（如图片上传）"的不变量收进
    /// 一个原子调用——调用方无法只消费 ParseResult 而忘记注册。
    pub(crate) fn register_parse_result(&mut self, result: &crate::models::ParseResult) {
        if let Some(output_dir) = &result.output_dir {
            self.register(PathBuf::from(output_dir));
        }
        if let Some(work_dir) = &result.work_dir {
            self.register(PathBuf::from(work_dir));
        }
    }
}

impl Drop for TempCleanupGuard {
    fn drop(&mut self) {
        let paths = std::mem::take(&mut self.paths);
        if paths.is_empty() {
            return;
        }

        let cleanup = move || {
            for path in &paths {
                // 幂等清理：路径不存在时跳过，避免无意义的失败日志
                if !path.exists() {
                    continue;
                }
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
                if let Err(e) = result {
                    warn!("清理临时文件失败: {} - {}", path.display(), e);
                }
            }
        };

        // 优先在 blocking 线程池执行同步文件删除，避免阻塞 tokio async 线程；
        // 不在 tokio runtime 中（如进程关闭阶段）时同步兜底执行，保证清理不丢。
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // fire-and-forget：提交到 blocking 线程池即返回，任务独立运行无需 await JoinHandle
                handle.spawn_blocking(cleanup);
            }
            Err(_) => cleanup(),
        }
    }
}

/// 同步解析文档（仅供测试验证）
///
/// 上传文档文件，在同一请求内同步返回解析得到的 Markdown 内容。
/// 与异步任务接口（`/upload`）不同，本接口不创建任务。
///
/// 可选 Query 参数（`upload_base_url` / `upload_path` / `upload_api_key` / `upload_type`）：
/// 出现任意一个即把解析产物中的图片上传到自定义后端（nuwax 风格），并把
/// Markdown 内的图片路径替换为远程 URL——此时响应的 Markdown 自包含、请求后
/// 依然可用；不传则保持原行为（图片为本地临时路径，请求结束后清理）。
///
/// ⚠️ **仅供测试验证使用**：MinerU 解析为重型资源操作，本接口通过信号量限流
/// （默认并发 2），且整体超时默认 10 分钟（超时后底层解析子进程可能仍在后台
/// 运行直至自然结束；携带 `upload_*` 参数时图片上传耗时也计入该超时）；
/// 请求体大小由路由级 `DefaultBodyLimit` 限制
/// （`document_parser.sync_parse_max_file_size`，默认 500MB，超限返回 413）；
/// 生产/大文件场景请使用异步任务接口 `POST /api/v1/documents/upload`。
#[utoipa::path(
    post,
    path = "/api/v1/documents/parse-sync",
    request_body(
        content = String,
        description = "文档文件（multipart/form-data，文件字段名为 file）",
        content_type = "multipart/form-data"
    ),
    params(
        ("upload_base_url" = Option<String>, Query, description = "自定义上传后端基地址；出现任意 upload_* 参数即上传解析产物图片到该后端"),
        ("upload_path" = Option<String>, Query, description = "自定义上传接口路径，默认 /api/v1/file/upload"),
        ("upload_api_key" = Option<String>, Query, description = "自定义上传后端 API Key（Bearer）"),
        ("upload_type" = Option<String>, Query, description = "上传存储类型：store（默认，永久）或 tmp（临时）")
    ),
    responses(
        (status = 200, description = "解析成功，返回 Markdown 内容（仅供测试验证）", body = SyncParseResponse),
        (status = 400, description = "请求参数错误 / 文件格式不支持 / upload_* 参数无法解析出上传目标"),
        (status = 413, description = "请求体超过同步接口大小上限（document_parser.sync_parse_max_file_size）"),
        (status = 408, description = "解析超时（默认 10 分钟）"),
        (status = 500, description = "服务器内部错误（含解析失败）")
    ),
    tag = "documents"
)]
pub async fn parse_document_sync(
    State(state): State<AppState>,
    Query(query): Query<crate::services::UploadTargetParams>,
    mut multipart: Multipart,
) -> axum::response::Response {
    let sync_timeout_secs = state.config.document_parser.sync_parse_timeout_secs as u64;
    let sync_timeout = Duration::from_secs(sync_timeout_secs);

    info!("Synchronous document parsing request starts");

    // 0. 解析自定义上传后端（Fail Fast：早于信号量排队与超时包裹）
    let upload_endpoint =
        match crate::services::resolve_upload_target(&query, &state.config.storage.custom_upload) {
            Ok(endpoint) => endpoint,
            Err(e) => {
                error!("Upload target resolution failed: {}", e);
                return ApiResponse::from_app_error::<SyncParseResponse>(e).into_response();
            }
        };

    // 并发限流：MinerU 为重资源，超过并发上限时等待
    let _permit = match state.sync_parse_semaphore.acquire().await {
        Ok(permit) => permit,
        Err(e) => {
            error!("Failed to acquire sync parse semaphore: {}", e);
            return ApiResponse::from_app_error::<SyncParseResponse>(AppError::internal_error(
                "获取同步解析并发信号量失败",
            ))
            .into_response();
        }
    };

    // 记录起始时间（不含信号量排队等待时间），用于统计解析耗时
    let start_time = Instant::now();

    // 整体超时包裹：上传 + 解析 + 构造响应，超时后内部 future 被取消，
    // TempCleanupGuard 的 Drop 仍会执行，临时文件不会泄漏
    let result = tokio::time::timeout(sync_timeout, async {
        // 1. multipart 流式上传（文件大小由路由级 DefaultBodyLimit 限制，
        //    此处仅做流式写入与格式检测）
        let upload_config = UploadConfig::with_global_config();
        let task_id = uuid::Uuid::new_v4().to_string();
        let (file_path, filename, file_size, _detected_format) =
            process_multipart_upload_streaming_with_task_id(
                &mut multipart,
                &upload_config,
                &task_id,
            )
            .await?;

        // 2. 注册临时文件清理（成功/失败/超时均会清理）
        let mut cleanup_guard = TempCleanupGuard::new();
        cleanup_guard.register(PathBuf::from(&file_path));
        // 注意：parser 内部使用独立的 task_id（UUID v7），与此处的上传 task_id（v4）
        // 不同，无法预注册 temp/<engine>/<task_id>/ 路径。成功路径依赖 ParseResult
        // 返回的 work_dir/output_dir 精确注册（见步骤 4）；超时路径的中间产物为
        // 已知限制（parser 子进程可能继续运行至自然结束，残留由运维定期清理）。

        // 3. 同步解析（内部自带大小校验与解析超时）
        let mut parse_result = state
            .document_service
            .parse_document_local(&file_path)
            .await
            .map_err(AppError::from)?;

        // 4. 解析成功即注册中间产物目录（MinerU 输出的图片/工作目录）。
        //    必须先于步骤 4.1 的图片上传：上传失败或整体超时在 await 点取消
        //    future 时，ParseResult 会被 drop，之后再无路径可注册 → 目录泄漏
        cleanup_guard.register_parse_result(&parse_result);

        // 4.1 自定义上传后端：上传产物图片并替换 Markdown 内路径
        //     （Markdown 本身不上传，内容直接在响应体返回）
        if let Some(ref endpoint) = upload_endpoint {
            parse_result = state
                .document_service
                .upload_images_for_custom_endpoint(endpoint, parse_result)
                .await
                .map_err(AppError::from)?;
        }

        // 5. 构造响应
        Ok::<SyncParseResponse, AppError>(SyncParseResponse {
            markdown_content: parse_result.markdown_content,
            format: parse_result.format,
            engine: parse_result.engine,
            processing_time_ms: start_time.elapsed().as_millis() as u64,
            word_count: parse_result.word_count,
            filename,
            file_size,
        })
    })
    .await;

    match result {
        Ok(Ok(response)) => {
            info!(
                "Synchronous document parsing completed, time: {}ms",
                response.processing_time_ms
            );
            ApiResponse::success_with_status(response, StatusCode::OK).into_response()
        }
        Ok(Err(e)) => {
            error!("Synchronous document parsing failed: {}", e);
            ApiResponse::from_app_error::<SyncParseResponse>(e).into_response()
        }
        Err(_) => {
            error!(
                "Synchronous document parsing timeout ({}s)",
                sync_timeout_secs
            );
            ApiResponse::error_with_status::<SyncParseResponse>(
                "PARSE_TIMEOUT".to_string(),
                format!(
                    "文档解析超时（超过 {sync_timeout_secs}s），请检查文件内容或改用异步任务接口 /api/v1/documents/upload"
                ),
                StatusCode::REQUEST_TIMEOUT,
            )
            .into_response()
        }
    }
}
