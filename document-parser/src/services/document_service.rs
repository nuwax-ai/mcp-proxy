use anyhow::{Context, Result as AnyhowResult};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};
use url;
use uuid::Uuid;

use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::models::{
    DocumentFormat, ParseResult, ParserEngine, SourceType, StructuredDocument,
    TaskStatus,
};
use crate::parsers::DualEngineParser;
use crate::processors::MarkdownProcessor;
use crate::processors::markdown_processor::CacheStatistics;
use crate::services::{OssService, TaskService};

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
            download_timeout: Duration::from_secs(app_config.document_parser.download_timeout as u64),
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
    oss_service: Option<Arc<OssService>>,
    config: DocumentServiceConfig,

    // Concurrency control
    processing_semaphore: Arc<Semaphore>,

    // HTTP client for downloads
    http_client: reqwest::Client,
}

impl DocumentService {
    /// 创建新的文档服务
    pub fn new(
        dual_parser: DualEngineParser,
        markdown_processor: MarkdownProcessor,
        task_service: Arc<TaskService>,
        oss_service: Option<Arc<OssService>>,
    ) -> Self {
        Self::with_config(
            dual_parser,
            markdown_processor,
            task_service,
            oss_service,
            DocumentServiceConfig::default(),
        )
    }

    /// 创建带配置的文档服务
    pub fn with_config(
        dual_parser: DualEngineParser,
        markdown_processor: MarkdownProcessor,
        task_service: Arc<TaskService>,
        oss_service: Option<Arc<OssService>>,
        config: DocumentServiceConfig,
    ) -> Self {
        let processing_semaphore = Arc::new(Semaphore::new(config.max_concurrent_tasks));

        // Configure HTTP client with timeouts
        let http_client = reqwest::Client::builder()
            .timeout(config.download_timeout)
            .user_agent("DocumentParser/1.0")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            dual_parser: Arc::new(dual_parser),
            markdown_processor: Arc::new(RwLock::new(markdown_processor)),
            task_service,
            oss_service,
            config,
            processing_semaphore,
            http_client,
        }
    }

    /// 解析文档 - Enhanced with proper async patterns and error handling
    #[instrument(skip(self), fields(task_id = %task_id, format = ?format))]
    pub async fn parse_document(
        &self,
        task_id: &str,
        file_path: &str,
        format: DocumentFormat,
    ) -> AnyhowResult<ParseResult> {
        // Acquire semaphore permit for concurrency control
        let _permit = self
            .processing_semaphore
            .acquire()
            .await
            .context("Failed to acquire processing permit")?;

        info!("开始解析文档: {} (格式: {:?})", file_path, format);

        // Wrap the entire operation in a timeout
        let result = timeout(self.config.task_timeout, async {
            self.parse_document_internal(task_id, file_path, format)
                .await
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

    /// Internal document parsing implementation
    async fn parse_document_internal(
        &self,
        task_id: &str,
        file_path: &str,
        format: DocumentFormat,
    ) -> AnyhowResult<ParseResult> {
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

        // Select parsing engine
        let selected_engine = ParserEngine::select_for_format(&format);
        self.update_task_parser_engine_safe(task_id, selected_engine.clone())
            .await;

        // Update progress
        self.update_task_progress_safe(task_id, 10).await;

        // Execute parsing with proper stage tracking
        let stage = match selected_engine {
            ParserEngine::MinerU => crate::models::ProcessingStage::MinerUExecuting,
            _ => crate::models::ProcessingStage::MarkItDownExecuting,
        };
        self.update_task_stage_safe(task_id, stage).await;

        let parse_result = self
            .dual_parser
            .parse_document(file_path.to_str().unwrap(), &format)
            .await
            .with_context(|| format!("文档解析失败: {}", file_path.display()))?;

        info!("文档解析成功: {}", file_path.display());
        self.update_task_progress_safe(task_id, 80).await;

        // Complete parsing
        self.update_task_progress_safe(task_id, 100).await;
        self.update_task_status_safe(task_id, TaskStatus::new_completed(Duration::from_secs(0)))
            .await;

        Ok(parse_result)
    }

    /// 从URL解析文档 - Enhanced with proper resource management
    #[instrument(skip(self), fields(task_id = %task_id, format = ?format))]
    pub async fn parse_document_from_url(
        &self,
        task_id: &str,
        url: &str,
        format: DocumentFormat,
    ) -> AnyhowResult<ParseResult> {
        info!("从URL解析文档: {} (格式: {:?})", url, format);

        // Update task status
        self.update_task_stage_safe(task_id, crate::models::ProcessingStage::DownloadingDocument)
            .await;

        // Download file with automatic cleanup
        let temp_file_guard = self
            .download_file_with_guard(url)
            .await
            .with_context(|| format!("下载文件失败: {}", url))?;

        // Update progress
        self.update_task_progress_safe(task_id, 30).await;

        // Parse document - temp file will be automatically cleaned up when guard is dropped
        let result = self
            .parse_document(task_id, temp_file_guard.path(), format)
            .await;

        // Explicit cleanup happens when temp_file_guard goes out of scope
        result
    }

    /// 从OSS解析文档 - Enhanced with proper resource management
    #[instrument(skip(self), fields(task_id = %task_id, format = ?format))]
    pub async fn parse_document_from_oss(
        &self,
        task_id: &str,
        oss_key: &str,
        format: DocumentFormat,
    ) -> AnyhowResult<ParseResult> {
        info!("从OSS解析文档: {} (格式: {:?})", oss_key, format);

        let oss_service = self
            .oss_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OSS服务未配置"))?;

        // Update task status
        self.update_task_stage_safe(task_id, crate::models::ProcessingStage::DownloadingDocument)
            .await;

        // Download from OSS with automatic cleanup
        let temp_file = oss_service
            .download_to_temp(oss_key)
            .await
            .with_context(|| format!("从OSS下载文件失败: {}", oss_key))?;

        let temp_file_guard = TempFileGuard::new(temp_file);

        // Update progress
        self.update_task_progress_safe(task_id, 30).await;

        // Parse document - temp file will be automatically cleaned up when guard is dropped
        self.parse_document(task_id, temp_file_guard.path(), format)
            .await
    }

    /// 生成结构化文档 - Enhanced with proper async patterns and error handling
    #[instrument(skip(self, markdown_content), fields(task_id = %task_id, content_length = markdown_content.len()))]
    pub async fn generate_structured_document(
        &self,
        task_id: &str,
        markdown_content: &str,
        title: Option<String>,
    ) -> AnyhowResult<StructuredDocument> {
        info!("生成结构化文档，内容长度: {} 字符", markdown_content.len());

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
                    "结构化文档生成完成，章节数: {}",
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
        let structured_doc = processor
            .process_markdown(markdown_content)
            .await
            .with_context(|| "Markdown处理失败")?;

        // Override title if provided
        let mut final_doc = if let Some(custom_title) = title {
            let mut doc = structured_doc;
            doc.document_title = custom_title;
            doc
        } else {
            structured_doc
        };

        // Calculate metadata
        final_doc.calculate_total_word_count();

        Ok(final_doc)
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

    /// Legacy sync version for backward compatibility
    fn detect_mime_type(&self, file_path: &str) -> Result<String, AppError> {
        let path = Path::new(file_path);
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

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

    /// 下载文件到临时目录 - Enhanced with proper async patterns and resource management
    async fn download_file_with_guard(&self, url: &str) -> AnyhowResult<TempFileGuard> {
        info!("下载文件: {}", url);

        // Validate URL
        let parsed_url = url::Url::parse(url).with_context(|| format!("无效的URL: {}", url))?;

        // Make HTTP request with timeout
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .with_context(|| format!("HTTP请求失败: {}", url))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "下载文件失败，HTTP状态码: {} - {}",
                response.status(),
                url
            ));
        }

        // Check content length if available
        if let Some(content_length) = response.content_length() {
            let global_config = GlobalFileSizeConfig::new();
            if content_length > global_config.max_file_size.bytes() {
                return Err(anyhow::anyhow!(
                    "文件大小超过限制: {} > {} bytes",
                    content_length,
                    global_config.max_file_size.bytes()
                ));
            }
        }

        // Stream content to temporary file
        let content = response
            .bytes()
            .await
            .with_context(|| format!("读取文件内容失败: {}", url))?;

        // Check actual content size
        let global_config = GlobalFileSizeConfig::new();
        if content.len() as u64 > global_config.max_file_size.bytes() {
            return Err(anyhow::anyhow!(
                "文件大小超过限制: {} > {} bytes",
                content.len(),
                global_config.max_file_size.bytes()
            ));
        }

        // Create temporary file with unique name in current directory temp folder
        let temp_dir = Path::new("./temp");
        tokio::fs::create_dir_all(temp_dir)
            .await
            .with_context(|| format!("创建临时目录失败: {}", temp_dir.display()))?;

        let file_name = parsed_url
            .path_segments()
            .and_then(|segments| segments.last())
            .unwrap_or("download");

        let unique_id = Uuid::new_v4();
        let temp_file = temp_dir.join(format!("doc_parser_{}_{}", unique_id, file_name));

        tokio::fs::write(&temp_file, content)
            .await
            .with_context(|| format!("写入临时文件失败: {}", temp_file.display()))?;

        info!("文件下载完成: {} -> {}", url, temp_file.display());
        Ok(TempFileGuard::new(temp_file.to_string_lossy().to_string()))
    }

    /// Legacy download method for backward compatibility
    async fn download_file(&self, url: &str) -> Result<String, AppError> {
        match self.download_file_with_guard(url).await {
            Ok(guard) => Ok(guard.path().to_string()),
            Err(e) => Err(AppError::Network(e.to_string())),
        }
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
        debug!("检查解析器健康状态");

        let health_check_result = timeout(Duration::from_secs(10), async {
            self.dual_parser.health_check().await
        })
        .await;

        let mut health_status = std::collections::HashMap::new();

        match health_check_result {
            Ok(Ok(_)) => {
                health_status.insert("parser_healthy".to_string(), true);
                health_status.insert("mineru_available".to_string(), true);
                health_status.insert("markitdown_available".to_string(), true);
                info!("解析器健康检查通过");
            }
            Ok(Err(e)) => {
                health_status.insert("parser_healthy".to_string(), false);
                health_status.insert("error_message".to_string(), false);
                warn!("解析器健康检查失败: {}", e);
            }
            Err(_) => {
                health_status.insert("parser_healthy".to_string(), false);
                health_status.insert("timeout".to_string(), true);
                warn!("解析器健康检查超时");
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
        debug!("清理Markdown处理器缓存");

        // Use write lock to ensure exclusive access during cache clearing
        let processor = self.markdown_processor.write().await;
        processor.clear_cache();

        info!("Markdown处理器缓存已清理");
        Ok(())
    }

    /// 生成结构化文档（无任务ID，同步）- Enhanced with proper async patterns
    #[instrument(skip(self, markdown_content), fields(content_length = markdown_content.len()))]
    pub async fn generate_structured_document_simple(
        &self,
        markdown_content: &str,
        _config: Option<crate::processors::MarkdownProcessorConfig>,
    ) -> AnyhowResult<StructuredDocument> {
        debug!("生成简单结构化文档");

        if markdown_content.is_empty() {
            return Err(anyhow::anyhow!("Markdown内容为空"));
        }

        // Use read lock for concurrent access
        let processor = self.markdown_processor.read().await;

        // Process with timeout
        let result = timeout(
            Duration::from_secs(30),
            processor.process_markdown(markdown_content),
        )
        .await;

        match result {
            Ok(Ok(mut doc)) => {
                doc.calculate_total_word_count();
                info!("简单结构化文档生成完成，章节数: {}", doc.total_sections);
                Ok(doc)
            }
            Ok(Err(e)) => {
                error!("结构化文档生成失败: {}", e);
                Err(anyhow::anyhow!("结构化文档生成失败: {}", e))
            }
            Err(_) => {
                error!("结构化文档生成超时");
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
        info!("创建文件上传任务: {} (大小: {} bytes)", filename, file_size);

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

        // Detect document format
        let extension = file_path_obj
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let format = DocumentFormat::from_extension(extension);

        // Create task
        let task = self
            .task_service
            .create_task(SourceType::Upload, Some(filename.to_string()), format)
            .await
            .map_err(|e| anyhow::anyhow!("创建任务失败: {}", e))?;

        Ok(task.id)
    }

    /// 创建URL下载任务
    pub async fn create_url_task(&self, url: &str, filename: &str) -> Result<String, AppError> {
        log::info!("创建URL下载任务: {} -> {}", url, filename);

        // 检测文档格式
        let extension = Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .ok_or_else(|| AppError::UnsupportedFormat("无法识别文件扩展名".to_string()))?;

        let format = DocumentFormat::from_extension(extension);

        // 创建任务
        let task = self
            .task_service
            .create_task(SourceType::Url, Some(filename.to_string()), format)
            .await
            .map_err(|e| AppError::Task(format!("创建任务失败: {}", e)))?;

        Ok(task.id)
    }

    /// 获取任务状态
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<crate::models::DocumentTask, AppError> {
        log::debug!("获取任务状态: {}", task_id);

        self.task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))
    }

    /// 下载并解析文档
    pub async fn download_and_parse_document(
        &self,
        task_id: &str,
        url: &str,
        format: DocumentFormat,
    ) -> Result<(), AppError> {
        log::info!("下载并解析文档: {} (格式: {:?})", url, format);

        // 使用现有的 parse_document_from_url 方法
        self.parse_document_from_url(task_id, url, format).await?;

        Ok(())
    }

    /// 解析OSS文档
    pub async fn parse_oss_document(
        &self,
        task_id: &str,
        oss_path: &str,
        format: DocumentFormat,
    ) -> Result<(), AppError> {
        log::info!("解析OSS文档: {} (格式: {:?})", oss_path, format);

        // 使用现有的 parse_document_from_oss 方法
        self.parse_document_from_oss(task_id, oss_path, format)
            .await?;

        Ok(())
    }
    

}
