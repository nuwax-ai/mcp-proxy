use anyhow::{Context, Result as AnyhowResult};
use async_trait::async_trait;
use oss_client::ApiFileClient;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use pulldown_cmark::{Event, LinkType, Parser, Tag, TagEnd};
use pulldown_cmark_to_cmark::cmark;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::models::{
    DocumentFormat, ParseResult, ParserEngine, SourceType, StructuredDocument, StructuredSection,
    TaskStatus,
};
use crate::parsers::DualEngineParser;
use crate::processors::MarkdownProcessor;
use crate::processors::markdown_processor::{CacheStatistics, MarkdownProcessorConfig};
use crate::services::TaskService;
use crate::{ImageInfo, ProcessingStage};

/// Configuration for DocumentService
#[derive(Debug, Clone)]
pub struct DocumentServiceConfig {
    pub max_concurrent_tasks: usize,
    pub task_timeout: Duration,
    pub download_timeout: Duration,
    // 文件大小限制现在由全局配置管理
    // temp_dir removed - now uses current directory approach
    pub enable_cache: bool,
    pub cache_ttl: Duration,
}

impl Default for DocumentServiceConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 10,
            task_timeout: Duration::from_secs(3600), // 60 minutes - 使用配置文件中的统一超时
            download_timeout: Duration::from_secs(60), // 1 minute
            // 文件大小限制现在由全局配置管理
            // temp_dir removed - now uses current directory approach
            enable_cache: true,
            cache_ttl: Duration::from_secs(3600), // 1 hour
        }
    }
}

impl DocumentServiceConfig {
    /// 从应用配置创建文档服务配置
    pub fn from_app_config(app_config: &crate::config::AppConfig) -> Self {
        Self {
            max_concurrent_tasks: app_config.document_parser.max_concurrent,
            task_timeout: Duration::from_secs(app_config.document_parser.processing_timeout as u64),
            download_timeout: Duration::from_secs(
                app_config.document_parser.download_timeout as u64,
            ),
            enable_cache: true,
            cache_ttl: Duration::from_secs(3600), // 1 hour
        }
    }
}

/// Resource cleanup guard for temporary files
pub struct TempFileGuard {
    path: String,
}

impl TempFileGuard {
    pub fn new(path: String) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            warn!("Failed to cleanup temporary file {}: {}", self.path, e);
        } else {
            debug!("Cleaned up temporary file: {}", self.path);
        }
    }
}

/// 文档服务 - Enhanced with proper async patterns and resource management
pub struct DocumentService {
    dual_parser: Arc<DualEngineParser>,
    markdown_processor: Arc<RwLock<MarkdownProcessor>>,
    task_service: Arc<TaskService>,
    oss_client: Option<Arc<dyn oss_client::OssClientTrait + Send + Sync>>,
    config: DocumentServiceConfig,
    // HTTP client for downloads
    http_client: reqwest::Client,
}

impl DocumentService {
    /// 图片上传并发度：保守值（自定义后端多为用户系统单实例，避免压垮；
    /// OSS 无压力）。后续需要时提升为配置项。
    const IMAGE_UPLOAD_CONCURRENCY: usize = 4;
    /// 图片 URL 换签并发度：换签是轻量元数据 GET（非文件上传），
    /// 可高于上传并发（IMAGE_UPLOAD_CONCURRENCY）。
    const IMAGE_SIGN_CONCURRENCY: usize = 8;
    /// 内嵌图片换签整体预算：尽力而为的增强——超预算降级返回未换签
    /// 内容（warn 日志），不让下载主体挂起或失败
    const IMAGE_SIGN_BUDGET: Duration = Duration::from_secs(120);
    /// 创建新的文档服务
    pub fn new(
        dual_parser: DualEngineParser,
        markdown_processor: MarkdownProcessor,
        task_service: Arc<TaskService>,
        oss_client: Option<Arc<dyn oss_client::OssClientTrait + Send + Sync>>,
    ) -> Result<Self, AppError> {
        Self::with_config(
            dual_parser,
            markdown_processor,
            task_service,
            oss_client,
            DocumentServiceConfig::default(),
        )
    }

    /// 创建带配置的文档服务
    pub fn with_config(
        dual_parser: DualEngineParser,
        markdown_processor: MarkdownProcessor,
        task_service: Arc<TaskService>,
        oss_client: Option<Arc<dyn oss_client::OssClientTrait + Send + Sync>>,
        config: DocumentServiceConfig,
    ) -> Result<Self, AppError> {
        // Configure HTTP client with timeouts
        let http_client = reqwest::Client::builder()
            .timeout(config.download_timeout)
            // 连接超时：目标主机不可达/SYN 黑洞时快速失败（也惠及自定义上传后端
            // 的共享连接池，避免 TCP 连接阶段挂满请求级总超时）
            .connect_timeout(Duration::from_secs(10))
            .user_agent("DocumentParser/1.0")
            .build()
            .map_err(|e| AppError::Internal(format!("创建 HTTP 客户端失败: {e}")))?;

        Ok(Self {
            dual_parser: Arc::new(dual_parser),
            markdown_processor: Arc::new(RwLock::new(markdown_processor)),
            task_service,
            oss_client,
            config,
            http_client,
        })
    }

    /// 共享 HTTP 客户端（连接池复用；handler 层构造自定义上传客户端时使用，
    /// 避免每请求新建连接池）
    pub(crate) fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    /// 解析文档 - Enhanced with proper async patterns and error handling
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn parse_document(
        &self,
        task_id: &str,
        file_path: &str,
    ) -> AnyhowResult<ParseResult> {
        info!("Start parsing the document: {}", file_path);

        // Wrap the entire operation in a timeout
        let result = timeout(self.config.task_timeout, async {
            self.parse_document_internal(task_id, file_path).await
        })
        .await;

        match result {
            Ok(parse_result) => parse_result,
            Err(_) => {
                let error_msg = format!("文档解析超时 ({}s)", self.config.task_timeout.as_secs());
                error!("{}", error_msg);

                // Update task with timeout error
                if let Err(e) = self
                    .task_service
                    .set_task_error(task_id, error_msg.clone())
                    .await
                {
                    warn!("Failed to update task error status: {}", e);
                }

                Err(anyhow::anyhow!(error_msg))
            }
        }
    }

    /// 本地解析文档（CLI 场景）：只执行解析，不做图片/Markdown 的 OSS 上传，
    /// 也不做任务持久化。图片保留在解析输出目录中（见 [`ParseResult::output_dir`]），
    /// 由调用方自行处理。
    pub async fn parse_document_local(&self, file_path: &str) -> AnyhowResult<ParseResult> {
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            return Err(anyhow::anyhow!("文件不存在: {}", path.display()));
        }
        let absolute_path = path
            .canonicalize()
            .context("无法获取文件绝对路径")?
            .to_string_lossy()
            .to_string();

        // 文件大小限制校验（与服务器路径保持一致）
        let metadata = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("获取文件信息失败: {}", path.display()))?;
        let global_config = GlobalFileSizeConfig::new();
        if metadata.len() > global_config.max_file_size.bytes() {
            return Err(anyhow::anyhow!(
                "文件大小超过限制: {} > {} bytes",
                metadata.len(),
                global_config.max_file_size.bytes()
            ));
        }

        info!("Start parsing the document locally: {}", absolute_path);
        let result = timeout(self.config.task_timeout, async {
            self.dual_parser
                .parse_document_auto(&absolute_path)
                .await
                .with_context(|| "文档解析失败[parse_document_local]".to_string())
        })
        .await;

        match result {
            Ok(parse_result) => parse_result,
            Err(_) => Err(anyhow::anyhow!(
                "文档解析超时 ({}s)",
                self.config.task_timeout.as_secs()
            )),
        }
    }

    /// Internal document parsing implementation
    async fn parse_document_internal(
        &self,
        task_id: &str,
        file_path: &str,
    ) -> AnyhowResult<ParseResult> {
        debug!(
            "parse_document_internal - Task ID: {}, File path: {}",
            task_id, file_path
        );

        // 验证文件是否存在
        if !std::path::Path::new(file_path).exists() {
            error!("File does not exist: {}", file_path);
            return Err(anyhow::anyhow!("文件不存在: {}", file_path));
        }

        // 获取文件的绝对路径
        let absolute_path = std::path::Path::new(file_path)
            .canonicalize()
            .context("无法获取文件绝对路径")?
            .to_string_lossy()
            .to_string();
        debug!("Absolute file path: {}", absolute_path);
        // 记录开始时间
        let start_time = std::time::Instant::now();
        // Update task status with proper error handling
        self.update_task_stage_safe(task_id, crate::models::ProcessingStage::FormatDetection)
            .await;

        // Validate file existence and size
        let file_path = Path::new(file_path);
        if !file_path.exists() {
            return Err(anyhow::anyhow!("文件不存在: {}", file_path.display()));
        }

        let metadata = tokio::fs::metadata(file_path)
            .await
            .with_context(|| format!("获取文件信息失败: {}", file_path.display()))?;

        let file_size = metadata.len();

        // Check file size limit
        let global_config = GlobalFileSizeConfig::new();
        if file_size > global_config.max_file_size.bytes() {
            return Err(anyhow::anyhow!(
                "文件大小超过限制: {} > {} bytes",
                file_size,
                global_config.max_file_size.bytes()
            ));
        }

        // Detect MIME type
        let mime_type = self
            .detect_mime_type_async(file_path)
            .await
            .context("MIME类型检测失败")?;

        // Update task file information
        self.update_task_file_info_safe(task_id, Some(file_size), Some(mime_type))
            .await;

        // 自动检测格式
        let detection = crate::parsers::format_detector::FormatDetector::new()
            .detect_format(&absolute_path, None)
            .context("文件格式检测失败")?;
        let format = detection.format;
        let selected_engine = ParserEngine::select_for_format(&format);
        debug!(
            "Detected format: {:?}, selected parsing engine: {:?}",
            format, selected_engine
        );
        // 将检测到的文档格式保存到任务记录
        self.update_task_document_format_safe(task_id, format.clone())
            .await;
        self.update_task_parser_engine_safe(task_id, selected_engine.clone())
            .await;

        self.update_task_progress_safe(task_id, 10).await;

        // Execute parsing with proper stage tracking
        let stage = match selected_engine {
            ParserEngine::MinerU => ProcessingStage::MinerUExecuting,
            _ => ProcessingStage::MarkItDownExecuting,
        };
        debug!("The mission phase is updated to: {:?}", stage);
        self.update_task_stage_safe(task_id, stage).await;

        // Parse document - 使用绝对路径
        info!(
            "Start calling the parser, using the absolute path: {}",
            absolute_path
        );
        let parse_result = self
            .dual_parser
            .parse_document_auto(&absolute_path)
            .await
            .with_context(|| "文档解析失败[parse_document_internal]".to_string())?;
        debug!(
            "Parser call completed, content length: {}",
            parse_result.markdown_content.len()
        );

        info!("Document parsed successfully: {}", file_path.display());
        self.update_task_progress_safe(task_id, 80).await;

        // 新增：处理图片上传和路径替换（按任务选择上传后端，一次解析两处复用）
        let resolved_uploader = self.resolve_task_uploader(task_id).await?;
        let final_result = self
            .process_images_with_uploader(task_id, parse_result, &resolved_uploader)
            .await?;

        info!(
            "Image processing and path replacement completed: {}",
            file_path.display()
        );
        self.update_task_progress_safe(task_id, 90).await;

        // 将处理后的新Markdown内容上传到 OSS 或自定义上传后端
        let (oss_markdown_url, oss_object_key) = self
            .upload_processed_markdown_to_oss(
                task_id,
                &final_result.markdown_content,
                &resolved_uploader,
            )
            .await?;

        info!(
            "Markdown content has been uploaded to OSS: {} -> {}",
            oss_object_key, oss_markdown_url
        );
        self.update_task_progress_safe(task_id, 95).await;

        // Complete parsing
        self.update_task_progress_safe(task_id, 100).await;

        // 保存解析结果到数据库（判据来自实际执行上传的 resolved，非重读任务快照）
        self.save_parse_result_to_task(
            task_id,
            &final_result,
            Some((oss_markdown_url, oss_object_key)),
            matches!(resolved_uploader.uploader, TaskUploader::Custom(_)),
        )
        .await?;

        // 计算真实处理耗时
        let processing_time = start_time.elapsed();

        self.update_task_status_safe(task_id, TaskStatus::new_completed(processing_time))
            .await;

        Ok(final_result)
    }

    /// 保存解析结果到任务
    ///
    /// `is_custom_backend` 由调用方从**实际执行上传**的 `resolved.uploader` 推导
    /// （`matches!(resolved.uploader, TaskUploader::Custom(_))`），保证判据与
    /// 实际后端一致——本函数内部不再重读任务快照（消除上传期间的 TOCTOU 窗口）。
    async fn save_parse_result_to_task(
        &self,
        task_id: &str,
        parse_result: &ParseResult,
        oss_markdown_data: Option<(String, String)>,
        is_custom_backend: bool,
    ) -> Result<(), AppError> {
        info!("Start saving parsing results to task: {}", task_id);

        // 获取任务
        let mut task = self
            .task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        // 创建结构化文档
        let structured_doc = self
            .create_structured_document_from_parse_result(task_id, parse_result)
            .await?;

        // 设置结构化文档到任务
        task.set_structured_document(structured_doc)?;

        // 如果有上传数据，设置到任务的oss_data中
        if let Some((oss_url, object_key)) = oss_markdown_data {
            // bucket 标签：自定义后端存 base_url（溯源）；OSS 维持现状取桶名
            let oss_bucket = if is_custom_backend {
                task.upload_config
                    .as_ref()
                    .map(|endpoint| endpoint.base_url.clone())
                    .unwrap_or_default()
            } else if let Some(ref oss_client) = self.oss_client {
                oss_client.get_config().bucket.clone()
            } else {
                "default".to_string() // 如果没有OSS客户端，使用默认值
            };

            let oss_data = crate::models::OssData {
                markdown_url: oss_url,
                markdown_object_key: Some(object_key),
                images: vec![],
                bucket: oss_bucket,
                // 判别字段：数据自描述，消费方（markdown 下载/URL）按此分支而非
                // 依赖任务上的 upload_config
                storage_type: if is_custom_backend {
                    Some(crate::models::StorageType::Custom)
                } else {
                    None
                },
            };
            task.set_oss_data(oss_data)?;
        }

        // 保存任务
        self.task_service.save_task(&task).await?;

        info!(
            "Successfully saved the parsing results to the task: {}",
            task_id
        );
        Ok(())
    }

    /// 从解析结果创建结构化文档
    async fn create_structured_document_from_parse_result(
        &self,
        task_id: &str,
        parse_result: &ParseResult,
    ) -> Result<StructuredDocument, AppError> {
        // 使用Markdown处理器创建结构化文档
        let processor = MarkdownProcessor::new(MarkdownProcessorConfig::default(), None);

        // 直接调用 parse_markdown_with_toc 获取完整的文档结构
        let doc_structure = processor
            .parse_markdown_with_toc(&parse_result.markdown_content)
            .await?;

        // 创建一个新的 StructuredDocument，使用解析出的标题
        let mut structured_doc = StructuredDocument::new(
            task_id.to_string(),
            doc_structure.title, // 使用解析出的标题
        )?;

        // 将解析出的 TOC 项目转换为 StructuredSection 并添加到结构化文档中
        for toc_item in doc_structure.toc {
            // 从 sections HashMap 中获取实际内容，如果没有则使用 content_preview
            let content = doc_structure
                .sections
                .get(&toc_item.id)
                .cloned()
                .or_else(|| toc_item.content_preview.clone())
                .unwrap_or_default();

            let section =
                StructuredSection::new(toc_item.id, toc_item.title, toc_item.level, content)?;
            structured_doc.add_section(section)?;
        }

        // 计算总字数
        structured_doc.calculate_total_word_count();

        Ok(structured_doc)
    }

    /// 使用指定上传器处理图片上传和路径替换（不负责解析任务的上传后端）
    async fn process_images_with_uploader(
        &self,
        task_id: &str,
        parse_result: ParseResult,
        resolved: &ResolvedUploader,
    ) -> AnyhowResult<ParseResult> {
        // 1. 检查上传后端是否可用
        match &resolved.uploader {
            TaskUploader::Disabled => {
                warn!("The OSS client is not configured and image uploading is skipped.");
                return Ok(parse_result);
            }
            TaskUploader::Oss(_) => {
                info!("Using OSS backend for image upload: {}", task_id);
            }
            TaskUploader::Custom(_) => {
                info!(
                    "Using custom upload backend for image upload: {} -> {}",
                    resolved.custom_base_url.as_deref().unwrap_or(""),
                    task_id
                );
            }
        }

        // 2. 扫描 MinerU 输出目录图片：优先 auto/images，再兼容 images
        let mut local_image_paths: Vec<PathBuf> = Vec::new();
        if let Some(output_path) = parse_result.output_dir.as_ref() {
            let collected = Self::collect_local_images_from_output(output_path).await?;
            info!("Scan to {} image files for upload", collected.len());
            local_image_paths.extend(collected);
        }

        // 3. 更新任务状态
        self.update_task_stage_safe(task_id, ProcessingStage::UploadingImages)
            .await;

        // 4. 按后端分派上传，生成文件名->URL 的映射列表
        let image_results: Vec<ImageInfo> = match &resolved.uploader {
            TaskUploader::Disabled => Vec::new(),
            TaskUploader::Oss(client) => {
                let ops = OssImageOps {
                    client,
                    bucket_dir: resolved.bucket_dir.as_deref(),
                    task_id,
                };
                self.upload_images_via(&ops, local_image_paths).await?
            }
            TaskUploader::Custom(client) => {
                let ops = CustomImageOps { client };
                self.upload_images_via(&ops, local_image_paths).await?
            }
        };
        info!(
            "Successfully uploaded {} pictures to the upload backend",
            image_results.len()
        );
        debug!("image_results: {:?}", image_results);

        // 5. 更新任务状态
        self.update_task_stage_safe(task_id, ProcessingStage::ReplacingImagePaths)
            .await;

        // 6. 替换Markdown中的图片路径
        let updated_content = self
            .replace_image_paths_in_markdown(&parse_result.markdown_content, &image_results)
            .await?;

        // 7. 创建新的解析结果（move 而非 clone：markdown_content 已在上方借用结束，
        //    深拷贝整个 Markdown 再立即覆盖是无谓开销）
        let mut final_result = parse_result;
        final_result.markdown_content = updated_content;

        info!(
            "Image path replacement completed, content length: {} characters, {} image paths replaced",
            final_result.markdown_content.len(),
            image_results.len()
        );
        Ok(final_result)
    }

    /// 通过上传操作抽象并发上传图片（后端无关的共用循环）
    ///
    /// 受限并发（`IMAGE_UPLOAD_CONCURRENCY`）：图片密集型 PDF（MinerU 常见
    /// 100-300 张图）串行上传纯等待可达数十秒；结果收集顺序无关
    /// （`replace_image_paths_in_markdown` 按文件名匹配），天然适合并发。
    /// `try_collect` 保持 Fail Fast：首个失败即返回——stream 被 drop 时
    /// `buffer_unordered` 的**在途请求会被取消**（Reqwest 请求中断）。
    async fn upload_images_via(
        &self,
        ops: &dyn ImageUploadOps,
        local_image_paths: Vec<PathBuf>,
    ) -> AnyhowResult<Vec<ImageInfo>> {
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        // 预过滤：无文件名（如路径以 .. 结尾）的跳过（与旧串行版一致）
        let uploadable: Vec<PathBuf> = local_image_paths
            .into_iter()
            .filter(|p| {
                let has_name = p.file_name().and_then(|n| n.to_str()).is_some();
                if !has_name {
                    warn!("Unable to get file name, skipping: {}", p.display());
                }
                has_name
            })
            .collect();

        let image_results: Vec<ImageInfo> = futures::stream::iter(uploadable)
            .map(|path| async move {
                // 直接传播 ops 的 AppError：其错误信息已含文件名/object_key/后端错误
                // 完整上下文；外层再包 context 会令消费端 to_string() 只见外层
                ops.upload_image(&path).await
            })
            .buffer_unordered(Self::IMAGE_UPLOAD_CONCURRENCY)
            .try_collect()
            .await?;

        Ok(image_results)
    }

    /// 同步解析链路：把解析产物图片上传到自定义后端并替换 Markdown 内路径
    ///
    /// Markdown 本身不上传（同步接口在响应体内直接返回内容）。
    /// 不创建任务、不更新任务状态。
    pub async fn upload_images_for_custom_endpoint(
        &self,
        endpoint: &crate::models::UploadEndpoint,
        parse_result: ParseResult,
    ) -> AnyhowResult<ParseResult> {
        let client = ApiFileClient::with_client(endpoint.to_api_config(), self.http_client.clone())
            .map_err(|e| anyhow::anyhow!("自定义上传客户端初始化失败: {e}"))?;
        let ops = CustomImageOps { client: &client };

        let mut local_image_paths: Vec<PathBuf> = Vec::new();
        if let Some(output_path) = parse_result.output_dir.as_ref() {
            let collected = Self::collect_local_images_from_output(output_path).await?;
            info!("Scan to {} image files for upload", collected.len());
            local_image_paths.extend(collected);
        }

        let image_results = self.upload_images_via(&ops, local_image_paths).await?;
        let updated_content = self
            .replace_image_paths_in_markdown(&parse_result.markdown_content, &image_results)
            .await?;

        let mut final_result = parse_result;
        final_result.markdown_content = updated_content;
        Ok(final_result)
    }

    /// 把 Markdown 内容中指向自定义后端的内联图片/链接 URL 批量换签重写
    ///
    /// 基于 [`extract_inline_dest_spans`]（CommonMark 语义，跳过代码块）提取
    /// 后并发换签（失败单张 warn 保留原文）；全部失败或无匹配时**原样返回
    /// 输入 Vec（零拷贝）**。非 UTF-8 内容不处理，原样返回。
    /// 调用方负责整体预算（本方法为尽力而为的增强，不应让下载主体失败）。
    pub async fn sign_embedded_urls(
        &self,
        content: Vec<u8>,
        client: &ApiFileClient,
        base_url: &str,
    ) -> Vec<u8> {
        use futures::StreamExt as _;

        let Ok(text) = std::str::from_utf8(&content) else {
            return content; // 非 UTF-8（二进制）不处理
        };
        let base_prefix = format!("{}/api/f/", base_url.trim_end_matches('/'));

        let spans = extract_inline_dest_spans(text, &base_prefix);
        if spans.is_empty() {
            return content; // 无匹配：零拷贝
        }

        // 去重后并发换签（轻量元数据 GET，并发高于上传）。
        // 超时只包网络阶段（URL 字符串进出），content 所有权不进被取消的
        // future——超预算降级为返回原内容而非丢失内容
        let unique: Vec<String> = spans
            .iter()
            .map(|(dest, _)| dest.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        info!(
            "Exchanging signed URLs for {} embedded backend image/link(s)",
            unique.len()
        );
        let exchange = futures::StreamExt::map(futures::stream::iter(unique), |url| async move {
            client
                .exchange_signed_url(&url)
                .await
                .map(|signed| (url, signed))
        })
        .buffer_unordered(Self::IMAGE_SIGN_CONCURRENCY)
        .collect::<Vec<_>>();
        let results = match tokio::time::timeout(Self::IMAGE_SIGN_BUDGET, exchange).await {
            Ok(results) => results,
            Err(_) => {
                warn!(
                    "Embedded image signed-URL rewriting timed out ({}s), returning unsigned content",
                    Self::IMAGE_SIGN_BUDGET.as_secs()
                );
                return content; // 降级：原内容原样返回（零拷贝）
            }
        };

        let signed: HashMap<String, String> = results
            .into_iter()
            .filter_map(|result| match result {
                Ok(pair) => Some(pair),
                Err(e) => {
                    warn!("Image URL signed-exchange failed, keeping original: {e}");
                    None
                }
            })
            .collect();
        if signed.is_empty() {
            return content; // 全部失败：零拷贝
        }

        rebuild_with_replacements(text, &spans, &signed).into_bytes()
    }

    /// 递归查找 `images` 目录，并收集其下所有图片文件
    async fn collect_local_images_from_output(output_path: &str) -> AnyhowResult<Vec<PathBuf>> {
        debug!(
            "Scan the directory named 'images' under the output directory: {}",
            output_path
        );
        let base_dir = Path::new(output_path);
        let mut found: Vec<PathBuf> = Vec::new();

        if !base_dir.exists() || !base_dir.is_dir() {
            return Ok(found);
        }

        // 第一步：在整个 output_path 下递归查找名为 "images" 的目录
        let mut to_visit: Vec<PathBuf> = vec![base_dir.to_path_buf()];
        let mut images_dirs: Vec<PathBuf> = Vec::new();

        while let Some(dir) = to_visit.pop() {
            let mut rd = fs::read_dir(&dir)
                .await
                .with_context(|| format!("读取目录失败: {}", dir.display()))?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .with_context(|| format!("遍历目录失败: {}", dir.display()))?
            {
                let path = entry.path();
                // 优先通过 metadata 判断类型，避免竞态
                match fs::metadata(&path).await {
                    Ok(meta) if meta.is_dir() => {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str())
                            && name == "images"
                        {
                            images_dirs.push(path.clone());
                        }
                        to_visit.push(path);
                    }
                    _ => {}
                }
            }
        }

        // 第二步：对每个 images 目录进行递归遍历，收集图片文件
        for root in images_dirs {
            let mut stack: Vec<PathBuf> = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                let mut rd = fs::read_dir(&dir)
                    .await
                    .with_context(|| format!("读取图片目录失败: {}", dir.display()))?;
                while let Some(entry) = rd
                    .next_entry()
                    .await
                    .with_context(|| format!("遍历图片目录失败: {}", dir.display()))?
                {
                    let path = entry.path();
                    match fs::metadata(&path).await {
                        Ok(meta) if meta.is_file() => {
                            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                                let ext_lower = ext.to_lowercase();
                                if matches!(
                                    ext_lower.as_str(),
                                    "png"
                                        | "jpg"
                                        | "jpeg"
                                        | "gif"
                                        | "bmp"
                                        | "webp"
                                        | "svg"
                                        | "tiff"
                                        | "tif"
                                ) {
                                    found.push(path);
                                }
                            }
                        }
                        Ok(meta) if meta.is_dir() => {
                            stack.push(path);
                        }
                        _ => {}
                    }
                }
            }
        }

        // 去重（同名同路径不重复）
        found.sort();
        found.dedup();
        Ok(found)
    }

    /// 替换Markdown中的图片路径
    pub async fn replace_image_paths_in_markdown(
        &self,
        markdown_content: &str,
        image_results: &[ImageInfo],
    ) -> AnyhowResult<String> {
        info!("Replace {} image paths in Markdown", image_results.len());

        // 创建文件名到OSS URL的映射表
        let mut filename_to_oss_url = HashMap::new();
        for result in image_results {
            // 使用文件名作为键（用于匹配Markdown中的图片引用）
            filename_to_oss_url.insert(result.original_filename.clone(), result.oss_url.clone());
            // 同时支持以哈希命名的文件匹配（去重后场景，Markdown 里可能是原名，也可能是路径/原名）
            if let Some(stem) = std::path::Path::new(&result.original_filename)
                .file_stem()
                .and_then(|s| s.to_str())
            {
                filename_to_oss_url
                    .entry(stem.to_string())
                    .or_insert(result.oss_url.clone());
            }
        }

        info!("Created {} filename mappings", filename_to_oss_url.len());

        // 使用pulldown-cmark解析Markdown，直接修改Event
        let parser = Parser::new(markdown_content);
        let mut updated_events = Vec::new();
        let mut replacements_count = 0;

        for event in parser {
            match event {
                Event::Start(Tag::Image {
                    dest_url,
                    link_type,
                    id,
                    title,
                }) => {
                    let original_url = dest_url.to_string();
                    if let Some(oss_url) =
                        self.find_oss_url_for_filename(&original_url, &filename_to_oss_url)
                    {
                        // 创建新的Image标签，使用OSS URL
                        let new_tag = Tag::Image {
                            dest_url: pulldown_cmark::CowStr::from(oss_url),
                            link_type,
                            id,
                            title,
                        };
                        updated_events.push(Event::Start(new_tag));
                        replacements_count += 1;
                    } else {
                        // 如果没有找到匹配的OSS URL，保持原样
                        updated_events.push(Event::Start(Tag::Image {
                            dest_url,
                            link_type,
                            id,
                            title,
                        }));
                    }
                }
                Event::End(TagEnd::Image) => {
                    updated_events.push(Event::End(TagEnd::Image));
                }
                _ => {
                    updated_events.push(event);
                }
            }
        }

        // 使用pulldown-cmark-to-cmark将修改后的Event转换回Markdown
        let mut output = String::new();
        cmark(updated_events.into_iter(), &mut output)?;

        info!(
            "Image path replacement completed, {} images processed, {} paths replaced",
            image_results.len(),
            replacements_count
        );
        Ok(output)
    }

    /// 查找匹配的OSS URL（通过文件名匹配）
    fn find_oss_url_for_filename(
        &self,
        image_path: &str,
        filename_to_oss_url: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        // 从 Markdown 图片路径中提取文件名进行匹配
        if let Some(filename) = Path::new(image_path).file_name()
            && let Some(filename_str) = filename.to_str()
            && let Some(oss_url) = filename_to_oss_url.get(filename_str)
        {
            return Some(oss_url.clone());
        }

        // 如果没有找到匹配，返回None
        None
    }

    /// 从URL解析文档 - Enhanced with proper resource management
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn parse_document_from_url(
        &self,
        task_id: &str,
        url: &str,
    ) -> AnyhowResult<ParseResult> {
        info!("Parse document from URL: {}", url);

        // Update task status
        self.update_task_stage_safe(task_id, crate::models::ProcessingStage::DownloadingDocument)
            .await;

        // 从URL提取文件名，先去掉查询参数
        let url_without_query = url.split('?').next().unwrap_or(url);
        let filename = url_without_query
            .split('/')
            .next_back()
            .unwrap_or("downloaded_file")
            .to_string();
        debug!("File name extracted from URL: {}", filename);
        debug!("URL (remove query parameters): {}", url_without_query);

        // 创建基于 taskId 的临时文件路径
        let file_path = self.create_temp_file_for_task("./temp", task_id, &filename)?;
        debug!("Created temporary file path: {}", file_path);

        // 下载文件到指定路径
        self.download_file_to_path(url, &file_path)
            .await
            .with_context(|| format!("下载文件失败: {url}"))?;
        debug!("File download completed: {}", file_path);

        // 将本地文件路径写回任务，便于后续清理临时文件
        if let Err(e) = self
            .task_service
            .update_task_source_info(
                task_id,
                Some(file_path.clone()),
                Some(url.to_string()),
                Some(filename.clone()),
            )
            .await
        {
            warn!(
                "Failed to update task local path: task_id={}, error={}",
                task_id, e
            );
        } else {
            debug!(
                "Updated task local path: task_id={}, path={}",
                task_id, file_path
            );
        }

        // Update progress
        self.update_task_progress_safe(task_id, 30).await;

        // Parse document
        debug!("Start parsing the document: {}", file_path);

        self.parse_document(task_id, &file_path).await
    }

    /// 生成结构化文档 - Enhanced with proper async patterns and error handling
    #[instrument(skip(self, markdown_content), fields(task_id = %task_id, content_length = markdown_content.len()))]
    pub async fn generate_structured_document(
        &self,
        task_id: &str,
        markdown_content: &str,
        title: Option<String>,
    ) -> AnyhowResult<StructuredDocument> {
        info!(
            "Generate structured document, content length: {} characters",
            markdown_content.len()
        );

        // Update task status
        self.update_task_stage_safe(task_id, crate::models::ProcessingStage::ProcessingMarkdown)
            .await;

        // Validate input
        if markdown_content.is_empty() {
            return Err(anyhow::anyhow!("Markdown内容为空"));
        }

        // Process with timeout to prevent hanging
        let result = timeout(Duration::from_secs(30), async {
            self.generate_structured_document_internal(markdown_content, title)
                .await
        })
        .await;

        match result {
            Ok(doc) => {
                info!(
                    "Structured document generation is completed, number of chapters: {}",
                    doc.as_ref().map(|d| d.total_sections).unwrap_or(0)
                );
                doc
            }
            Err(_) => {
                let error_msg = "结构化文档生成超时";
                error!("{}", error_msg);
                Err(anyhow::anyhow!(error_msg))
            }
        }
    }

    /// Internal structured document generation
    async fn generate_structured_document_internal(
        &self,
        markdown_content: &str,
        title: Option<String>,
    ) -> AnyhowResult<StructuredDocument> {
        // Use read lock for concurrent access to markdown processor
        let processor = self.markdown_processor.read().await;

        // Process markdown content
        let doc = processor.process_markdown(markdown_content).await?;

        // 创建一个新的 StructuredDocument
        let mut structured_doc = StructuredDocument::new(
            "default_task".to_string(),
            doc, // 使用返回的标题作为文档标题
        )?;

        // 设置自定义标题
        if let Some(custom_title) = title {
            structured_doc.document_title = custom_title;
        }

        // 计算总字数
        structured_doc.calculate_total_word_count();

        Ok(structured_doc)
    }

    /// Safe task update methods - handle errors gracefully without failing the main operation
    async fn update_task_stage_safe(&self, task_id: &str, stage: crate::models::ProcessingStage) {
        if let Err(e) = self.task_service.update_task_stage(task_id, stage).await {
            warn!("Failed to update task stage for {}: {}", task_id, e);
        }
    }

    async fn update_task_progress_safe(&self, task_id: &str, progress: u32) {
        if let Err(e) = self
            .task_service
            .update_task_progress(task_id, progress)
            .await
        {
            warn!("Failed to update task progress for {}: {}", task_id, e);
        }
    }

    async fn update_task_file_info_safe(
        &self,
        task_id: &str,
        file_size: Option<u64>,
        mime_type: Option<String>,
    ) {
        if let Err(e) = self
            .task_service
            .set_task_file_info(task_id, file_size, mime_type)
            .await
        {
            warn!("Failed to update task file info for {}: {}", task_id, e);
        }
    }

    async fn update_task_parser_engine_safe(&self, task_id: &str, engine: ParserEngine) {
        if let Err(e) = self
            .task_service
            .set_task_parser_engine(task_id, engine)
            .await
        {
            warn!("Failed to update task parser engine for {}: {}", task_id, e);
        }
    }

    async fn update_task_document_format_safe(&self, task_id: &str, format: DocumentFormat) {
        if let Err(e) = self
            .task_service
            .update_task(task_id, None, None, format)
            .await
        {
            warn!(
                "Failed to update task document format for {}: {}",
                task_id, e
            );
        }
    }

    async fn update_task_status_safe(&self, task_id: &str, status: TaskStatus) {
        if let Err(e) = self.task_service.update_task_status(task_id, status).await {
            warn!("Failed to update task status for {}: {}", task_id, e);
        }
    }

    /// 检测文件MIME类型 - Enhanced async version
    async fn detect_mime_type_async(&self, file_path: &Path) -> AnyhowResult<String> {
        let extension = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        let mime_type = match extension.to_lowercase().as_str() {
            "pdf" => "application/pdf",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "ppt" => "application/vnd.ms-powerpoint",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "txt" => "text/plain",
            "md" => "text/markdown",
            "html" | "htm" => "text/html",
            "xml" => "application/xml",
            "json" => "application/json",
            "csv" => "text/csv",
            "rtf" => "application/rtf",
            "odt" => "application/vnd.oasis.opendocument.text",
            "ods" => "application/vnd.oasis.opendocument.spreadsheet",
            "odp" => "application/vnd.oasis.opendocument.presentation",
            _ => "application/octet-stream",
        };

        Ok(mime_type.to_string())
    }

    /// 基于 taskId 创建临时文件路径
    fn create_temp_file_for_task(
        &self,
        temp_dir: &str,
        task_id: &str,
        filename: &str,
    ) -> Result<String, AppError> {
        use std::path::Path;

        debug!(
            "Create a temporary file - input parameters: temp_dir={}, task_id={}, filename={}",
            temp_dir, task_id, filename
        );

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

        debug!("Extracted file extension: {}", extension);

        // 从文件名中移除扩展名，然后清理文件名
        let stem = Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");

        let clean_stem = stem
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect::<String>();

        debug!("Cleaned file name body: {}", clean_stem);

        // 使用 taskId 作为文件名的一部分，确保唯一性和可追踪性
        let task_filename = format!("task_{task_id}_{clean_stem}.{extension}");
        let file_path = temp_path.join(task_filename);

        // 验证路径安全性（防止路径遍历）
        if !file_path.starts_with(temp_path) {
            return Err(AppError::File("文件路径不安全".to_string()));
        }

        let final_path = file_path.to_string_lossy().to_string();
        debug!("Create temporary file - final path: {}", final_path);

        Ok(final_path)
    }

    /// 下载文件到指定路径
    async fn download_file_to_path(&self, url: &str, file_path: &str) -> Result<(), AppError> {
        // URL 验证 - 只验证格式，不改变编码状态
        crate::handlers::validation::RequestValidator::validate_url_format(url)?;

        // 发起HTTP请求
        let response = self
            .http_client
            .get(url)
            .timeout(std::time::Duration::from_secs(300)) // 5分钟超时
            .send()
            .await
            .map_err(|e| AppError::Network(format!("HTTP请求失败: {e}")))?;

        if !response.status().is_success() {
            return Err(AppError::Network(format!(
                "HTTP请求失败，状态码: {}",
                response.status()
            )));
        }

        // 创建文件并写入内容
        let mut file = File::create(file_path)
            .await
            .map_err(|e| AppError::File(format!("创建文件失败: {e}")))?;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AppError::Network(format!("读取响应数据失败: {e}")))?;

            file.write_all(&chunk)
                .await
                .map_err(|e| AppError::File(format!("写入文件失败: {e}")))?;
        }

        file.flush()
            .await
            .map_err(|e| AppError::File(format!("刷新文件失败: {e}")))?;

        Ok(())
    }

    /// 获取支持的格式
    pub fn get_supported_formats(&self) -> Vec<DocumentFormat> {
        crate::parsers::DualEngineParser::get_supported_formats()
    }

    /// 检查解析器健康状态 - Enhanced with proper error handling
    #[instrument(skip(self))]
    pub async fn check_parser_health(
        &self,
    ) -> AnyhowResult<std::collections::HashMap<String, bool>> {
        debug!("Check parser health status");

        let health_check_result = timeout(Duration::from_secs(60), async {
            self.dual_parser.health_check().await
        })
        .await;

        let mut health_status = std::collections::HashMap::new();

        match health_check_result {
            Ok(Ok(_)) => {
                health_status.insert("parser_healthy".to_string(), true);
                health_status.insert("mineru_available".to_string(), true);
                health_status.insert("markitdown_available".to_string(), true);
                info!("Parser health check passed");
            }
            Ok(Err(e)) => {
                health_status.insert("parser_healthy".to_string(), false);
                health_status.insert("error_message".to_string(), false);
                warn!("Parser health check failed: {}", e);
            }
            Err(_) => {
                health_status.insert("parser_healthy".to_string(), false);
                health_status.insert("timeout".to_string(), true);
                warn!("Resolver health check timeout");
            }
        }

        Ok(health_status)
    }

    /// 获取解析器统计信息
    pub fn get_parser_stats(&self) -> crate::parsers::ParserStats {
        self.dual_parser.get_parser_stats()
    }

    /// 清理Markdown处理器缓存 - Enhanced with proper async patterns
    #[instrument(skip(self))]
    pub async fn clear_processor_cache(&self) -> AnyhowResult<()> {
        debug!("Clean Markdown processor cache");

        // Use write lock to ensure exclusive access during cache clearing
        let processor = self.markdown_processor.write().await;
        processor.clear_cache().await;

        info!("Markdown processor cache cleared");
        Ok(())
    }

    /// 生成结构化文档（无任务ID，同步）- Enhanced with proper async patterns
    #[instrument(skip(self, markdown_content), fields(content_length = markdown_content.len()))]
    pub async fn generate_structured_document_simple(
        &self,
        markdown_content: &str,
    ) -> AnyhowResult<StructuredDocument> {
        debug!("Generate simple structured documents");

        if markdown_content.is_empty() {
            return Err(anyhow::anyhow!("Markdown内容为空"));
        }

        // Use read lock for concurrent access
        let processor = self.markdown_processor.read().await;

        // Process with timeout to get complete document structure
        let result = timeout(
            Duration::from_secs(30),
            processor.parse_markdown_with_toc(markdown_content),
        )
        .await;

        match result {
            Ok(Ok(doc_structure)) => {
                // 创建一个新的 StructuredDocument
                let mut structured_doc = StructuredDocument::new(
                    "default_task".to_string(),
                    doc_structure.title, // 使用解析出的标题
                )?;

                // 将解析出的 TOC 项目转换为 StructuredSection 并添加到结构化文档中
                for toc_item in doc_structure.toc {
                    // 从 sections HashMap 中获取实际内容，如果没有则使用 content_preview
                    let content = doc_structure
                        .sections
                        .get(&toc_item.id)
                        .cloned()
                        .or_else(|| toc_item.content_preview.clone())
                        .unwrap_or_default();

                    let section = StructuredSection::new(
                        toc_item.id,
                        toc_item.title,
                        toc_item.level,
                        content,
                    )?;
                    structured_doc.add_section(section)?;
                }

                // 计算总字数
                structured_doc.calculate_total_word_count();

                info!(
                    "Simple structured document generation is completed, number of chapters: {}",
                    structured_doc.total_sections
                );

                Ok(structured_doc)
            }
            Ok(Err(e)) => {
                error!("Structured document generation failed: {}", e);
                Err(anyhow::anyhow!("结构化文档生成失败: {}", e))
            }
            Err(_) => {
                error!("Structured document generation timeout");
                Err(anyhow::anyhow!("结构化文档生成超时"))
            }
        }
    }

    /// 获取处理器缓存统计 - Enhanced with proper async patterns
    #[instrument(skip(self))]
    pub async fn get_processor_cache_stats(&self) -> CacheStatistics {
        let processor = self.markdown_processor.read().await;
        processor.get_cache_stats().await
    }

    /// 创建文件上传任务 - Enhanced with proper validation and error handling
    #[instrument(skip(self), fields(filename = %filename, file_size = file_size))]
    pub async fn create_upload_task(
        &self,
        file_path: &str,
        filename: &str,
        file_size: u64,
    ) -> AnyhowResult<String> {
        info!(
            "Create file upload task: {} (size: {} bytes)",
            filename, file_size
        );

        // Validate file size
        let global_config = GlobalFileSizeConfig::new();
        if file_size > global_config.max_file_size.bytes() {
            return Err(anyhow::anyhow!(
                "文件大小超过限制: {} > {} bytes",
                file_size,
                global_config.max_file_size.bytes()
            ));
        }

        // Validate file exists
        let file_path_obj = Path::new(file_path);
        if !file_path_obj.exists() {
            return Err(anyhow::anyhow!("文件不存在: {}", file_path));
        }

        // Create task
        let task = self
            .task_service
            .create_task(
                SourceType::Upload,
                Some(filename.to_string()),
                Some(filename.to_string()),
                None,
            )
            .await
            .context("创建任务失败")?;

        Ok(task.id)
    }

    /// 创建URL下载任务
    pub async fn create_url_task(&self, url: &str, filename: &str) -> Result<String, AppError> {
        log::info!("Create URL download task: {url} -> {filename}");

        // 创建任务：URL 作为 source_url，原始文件名保留
        let task = self
            .task_service
            .create_task(
                SourceType::Url,
                Some(url.to_string()),
                Some(filename.to_string()),
                None,
            )
            .await
            .map_err(|e| AppError::Task(format!("创建任务失败: {e}")))?;

        Ok(task.id)
    }

    /// 获取任务状态
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<crate::models::DocumentTask, AppError> {
        log::debug!("Get task status: {task_id}");

        self.task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))
    }

    /// 将处理后的Markdown内容上传到 OSS 或自定义上传后端
    ///
    /// 返回 `(url, key)`：OSS 时 key 为对象键；自定义后端时为服务端返回的文件 key。
    async fn upload_processed_markdown_to_oss(
        &self,
        task_id: &str,
        markdown_content: &str,
        resolved: &ResolvedUploader,
    ) -> AnyhowResult<(String, String)> {
        match &resolved.uploader {
            TaskUploader::Disabled => Err(anyhow::anyhow!("OSS客户端未配置，无法上传Markdown内容")),
            TaskUploader::Oss(oss_client) => {
                info!(
                    "Start uploading the processed Markdown content to OSS: {}",
                    task_id
                );
                let object_key = build_markdown_object_key(resolved.bucket_dir.as_deref(), task_id);
                let upload_result = oss_client
                    .upload_content(
                        markdown_content.as_bytes(),
                        &object_key,
                        Some("text/markdown"),
                    )
                    .await
                    .with_context(|| format!("上传Markdown内容到OSS失败: {object_key}"))?;
                info!(
                    "Markdown content uploaded successfully: {} -> {}",
                    object_key, upload_result
                );
                Ok((upload_result, object_key))
            }
            TaskUploader::Custom(client) => {
                info!(
                    "Start uploading the processed Markdown content to custom backend: {}",
                    task_id
                );
                let uploaded = client
                    .upload_bytes(
                        markdown_content.as_bytes(),
                        &format!("{task_id}.md"),
                        Some("text/markdown"),
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("上传Markdown内容到自定义后端失败 ({}): {e}", task_id)
                    })?;
                info!(
                    "Markdown content uploaded successfully to custom backend: {} -> {}",
                    uploaded.key, uploaded.url
                );
                Ok((uploaded.url, uploaded.key))
            }
        }
    }

    /// 解析任务的上传后端：`task.upload_config` 优先（自定义 REST 后端），否则 OSS
    ///
    /// 同时一次性带出 OSS 分支需要的 `bucket_dir`（替代旧逻辑每张图片一次 sled 读，
    /// 生成的对象键格式不变）。
    async fn resolve_task_uploader(&self, task_id: &str) -> AnyhowResult<ResolvedUploader> {
        let task = self
            .task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {task_id}"))?;

        match task.upload_config {
            Some(endpoint) => {
                let client =
                    ApiFileClient::with_client(endpoint.to_api_config(), self.http_client.clone())
                        .map_err(|e| anyhow::anyhow!("自定义上传客户端初始化失败: {e}"))?;
                Ok(ResolvedUploader {
                    custom_base_url: Some(endpoint.base_url.clone()),
                    uploader: TaskUploader::Custom(client),
                    bucket_dir: None, // 自定义后端服务端自管 key，忽略 bucket_dir
                })
            }
            None => Ok(ResolvedUploader {
                custom_base_url: None,
                uploader: match &self.oss_client {
                    Some(client) => TaskUploader::Oss(client.clone()),
                    None => TaskUploader::Disabled,
                },
                bucket_dir: task
                    .bucket_dir
                    .as_ref()
                    .map(|dir| dir.trim_matches('/').to_string()),
            }),
        }
    }
}

/// 任务产物上传器：按 `task.upload_config` 选择后端（编译期穷举分派）
enum TaskUploader {
    /// 阿里云 OSS（现状默认）
    Oss(Arc<dyn oss_client::OssClientTrait + Send + Sync>),
    /// 自定义上传后端（nuwax 风格 REST）
    Custom(ApiFileClient),
    /// 无可用上传后端（OSS 未配置且任务未指定自定义端点）
    Disabled,
}

/// 一次解析出的上传上下文
struct ResolvedUploader {
    uploader: TaskUploader,
    /// 仅 OSS 分支使用：对象键前缀子目录（已 trim `/`）
    bucket_dir: Option<String>,
    /// 仅 Custom 分支使用：后端基地址（日志溯源）
    custom_base_url: Option<String>,
}

/// 图片上传操作抽象：OSS 与自定义后端各一个实现
#[async_trait]
trait ImageUploadOps: Sync {
    /// 上传单张图片并返回填好后端真实值的 [`ImageInfo`]
    async fn upload_image(&self, local_path: &Path) -> Result<ImageInfo, AppError>;
}

/// OSS 后端实现：行为与旧版完全一致（sha256 对象键去重 + bucket_dir 前缀）
struct OssImageOps<'a> {
    client: &'a Arc<dyn oss_client::OssClientTrait + Send + Sync>,
    bucket_dir: Option<&'a str>,
    task_id: &'a str,
}

#[async_trait]
impl ImageUploadOps for OssImageOps<'_> {
    async fn upload_image(&self, local_path: &Path) -> Result<ImageInfo, AppError> {
        let original_filename = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        // 计算文件SHA-256哈希作为对象键名称，确保相同图片去重
        let hash_hex = compute_file_sha256_hex(local_path).await.map_err(|e| {
            AppError::File(format!("计算图片哈希失败: {}: {}", local_path.display(), e))
        })?;

        // 保留原始扩展名
        let ext_lower = local_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let ext_suffix = if ext_lower.is_empty() {
            String::new()
        } else {
            format!(".{ext_lower}")
        };

        let object_key =
            build_image_object_key(self.bucket_dir, self.task_id, &hash_hex, &ext_suffix);

        let oss_url = self
            .client
            .upload_file(local_path.to_string_lossy().as_ref(), &object_key)
            .await
            .map_err(|e| {
                AppError::Oss(format!(
                    "上传图片失败: {original_filename} -> {object_key}: {e}"
                ))
            })?;

        let file_size = read_file_size(local_path).await?;
        let mime_type = oss_client::detect_mime_type(local_path.to_string_lossy().as_ref());

        Ok(ImageInfo::with_full_info(
            local_path.to_string_lossy().to_string(),
            original_filename,
            object_key,
            oss_url,
            file_size,
            mime_type,
        ))
    }
}

/// 自定义后端实现：服务端自管 key（忽略哈希去重），上传后端返回的 url/key 为准
struct CustomImageOps<'a> {
    client: &'a ApiFileClient,
}

#[async_trait]
impl ImageUploadOps for CustomImageOps<'_> {
    async fn upload_image(&self, local_path: &Path) -> Result<ImageInfo, AppError> {
        let original_filename = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let uploaded = self
            .client
            .upload_file_by_path(
                local_path.to_string_lossy().as_ref(),
                Some(&original_filename),
            )
            .await
            .map_err(|e| {
                AppError::Oss(format!(
                    "上传图片到自定义后端失败: {original_filename}: {e}"
                ))
            })?;

        let file_size = read_file_size(local_path).await?;
        let mime_type = oss_client::detect_mime_type(local_path.to_string_lossy().as_ref());

        Ok(ImageInfo::with_full_info(
            local_path.to_string_lossy().to_string(),
            original_filename,
            uploaded.key,
            uploaded.url,
            file_size,
            mime_type,
        ))
    }
}

/// 读取文件大小（两个后端共用的元数据读取）
async fn read_file_size(local_path: &Path) -> Result<u64, AppError> {
    let metadata = fs::metadata(local_path).await.map_err(|e| {
        AppError::File(format!("读取文件信息失败: {}: {}", local_path.display(), e))
    })?;
    Ok(metadata.len())
}

/// 提取匹配后端前缀的内联图片/链接 dest 及其在原文中的字节 span
///
/// 使用 pulldown-cmark 事件流（offset 迭代器）——**跳过 code span 与 fenced
/// code block 内的文本**（解析器将二者折叠为 Code/CodeBlock 事件，不产生
/// Image/Link 事件），因此代码示例里的后端 URL 不会被误改写。仅处理
/// `LinkType::Inline`（dest 原文保证在事件 span 内；引用式链接的 dest 在
/// 文档别处，跳过）。span 内用 `find(dest)` 相对定位原文子串，转义/实体
/// 导致原文与 unescape 后的 dest 不一致时 find 失败即安全跳过。
/// 返回值按 span 起点有序（offset 迭代器天然有序）。
fn extract_inline_dest_spans(
    content: &str,
    base_prefix: &str,
) -> Vec<(String, std::ops::Range<usize>)> {
    let mut spans = Vec::new();
    for (event, range) in Parser::new(content).into_offset_iter() {
        let (dest_url, link_type) = match event {
            Event::Start(Tag::Image {
                dest_url,
                link_type,
                ..
            }) => (dest_url, link_type),
            Event::Start(Tag::Link {
                dest_url,
                link_type,
                ..
            }) => (dest_url, link_type),
            _ => continue,
        };
        if link_type != LinkType::Inline {
            continue;
        }
        let dest = dest_url.to_string();
        if !dest.starts_with(base_prefix) {
            continue;
        }
        // 在事件 span 内定位 dest 原文子串（span = `![alt](dest "title")` 整体）
        if let Some(local) = content[range.clone()].find(&dest) {
            let start = range.start + local;
            let end = start + dest.len();
            spans.push((dest, start..end));
        }
    }
    spans
}

/// 按 span 单趟重建内容（O(M) 一次分配）
///
/// `signed` 缺失的 span（单张换签失败）保留原文；spans 必须按起点有序。
fn rebuild_with_replacements(
    content: &str,
    spans: &[(String, std::ops::Range<usize>)],
    signed: &HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut prev = 0;
    for (dest, range) in spans {
        out.push_str(&content[prev..range.start]);
        match signed.get(dest) {
            Some(replacement) => out.push_str(replacement),
            None => out.push_str(&content[range.clone()]),
        }
        prev = range.end;
    }
    out.push_str(&content[prev..]);
    out
}

/// 计算文件的 SHA-256 哈希（hex）
async fn compute_file_sha256_hex(path: &std::path::Path) -> AnyhowResult<String> {
    let mut file = File::open(path)
        .await
        .with_context(|| format!("打开文件失败用于计算哈希: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 64]; // 64KB 缓冲
    loop {
        let n = file
            .read(&mut buffer)
            .await
            .with_context(|| format!("读取文件失败用于计算哈希: {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let digest = hasher.finalize();
    Ok(hex::encode(digest))
}

/// 生成 Markdown 的对象键：`[bucket_dir/]processed_markdown/<task_id>/<task_id>.md`
///
/// 与旧 `generate_unified_oss_object_key` 的拼接逻辑逐字符一致。
fn build_markdown_object_key(bucket_dir: Option<&str>, task_id: &str) -> String {
    match bucket_dir
        .map(|d| d.trim_matches('/'))
        .filter(|d| !d.is_empty())
    {
        Some(dir) => format!("{dir}/processed_markdown/{task_id}/{task_id}.md"),
        None => format!("processed_markdown/{task_id}/{task_id}.md"),
    }
}

/// 生成图片的对象键：`[bucket_dir/]parsed_images/<task_id>/sha256/<hash><ext>`
///
/// 与旧 `generate_unified_oss_object_key` 的拼接逻辑逐字符一致。
fn build_image_object_key(
    bucket_dir: Option<&str>,
    task_id: &str,
    hash_hex: &str,
    ext_suffix: &str,
) -> String {
    match bucket_dir
        .map(|d| d.trim_matches('/'))
        .filter(|d| !d.is_empty())
    {
        Some(dir) => format!("{dir}/parsed_images/{task_id}/sha256/{hash_hex}{ext_suffix}"),
        None => format!("parsed_images/{task_id}/sha256/{hash_hex}{ext_suffix}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_collect_local_images_from_output_auto_images() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("output");
        let auto_images = output_dir.join("auto").join("images");
        tokio::fs::create_dir_all(&auto_images).await.unwrap();

        // 创建图片与非图片文件
        let img1 = auto_images.join("a.png");
        let img2 = auto_images.join("b.JPG");
        let not_img = auto_images.join("c.txt");
        tokio::fs::write(&img1, b"fake").await.unwrap();
        tokio::fs::write(&img2, b"fake").await.unwrap();
        tokio::fs::write(&not_img, b"nope").await.unwrap();

        // 扫描
        let collected = DocumentService::collect_local_images_from_output(
            output_dir.to_string_lossy().as_ref(),
        )
        .await
        .unwrap();

        // 断言只包含两张图片
        let mut names: Vec<String> = collected
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();

        assert_eq!(names.len(), 2);
        assert_eq!(names[0], "a.png");
        assert_eq!(names[1], "b.JPG");
    }

    /// 对象键格式回归锚点：与重构前 generate_unified_oss_object_key 输出逐字符一致
    #[test]
    fn test_build_object_key_formats_match_legacy() {
        // 无 bucket_dir
        assert_eq!(
            build_markdown_object_key(None, "task_123"),
            "processed_markdown/task_123/task_123.md"
        );
        assert_eq!(
            build_image_object_key(None, "task_123", "abc123", ".jpg"),
            "parsed_images/task_123/sha256/abc123.jpg"
        );

        // 有 bucket_dir
        assert_eq!(
            build_markdown_object_key(Some("my_bucket"), "task_456"),
            "my_bucket/processed_markdown/task_456/task_456.md"
        );
        assert_eq!(
            build_image_object_key(Some("my_bucket"), "task_456", "def456", ".png"),
            "my_bucket/parsed_images/task_456/sha256/def456.png"
        );

        // bucket_dir 带首尾斜杠 → trim 后作为前缀
        assert_eq!(
            build_markdown_object_key(Some("/proj/docs/"), "task_1"),
            "proj/docs/processed_markdown/task_1/task_1.md"
        );

        // bucket_dir trim 后为空 → 不加前缀（与旧逻辑一致）
        assert_eq!(
            build_markdown_object_key(Some("/"), "task_1"),
            "processed_markdown/task_1/task_1.md"
        );

        // 无扩展名图片
        assert_eq!(
            build_image_object_key(None, "task_1", "hash", ""),
            "parsed_images/task_1/sha256/hash"
        );
    }

    /// 图片上传共用循环：mock 上传操作验证调用与结果收集
    struct CountingMockOps {
        fail_on: Option<String>,
    }

    #[async_trait]
    impl ImageUploadOps for CountingMockOps {
        async fn upload_image(&self, local_path: &Path) -> Result<ImageInfo, AppError> {
            let name = local_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if self.fail_on.as_deref() == Some(name.as_str()) {
                return Err(AppError::Oss(format!("mock failure: {name}")));
            }
            Ok(ImageInfo::with_full_info(
                local_path.to_string_lossy().to_string(),
                name.clone(),
                format!("key/{name}"),
                format!("https://mock.example.com/{name}"),
                1,
                "image/png".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn test_upload_images_via_collects_results() {
        let temp_dir = TempDir::new().unwrap();
        let a = temp_dir.path().join("a.png");
        let b = temp_dir.path().join("b.png");
        tokio::fs::write(&a, b"x").await.unwrap();
        tokio::fs::write(&b, b"y").await.unwrap();

        let service = crate::tests::test_helpers::create_test_app_state().await;
        let ops = CountingMockOps { fail_on: None };
        let results = service
            .document_service
            .upload_images_via(&ops, vec![a, b])
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].oss_url, "https://mock.example.com/a.png");

        // 失败路径：单张失败整批报错（Fail Fast）
        let b2 = temp_dir.path().join("b.png");
        let ops = CountingMockOps {
            fail_on: Some("b.png".to_string()),
        };
        let err = service
            .document_service
            .upload_images_via(&ops, vec![b2])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("b.png"));
    }

    // ===== 内嵌 URL 换签（CommonMark 语义） =====

    const SIGN_TEST_PREFIX: &str = "https://agent.example.com/api/f/";

    #[test]
    fn extract_skips_urls_inside_code_blocks() {
        // fenced code block 内的示例 URL 不收集（手写扫描器会误伤的场景）
        let content = "# t\n\n![ok](https://agent.example.com/api/f/s3/a.png)\n\n```rust\n// demo: ![demo](https://agent.example.com/api/f/s3/b.png)\n```\n\ninline `![x](https://agent.example.com/api/f/s3/c.png)` code\n";
        let spans = extract_inline_dest_spans(content, SIGN_TEST_PREFIX);
        let urls: Vec<&str> = spans.iter().map(|(d, _)| d.as_str()).collect();
        assert_eq!(urls, vec!["https://agent.example.com/api/f/s3/a.png"]);
    }

    #[test]
    fn extract_handles_title_multiple_and_foreign_prefix() {
        let content = "![a](https://agent.example.com/api/f/s3/x.png \"fig.1\")\n[link](https://agent.example.com/api/f/s3/x.png)\n![ext](https://other.example.com/api/f/s3/y.png)\n";
        let spans = extract_inline_dest_spans(content, SIGN_TEST_PREFIX);
        // 带 title 的图片 + 同 URL 的链接（Link 事件）都收集；外站前缀跳过
        assert_eq!(spans.len(), 2);
        assert!(
            spans
                .iter()
                .all(|(d, _)| d == "https://agent.example.com/api/f/s3/x.png")
        );
        // span 精确指向 URL 子串
        for (dest, range) in &spans {
            assert_eq!(&content[range.clone()], dest);
        }
    }

    #[test]
    fn extract_skips_reference_links() {
        // 引用式链接 dest 在文档别处定义，不在事件 span 内——安全跳过
        let content = "[ref]: https://agent.example.com/api/f/s3/r.png\n\n![alt][ref]\n";
        let spans = extract_inline_dest_spans(content, SIGN_TEST_PREFIX);
        assert!(spans.is_empty());
    }

    #[test]
    fn rebuild_replaces_only_signed_and_keeps_rest() {
        let content = "pre ![a](P/U1.png) mid [b](P/U2.png) post ![c](P/U3.png) end";
        // span 由 extract 计算（真实偏移），保证 rebuild 消费的就是生产路径的输入
        let spans = extract_inline_dest_spans(content, "P/");
        assert_eq!(spans.len(), 3);
        let mut signed = HashMap::new();
        signed.insert("P/U1.png".to_string(), "S1".to_string());
        signed.insert("P/U3.png".to_string(), "S3".to_string()); // U2 换签失败：保留原文
        let out = rebuild_with_replacements(content, &spans, &signed);
        assert_eq!(out, "pre ![a](S1) mid [b](P/U2.png) post ![c](S3) end");
    }
}
