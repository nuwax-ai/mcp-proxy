use axum::{
    extract::{Multipart, Query, State},
    Json,
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_stream::StreamExt;
use crate::app_state::AppState;
use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::models::{
    SourceType, DocumentFormat, StructuredDocument
};
use crate::processors::MarkdownProcessorConfig;
use crate::utils::file_utils::get_file_extension;
use crate::handlers::validation::{RequestValidator, FileNameSanitizer};
use crate::handlers::response::{UploadResponse, FileInfo, ApiResponse};
use tracing::{info, error, warn};

/// 文档上传请求参数
#[derive(Debug, Deserialize)]
pub struct UploadDocumentRequest {
    pub format: Option<DocumentFormat>, // 可选，支持自动检测
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
    pub max_file_size: Option<u64>, // 最大文件大小（字节）
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
                "pdf".to_string(), "docx".to_string(), "doc".to_string(),
                "txt".to_string(), "md".to_string(), "html".to_string(),
                "htm".to_string(), "rtf".to_string(), "odt".to_string(),
                "xlsx".to_string(), "xls".to_string(), "csv".to_string(),
                "pptx".to_string(), "ppt".to_string(), "odp".to_string(),
                "jpg".to_string(), "jpeg".to_string(), "png".to_string(),
                "gif".to_string(), "bmp".to_string(), "tiff".to_string(),
                "mp3".to_string(), "wav".to_string(), "m4a".to_string(),
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
#[derive(Debug, Deserialize)]
pub struct DownloadDocumentRequest {
    pub url: String,
    pub format: DocumentFormat,
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
}

/// OSS文档解析请求参数
#[derive(Debug, Deserialize)]
pub struct ParseOssDocumentRequest {
    pub oss_path: String,
    pub format: DocumentFormat,
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
}

/// 生成结构化文档请求参数
#[derive(Debug, Deserialize)]
pub struct GenerateStructuredDocumentRequest {
    pub markdown_content: String,
    pub enable_toc: Option<bool>,
    pub max_toc_depth: Option<usize>,
    pub enable_anchors: Option<bool>,
}

/// 文档解析响应
#[derive(Debug, Serialize)]
pub struct DocumentParseResponse {
    pub task_id: String,
    pub message: String,
}

/// 结构化文档响应
#[derive(Debug, Serialize)]
pub struct StructuredDocumentResponse {
    pub document: StructuredDocument,
}

/// 支持格式响应
#[derive(Debug, Serialize)]
pub struct SupportedFormatsResponse {
    pub formats: Vec<DocumentFormat>,
}

/// 解析器统计响应
#[derive(Debug, Serialize)]
pub struct ParserStatsResponse {
    pub stats: HashMap<String, serde_json::Value>,
}

/// 处理器缓存统计响应
#[derive(Debug, Serialize)]
pub struct ProcessorCacheStatsResponse {
    pub cache_stats: HashMap<String, serde_json::Value>,
}

/// 上传文档处理器
/// 上传文档并启动解析任务
/// 
/// 支持多种文档格式的上传，包括自动格式检测和验证
/// 返回任务ID用于后续查询解析状态
pub async fn upload_document(
    State(state): State<AppState>,
    Query(params): Query<UploadDocumentRequest>,
    mut multipart: Multipart,
) -> impl axum::response::IntoResponse {
    info!("文档上传请求开始: {:?}", params);
    
    // 1. 验证请求参数
    if let Err(e) = validate_upload_request(&params) {
        error!("上传请求参数验证失败: {}", e);
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }
    
    let upload_config = UploadConfig::with_global_config();
    let max_size = params.max_file_size.unwrap_or(upload_config.max_file_size);
    
    // 2. 处理文件上传（带超时）
    let upload_timeout = std::time::Duration::from_secs(upload_config.upload_timeout_secs);
    let upload_result = tokio::time::timeout(
        upload_timeout,
        process_multipart_upload_streaming(&mut multipart, &upload_config, max_size)
    ).await;
    
    let (file_path, original_filename, file_size, detected_format) = match upload_result {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            error!("文件上传处理失败: {}", e);
            return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
        }
        Err(_) => {
            error!("文件上传超时");
            return ApiResponse::error_with_status::<UploadResponse>(
                "UPLOAD_TIMEOUT".to_string(),
                "文件上传超时".to_string(),
                StatusCode::REQUEST_TIMEOUT,
            ).into_response();
        }
    };
    
    // 3. 确定最终文档格式
    let document_format = params.format.unwrap_or(detected_format.clone());
    
    // 4. 验证格式兼容性
    if let Err(e) = RequestValidator::validate_document_format(&document_format) {
        error!("文档格式验证失败: {}", e);
        let _ = cleanup_temp_file(&file_path).await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }
    
    // 5. 验证TOC配置
    let (enable_toc, max_toc_depth) = match RequestValidator::validate_toc_config(
        params.enable_toc,
        params.max_toc_depth
    ) {
        Ok(config) => config,
        Err(e) => {
            error!("TOC配置验证失败: {}", e);
            let _ = cleanup_temp_file(&file_path).await;
            return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
        }
    };
    
    // 6. 创建处理器配置
    let _processor_config = MarkdownProcessorConfig {
        enable_toc,
        max_toc_depth,
        enable_anchors: true,
        enable_cache: true,
        ..Default::default()
    };
    
    // 7. 创建解析任务
    let task = match state.task_service.create_task(
        SourceType::Upload,
        Some(file_path.clone()),
        document_format.clone(),
    ).await {
        Ok(task) => task,
        Err(e) => {
            error!("任务创建失败: {}", e);
            let _ = cleanup_temp_file(&file_path).await;
            return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
        }
    };
    
    // 8. 更新任务的文件信息
    let mime_type = detect_mime_type_from_format(&document_format);
    if let Err(e) = state.task_service.set_task_file_info(&task.id, Some(file_size), Some(mime_type)).await {
        error!("更新任务文件信息失败: {}", e);
        let _ = cleanup_temp_file(&file_path).await;
        return ApiResponse::from_app_error::<UploadResponse>(e).into_response();
    }
    
    // 9. 异步启动文档解析（不等待完成）
    let document_service = state.document_service.clone();
    let task_id_clone = task.id.clone();
    let file_path_clone = file_path.clone();
    
    // 在后台异步执行解析
    tokio::spawn(async move {
        info!("开始异步解析文档: task_id={}, file={}", task_id_clone, file_path_clone);
        match document_service.parse_document(&task_id_clone, &file_path_clone, document_format).await {
            Ok(_result) => {
                info!("文档解析完成: task_id={}", task_id_clone);
            }
            Err(e) => {
                error!("文档解析失败: task_id={}, error={}", task_id_clone, e);
            }
        }
    });
    
    let sanitized_filename = FileNameSanitizer::sanitize(&original_filename)
        .unwrap_or_else(|_| {
            warn!("文件名清理失败，使用原始文件名: {}", original_filename);
            original_filename.clone()
        });
    
    let response = UploadResponse {
        task_id: task.id.clone(),
        message: format!("文档 '{}' 上传成功，解析任务已启动 (任务ID: {})", sanitized_filename, task.id),
        file_info: FileInfo {
            filename: sanitized_filename,
            size: file_size,
            format: format!("{:?}", detected_format),
            mime_type: detect_mime_type_from_format(&detected_format),
        },
    };
    
    info!("文档上传完成，解析任务已在后台启动: task_id={}", task.id);
    ApiResponse::success_with_status(response, StatusCode::ACCEPTED).into_response()
}

/// 处理multipart文件上传（改进的流式处理）
async fn process_multipart_upload_streaming(
    multipart: &mut Multipart,
    config: &UploadConfig,
    max_size: u64,
) -> Result<(String, String, u64, DocumentFormat), AppError> {
    let mut file_count = 0;
    
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        AppError::Validation(format!("解析multipart数据失败: {}", e))
    })? {
        if let Some(name) = field.name() {
            if name == "file" {
                file_count += 1;
                
                // 限制同时上传的文件数量
                if file_count > 1 {
                    return Err(AppError::Validation("只能同时上传一个文件".to_string()));
                }
                
                let filename = field.file_name()
                    .ok_or_else(|| AppError::Validation("缺少文件名".to_string()))?
                    .to_string();
                
                // 验证文件名
                let sanitized_filename = FileNameSanitizer::sanitize(&filename)?;
                
                // 验证文件扩展名
                let extension = RequestValidator::validate_file_extension(
                    &sanitized_filename, 
                    &config.allowed_extensions
                )?;
                
                info!("开始处理上传文件: {} (清理后: {})", filename, sanitized_filename);
                
                // 创建临时文件
                let temp_file_path = create_temp_file_secure("./temp", &sanitized_filename)?;
                
                // 流式写入文件（带进度监控）
                let (file_size, detected_format) = stream_write_file_with_validation(
                    field, 
                    &temp_file_path, 
                    max_size, 
                    config.chunk_size,
                    &extension
                ).await?;
                
                return Ok((temp_file_path, filename, file_size, detected_format));
            }
        }
    }
    
    Err(AppError::Validation("未找到文件字段".to_string()))
}

/// 流式写入文件（带验证和进度监控）
async fn stream_write_file_with_validation(
    mut field: axum::extract::multipart::Field<'_>,
    file_path: &str,
    max_size: u64,
    chunk_size: usize,
    expected_extension: &str,
) -> Result<(u64, DocumentFormat), AppError> {
    let file = File::create(file_path).await
        .map_err(|e| AppError::File(format!("创建文件失败: {}", e)))?;
    
    let mut writer = BufWriter::with_capacity(chunk_size, file);
    let mut total_size = 0u64;
    let mut first_chunk: Option<Vec<u8>> = None;
    let mut chunk_count = 0u64;
    
    // 创建进度监控
    let progress_interval = std::cmp::max(1, max_size / 100); // 每1%报告一次进度
    let mut next_progress_report = progress_interval;
    
    while let Some(chunk) = field.chunk().await.map_err(|e| {
        AppError::File(format!("读取文件块失败: {}", e))
    })? {
        chunk_count += 1;
        let chunk_len = chunk.len() as u64;
        total_size += chunk_len;
        
        // 检查文件大小限制
        if total_size > max_size {
            // 清理已创建的文件
            let _ = tokio::fs::remove_file(file_path).await;
            return Err(AppError::Validation(
                format!("文件大小超过限制: {} > {} 字节", total_size, max_size)
            ));
        }
        
        // 验证chunk大小（防止恶意大chunk）
        if chunk_len > chunk_size as u64 * 10 {
            let _ = tokio::fs::remove_file(file_path).await;
            return Err(AppError::Validation(
                format!("文件块过大: {} 字节", chunk_len)
            ));
        }
        
        // 保存第一个块用于格式检测
        if first_chunk.is_none() && !chunk.is_empty() {
            first_chunk = Some(chunk.to_vec());
        }
        
        // 写入文件
        writer.write_all(&chunk).await
            .map_err(|e| {
                // 写入失败时清理文件
                let file_path_owned = file_path.to_string();
                tokio::spawn(async move {
                    let _ = tokio::fs::remove_file(file_path_owned).await;
                });
                AppError::File(format!("写入文件失败: {}", e))
            })?;
        
        // 进度报告
        if total_size >= next_progress_report {
            let progress = (total_size * 100) / max_size;
            info!("文件上传进度: {}% ({} / {} 字节)", progress, total_size, max_size);
            next_progress_report += progress_interval;
        }
    }
    
    // 确保所有数据都写入磁盘
    writer.flush().await
        .map_err(|e| AppError::File(format!("刷新文件缓冲区失败: {}", e)))?;
    
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
    let detected_format = detect_document_format_enhanced(
        file_path, 
        first_chunk.as_deref(), 
        expected_extension
    )?;
    
    info!("文件上传完成: {} 字节, {} 个块, 格式: {:?}", 
          total_size, chunk_count, detected_format);
    
    Ok((total_size, detected_format))
}

/// 创建安全的临时文件
fn create_temp_file_secure(temp_dir: &str, filename: &str) -> Result<String, AppError> {
    // 确保临时目录存在
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| AppError::File(format!("创建临时目录失败: {}", e)))?;
    
    // 验证临时目录权限
    let temp_path = Path::new(temp_dir);
    if !temp_path.exists() || !temp_path.is_dir() {
        return Err(AppError::File("临时目录无效".to_string()));
    }
    
    // 生成安全的唯一文件名
    let uuid = uuid::Uuid::new_v4();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    
    // 提取文件扩展名
    let extension = Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("tmp");
    
    let unique_filename = format!("upload_{}_{}.{}", timestamp, uuid, extension);
    let file_path = temp_path.join(unique_filename);
    
    // 验证路径安全性（防止路径遍历）
    if !file_path.starts_with(temp_path) {
        return Err(AppError::File("文件路径不安全".to_string()));
    }
    
    // 检查文件是否已存在（虽然UUID冲突概率极低）
    if file_path.exists() {
        return Err(AppError::File("临时文件已存在".to_string()));
    }
    
    Ok(file_path.to_string_lossy().to_string())
}

/// 增强的文档格式检测
fn detect_document_format_enhanced(
    file_path: &str, 
    first_chunk: Option<&[u8]>, 
    expected_extension: &str
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
        warn!("文件扩展名与内容格式不匹配: {:?} vs {:?}", extension_format, content_format);
        
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
        (DocumentFormat::Text, DocumentFormat::Txt) |
        (DocumentFormat::Txt, DocumentFormat::Text) |
        (DocumentFormat::Text, DocumentFormat::Md) |
        (DocumentFormat::Md, DocumentFormat::Text) => true,
        (a, b) => a == b,
    }
}

/// 检查是否为可靠的魔数检测
fn is_reliable_magic_number_detection(format: &DocumentFormat) -> bool {
    matches!(format, 
        DocumentFormat::PDF | 
        DocumentFormat::Image | 
        DocumentFormat::Audio
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
    if data.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || 
       data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
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
    if data.starts_with(b"ID3") || 
       (data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0) {
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
        warn!("清理临时文件失败: {} - {}", file_path, e);
    }
}

/// 检查URL下载是否支持该格式
fn is_format_supported_for_url(format: &DocumentFormat) -> bool {
    matches!(format, 
        DocumentFormat::PDF |
        DocumentFormat::Text |
        DocumentFormat::HTML |
        DocumentFormat::Txt |
        DocumentFormat::Md
    )
}

/// 检查OSS是否支持该格式
fn is_format_supported_for_oss(format: &DocumentFormat) -> bool {
    matches!(format, 
        DocumentFormat::PDF |
        DocumentFormat::Word |
        DocumentFormat::Excel |
        DocumentFormat::PowerPoint |
        DocumentFormat::Text |
        DocumentFormat::Txt |
        DocumentFormat::Md |
        DocumentFormat::HTML
    )
}

/// URL下载文档处理器
pub async fn download_document_from_url(
    State(state): State<AppState>,
    Json(request): Json<DownloadDocumentRequest>,
) -> impl axum::response::IntoResponse {
    info!("URL文档下载请求开始: {:?}", request);
    
    // 验证URL
    let validated_url = match RequestValidator::validate_url(&request.url) {
        Ok(url) => url.to_string(),
        Err(e) => {
            error!("URL验证失败: {}", e);
            return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
        }
    };
    
    // 验证文档格式
    if let Err(e) = RequestValidator::validate_document_format(&request.format) {
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }
    
    // 检查格式是否支持URL下载
    if !is_format_supported_for_url(&request.format) {
        return ApiResponse::validation_error::<DocumentParseResponse>(
            &format!("URL下载不支持的文档格式: {:?}", request.format)
        ).into_response();
    }
    
    // 验证TOC配置
    let (enable_toc, max_toc_depth) = match RequestValidator::validate_toc_config(
        request.enable_toc,
        request.max_toc_depth
    ) {
        Ok(config) => config,
        Err(e) => {
            return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
        }
    };
    
    // 创建处理器配置
    let _processor_config = MarkdownProcessorConfig {
        enable_toc,
        max_toc_depth,
        enable_anchors: true,
        enable_cache: true,
        ..Default::default()
    };
    
    // 创建任务
    let task = match state.task_service.create_task(
        SourceType::Url,
        Some(validated_url.clone()),
        request.format.clone(),
    ).await {
        Ok(task) => task,
        Err(e) => {
            error!("创建任务失败: {}", e);
            return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
        }
    };
    
    // 异步下载和解析文档
    match state.document_service.download_and_parse_document(
        &task.id, 
        &validated_url, 
        request.format
    ).await {
        Ok(_result) => {
            info!("URL文档下载和解析任务已启动: {}", task.id);
            
            let response = DocumentParseResponse {
                task_id: task.id,
                message: format!("URL文档下载任务已启动: {}", validated_url),
            };
            
            ApiResponse::success_with_status(response, StatusCode::ACCEPTED).into_response()
        }
        Err(e) => {
            error!("OSS文档解析启动失败: {}", e);
            ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response()
        }
    }
}

/// OSS文档解析处理器
pub async fn parse_oss_document(
    State(state): State<AppState>,
    Json(request): Json<ParseOssDocumentRequest>,
) -> impl axum::response::IntoResponse {
    info!("OSS文档解析请求开始: {:?}", request);
    
    // 验证OSS路径
    if let Err(e) = RequestValidator::validate_oss_path(&request.oss_path) {
        error!("OSS路径验证失败: {}", e);
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }
    
    // 验证文档格式
    if let Err(e) = RequestValidator::validate_document_format(&request.format) {
        return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
    }
    
    // 检查格式是否支持OSS解析
    if !is_format_supported_for_oss(&request.format) {
        return ApiResponse::validation_error::<DocumentParseResponse>(
            &format!("OSS解析不支持的文档格式: {:?}", request.format)
        ).into_response();
    }
    
    // 验证TOC配置
    let (enable_toc, max_toc_depth) = match RequestValidator::validate_toc_config(
        request.enable_toc,
        request.max_toc_depth
    ) {
        Ok(config) => config,
        Err(e) => {
            return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
        }
    };
    
    // 创建处理器配置
    let _processor_config = MarkdownProcessorConfig {
        enable_toc,
        max_toc_depth,
        enable_anchors: true,
        enable_cache: true,
        streaming_buffer_size: 64 * 1024,
        large_document_threshold: 1024 * 1024,
        enable_content_validation: true,
        max_cache_entries: 1000,
        cache_ttl_seconds: 3600,
    };
    
    // 创建任务
    let task = match state.task_service.create_task(
        SourceType::Oss,
        Some(request.oss_path.clone()),
        request.format.clone(),
    ).await {
        Ok(task) => task,
        Err(e) => {
            error!("创建任务失败: {}", e);
            return ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response();
        }
    };
    
    // 异步解析OSS文档
    match state.document_service.parse_oss_document(
        &task.id, 
        &request.oss_path, 
        request.format
    ).await {
        Ok(_result) => {
            info!("OSS文档解析任务已启动: {}", task.id);
            
            let response = DocumentParseResponse {
                task_id: task.id,
                message: format!("OSS文档解析任务已启动: {}", request.oss_path),
            };
            
            ApiResponse::success_with_status(response, StatusCode::ACCEPTED).into_response()
        }
        Err(e) => {
            error!("OSS文档解析启动失败: {}", e);
            ApiResponse::from_app_error::<DocumentParseResponse>(e).into_response()
        }
    }
}

/// 生成结构化文档处理器
pub async fn generate_structured_document(
    State(state): State<AppState>,
    Json(request): Json<GenerateStructuredDocumentRequest>,
) -> impl axum::response::IntoResponse {
    info!("生成结构化文档请求开始");
    
    // 验证Markdown内容
    if let Err(e) = RequestValidator::validate_markdown_content(&request.markdown_content) {
        return ApiResponse::from_app_error::<StructuredDocumentResponse>(e).into_response();
    }
    
    // 验证TOC配置
    let (enable_toc, max_toc_depth) = match RequestValidator::validate_toc_config(
        request.enable_toc,
        request.max_toc_depth
    ) {
        Ok(config) => config,
        Err(e) => {
            return ApiResponse::from_app_error::<StructuredDocumentResponse>(e).into_response();
        }
    };
    
    // 创建处理器配置
    let processor_config = MarkdownProcessorConfig {
        enable_toc,
        max_toc_depth,
        enable_anchors: request.enable_anchors.unwrap_or(true),
        enable_cache: false, // 直接处理不使用缓存
        streaming_buffer_size: 64 * 1024,
        large_document_threshold: 1024 * 1024,
        enable_content_validation: true,
        max_cache_entries: 1000,
        cache_ttl_seconds: 3600,
    };
    
    // 直接处理Markdown内容
    match state.document_service.generate_structured_document_simple(
        &request.markdown_content,
        Some(processor_config),
    ).await {
        Ok(document) => {
            info!("结构化文档生成成功");
            
            let response = StructuredDocumentResponse {
                document,
            };
            
            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("结构化文档生成失败: {}", e);
            ApiResponse::from_app_error::<StructuredDocumentResponse>(e.into()).into_response()
        }
    }
}

/// 获取支持的文档格式
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
pub async fn get_parser_stats(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let stats_data = state.document_service.get_parser_stats();
    let mut stats = HashMap::new();
    stats.insert("mineru_name".to_string(), serde_json::Value::String(stats_data.mineru_name));
    stats.insert("mineru_description".to_string(), serde_json::Value::String(stats_data.mineru_description));
    stats.insert("markitdown_name".to_string(), serde_json::Value::String(stats_data.markitdown_name));
    stats.insert("markitdown_description".to_string(), serde_json::Value::String(stats_data.markitdown_description));
    stats.insert("supported_formats".to_string(), serde_json::to_value(stats_data.supported_formats).unwrap_or_default());
    
    let response = ParserStatsResponse { stats };
    ApiResponse::success(response).into_response()
}

/// 检查解析器健康状态
pub async fn check_parser_health(
    State(state): State<AppState>,
) -> impl IntoResponse  {
    match state.document_service.check_parser_health().await {
        Ok(health_status) => {
            ApiResponse::success(health_status).into_response()
        }
        Err(e) => {
            error!("检查解析器健康状态失败: {}", e);
            ApiResponse::from_app_error::<HashMap<String, bool>>(e.into()).into_response()
        }
    }
}

/// 清理处理器缓存
pub async fn clear_processor_cache(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    state.document_service.clear_processor_cache().await;
    ApiResponse::message("处理器缓存已清空".to_string()).into_response()
}

/// 获取处理器缓存统计
pub async fn get_processor_cache_stats(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let cache_statistics = state.document_service.get_processor_cache_stats().await;
    let mut cache_stats = std::collections::HashMap::new();
    cache_stats.insert("total_entries".to_string(), serde_json::Value::Number(serde_json::Number::from(cache_statistics.total_entries)));
    cache_stats.insert("expired_entries".to_string(), serde_json::Value::Number(serde_json::Number::from(cache_statistics.expired_entries)));
    
    let response = ProcessorCacheStatsResponse { cache_stats };
    ApiResponse::success(response).into_response()
}

/// 验证上传请求参数
fn validate_upload_request(params: &UploadDocumentRequest) -> Result<(), AppError> {
    // 验证文件大小限制
    if let Some(max_size) = params.max_file_size {
        const MAX_ALLOWED_SIZE: u64 = 500 * 1024 * 1024; // 500MB 硬限制
        const MIN_ALLOWED_SIZE: u64 = 1; // 1字节最小限制
        
        if max_size > MAX_ALLOWED_SIZE {
            return Err(AppError::Validation(
                format!("文件大小限制不能超过{}MB", MAX_ALLOWED_SIZE / (1024 * 1024))
            ));
        }
        if max_size < MIN_ALLOWED_SIZE {
            return Err(AppError::Validation(
                "文件大小限制必须大于0".to_string()
            ));
        }
    }
    
    // 验证TOC配置
    RequestValidator::validate_toc_config(params.enable_toc, params.max_toc_depth)?;
    
    // 验证文档格式（如果指定）
    if let Some(ref format) = params.format {
        RequestValidator::validate_document_format(format)?;
    }
    
    Ok(())
}

/// 根据文档格式检测MIME类型
fn detect_mime_type_from_format(format: &DocumentFormat) -> String {
    match format {
        DocumentFormat::PDF => "application/pdf".to_string(),
        DocumentFormat::Word => "application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string(),
        DocumentFormat::Excel => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
        DocumentFormat::PowerPoint => "application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string(),
        DocumentFormat::Image => "image/jpeg".to_string(),
        DocumentFormat::Audio => "audio/mpeg".to_string(),
        DocumentFormat::HTML => "text/html".to_string(),
        DocumentFormat::Text => "text/plain".to_string(),
        DocumentFormat::Txt => "text/plain".to_string(),
        DocumentFormat::Md => "text/markdown".to_string(),
        DocumentFormat::Other(ext) => format!("application/{}", ext),
    }
}
