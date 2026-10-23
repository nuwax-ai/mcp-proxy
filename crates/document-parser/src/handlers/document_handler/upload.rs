//! 文档上传与 URL 下载解析接口

use super::detection::detect_document_format_enhanced;
use crate::app_state::AppState;
use crate::error::AppError;
use crate::handlers::response::{ApiResponse, FileInfo, UploadResponse};
use crate::handlers::validation::{FileNameSanitizer, RequestValidator};
use crate::models::{DocumentFormat, SourceType};
use crate::processors::MarkdownProcessorConfig;
use axum::{
    Json,
    extract::{Multipart, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::{error, info, warn};
use utoipa::ToSchema;

/// 文档上传请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadDocumentRequest {
    /// 是否启用目录生成，默认为false
    #[serde(default)]
    #[schema(example = true)]
    pub enable_toc: Option<bool>,
    /// 目录最大深度，默认为6
    #[serde(default)]
    #[schema(example = 3, minimum = 1, maximum = 10)]
    pub max_toc_depth: Option<usize>,
    /// 可选：指定上传到OSS时的子目录（将作为系统预定义路径下的子目录）
    /// 例如：processed_markdown/<bucket_dir>/... 或 parsed_images/<bucket_dir>/...
    #[serde(default)]
    #[schema(example = "projectA/docs/v1")]
    pub bucket_dir: Option<String>,
    /// 可选：自定义上传后端基地址（如 https://agent.example.com）。
    /// 出现任意 upload_* 参数即启用自定义上传后端（产物不再走 OSS）
    #[serde(default)]
    #[schema(example = "https://agent.example.com")]
    pub upload_base_url: Option<String>,
    /// 可选：自定义上传接口路径，默认 /api/v1/file/upload
    #[serde(default)]
    #[schema(example = "/api/v1/file/upload")]
    pub upload_path: Option<String>,
    /// 可选：自定义上传后端 API Key（Bearer）
    #[serde(default)]
    pub upload_api_key: Option<String>,
    /// 可选：上传存储类型，store（默认，永久）或 tmp（临时）
    #[serde(default)]
    #[schema(value_type = String, example = "store")]
    pub upload_type: Option<oss_client::CustomUploadType>,
}

impl From<&UploadDocumentRequest> for crate::services::UploadTargetParams {
    fn from(params: &UploadDocumentRequest) -> Self {
        Self {
            upload_base_url: params.upload_base_url.clone(),
            upload_path: params.upload_path.clone(),
            upload_api_key: params.upload_api_key.clone(),
            upload_type: params.upload_type,
        }
    }
}

/// 上传配置
#[derive(Debug, Clone)]
pub struct UploadConfig {
    pub allowed_extensions: Vec<String>,
    // temp_dir removed - now uses current directory approach
    pub chunk_size: usize,
    pub max_concurrent_uploads: usize,
    pub upload_timeout_secs: u64,
}

impl UploadConfig {
    /// 使用全局配置创建上传配置
    ///
    /// 注意：文件大小限制由 HTTP middleware（`DefaultBodyLimit`）统一管理，
    /// 此处不再配置大小上限。
    pub fn with_global_config() -> Self {
        Self {
            allowed_extensions: vec![
                "pdf".to_string(),
                "docx".to_string(),
                "doc".to_string(),
                "txt".to_string(),
                "md".to_string(),
                "html".to_string(),
                "htm".to_string(),
                "rtf".to_string(),
                "odt".to_string(),
                "xlsx".to_string(),
                "xls".to_string(),
                "csv".to_string(),
                "pptx".to_string(),
                "ppt".to_string(),
                "odp".to_string(),
                "jpg".to_string(),
                "jpeg".to_string(),
                "png".to_string(),
                "gif".to_string(),
                "bmp".to_string(),
                "tiff".to_string(),
                "mp3".to_string(),
                "wav".to_string(),
                "m4a".to_string(),
                "aac".to_string(),
            ],
            // temp_dir removed - now uses current directory approach
            chunk_size: 64 * 1024, // 64KB chunks for better performance
            max_concurrent_uploads: 10,
            upload_timeout_secs: 300, // 5 minutes
        }
    }
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self::with_global_config()
    }
}

/// URL下载文档请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct DownloadDocumentRequest {
    /// 要下载的文档URL地址
    #[schema(example = "https://example.com/document.pdf")]
    pub url: String,
    /// 是否启用目录生成，默认为false
    #[serde(default)]
    #[schema(example = true)]
    pub enable_toc: Option<bool>,
    /// 目录最大深度，默认为6
    #[serde(default)]
    #[schema(example = 3, minimum = 1, maximum = 10)]
    pub max_toc_depth: Option<usize>,
    /// 可选：指定上传到OSS时的子目录（将作为系统预定义路径下的子目录）
    #[serde(default)]
    #[schema(example = "projectA/docs/v1")]
    pub bucket_dir: Option<String>,
    /// 可选：自定义上传后端基地址（如 https://agent.example.com）。
    /// 出现任意 upload_* 字段即启用自定义上传后端（产物不再走 OSS）
    #[serde(default)]
    #[schema(example = "https://agent.example.com")]
    pub upload_base_url: Option<String>,
    /// 可选：自定义上传接口路径，默认 /api/v1/file/upload
    #[serde(default)]
    #[schema(example = "/api/v1/file/upload")]
    pub upload_path: Option<String>,
    /// 可选：自定义上传后端 API Key（Bearer）
    #[serde(default)]
    pub upload_api_key: Option<String>,
    /// 可选：上传存储类型，store（默认，永久）或 tmp（临时）
    #[serde(default)]
    #[schema(value_type = String, example = "store")]
    pub upload_type: Option<oss_client::CustomUploadType>,
}

impl From<&DownloadDocumentRequest> for crate::services::UploadTargetParams {
    fn from(params: &DownloadDocumentRequest) -> Self {
        Self {
            upload_base_url: params.upload_base_url.clone(),
            upload_path: params.upload_path.clone(),
            upload_api_key: params.upload_api_key.clone(),
            upload_type: params.upload_type,
        }
    }
}

/// OSS文档解析请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct ParseOssDocumentRequest {
    pub oss_path: String,
    pub format: DocumentFormat,
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
}

/// 文档解析响应
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentParseResponse {
    pub task_id: String,
    pub message: String,
}

/// 上传文档处理器
/// 上传文档并启动解析任务
///
/// 支持多种文档格式的上传，包括自动格式检测和验证
/// 返回任务ID用于后续查询解析状态
#[utoipa::path(
    post,
    path = "/api/v1/documents/upload",
    request_body(content = String, description = "Multipart form data with file", content_type = "multipart/form-data"),
    params(
        ("enable_toc" = Option<bool>, Query, description = "是否启用目录生成"),
        ("max_toc_depth" = Option<usize>, Query, description = "目录最大深度"),
        ("bucket_dir" = Option<String>, Query, description = "上传到OSS时的子目录，将附加在系统预设路径之后"),
        ("upload_base_url" = Option<String>, Query, description = "自定义上传后端基地址；出现任意 upload_* 参数即启用自定义上传后端"),
        ("upload_path" = Option<String>, Query, description = "自定义上传接口路径，默认 /api/v1/file/upload"),
        ("upload_api_key" = Option<String>, Query, description = "自定义上传后端 API Key（Bearer）"),
        ("upload_type" = Option<String>, Query, description = "上传存储类型：store（默认，永久）或 tmp（临时）")
    ),
    responses(
        (status = 202, description = "文档上传成功，解析任务已启动", body = UploadResponse),
        (status = 400, description = "请求参数错误（含 upload_* 参数无法解析出上传目标）"),
        (status = 413, description = "文件过大"),
        (status = 415, description = "不支持的文件格式"),
        (status = 408, description = "上传超时")
    ),
    tag = "documents"
)]
pub async fn upload_document(
    State(state): State<AppState>,
    Query(params): Query<UploadDocumentRequest>,
    mut multipart: Multipart,
) -> impl axum::response::IntoResponse {
    info!("Document upload request starts: {:?}", params);

    // 0. 解析自定义上传后端（Fail Fast：任一 upload_* 字段出现但 base_url 不可解析即 400，
    //    早于任务创建与 multipart 读取，不产生任何资源）
    let upload_endpoint = match crate::services::resolve_upload_target(
        &(&params).into(),
        &state.config.storage.custom_upload,
    ) {
        Ok(endpoint) => endpoint,
        Err(e) => {
            error!("Upload target resolution failed: {}", e);
            return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
        }
    };

    // 1. 验证请求参数
    if let Err(e) = validate_upload_request(&params) {
        error!("Upload request parameter verification failed: {}", e);
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    let upload_config = UploadConfig::with_global_config();

    // 2. 先创建任务以获取 task_id
    let task = match state
        .task_service
        .create_task(
            SourceType::Upload,
            None, // source_path 稍后设置
            None, // original_filename 稍后设置
            None, // 临时设置，稍后更新
        )
        .await
    {
        Ok(task) => task,
        Err(e) => {
            error!("Task creation failed: {}", e);
            return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
        }
    };

    let task_id = task.id.clone();

    // 3. 处理文件上传，使用 task_id 创建基于任务的文件路径
    let upload_timeout = std::time::Duration::from_secs(upload_config.upload_timeout_secs);
    let upload_result = tokio::time::timeout(
        upload_timeout,
        process_multipart_upload_streaming_with_task_id(&mut multipart, &upload_config, &task_id),
    )
    .await;

    let (file_path, original_filename, file_size, detected_format) = match upload_result {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            error!("File upload processing failed: {}", e);
            // 任务已创建，中止避免僵尸 Pending（临时文件由流式写入内部的
            // scopeguard 兜底清理，无需传路径）
            abort_task_and_cleanup(&state, &task_id, None, format!("文件上传处理失败: {e}")).await;
            return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
        }
        Err(_) => {
            error!("File upload timeout");
            abort_task_and_cleanup(&state, &task_id, None, "文件上传超时".to_string()).await;
            return ApiResponse::error_with_status::<UploadResponse>(
                "UPLOAD_TIMEOUT".to_string(),
                "文件上传超时".to_string(),
                StatusCode::REQUEST_TIMEOUT,
            )
            .into_response();
        }
    };

    // 4. 确定最终文档格式（统一采用自动检测结果）
    let document_format = detected_format.clone();

    // 5. 验证格式兼容性
    if let Err(e) = RequestValidator::validate_document_format(&document_format) {
        error!("Document format verification failed: {}", e);
        abort_task_and_cleanup(
            &state,
            &task_id,
            Some(&file_path),
            format!("文档格式校验失败: {e}"),
        )
        .await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    // 6. 验证TOC配置
    let (_enable_toc, _max_toc_depth) =
        match RequestValidator::validate_toc_config(params.enable_toc, params.max_toc_depth) {
            Ok(config) => config,
            Err(e) => {
                error!("TOC configuration verification failed: {}", e);
                abort_task_and_cleanup(
                    &state,
                    &task_id,
                    Some(&file_path),
                    format!("TOC 配置校验失败: {e}"),
                )
                .await;
                return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
            }
        };

    // 7. 创建处理器配置
    let _processor_config = MarkdownProcessorConfig::with_global_config();

    // 8. 更新任务信息
    if let Err(e) = state
        .task_service
        .update_task(
            &task_id,
            Some(file_path.clone()),
            Some(original_filename.clone()),
            document_format.clone(),
        )
        .await
    {
        error!("Failed to update task information: {}", e);
        abort_task_and_cleanup(
            &state,
            &task_id,
            Some(&file_path),
            format!("更新任务信息失败: {e}"),
        )
        .await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    // 8.1 保存 bucket_dir 到任务（如果提供）。保存失败直接中止：bucket_dir
    // 决定产物落点，静默回退默认目录会产生与请求不符的产物位置（与 8.2
    // 的 Fail Fast 语义对齐）
    if let Some(ref dir) = params.bucket_dir
        && let Err(e) = state
            .task_service
            .set_task_bucket_dir(&task_id, Some(dir.clone()))
            .await
    {
        let _ =
            abort_task_and_cleanup(&state, &task_id, None, format!("保存 bucket_dir 失败: {e}"))
                .await;
        return ApiResponse::internal_error::<DocumentParseResponse>(&format!(
            "保存 bucket_dir 失败: {e}"
        ))
        .into_response();
    }

    // 8.2 保存自定义上传端点到任务（入队前；worker 只读任务，必须先落盘）。
    // 保存失败直接中止：后端选择决定产物去向，静默回退 OSS 会产生错误语义（Fail Fast）
    if let Some(ref endpoint) = upload_endpoint
        && let Err(e) = state
            .task_service
            .set_task_upload_config(&task_id, Some(endpoint.clone()))
            .await
    {
        error!("Failed to save upload_config: {}", e);
        abort_task_and_cleanup(
            &state,
            &task_id,
            Some(&file_path),
            format!("保存自定义上传配置失败: {e}"),
        )
        .await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    // 9. 更新任务的文件信息
    let mime_type = detect_mime_type_from_format(&document_format);
    if let Err(e) = state
        .task_service
        .set_task_file_info(&task_id, Some(file_size), Some(mime_type))
        .await
    {
        error!("Failed to update task file information: {}", e);
        abort_task_and_cleanup(
            &state,
            &task_id,
            Some(&file_path),
            format!("更新任务文件信息失败: {e}"),
        )
        .await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    // 10. 入队由 worker 池处理
    if let Err(e) = state.task_queue.enqueue_task(task_id.clone(), 1).await {
        error!("Failed to join the team: {}", e);
        abort_task_and_cleanup(
            &state,
            &task_id,
            Some(&file_path),
            format!("任务入队失败: {e}"),
        )
        .await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    let sanitized_filename = FileNameSanitizer::sanitize(&original_filename).unwrap_or_else(|_| {
        warn!(
            "Filename sanitization failed, original filename used: {}",
            original_filename
        );
        original_filename.clone()
    });

    let response = UploadResponse {
        task_id: task_id.clone(),
        message: format!(
            "文档 '{sanitized_filename}' 上传成功，解析任务已启动 (任务ID: {task_id})"
        ),
        file_info: FileInfo {
            filename: sanitized_filename,
            size: file_size,
            format: format!("{detected_format:?}"),
            mime_type: detect_mime_type_from_format(&detected_format),
        },
    };

    info!(
        "The document upload is completed and the parsing task has been started in the background: task_id={}",
        task_id
    );
    ApiResponse::success_with_status(response, StatusCode::ACCEPTED).into_response()
}

/// 处理multipart文件上传
#[allow(dead_code)]
async fn process_multipart_upload_streaming(
    multipart: &mut Multipart,
    config: &UploadConfig,
) -> Result<(String, String, u64, DocumentFormat), AppError> {
    process_multipart_upload_streaming_with_task_id(
        multipart,
        config,
        &uuid::Uuid::new_v4().to_string(),
    )
    .await
}

/// 将 multipart 读取错误转换为 AppError
///
/// 请求体超过大小限制（`DefaultBodyLimit`，由 `Limited` body 在读取时产生）时
/// 返回 413 语义的 [`AppError::PayloadTooLarge`]，其余解析错误由 `fallback`
/// 决定具体错误类型。
fn map_multipart_read_error(
    err: axum::extract::multipart::MultipartError,
    fallback: impl FnOnce(axum::extract::multipart::MultipartError) -> AppError,
) -> AppError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::PayloadTooLarge("请求体超过大小限制".to_string())
    } else {
        fallback(err)
    }
}

/// 处理multipart文件上传（带task_id）
///
/// 文件大小限制由 HTTP middleware（`DefaultBodyLimit`）统一管理，此处不再校验大小。
pub(crate) async fn process_multipart_upload_streaming_with_task_id(
    multipart: &mut Multipart,
    config: &UploadConfig,
    task_id: &str,
) -> Result<(String, String, u64, DocumentFormat), AppError> {
    let mut file_count = 0;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        map_multipart_read_error(e, |e| {
            AppError::Validation(format!("解析multipart数据失败: {e}"))
        })
    })? {
        if field.name().is_some() {
            file_count += 1;

            // 限制同时上传的文件数量
            if file_count > 1 {
                return Err(AppError::Validation("只能同时上传一个文件".to_string()));
            }

            let filename = field
                .file_name()
                .ok_or_else(|| AppError::Validation("缺少文件名".to_string()))?
                .to_string();

            // 验证文件名
            let sanitized_filename = FileNameSanitizer::sanitize(&filename)?;

            // 验证文件扩展名
            let extension = RequestValidator::validate_file_extension(
                &sanitized_filename,
                &config.allowed_extensions,
            )?;

            info!(
                "Start processing the uploaded file: {} (after cleaning: {})",
                filename, sanitized_filename
            );

            // 创建基于task_id的临时文件
            let temp_file_path = create_temp_file_for_task("./temp", task_id, &sanitized_filename)?;

            // 流式写入文件（带进度监控）
            let (file_size, detected_format) = stream_write_file_with_validation(
                field,
                &temp_file_path,
                config.chunk_size,
                &extension,
            )
            .await?;

            return Ok((temp_file_path, filename, file_size, detected_format));
        }
    }

    Err(AppError::Validation("未找到文件字段".to_string()))
}

/// 处理multipart文件上传（改进的流式处理）
///
/// 流式写入文件（带验证）。文件大小限制由 HTTP middleware（`DefaultBodyLimit`）
/// 统一管理，此处不再校验大小。
async fn stream_write_file_with_validation(
    mut field: axum::extract::multipart::Field<'_>,
    file_path: &str,
    chunk_size: usize,
    expected_extension: &str,
) -> Result<(u64, DocumentFormat), AppError> {
    let file = File::create(file_path)
        .await
        .map_err(|e| AppError::File(format!("创建文件失败: {e}")))?;

    // 兜底清理：写入中途 future 被取消（超时）时删除部分文件（scopeguard Drop 时执行）
    let cleaner = scopeguard::guard(file_path.to_string(), |path| {
        let _ = std::fs::remove_file(path);
    });
    let mut writer = BufWriter::with_capacity(chunk_size, file);
    let mut total_size = 0u64;
    let mut first_chunk: Option<Vec<u8>> = None;
    let mut chunk_count = 0u64;

    while let Some(chunk) = field.chunk().await.map_err(|e| {
        map_multipart_read_error(e, |e| AppError::File(format!("读取文件块失败: {e}")))
    })? {
        chunk_count += 1;
        let chunk_len = chunk.len() as u64;
        total_size += chunk_len;

        // 保存第一个块用于格式检测
        if first_chunk.is_none() && !chunk.is_empty() {
            first_chunk = Some(chunk.to_vec());
        }

        // 写入文件（失败时由 scopeguard 兜底删除，无需在此手动清理）
        writer
            .write_all(&chunk)
            .await
            .map_err(|e| AppError::File(format!("写入文件失败: {e}")))?;
    }

    // 确保所有数据都写入磁盘
    writer
        .flush()
        .await
        .map_err(|e| AppError::File(format!("刷新文件缓冲区失败: {e}")))?;

    // 验证最小文件大小（不满足时由 scopeguard 兜底删除，无需手动清理）
    if total_size == 0 {
        return Err(AppError::Validation("文件为空".to_string()));
    }

    if total_size < 10 {
        return Err(AppError::Validation("文件过小，可能已损坏".to_string()));
    }

    // 检测文档格式
    let detected_format =
        detect_document_format_enhanced(file_path, first_chunk.as_deref(), expected_extension)?;

    info!(
        "File upload completed: {} bytes, {} blocks, format: {:?}",
        total_size, chunk_count, detected_format
    );

    // 写入完成，取消兜底清理，文件由调用方接管
    std::mem::forget(cleaner);

    Ok((total_size, detected_format))
}

/// 基于 taskId 创建临时文件路径
fn create_temp_file_for_task(
    temp_dir: &str,
    task_id: &str,
    filename: &str,
) -> Result<String, AppError> {
    // 确保临时目录存在
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| AppError::File(format!("创建临时目录失败: {e}")))?;

    // 验证临时目录权限
    let temp_path = Path::new(temp_dir);
    if !temp_path.exists() || !temp_path.is_dir() {
        return Err(AppError::File("临时目录无效".to_string()));
    }

    // 提取文件扩展名
    let extension = Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("tmp");

    // 使用 taskId 作为文件名的一部分，确保唯一性和可追踪性
    let task_filename = format!(
        "task_{}_{}.{}",
        task_id,
        filename
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>(),
        extension
    );
    let file_path = temp_path.join(task_filename);

    // 验证路径安全性（防止路径遍历）
    if !file_path.starts_with(temp_path) {
        return Err(AppError::File("文件路径不安全".to_string()));
    }

    Ok(file_path.to_string_lossy().to_string())
}

/// 清理临时文件
async fn cleanup_temp_file(file_path: &str) {
    if let Err(e) = tokio::fs::remove_file(file_path).await {
        warn!("Failed to clean up temporary files: {} - {}", file_path, e);
    }
}

/// 检查URL下载是否支持该格式
#[allow(dead_code)]
fn is_format_supported_for_url(format: &DocumentFormat) -> bool {
    matches!(
        format,
        DocumentFormat::PDF
            | DocumentFormat::Text
            | DocumentFormat::HTML
            | DocumentFormat::Txt
            | DocumentFormat::Md
    )
}

/// 检查OSS是否支持该格式
#[allow(dead_code)]
fn is_format_supported_for_oss(format: &DocumentFormat) -> bool {
    matches!(
        format,
        DocumentFormat::PDF
            | DocumentFormat::Word
            | DocumentFormat::Excel
            | DocumentFormat::PowerPoint
            | DocumentFormat::Text
            | DocumentFormat::Txt
            | DocumentFormat::Md
            | DocumentFormat::HTML
    )
}

/// 上传文档处理器,通过 URL 自动下载文档解析
/// 上传文档并启动解析任务
///
/// 支持多种文档格式的上传，包括自动格式检测和验证
/// 返回任务ID用于后续查询解析状态
#[utoipa::path(
    post,
    path = "/api/v1/documents/uploadFromUrl",
    request_body = DownloadDocumentRequest,
    responses(
        (status = 202, description = "URL文档下载任务已启动", body = DocumentParseResponse),
        (status = 400, description = "请求参数错误"),
        (status = 500, description = "服务器内部错误")
    ),
    tag = "documents"
)]
pub async fn download_document_from_url(
    State(state): State<AppState>,
    Json(request): Json<DownloadDocumentRequest>,
) -> impl axum::response::IntoResponse {
    info!("URL document download request starts: {:?}", request);

    // 验证URL格式（弱校验——本产品主场景是内网私有部署，用户用内网 IP/本地
    // 地址拉文件是合法流量，不做 SSRF 拦截；task_handler 的 Url 任务用强校验
    // validate_url 是历史行为，如需统一放开另行决策）
    if let Err(e) = RequestValidator::validate_url_format(&request.url) {
        error!("URL verification failed: {}", e);
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }

    // 解析自定义上传后端（Fail Fast：参数矛盾/非法即 400，早于任务创建）
    let upload_endpoint = match crate::services::resolve_upload_target(
        &(&request).into(),
        &state.config.storage.custom_upload,
    ) {
        Ok(endpoint) => endpoint,
        Err(e) => {
            error!("Upload target resolution failed: {}", e);
            return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
        }
    };

    // 使用原始URL，保持编码状态
    let original_url = &request.url;

    // 验证TOC配置
    let (_enable_toc, _max_toc_depth) =
        match RequestValidator::validate_toc_config(request.enable_toc, request.max_toc_depth) {
            Ok(config) => config,
            Err(e) => {
                return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
            }
        };

    // 创建任务
    let task = match state
        .task_service
        .create_task(
            SourceType::Url,
            Some(original_url.to_string()), // 使用原始URL
            None,                           // URL 下载暂时不设置原始文件名
            None,
        )
        .await
    {
        Ok(task) => task,
        Err(e) => {
            error!("Failed to create task: {}", e);
            return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
        }
    };

    // 如果提供了 bucket_dir，保存到任务（保存失败中止——同 upload 侧语义）
    if let Some(ref dir) = request.bucket_dir
        && let Err(e) = state
            .task_service
            .set_task_bucket_dir(&task.id, Some(dir.clone()))
            .await
    {
        let _ =
            abort_task_and_cleanup(&state, &task.id, None, format!("保存 bucket_dir 失败: {e}"))
                .await;
        return ApiResponse::internal_error::<DocumentParseResponse>(&format!(
            "保存 bucket_dir 失败: {e}"
        ))
        .into_response();
    }

    // 保存自定义上传端点到任务（入队前；保存失败直接中止，避免 worker 静默回退 OSS）
    if let Some(ref endpoint) = upload_endpoint
        && let Err(e) = state
            .task_service
            .set_task_upload_config(&task.id, Some(endpoint.clone()))
            .await
    {
        error!("Failed to save upload_config: {}", e);
        abort_task_and_cleanup(
            &state,
            &task.id,
            None, // URL 任务无本地临时文件
            format!("保存自定义上传配置失败: {e}"),
        )
        .await;
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }

    // 入队由 worker 池处理
    if let Err(e) = state.task_queue.enqueue_task(task.id.clone(), 1).await {
        error!("URL task enqueue failed: {}", e);
        abort_task_and_cleanup(&state, &task.id, None, format!("任务入队失败: {e}")).await;
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }

    info!("URL document download task has been started: {}", task.id);

    let response = DocumentParseResponse {
        task_id: task.id,
        message: format!("URL文档下载任务已启动: {original_url}"),
    };

    ApiResponse::success_with_status(response, StatusCode::ACCEPTED).into_response()
}

/// 任务创建后失败路径的统一中止：标记任务 Failed（不消耗重试额度）+ 清理临时文件
///
/// 避免任务停留在不可重试的 Pending 僵尸状态；中止动作自身失败会记录 error
/// 日志（不再静默吞掉），随后调用方仍返回原始错误响应。
async fn abort_task_and_cleanup(
    state: &AppState,
    task_id: &str,
    temp_file_path: Option<&str>,
    message: String,
) {
    if let Some(path) = temp_file_path {
        let _ = cleanup_temp_file(path).await;
    }
    // abort_task 内部已对读/写失败记录 error 日志；此处尽力而为不传播
    let _ = state.task_service.abort_task(task_id, message).await;
}

/// 验证上传请求参数
fn validate_upload_request(params: &UploadDocumentRequest) -> Result<(), AppError> {
    // 验证TOC配置
    RequestValidator::validate_toc_config(params.enable_toc, params.max_toc_depth)?;

    // 验证文档格式（如果指定）
    // 已移除由用户指定格式，统一走自动检测

    Ok(())
}

/// 从URL检测文档格式
#[allow(dead_code)]
fn detect_format_from_url(url: &str) -> Option<DocumentFormat> {
    // 从URL路径中提取文件扩展名
    let path = url.split('?').next().unwrap_or(url); // 移除查询参数
    let extension = path.split('.').next_back()?.to_lowercase();

    match extension.as_str() {
        "pdf" => Some(DocumentFormat::PDF),
        "doc" | "docx" => Some(DocumentFormat::Word),
        "xls" | "xlsx" => Some(DocumentFormat::Excel),
        "ppt" | "pptx" => Some(DocumentFormat::PowerPoint),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" => Some(DocumentFormat::Image),
        "mp3" | "wav" | "m4a" | "aac" => Some(DocumentFormat::Audio),
        "html" | "htm" => Some(DocumentFormat::HTML),
        "txt" => Some(DocumentFormat::Txt),
        "md" | "markdown" => Some(DocumentFormat::Md),
        _ => None,
    }
}

/// 根据文档格式检测MIME类型
fn detect_mime_type_from_format(format: &DocumentFormat) -> String {
    match format {
        DocumentFormat::PDF => "application/pdf".to_string(),
        DocumentFormat::Word => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string()
        }
        DocumentFormat::Excel => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()
        }
        DocumentFormat::PowerPoint => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string()
        }
        DocumentFormat::Image => "image/jpeg".to_string(),
        DocumentFormat::Audio => "audio/mpeg".to_string(),
        DocumentFormat::HTML => "text/html".to_string(),
        DocumentFormat::Text => "text/plain".to_string(),
        DocumentFormat::Txt => "text/plain".to_string(),
        DocumentFormat::Md => "text/markdown".to_string(),
        DocumentFormat::Other(ext) => format!("application/{ext}"),
    }
}
