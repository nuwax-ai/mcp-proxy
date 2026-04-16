use crate::app_state::AppState;
use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::handlers::response::{ApiResponse, FileInfo, UploadResponse};
use crate::handlers::validation::{FileNameSanitizer, RequestValidator};
use crate::models::{DocumentFormat, SourceType, StructuredDocument};
use crate::processors::MarkdownProcessorConfig;
use crate::utils::file_utils::get_file_extension;
use axum::{
    Json,
    extract::{Multipart, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
}

/// 上传配置
#[derive(Debug, Clone)]
pub struct UploadConfig {
    pub max_file_size: u64,
    pub allowed_extensions: Vec<String>,
    // temp_dir removed - now uses current directory approach
    pub chunk_size: usize,
    pub max_concurrent_uploads: usize,
    pub upload_timeout_secs: u64,
}

impl UploadConfig {
    /// 使用全局配置创建上传配置
    pub fn with_global_config() -> Self {
        Self {
            max_file_size: GlobalFileSizeConfig::default().max_file_size.bytes(),
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
}

/// OSS文档解析请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct ParseOssDocumentRequest {
    pub oss_path: String,
    pub format: DocumentFormat,
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
}

/// 生成结构化文档请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct GenerateStructuredDocumentRequest {
    pub markdown_content: String,
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
    pub enable_anchors: Option<bool>,
}

/// 文档解析响应
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentParseResponse {
    pub task_id: String,
    pub message: String,
}

/// 结构化文档响应
#[derive(Debug, Serialize, ToSchema)]
pub struct StructuredDocumentResponse {
    pub document: StructuredDocument,
}

/// 支持格式响应
#[derive(Debug, Serialize, ToSchema)]
pub struct SupportedFormatsResponse {
    pub formats: Vec<DocumentFormat>,
}

/// 解析器统计响应
#[derive(Debug, Serialize, ToSchema)]
pub struct ParserStatsResponse {
    pub stats: HashMap<String, serde_json::Value>,
}

/// 处理器缓存统计响应
#[derive(Debug, Serialize, ToSchema)]
pub struct ProcessorCacheStatsResponse {
    pub cache_stats: HashMap<String, serde_json::Value>,
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
        ("bucket_dir" = Option<String>, Query, description = "上传到OSS时的子目录，将附加在系统预设路径之后")
    ),
    responses(
        (status = 202, description = "文档上传成功，解析任务已启动", body = UploadResponse),
        (status = 400, description = "请求参数错误"),
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

    // 1. 验证请求参数
    if let Err(e) = validate_upload_request(&params) {
        error!("Upload request parameter verification failed: {}", e);
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    let upload_config = UploadConfig::with_global_config();
    let max_size = upload_config.max_file_size;

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
        process_multipart_upload_streaming_with_task_id(
            &mut multipart,
            &upload_config,
            max_size,
            &task_id,
        ),
    )
    .await;

    let (file_path, original_filename, file_size, detected_format) = match upload_result {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            error!("File upload processing failed: {}", e);
            return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
        }
        Err(_) => {
            error!("File upload timeout");
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
        let _ = cleanup_temp_file(&file_path).await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    // 6. 验证TOC配置
    let (_enable_toc, _max_toc_depth) =
        match RequestValidator::validate_toc_config(params.enable_toc, params.max_toc_depth) {
            Ok(config) => config,
            Err(e) => {
                error!("TOC configuration verification failed: {}", e);
                let _ = cleanup_temp_file(&file_path).await;
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
        let _ = cleanup_temp_file(&file_path).await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    // 8.1 保存 bucket_dir 到任务（如果提供）
    if let Some(ref dir) = params.bucket_dir {
        if let Err(e) = state
            .task_service
            .set_task_bucket_dir(&task_id, Some(dir.clone()))
            .await
        {
            warn!("Failed to save bucket_dir: {}", e);
        }
    }

    // 9. 更新任务的文件信息
    let mime_type = detect_mime_type_from_format(&document_format);
    if let Err(e) = state
        .task_service
        .set_task_file_info(&task_id, Some(file_size), Some(mime_type))
        .await
    {
        error!("Failed to update task file information: {}", e);
        let _ = cleanup_temp_file(&file_path).await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }

    // 10. 入队由 worker 池处理
    if let Err(e) = state.task_queue.enqueue_task(task_id.clone(), 1).await {
        error!("Failed to join the team: {}", e);
        let _ = cleanup_temp_file(&file_path).await;
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
    max_size: u64,
) -> Result<(String, String, u64, DocumentFormat), AppError> {
    process_multipart_upload_streaming_with_task_id(
        multipart,
        config,
        max_size,
        &uuid::Uuid::new_v4().to_string(),
    )
    .await
}

/// 处理multipart文件上传（带task_id）
async fn process_multipart_upload_streaming_with_task_id(
    multipart: &mut Multipart,
    config: &UploadConfig,
    max_size: u64,
    task_id: &str,
) -> Result<(String, String, u64, DocumentFormat), AppError> {
    let mut file_count = 0;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Validation(format!("解析multipart数据失败: {e}")))?
    {
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
                max_size,
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

/// 流式写入文件（带验证和进度监控）
async fn stream_write_file_with_validation(
    mut field: axum::extract::multipart::Field<'_>,
    file_path: &str,
    max_size: u64,
    chunk_size: usize,
    expected_extension: &str,
) -> Result<(u64, DocumentFormat), AppError> {
    let file = File::create(file_path)
        .await
        .map_err(|e| AppError::File(format!("创建文件失败: {e}")))?;

    let mut writer = BufWriter::with_capacity(chunk_size, file);
    let mut total_size = 0u64;
    let mut first_chunk: Option<Vec<u8>> = None;
    let mut chunk_count = 0u64;

    // 创建进度监控
    let progress_interval = std::cmp::max(1, max_size / 100); // 每1%报告一次进度
    let mut next_progress_report = progress_interval;

    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::File(format!("读取文件块失败: {e}")))?
    {
        chunk_count += 1;
        let chunk_len = chunk.len() as u64;
        total_size += chunk_len;

        // 检查文件大小限制
        if total_size > max_size {
            // 清理已创建的文件
            let _ = tokio::fs::remove_file(file_path).await;
            return Err(AppError::Validation(format!(
                "文件大小超过限制: {total_size} > {max_size} 字节"
            )));
        }

        // 保存第一个块用于格式检测
        if first_chunk.is_none() && !chunk.is_empty() {
            first_chunk = Some(chunk.to_vec());
        }

        // 写入文件
        writer.write_all(&chunk).await.map_err(|e| {
            // 写入失败时清理文件
            let file_path_owned = file_path.to_string();
            tokio::spawn(async move {
                let _ = tokio::fs::remove_file(file_path_owned).await;
            });
            AppError::File(format!("写入文件失败: {e}"))
        })?;

        // 进度报告
        if total_size >= next_progress_report {
            let progress = (total_size * 100) / max_size;
            info!(
                "File upload progress: {}% ({} / {} bytes)",
                progress, total_size, max_size
            );
            next_progress_report += progress_interval;
        }
    }

    // 确保所有数据都写入磁盘
    writer
        .flush()
        .await
        .map_err(|e| AppError::File(format!("刷新文件缓冲区失败: {e}")))?;

    // 验证最小文件大小
    if total_size == 0 {
        let _ = tokio::fs::remove_file(file_path).await;
        return Err(AppError::Validation("文件为空".to_string()));
    }

    if total_size < 10 {
        let _ = tokio::fs::remove_file(file_path).await;
        return Err(AppError::Validation("文件过小，可能已损坏".to_string()));
    }

    // 检测文档格式
    let detected_format =
        detect_document_format_enhanced(file_path, first_chunk.as_deref(), expected_extension)?;

    info!(
        "File upload completed: {} bytes, {} blocks, format: {:?}",
        total_size, chunk_count, detected_format
    );

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

/// 增强的文档格式检测
fn detect_document_format_enhanced(
    file_path: &str,
    first_chunk: Option<&[u8]>,
    expected_extension: &str,
) -> Result<DocumentFormat, AppError> {
    // 1. 通过文件扩展名检测
    let extension_format = if let Some(extension) = get_file_extension(file_path) {
        DocumentFormat::from_extension(&extension)
    } else {
        DocumentFormat::from_extension(expected_extension)
    };

    // 2. 通过文件内容检测（魔数）
    let content_format = if let Some(chunk) = first_chunk {
        detect_format_by_magic_number_enhanced(chunk).unwrap_or(extension_format.clone())
    } else {
        extension_format.clone()
    };

    // 3. 验证格式一致性
    if !formats_compatible(&extension_format, &content_format) {
        warn!(
            "File extension and content format mismatch: {:?} vs {:?}",
            extension_format, content_format
        );

        // 如果内容检测更可靠，使用内容格式
        if is_reliable_magic_number_detection(&content_format) {
            return Ok(content_format);
        }
    }

    // 4. 返回最终格式
    Ok(extension_format)
}

/// 检查两种格式是否兼容
fn formats_compatible(format1: &DocumentFormat, format2: &DocumentFormat) -> bool {
    match (format1, format2) {
        (DocumentFormat::Text, DocumentFormat::Txt)
        | (DocumentFormat::Txt, DocumentFormat::Text)
        | (DocumentFormat::Text, DocumentFormat::Md)
        | (DocumentFormat::Md, DocumentFormat::Text) => true,
        (a, b) => a == b,
    }
}

/// 检查是否为可靠的魔数检测
fn is_reliable_magic_number_detection(format: &DocumentFormat) -> bool {
    matches!(
        format,
        DocumentFormat::PDF | DocumentFormat::Image | DocumentFormat::Audio
    )
}

/// 增强的魔数检测文件格式
fn detect_format_by_magic_number_enhanced(data: &[u8]) -> Result<DocumentFormat, AppError> {
    if data.len() < 4 {
        return Err(AppError::Validation("文件数据不足以检测格式".to_string()));
    }

    // PDF: %PDF
    if data.starts_with(b"%PDF") {
        return Ok(DocumentFormat::PDF);
    }

    // ZIP-based formats: PK\x03\x04 或 PK\x05\x06 或 PK\x07\x08
    if data.len() >= 4 && data.starts_with(b"PK") {
        // 进一步检测ZIP内容类型
        return detect_zip_based_format(data);
    }

    // 图片格式
    if let Ok(format) = detect_image_format(data) {
        return Ok(format);
    }

    // 音频格式
    if let Ok(format) = detect_audio_format(data) {
        return Ok(format);
    }

    // HTML/XML格式
    if let Ok(format) = detect_text_format(data) {
        return Ok(format);
    }

    Err(AppError::Validation("无法通过文件内容检测格式".to_string()))
}

/// 检测ZIP格式的具体类型
fn detect_zip_based_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    // 这里可以通过读取ZIP文件的目录结构来判断具体格式
    // 简化实现，返回Word格式作为默认
    if data.len() >= 30 {
        // 检查是否包含Office文档的特征
        let data_str = String::from_utf8_lossy(&data[0..std::cmp::min(512, data.len())]);
        if data_str.contains("word/") {
            return Ok(DocumentFormat::Word);
        } else if data_str.contains("xl/") {
            return Ok(DocumentFormat::Excel);
        } else if data_str.contains("ppt/") {
            return Ok(DocumentFormat::PowerPoint);
        }
    }

    // 默认返回Word格式
    Ok(DocumentFormat::Word)
}

/// 检测图片格式
fn detect_image_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    if data.len() < 8 {
        return Err(AppError::Validation("数据不足".to_string()));
    }

    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(DocumentFormat::Image);
    }

    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok(DocumentFormat::Image);
    }

    // GIF: GIF87a 或 GIF89a
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Ok(DocumentFormat::Image);
    }

    // BMP: BM
    if data.starts_with(b"BM") {
        return Ok(DocumentFormat::Image);
    }

    // TIFF: II*\0 或 MM\0*
    if data.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
        return Ok(DocumentFormat::Image);
    }

    Err(AppError::Validation("不是图片格式".to_string()))
}

/// 检测音频格式
fn detect_audio_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    if data.len() < 4 {
        return Err(AppError::Validation("数据不足".to_string()));
    }

    // MP3: ID3 或 FF FB/FF F3/FF F2
    if data.starts_with(b"ID3") || (data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0)
    {
        return Ok(DocumentFormat::Audio);
    }

    // WAV: RIFF....WAVE
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WAVE" {
        return Ok(DocumentFormat::Audio);
    }

    // M4A/AAC: ftyp
    if data.len() >= 8 && &data[4..8] == b"ftyp" {
        return Ok(DocumentFormat::Audio);
    }

    Err(AppError::Validation("不是音频格式".to_string()))
}

/// 检测文本格式
fn detect_text_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    let data_str = String::from_utf8_lossy(data).to_lowercase();

    // HTML
    if data_str.contains("<html") || data_str.contains("<!doctype html") {
        return Ok(DocumentFormat::HTML);
    }

    // XML
    if data_str.starts_with("<?xml") {
        return Ok(DocumentFormat::HTML); // 将XML归类为HTML处理
    }

    // Markdown (简单检测)
    if data_str.contains("# ") || data_str.contains("## ") || data_str.contains("```") {
        return Ok(DocumentFormat::Md);
    }

    // 默认文本
    Ok(DocumentFormat::Text)
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

    // 验证URL格式（但不改变编码状态）
    if let Err(e) = RequestValidator::validate_url_format(&request.url) {
        error!("URL verification failed: {}", e);
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }

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

    // 如果提供了 bucket_dir，保存到任务
    if let Some(ref dir) = request.bucket_dir {
        if let Err(e) = state
            .task_service
            .set_task_bucket_dir(&task.id, Some(dir.clone()))
            .await
        {
            warn!("Failed to save bucket_dir: {}", e);
        }
    }

    // 入队由 worker 池处理
    if let Err(e) = state.task_queue.enqueue_task(task.id.clone(), 1).await {
        error!("URL task enqueue failed: {}", e);
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }

    info!("URL document download task has been started: {}", task.id);

    let response = DocumentParseResponse {
        task_id: task.id,
        message: format!("URL文档下载任务已启动: {original_url}"),
    };

    ApiResponse::success_with_status(response, StatusCode::ACCEPTED).into_response()
}

/// 生成结构化文档处理器
#[utoipa::path(
    post,
    path = "/api/v1/documents/structured",
    request_body = GenerateStructuredDocumentRequest,
    responses(
        (status = 200, description = "结构化文档生成成功", body = StructuredDocumentResponse),
        (status = 400, description = "请求参数错误")
    ),
    tag = "documents"
)]
pub async fn generate_structured_document(
    State(state): State<AppState>,
    Json(request): Json<GenerateStructuredDocumentRequest>,
) -> impl axum::response::IntoResponse {
    info!("Generate structured document request starts");

    // 验证Markdown内容
    if let Err(e) = RequestValidator::validate_markdown_content(&request.markdown_content) {
        return ApiResponse::from_app_error::<StructuredDocumentResponse>(e).into_response();
    }

    // 验证TOC配置
    let (_enable_toc, _max_toc_depth) =
        match RequestValidator::validate_toc_config(request.enable_toc, request.max_toc_depth) {
            Ok(config) => config,
            Err(e) => {
                return ApiResponse::from_app_error::<StructuredDocumentResponse>(e)
                    .into_response();
            }
        };

    // 使用全局配置的 Markdown 处理器（无需在此处创建配置）

    // 直接处理Markdown内容
    match state
        .document_service
        .generate_structured_document_simple(&request.markdown_content)
        .await
    {
        Ok(document) => {
            info!("Structured document generated successfully");

            let response = StructuredDocumentResponse { document };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("Structured document generation failed: {}", e);
            ApiResponse::from_app_error::<StructuredDocumentResponse>(e.into()).into_response()
        }
    }
}

/// 获取支持的文档格式
#[utoipa::path(
    get,
    path = "/api/v1/documents/formats",
    responses(
        (status = 200, description = "支持的文档格式列表", body = SupportedFormatsResponse)
    ),
    tag = "documents"
)]
pub async fn get_supported_formats(
    State(_state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let formats = vec![
        DocumentFormat::PDF,
        DocumentFormat::Word,
        DocumentFormat::Excel,
        DocumentFormat::PowerPoint,
        DocumentFormat::Image,
        DocumentFormat::Audio,
        DocumentFormat::HTML,
        DocumentFormat::Text,
        DocumentFormat::Txt,
        DocumentFormat::Md,
    ];

    let response = SupportedFormatsResponse { formats };
    ApiResponse::success(response).into_response()
}

/// 获取解析器统计信息
#[utoipa::path(
    get,
    path = "/api/v1/documents/parser/stats",
    responses(
        (status = 200, description = "解析器统计信息", body = ParserStatsResponse)
    ),
    tag = "documents"
)]
pub async fn get_parser_stats(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let stats_data = state.document_service.get_parser_stats();
    let mut stats = HashMap::new();
    stats.insert(
        "mineru_name".to_string(),
        serde_json::Value::String(stats_data.mineru_name),
    );
    stats.insert(
        "mineru_description".to_string(),
        serde_json::Value::String(stats_data.mineru_description),
    );
    stats.insert(
        "markitdown_name".to_string(),
        serde_json::Value::String(stats_data.markitdown_name),
    );
    stats.insert(
        "markitdown_description".to_string(),
        serde_json::Value::String(stats_data.markitdown_description),
    );
    stats.insert(
        "supported_formats".to_string(),
        serde_json::to_value(stats_data.supported_formats).unwrap_or_default(),
    );

    let response = ParserStatsResponse { stats };
    ApiResponse::success(response).into_response()
}

/// 检查解析器健康状态
#[utoipa::path(
    get,
    path = "/api/v1/documents/parser/health",
    responses(
        (status = 200, description = "解析器健康状态"),
        (status = 500, description = "解析器不健康")
    ),
    tag = "documents"
)]
pub async fn check_parser_health(State(state): State<AppState>) -> impl IntoResponse {
    match state.document_service.check_parser_health().await {
        Ok(health_status) => ApiResponse::success(health_status).into_response(),
        Err(e) => {
            error!("Failed to check parser health status: {}", e);
            ApiResponse::from_app_error::<HashMap<String, bool>>(e.into()).into_response()
        }
    }
}

/// 清理处理器缓存
#[utoipa::path(
    delete,
    path = "/api/v1/documents/processor/cache",
    responses(
        (status = 200, description = "处理器缓存已清空")
    ),
    tag = "documents"
)]
pub async fn clear_processor_cache(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    match state.document_service.clear_processor_cache().await {
        Ok(_) => ApiResponse::message("处理器缓存已清空".to_string()).into_response(),
        Err(e) => ApiResponse::from_app_error::<String>(e.into()).into_response(),
    }
}

/// 获取处理器缓存统计
#[utoipa::path(
    get,
    path = "/api/v1/documents/processor/cache/stats",
    responses(
        (status = 200, description = "处理器缓存统计信息", body = ProcessorCacheStatsResponse)
    ),
    tag = "documents"
)]
pub async fn get_processor_cache_stats(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let cache_statistics = state.document_service.get_processor_cache_stats().await;
    let mut cache_stats = std::collections::HashMap::new();
    cache_stats.insert(
        "total_entries".to_string(),
        serde_json::Value::Number(serde_json::Number::from(cache_statistics.total_entries)),
    );
    cache_stats.insert(
        "expired_entries".to_string(),
        serde_json::Value::Number(serde_json::Number::from(cache_statistics.expired_entries)),
    );

    let response = ProcessorCacheStatsResponse { cache_stats };
    ApiResponse::success(response).into_response()
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
