//! 同步解析接口（仅供测试验证）
//!
//! 上传文档并在同一请求内同步返回 Markdown 结果，不创建任务、不进行 OSS 上传。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::upload::{UploadConfig, process_multipart_upload_streaming_with_task_id};
use crate::app_state::AppState;
use crate::error::AppError;
use crate::handlers::response::ApiResponse;
use crate::models::{DocumentFormat, ParserEngine};
use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Serialize;
use tracing::{error, info, warn};
use utoipa::ToSchema;

/// 同步解析文档响应
///
/// 同步解析接口（`/parse-sync`）的响应结构，返回解析得到的 Markdown 内容及元信息。
/// **仅供测试验证使用**：不创建任务、不进行 OSS 上传；markdown 中的图片路径为
/// 解析时的本地临时路径，请求结束后随临时文件一并清理（不提供图片访问能力）。
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
/// 与异步任务接口（`/upload`）不同，本接口不创建任务、不进行 OSS 上传。
/// markdown 中的图片路径为解析时的本地临时路径，请求结束后随临时文件
/// 一并清理（本接口不提供图片访问能力，仅适合纯文本/表格内容验证）。
///
/// ⚠️ **仅供测试验证使用**：MinerU 解析为重型资源操作，本接口通过信号量限流
/// （默认并发 2），且整体超时默认 10 分钟（超时后底层解析子进程可能仍在后台
/// 运行直至自然结束）；请求体大小由路由级 `DefaultBodyLimit` 限制
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
    responses(
        (status = 200, description = "解析成功，返回 Markdown 内容（仅供测试验证）", body = SyncParseResponse),
        (status = 400, description = "请求参数错误 / 文件格式不支持"),
        (status = 413, description = "请求体超过同步接口大小上限（document_parser.sync_parse_max_file_size）"),
        (status = 408, description = "解析超时（默认 10 分钟）"),
        (status = 500, description = "服务器内部错误（含解析失败）")
    ),
    tag = "documents"
)]
pub async fn parse_document_sync(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> axum::response::Response {
    let sync_timeout_secs = state.config.document_parser.sync_parse_timeout_secs as u64;
    let sync_timeout = Duration::from_secs(sync_timeout_secs);

    info!("Synchronous document parsing request starts");

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
        // 预注册解析引擎的中间产物目录兜底：parse 过程中 MinerU/MarkItDown 子进程
        // 即会创建 temp/<engine>/<task_id>/，若 parse 超时或失败，下面的精确注册
        //（output_dir/work_dir）来不及执行，此处按固定路径模式预注册，
        // 确保所有路径下中间产物都不泄漏（幂等：不存在则 Drop 时跳过）。
        cleanup_guard.register(PathBuf::from(format!("temp/mineru/{task_id}")));
        cleanup_guard.register(PathBuf::from(format!("temp/markitdown/{task_id}")));

        // 3. 同步解析（内部自带大小校验与解析超时）
        let parse_result = state
            .document_service
            .parse_document_local(&file_path)
            .await
            .map_err(AppError::from)?;

        // 4. 注册解析产生的中间产物目录（MinerU 输出的图片/工作目录）
        if let Some(output_dir) = &parse_result.output_dir {
            cleanup_guard.register(PathBuf::from(output_dir));
        }
        if let Some(work_dir) = &parse_result.work_dir {
            cleanup_guard.register(PathBuf::from(work_dir));
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
