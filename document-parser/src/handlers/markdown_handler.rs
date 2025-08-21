use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use reqwest;
use std::time::Duration;
// 移除了不再使用的流式处理导入
use crate::app_state::AppState;
use crate::config::{FileSizePurpose, get_file_size_limit};
use crate::error::AppError;
use crate::handlers::response::ApiResponse;
use crate::handlers::validation::RequestValidator;
use crate::models::StructuredDocument;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use utoipa::ToSchema;

/// Markdown处理请求参数
/// 
/// 用于配置Markdown文档处理的各项参数，支持自定义TOC生成、锚点设置、缓存等选项。
#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkdownProcessRequest {
    /// 是否启用目录（Table of Contents）生成
    /// 当设置为true时，会自动解析文档标题并生成层级目录结构
    pub enable_toc: Option<bool>,
    
    /// 目录的最大深度限制
    /// 控制生成的目录层级数量，避免过深的嵌套结构
    pub max_toc_depth: Option<usize>,
    
    /// 是否启用锚点（Anchor）功能
    /// 为每个标题生成锚点链接，便于文档内部导航
    pub enable_anchors: Option<bool>,
    
    /// 是否启用缓存功能
    /// 缓存处理结果以提高重复请求的响应速度
    pub enable_cache: Option<bool>,
}

/// Markdown URL响应
/// 
/// 表示Markdown文档处理完成后的访问链接信息，包含临时URL、文件元数据等。
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MarkdownUrlResponse {
    /// 文档的访问URL，可以是临时链接或永久链接
    pub url: String,
    
    /// 文档处理任务的唯一标识符
    pub task_id: String,
    
    /// 标记URL是否为临时链接
    /// true表示临时链接，false表示永久链接
    pub temporary: bool,
    
    /// 临时URL的过期时间（小时）
    /// 仅当temporary为true时有效，None表示永不过期
    pub expires_in_hours: Option<u64>,
    
    /// 文档文件的大小（字节）
    pub file_size: Option<u64>,
    
    /// 文档的MIME类型，如 "text/markdown"、"application/pdf" 等
    pub content_type: String,
    
    /// 存储在OSS中的文件名
    /// 用于OSS存储系统的文件标识
    pub oss_file_name: Option<String>,
    
    /// OSS存储桶名称
    /// 指定文档存储的OSS存储桶
    pub oss_bucket: Option<String>,
}

/// 同步处理响应
/// 
/// 表示Markdown文档同步处理完成后的结果，包含结构化文档和性能指标。
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SectionsSyncResponse {
    /// 处理完成的结构化文档对象
    /// 包含完整的文档结构、目录和内容信息
    pub document: StructuredDocument,
    
    /// 文档处理耗时（毫秒）
    /// 用于性能监控和优化参考
    pub processing_time_ms: u64,
    
    /// 文档的总字数统计
    /// 可选字段，用于内容分析和统计
    pub word_count: Option<usize>,
}

/// 下载参数
/// 
/// 用于配置文档下载行为的参数，支持临时URL生成、格式选择等选项。
#[derive(Debug, Deserialize, ToSchema)]
pub struct DownloadParams {
    /// 是否生成临时URL
    /// 当设置为true时，生成有时效性的临时下载链接
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp: Option<bool>,
    
    /// 临时URL过期时间（小时）
    /// 控制临时下载链接的有效期，仅在temp为true时生效
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_hours: Option<u64>,
    
    /// 是否强制重新生成URL
    /// 忽略缓存，强制生成新的下载链接
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_regenerate: Option<bool>,
    
    /// 下载格式
    /// 指定文档的下载格式，如 "pdf"、"docx"、"html" 等
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// 流式下载配置
/// 
/// 用于配置大文件流式下载的参数，优化内存使用和传输性能。
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// 每个数据块的大小（字节）
    /// 控制流式传输时每个数据块的大小，影响内存使用和网络效率
    pub chunk_size: usize,
    
    /// 缓冲区大小（字节）
    /// 用于临时存储传输数据的缓冲区容量
    pub buffer_size: usize,
    
    /// 是否启用压缩
    /// 在传输过程中启用数据压缩，减少网络带宽使用
    pub enable_compression: bool,
    
    /// 支持的最大文件大小（字节）
    /// 超过此大小的文件将使用流式下载，避免内存溢出
    pub max_file_size: u64,
}

/// 同步解析 Markdown 文件，返回结构化章节
/// 解析Markdown章节 - 支持multipart/form-data格式
#[utoipa::path(
    post,
    path = "/api/v1/documents/markdown/parse",
    request_body(content = String, description = "Markdown文件内容", content_type = "multipart/form-data"),
    params(
        ("enable_toc" = Option<bool>, Query, description = "是否启用目录生成"),
        ("max_toc_depth" = Option<usize>, Query, description = "目录最大深度"),
        ("enable_anchors" = Option<bool>, Query, description = "是否启用锚点"),
        ("enable_cache" = Option<bool>, Query, description = "是否启用缓存")
    ),
    responses(
        (status = 200, description = "解析成功", body = SectionsSyncResponse),
        (status = 400, description = "请求参数错误"),
        (status =413, description = "文件过大"),
        (status = 500, description = "服务器内部错误")
    ),
    tag = "markdown"
)]
pub async fn parse_markdown_sections(
    State(state): State<AppState>,
    Query(params): Query<MarkdownProcessRequest>,
    mut multipart: Multipart,
) -> axum::response::Response {
    info!("同步解析Markdown请求开始(multipart): {:?}", params);
    let start_time = std::time::Instant::now();

    // 验证TOC配置
    let (enable_toc, max_toc_depth) =
        match RequestValidator::validate_toc_config(params.enable_toc, params.max_toc_depth) {
            Ok(config) => config,
            Err(e) => {
                return ApiResponse::from_app_error::<SectionsSyncResponse>(e).into_response();
            }
        };

    // 处理multipart上传
    let content = match process_markdown_multipart(&mut multipart).await {
        Ok(content) => content,
        Err(e) => {
            error!("处理Markdown上传失败: {}", e);
            return ApiResponse::from_app_error::<SectionsSyncResponse>(e).into_response();
        }
    };

    // 处理Markdown内容
    process_markdown_content(state, content, enable_toc, max_toc_depth, start_time).await
}

/// 通用的Markdown内容处理函数
async fn process_markdown_content(
    state: AppState,
    content: String,
    enable_toc: bool,
    max_toc_depth: usize,
    start_time: std::time::Instant,
) -> axum::response::Response {
    // 验证Markdown内容
    if let Err(e) = RequestValidator::validate_markdown_content(&content) {
        return ApiResponse::from_app_error::<SectionsSyncResponse>(e).into_response();
    }

    // 处理Markdown内容（直接使用全局配置初始化的处理器）
    match state
        .document_service
        .generate_structured_document_simple(&content)
        .await
    {
        Ok(document) => {
            let processing_time = start_time.elapsed().as_millis() as u64;
            let word_count = calculate_word_count(&content);

            info!(
                "Markdown解析成功，耗时: {}ms，字数: {:?}",
                processing_time, word_count
            );

            let response = SectionsSyncResponse {
                document,
                processing_time_ms: processing_time,
                word_count,
            };

            ApiResponse::success_with_status(response, StatusCode::OK).into_response()
        }
        Err(e) => {
            error!("解析Markdown失败: {}", e);
            ApiResponse::from_app_error::<SectionsSyncResponse>(e.into()).into_response()
        }
    }
}

/// 处理Markdown multipart上传
async fn process_markdown_multipart(multipart: &mut Multipart) -> Result<String, AppError> {
    

    let max_markdown_size =
        get_file_size_limit(&FileSizePurpose::ContentValidation).bytes() as usize;
    let mut content: Option<String> = None;
    let mut total_size = 0usize;
    let mut field_count = 0;

    info!(
        "开始处理multipart数据，最大文件大小: {} 字节",
        max_markdown_size
    );

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error!("解析multipart数据失败: {}", e);
        AppError::Validation(format!("解析multipart数据失败: {e}"))
    })? {
        field_count += 1;
        info!("处理第 {} 个字段", field_count);

        if let Some(name) = field.name() {
            info!("字段名称: {}", name);

            if let Some(filename) = field.file_name() {
                info!("文件名: {}", filename);
            }

            if let Some(content_type) = field.content_type() {
                info!("内容类型: {}", content_type);
            }

            if name == "file" {
                // 检查文件名
                if let Some(filename) = field.file_name() {
                    info!("处理Markdown文件: {}", filename);

                    // 验证文件扩展名
                    if !is_markdown_file(filename) {
                        error!("不支持的文件类型: {}", filename);
                        return Err(AppError::Validation("只支持.md和.markdown文件".to_string()));
                    }
                }

                // 流式读取内容
                let mut buffer = Vec::new();
                let mut stream = field;
                let mut chunk_count = 0;

                while let Some(chunk) = stream.chunk().await.map_err(|e| {
                    error!("读取文件块失败: {}", e);
                    AppError::File(format!("读取文件块失败: {e}"))
                })? {
                    chunk_count += 1;
                    total_size += chunk.len();
                    info!(
                        "读取第 {} 个数据块，大小: {} 字节，总大小: {} 字节",
                        chunk_count,
                        chunk.len(),
                        total_size
                    );

                    if total_size > max_markdown_size {
                        error!("文件过大: {} > {} 字节", total_size, max_markdown_size);
                        return Err(AppError::Validation(format!(
                            "Markdown文件过大: {total_size} > {max_markdown_size} 字节"
                        )));
                    }

                    buffer.extend_from_slice(&chunk);
                }

                info!(
                    "文件读取完成，总共 {} 个数据块，总大小: {} 字节",
                    chunk_count, total_size
                );

                // 转换为UTF-8字符串
                let content_str = String::from_utf8(buffer).map_err(|e| {
                    error!("文件不是有效的UTF-8格式: {}", e);
                    AppError::Validation(format!("文件不是有效的UTF-8格式: {e}"))
                })?;

                info!("文件内容长度: {} 字符", content_str.len());
                info!(
                    "文件内容预览 (前200字符): {}",
                    if content_str.len() > 200 {
                        format!("{}...", &content_str[..200])
                    } else {
                        content_str.clone()
                    }
                );

                content = Some(content_str);
                break;
            } else {
                info!("跳过非file字段: {}", name);
            }
        } else {
            info!("字段没有名称");
        }
    }

    info!("multipart处理完成，总共处理了 {} 个字段", field_count);

    match &content {
        Some(c) => {
            info!("成功获取到内容，长度: {} 字符", c.len());
            Ok(c.clone())
        }
        None => {
            error!("未找到名为 'file' 的表单字段");
            Err(AppError::Validation(
                "未找到名为 'file' 的表单字段".to_string(),
            ))
        }
    }
}

/// 检查是否为Markdown文件
fn is_markdown_file(filename: &str) -> bool {
    let filename_lower = filename.to_lowercase();
    filename_lower.ends_with(".md") || filename_lower.ends_with(".markdown")
}

/// 计算字数
fn calculate_word_count(content: &str) -> Option<usize> {
    // 简单的字数统计，可以根据需要改进
    let word_count = content
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .count();

    if word_count > 0 {
        Some(word_count)
    } else {
        None
    }
}

/// 下载 Markdown 文件（支持断点续传和流式下载）
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/markdown/download",
    params(
        ("task_id" = String, Path, description = "任务ID"),
        ("temp" = Option<bool>, Query, description = "是否生成临时URL"),
        ("expires_hours" = Option<u64>, Query, description = "临时URL过期时间（小时）"),
        ("force_regenerate" = Option<bool>, Query, description = "是否强制重新生成"),
        ("format" = Option<String>, Query, description = "下载格式")
    ),
    responses(
        (status = 200, description = "下载成功", content_type = "text/markdown"),
        (status = 206, description = "部分内容下载", content_type = "text/markdown"),
        (status = 400, description = "请求参数错误"),
        (status = 404, description = "文件不存在"),
        (status = 500, description = "服务器内部错误")
    ),
    tag = "markdown"
)]
pub async fn download_markdown(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Query(params): Query<DownloadParams>,
    headers_in: HeaderMap,
) -> impl axum::response::IntoResponse {
    info!("下载Markdown请求: task_id={}, params={:?}", task_id, params);

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<String>(e).into_response();
    }

    // 获取任务信息
    let task = match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            return ApiResponse::not_found::<String>(&format!("任务不存在: {task_id}"))
                .into_response();
        }
        Err(e) => {
            error!("查询任务失败: task_id={}, error={}", task_id, e);
            return ApiResponse::from_app_error::<String>(e).into_response();
        }
    };

    // 获取流式配置
    let streaming_config = StreamingConfig {
        chunk_size: 64 * 1024,   // 64KB chunks
        buffer_size: 256 * 1024, // 256KB buffer
        enable_compression: headers_in
            .get(header::ACCEPT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.contains("gzip"))
            .unwrap_or(false),
        max_file_size: state
            .config
            .file_size_config
            .get_file_size_limit(&FileSizePurpose::DocumentParser)
            .bytes(),
    };

    // 尝试从不同来源获取Markdown内容
    let markdown_source = determine_markdown_source(&task, &params);

    match markdown_source {
        MarkdownSource::Oss(oss_url) => {
            download_from_oss(&state, &task, &oss_url, &headers_in, &streaming_config).await
        }
        MarkdownSource::StructuredDocument(doc) => {
            download_from_structured_document(&task_id, &doc, &headers_in, &streaming_config).await
        }
        MarkdownSource::NotAvailable => {
            ApiResponse::not_found::<String>("该任务未关联Markdown文件且未生成结构化文档")
                .into_response()
        }
    }
}

/// Markdown来源类型
enum MarkdownSource {
    Oss(String),
    StructuredDocument(StructuredDocument),
    NotAvailable,
}

/// 确定Markdown来源
fn determine_markdown_source(
    task: &crate::models::DocumentTask,
    params: &DownloadParams,
) -> MarkdownSource {
    // 如果强制重新生成，优先使用结构化文档
    if params.force_regenerate.unwrap_or(false) {
        if let Some(doc) = &task.structured_document {
            return MarkdownSource::StructuredDocument(doc.clone());
        }
    }

    // 优先从OSS获取
    if let Some(oss) = &task.oss_data {
        if !oss.markdown_url.is_empty() {
            return MarkdownSource::Oss(oss.markdown_url.clone());
        }
    }

    // 其次从结构化文档生成
    if let Some(doc) = &task.structured_document {
        return MarkdownSource::StructuredDocument(doc.clone());
    }

    MarkdownSource::NotAvailable
}

/// 从OSS下载Markdown
async fn download_from_oss(
    state: &AppState,
    task: &crate::models::DocumentTask,
    oss_url: &str,
    headers_in: &HeaderMap,
    _streaming_config: &StreamingConfig,
) -> Response {
    info!("从OSS下载Markdown: task_id={}, url={}", task.id, oss_url);

    let oss_client = match &state.oss_client {
        Some(client) => client,
        None => {
            return ApiResponse::internal_error::<String>("OSS客户端未配置").into_response();
        }
    };

    // 优先使用任务中存储的 markdown_object_key，如果没有则从URL解析
    let object_key = if let Some(ref oss_data) = task.oss_data {
        if let Some(ref stored_key) = oss_data.markdown_object_key {
            info!("使用存储的object_key: {}", stored_key);
            stored_key.clone()
        } else {
            // 回退到从URL解析
            let parsed_key = oss_url
                .trim_start_matches("https://")
                .split('/')
                .skip(1) // 跳过域名部分
                .collect::<Vec<&str>>()
                .join("/");
            info!("从URL解析的object_key: {}", parsed_key);
            parsed_key
        }
    } else {
        // 如果没有oss_data，从URL解析
        let parsed_key = oss_url
            .trim_start_matches("https://")
            .split('/')
            .skip(1) // 跳过域名部分
            .collect::<Vec<&str>>()
            .join("/");
        info!("从URL解析的object_key (无oss_data): {}", parsed_key);
        parsed_key
    };

    // 生成下载URL
    let download_url =
        match oss_client.generate_download_url(&object_key, Some(Duration::from_secs(3600))) {
            Ok(url) => url,
            Err(e) => {
                error!(
                    "生成下载URL失败: task_id={}, object_key={}, error={}",
                    task.id, object_key, e
                );
                return ApiResponse::internal_error::<String>("生成下载URL失败").into_response();
            }
        };

    // 通过HTTP请求下载文件内容
    let client = reqwest::Client::new();
    match client.get(&download_url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                match response.bytes().await {
                    Ok(content) => {
                        info!(
                            "成功从OSS下载Markdown内容: task_id={}, size={} bytes",
                            task.id,
                            content.len()
                        );

                        let range_header = headers_in
                            .get(header::RANGE)
                            .and_then(|v| v.to_str().ok())
                            .map(|s| s.to_string());

                        build_range_response(&task.id, content.to_vec(), range_header)
                    }
                    Err(e) => {
                        error!("读取响应内容失败: task_id={}, error={}", task.id, e);
                        ApiResponse::internal_error::<String>("读取文件内容失败").into_response()
                    }
                }
            } else {
                error!(
                    "下载请求失败: task_id={}, status={}",
                    task.id,
                    response.status()
                );
                ApiResponse::internal_error::<String>("下载文件失败").into_response()
            }
        }
        Err(e) => {
            error!("HTTP请求失败: task_id={}, error={}", task.id, e);
            ApiResponse::internal_error::<String>("网络请求失败").into_response()
        }
    }
}

/// 从结构化文档生成Markdown
async fn download_from_structured_document(
    task_id: &str,
    doc: &StructuredDocument,
    headers_in: &HeaderMap,
    _streaming_config: &StreamingConfig,
) -> Response {
    info!("从结构化文档生成Markdown: task_id={}", task_id);

    // 生成Markdown内容
    let markdown_content = generate_markdown_from_document(doc);
    let body_bytes = markdown_content.into_bytes();

    // 检查文件大小
    if body_bytes.len() > _streaming_config.max_file_size as usize {
        return ApiResponse::validation_error::<String>("生成的Markdown文件过大").into_response();
    }

    let range_header = headers_in
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    build_range_response(task_id, body_bytes, range_header)
}

/// 从结构化文档生成Markdown内容
fn generate_markdown_from_document(doc: &StructuredDocument) -> String {
    let mut md = String::new();

    // 添加文档标题
    md.push_str(&format!("# {}\n\n", doc.document_title));

    // 添加元数据
    if let Some(word_count) = doc.word_count {
        md.push_str(&format!("*字数: {word_count}*\n\n"));
    }

    if let Some(processing_time) = &doc.processing_time {
        md.push_str(&format!("*处理时间: {processing_time}*\n\n"));
    }

    md.push_str("---\n\n");

    // 递归写入章节
    fn write_section(buf: &mut String, sec: &crate::models::StructuredSection) {
        let level = sec.level.clamp(1, 6) as usize;
        buf.push_str(&format!("{} {}\n\n", "#".repeat(level), sec.title));

        if !sec.content.is_empty() {
            buf.push_str(&sec.content);
            buf.push_str("\n\n");
        }

        for child in &sec.children {
            write_section(buf, child);
        }
    }

    for sec in &doc.toc {
        write_section(&mut md, sec);
    }

    md
}

/// 检查是否有Range请求
fn has_range_request(headers: &HeaderMap) -> bool {
    headers.get(header::RANGE).is_some()
}

// 移除了不再使用的 build_streaming_response 函数

/// 获取 Markdown OSS URL（如果有）
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/markdown/url",
    params(
        ("task_id" = String, Path, description = "任务ID"),
        ("temp" = Option<bool>, Query, description = "是否生成临时URL"),
        ("expires_hours" = Option<u64>, Query, description = "临时URL过期时间（小时）"),
        ("force_regenerate" = Option<bool>, Query, description = "是否强制重新生成"),
        ("format" = Option<String>, Query, description = "下载格式")
    ),
    responses(
        (status = 200, description = "获取成功", body = MarkdownUrlResponse),
        (status = 400, description = "请求参数错误"),
        (status = 404, description = "任务不存在或未关联Markdown文件"),
        (status = 500, description = "服务器内部错误")
    ),
    tag = "markdown"
)]
pub async fn get_markdown_url(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Query(params): Query<DownloadParams>,
) -> axum::response::Response {
    info!(
        "获取Markdown URL请求: task_id={}, params={:?}",
        task_id, params
    );

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<MarkdownUrlResponse>(e).into_response();
    }

    // 获取任务信息
    let task = match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            return ApiResponse::not_found::<MarkdownUrlResponse>(&format!(
                "任务不存在: {task_id}"
            ))
            .into_response();
        }
        Err(e) => {
            error!("查询任务失败: task_id={}, error={}", task_id, e);
            return ApiResponse::from_app_error::<MarkdownUrlResponse>(e).into_response();
        }
    };

    // 检查是否有OSS数据
    let oss_data = match &task.oss_data {
        Some(oss) if !oss.markdown_url.is_empty() => oss,
        _ => {
            return ApiResponse::not_found::<MarkdownUrlResponse>("该任务未关联Markdown的OSS地址")
                .into_response();
        }
    };

    let generate_temp = params.temp.unwrap_or(false);
    let expires_in_hours = params.expires_hours.unwrap_or(24); // 默认24小时过期

    // 验证过期时间
    if expires_in_hours == 0 || expires_in_hours > 168 {
        // 最长7天
        return ApiResponse::validation_error::<MarkdownUrlResponse>("过期时间必须在1-168小时之间")
            .into_response();
    }

    if generate_temp {
        // 生成临时预签名URL
        let oss_client = match &state.oss_client {
            Some(client) => client,
            None => {
                return ApiResponse::internal_error::<MarkdownUrlResponse>("OSS服务未配置")
                    .into_response();
            }
        };

        let expires_in = Duration::from_secs(expires_in_hours * 3600);

        // 使用 object_key 而不是完整的 URL 来生成临时 URL
        let object_key = match &oss_data.markdown_object_key {
            Some(key) => key,
            None => {
                error!("任务 {} 的 OSS 数据缺少 object_key", task_id);
                return ApiResponse::internal_error::<MarkdownUrlResponse>(
                    "OSS 对象键缺失，无法生成临时URL",
                )
                .into_response();
            }
        };

        match oss_client.generate_download_url(object_key, Some(expires_in)) {
            Ok(temp_url) => {
                info!(
                    "生成临时URL成功: task_id={}, expires_in={}h",
                    task_id, expires_in_hours
                );

                let response = MarkdownUrlResponse {
                    url: temp_url,
                    task_id: task_id.clone(),
                    temporary: true,
                    expires_in_hours: Some(expires_in_hours),
                    file_size: get_file_size_from_oss(&state, &oss_data.markdown_url).await,
                    content_type: "text/markdown; charset=utf-8".to_string(),
                    oss_file_name: oss_data.markdown_object_key.clone(),
                    oss_bucket: Some(oss_data.bucket.clone()),
                };

                ApiResponse::success_with_status(response, StatusCode::OK).into_response()
            }
            Err(e) => {
                error!("生成临时URL失败: task_id={}, error={}", task_id, e);
                ApiResponse::internal_error::<MarkdownUrlResponse>(&format!(
                    "生成下载URL失败: {e}"
                ))
                .into_response()
            }
        }
    } else {
        // 返回原始OSS URL
        info!("返回原始OSS URL: task_id={}", task_id);

        let response = MarkdownUrlResponse {
            url: oss_data.markdown_url.clone(),
            task_id: task_id.clone(),
            temporary: false,
            expires_in_hours: None,
            file_size: get_file_size_from_oss(&state, &oss_data.markdown_url).await,
            content_type: "text/markdown; charset=utf-8".to_string(),
            oss_file_name: oss_data.markdown_object_key.clone(),
            oss_bucket: Some(oss_data.bucket.clone()),
        };

        ApiResponse::success_with_status(response, StatusCode::OK).into_response()
    }
}

/// 获取OSS文件大小
async fn get_file_size_from_oss(state: &AppState, oss_url: &str) -> Option<u64> {
    if let Some(oss_client) = &state.oss_client {
        // 暂时跳过文件大小获取功能，需要重新实现
        None
        /*
        match oss_client.get_object_metadata(oss_url).await {
            Ok(metadata) => {
                metadata.get("content-length")
                    .and_then(|s| s.parse::<u64>().ok())
            }
            Err(e) => {
                warn!("获取OSS文件元数据失败: {}", e);
                None
            }
        }
        */
    } else {
        None
    }
}

/// 解析 Range 请求头（增强版）
fn parse_range_header(range_str: &str, file_size: u64) -> Option<(u64, u64)> {
    if !range_str.starts_with("bytes=") {
        return None;
    }

    let range_part = &range_str[6..]; // 去掉 "bytes=" 前缀

    // 支持多个范围，但这里只处理第一个
    let first_range = range_part.split(',').next()?.trim();
    let parts: Vec<&str> = first_range.split('-').collect();

    if parts.len() != 2 {
        return None;
    }

    let start = if parts[0].is_empty() {
        // 后缀范围请求，如 "-500" 表示最后500字节
        if let Ok(suffix_length) = parts[1].parse::<u64>() {
            if suffix_length == 0 || suffix_length > file_size {
                return None;
            }
            file_size.saturating_sub(suffix_length)
        } else {
            return None;
        }
    } else {
        parts[0].parse::<u64>().ok()?
    };

    let end = if parts[1].is_empty() {
        // 从start到文件末尾
        file_size.saturating_sub(1)
    } else {
        let parsed_end = parts[1].parse::<u64>().ok()?;
        if parsed_end >= file_size {
            file_size.saturating_sub(1)
        } else {
            parsed_end
        }
    };

    if start <= end && start < file_size {
        Some((start, end))
    } else {
        None
    }
}

/// 验证Range请求的有效性
fn validate_range_request(start: u64, end: u64, file_size: u64) -> bool {
    start <= end && start < file_size && end < file_size
}

/// 计算Range响应的内容长度
fn calculate_content_length(start: u64, end: u64) -> u64 {
    end.saturating_sub(start).saturating_add(1)
}

/// 构建Range响应（增强版）
fn build_range_response(
    filename_hint: &str,
    full: Vec<u8>,
    range_header: Option<String>,
) -> Response {
    let total_len = full.len() as u64;

    // 验证文件大小
    if total_len == 0 {
        return build_empty_response();
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/markdown; charset=utf-8"),
    );

    // 安全的文件名处理
    let safe_filename = sanitize_filename_for_header(filename_hint);
    headers.insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{safe_filename}.md\""))
            .unwrap_or(header::HeaderValue::from_static(
                "attachment; filename=\"document.md\"",
            )),
    );

    headers.insert(
        header::ACCEPT_RANGES,
        header::HeaderValue::from_static("bytes"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        header::HeaderValue::from_str(&total_len.to_string())
            .unwrap_or(header::HeaderValue::from_static("0")),
    );

    // 添加缓存控制
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("public, max-age=3600"),
    );

    // 添加ETag
    let etag = generate_etag(&full);
    headers.insert(
        header::ETAG,
        header::HeaderValue::from_str(&etag)
            .unwrap_or(header::HeaderValue::from_static("\"unknown\"")),
    );

    // 处理Range请求
    if let Some(range_str) = range_header {
        if let Some((start, end)) = parse_range_header(&range_str, total_len) {
            if validate_range_request(start, end, total_len) {
                let start_usize = start as usize;
                let end_usize = end as usize;

                // 安全的切片操作
                if start_usize < full.len() && end_usize < full.len() && start_usize <= end_usize {
                    let slice = full[start_usize..=end_usize].to_vec();
                    let content_length = calculate_content_length(start, end);

                    let mut range_headers = headers;
                    range_headers.insert(
                        header::CONTENT_RANGE,
                        header::HeaderValue::from_str(&format!(
                            "bytes {start}-{end}/{total_len}"
                        ))
                        .unwrap_or(header::HeaderValue::from_static("bytes */*")),
                    );
                    range_headers.insert(
                        header::CONTENT_LENGTH,
                        header::HeaderValue::from_str(&content_length.to_string())
                            .unwrap_or(header::HeaderValue::from_static("0")),
                    );

                    info!("返回Range响应: {}-{}/{} 字节", start, end, total_len);
                    return (StatusCode::PARTIAL_CONTENT, range_headers, slice).into_response();
                }
            }
        }

        // Range请求无效
        warn!("无效的Range请求: {}", range_str);
        return range_not_satisfiable(total_len);
    }

    info!("返回完整文件响应: {} 字节", total_len);
    (StatusCode::OK, headers, full).into_response()
}

/// 构建空响应
fn build_empty_response() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        header::HeaderValue::from_static("0"),
    );

    (StatusCode::OK, headers, Vec::<u8>::new()).into_response()
}

/// 为HTTP头部清理文件名
fn sanitize_filename_for_header(filename: &str) -> String {
    filename
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect::<String>()
        .chars()
        .take(50) // 限制长度
        .collect()
}

/// 生成ETag
fn generate_etag(content: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let hash = hasher.finish();

    format!("\"{hash}\"")
}

fn range_not_satisfiable(total_len: u64) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_RANGE,
        header::HeaderValue::from_str(&format!("bytes */{total_len}"))
            .unwrap_or(header::HeaderValue::from_static("bytes */*")),
    );
    (StatusCode::RANGE_NOT_SATISFIABLE, headers, Vec::<u8>::new()).into_response()
}

/// 从URL下载文件内容
async fn download_file_content(url: &str) -> Result<String, AppError> {
    info!("开始下载文件: {}", url);

    // 创建HTTP客户端，设置超时
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Network(format!("创建HTTP客户端失败: {e}")))?;

    // 发送GET请求
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("下载文件失败: {e}")))?;

    // 检查响应状态
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "下载文件失败，HTTP状态码: {}",
            response.status()
        )));
    }

    // 获取文件内容
    let content = response
        .text()
        .await
        .map_err(|e| AppError::Network(format!("读取文件内容失败: {e}")))?;

    info!("文件下载完成: {} 字符", content.len());
    Ok(content)
}
