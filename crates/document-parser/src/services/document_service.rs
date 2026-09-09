use anyhow::{Context, Result as AnyhowResult};
use oss_client::ApiFileClient;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use futures_util::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::ProcessingStage;
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
use crate::services::upload_pipeline::{ResolvedUploader, TaskUploader};

/// 图片上传/路径替换/换签族方法（`impl DocumentService` 的子模块拆分）。
mod images;

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
    pub(crate) async fn resolve_task_uploader(
        &self,
        task_id: &str,
    ) -> AnyhowResult<ResolvedUploader> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImageInfo;
    use crate::services::upload_pipeline::{
        ImageUploadOps, build_image_object_key, build_markdown_object_key,
        extract_inline_dest_spans, rebuild_with_replacements,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;
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
    fn extract_nested_image_in_link_ordered_and_no_panic() {
        // 可点击缩略图（badge）形态：嵌套使事件序与位置序相反——
        // 修复前 spans 降序导致 rebuild 切片 panic，现应按位置序安全重建
        let content = "[![thumb](https://agent.example.com/api/f/s3/a.png)](https://agent.example.com/api/f/s3/b.png)";
        let spans = extract_inline_dest_spans(content, SIGN_TEST_PREFIX);
        assert_eq!(spans.len(), 2, "嵌套两层都应收集");
        // 严格递增且不重叠
        assert!(spans[0].1.start < spans[1].1.start && spans[0].1.end <= spans[1].1.start);

        let mut signed = HashMap::new();
        signed.insert(
            "https://agent.example.com/api/f/s3/a.png".to_string(),
            "SA".to_string(),
        );
        signed.insert(
            "https://agent.example.com/api/f/s3/b.png".to_string(),
            "SB".to_string(),
        );
        let out = rebuild_with_replacements(content, &spans, &signed);
        assert_eq!(
            out, "[![thumb](SA)](SB)",
            "两个 href 都应被替换（嵌套顺序无关）"
        );
    }

    #[test]
    fn extract_alt_echoing_url_replaces_href_not_alt() {
        // alt 回显 URL（LLM 生成内容常见）：定位必须命中 href 而非 alt 文本
        let content =
            "![https://agent.example.com/api/f/s3/x.png](https://agent.example.com/api/f/s3/x.png)";
        let spans = extract_inline_dest_spans(content, SIGN_TEST_PREFIX);
        assert_eq!(spans.len(), 1);
        let (dest, range) = &spans[0];
        assert_eq!(dest, "https://agent.example.com/api/f/s3/x.png");
        // span 起点应在 href 区段（`](` 之后），而非 alt 内的首次出现
        assert_eq!(&content[range.clone()], dest, "span 应精确覆盖 href 子串");
        let mut signed = HashMap::new();
        signed.insert(dest.clone(), "SIGNED".to_string());
        let out = rebuild_with_replacements(content, &spans, &signed);
        assert_eq!(out, "![https://agent.example.com/api/f/s3/x.png](SIGNED)");

        // 裸链接惯用法 [url](url)
        let content2 = "[P/y.png](P/y.png)";
        let spans2 = extract_inline_dest_spans(content2, "P/");
        assert_eq!(spans2.len(), 1);
        assert_eq!(&content2[spans2[0].1.clone()], "P/y.png");
        // span 应指向括号内（offset 10 起），而非链接文本（offset 1 起）
        assert_eq!(spans2[0].1.start, content2.find("](P/").unwrap() + 2);
    }

    #[test]
    fn extract_title_containing_bracket_paren_locates_real_href() {
        // title 内含 `](` 字面量（CommonMark 合法）：rfind 会越过真 href，
        // 应回退首个 `](` 定位——真 href 被换签、title 不被污染
        let content = "![img](P/a.png \"ti](P/a.png\")";
        let spans = extract_inline_dest_spans(content, "P/");
        assert_eq!(spans.len(), 1);
        let (dest, range) = &spans[0];
        assert_eq!(dest, "P/a.png");
        // span 必须覆盖括号内的真 href，而非 title 里的副本
        assert_eq!(&content[range.clone()], "P/a.png");
        let mut signed = HashMap::new();
        signed.insert("P/a.png".to_string(), "S".to_string());
        let out = rebuild_with_replacements(content, &spans, &signed);
        assert_eq!(out, "![img](S \"ti](P/a.png\")");

        // title 含 `](` 但无 dest 副本：仍应命中真 href（而非静默跳过）
        let content2 = "![img](P/b.png \"ti](tle\")";
        let spans2 = extract_inline_dest_spans(content2, "P/");
        assert_eq!(spans2.len(), 1, "title 含 ]( 不应导致链接被跳过");
        assert_eq!(&content2[spans2[0].1.clone()], "P/b.png");
    }

    #[test]
    fn extract_same_url_nested_positions_both_hrefs_correctly() {
        // 同 URL 嵌套 `[![x](P/z)](P/z)`：`](` 从 span 尾部定位使内层 Image
        // 与外层 Link 各自命中自己的 href——两个 span 均覆盖正确子串、
        // 不重叠且有序（修复前 find 首次命中会让外层落在内层 href 上）
        let content = "[![x](P/z)](P/z)";
        let spans = extract_inline_dest_spans(content, "P/");
        assert_eq!(spans.len(), 2, "嵌套两层各收集一个: {spans:?}");
        for (dest, range) in &spans {
            assert_eq!(dest, "P/z");
            assert_eq!(
                &content[range.clone()],
                "P/z",
                "每个 span 精确覆盖一个 href"
            );
        }
        assert!(spans[0].1.end <= spans[1].1.start, "不重叠且有序");
        // rebuild：两个 href 都替换、不 panic
        let mut signed = HashMap::new();
        signed.insert("P/z".to_string(), "S".to_string());
        let out = rebuild_with_replacements(content, &spans, &signed);
        assert_eq!(out, "[![x](S)](S)");
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
