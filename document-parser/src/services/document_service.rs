use anyhow::{Context, Result as AnyhowResult};
use oss_client::OssClientTrait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};
use url;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
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
    /// 创建新的文档服务
    pub fn new(
        dual_parser: DualEngineParser,
        markdown_processor: MarkdownProcessor,
        task_service: Arc<TaskService>,
        oss_client: Option<Arc<dyn oss_client::OssClientTrait + Send + Sync>>,
    ) -> Self {
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
    ) -> Self {
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
            oss_client,
            config,
            http_client,
        }
    }

    /// 解析文档 - Enhanced with proper async patterns and error handling
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn parse_document(
        &self,
        task_id: &str,
        file_path: &str,
    ) -> AnyhowResult<ParseResult> {
        info!("开始解析文档: {}", file_path);

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

    /// Internal document parsing implementation
    async fn parse_document_internal(
        &self,
        task_id: &str,
        file_path: &str,
    ) -> AnyhowResult<ParseResult> {
        debug!(
            "parse_document_internal - 任务ID: {}, 文件路径: {}",
            task_id, file_path
        );

        // 验证文件是否存在
        if !std::path::Path::new(file_path).exists() {
            error!("文件不存在: {}", file_path);
            return Err(anyhow::anyhow!("文件不存在: {}", file_path));
        }

        // 获取文件的绝对路径
        let absolute_path = std::path::Path::new(file_path)
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("无法获取文件绝对路径: {}", e))?
            .to_string_lossy()
            .to_string();
        debug!("文件绝对路径: {}", absolute_path);
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
            "检测到格式: {:?}, 选择的解析引擎: {:?}",
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
        debug!("任务阶段更新为: {:?}", stage);
        self.update_task_stage_safe(task_id, stage).await;

        // Parse document - 使用绝对路径
        info!("开始调用解析器，使用绝对路径: {}", absolute_path);
        let parse_result = self
            .dual_parser
            .parse_document_auto(&absolute_path)
            .await
            .with_context(|| format!("文档解析失败[parse_document_internal]"))?;
        debug!(
            "解析器调用完成，内容长度: {}",
            parse_result.markdown_content.len()
        );

        info!("文档解析成功: {}", file_path.display());
        self.update_task_progress_safe(task_id, 80).await;

        // 新增：处理图片上传和路径替换
        let final_result = self
            .process_images_and_replace_paths(task_id, parse_result)
            .await?;

        info!("图片处理和路径替换完成: {}", file_path.display());
        self.update_task_progress_safe(task_id, 90).await;

        // 将处理后的新Markdown内容上传到OSS
        let (oss_markdown_url, oss_object_key) = self
            .upload_processed_markdown_to_oss(task_id, &final_result.markdown_content)
            .await?;

        info!(
            "Markdown内容已上传到OSS: {} -> {}",
            oss_object_key, oss_markdown_url
        );
        self.update_task_progress_safe(task_id, 95).await;

        // Complete parsing
        self.update_task_progress_safe(task_id, 100).await;

        // 保存解析结果到数据库
        self.save_parse_result_to_task(
            task_id,
            &final_result,
            Some((oss_markdown_url, oss_object_key)),
        )
        .await?;

        // 计算真实处理耗时
        let processing_time = start_time.elapsed();

        self.update_task_status_safe(task_id, TaskStatus::new_completed(processing_time))
            .await;

        Ok(final_result)
    }

    /// 保存解析结果到任务
    async fn save_parse_result_to_task(
        &self,
        task_id: &str,
        parse_result: &ParseResult,
        oss_markdown_data: Option<(String, String)>,
    ) -> Result<(), AppError> {
        info!("开始保存解析结果到任务: {}", task_id);

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

        // 如果有OSS数据，设置到任务的oss_data中
        if let Some((oss_url, object_key)) = oss_markdown_data {
            // 从OSS客户端获取正确的bucket名称
            let oss_bucket = if let Some(ref oss_client) = self.oss_client {
                oss_client.get_config().bucket.clone()
            } else {
                "default".to_string() // 如果没有OSS客户端，使用默认值
            };

            let oss_data = crate::models::OssData {
                markdown_url: oss_url,
                markdown_object_key: Some(object_key),
                images: vec![],
                bucket: oss_bucket,
            };
            task.set_oss_data(oss_data)?;
        }

        // 保存任务
        self.task_service.save_task(&task).await?;

        info!("成功保存解析结果到任务: {}", task_id);
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

    /// 处理图片上传和路径替换
    pub async fn process_images_and_replace_paths(
        &self,
        task_id: &str,
        parse_result: ParseResult,
    ) -> AnyhowResult<ParseResult> {
        info!("开始处理图片上传和路径替换");

        // 检查OSS客户端是否可用
        let oss_client: &Arc<dyn oss_client::OssClientTrait + Send + Sync> = match &self.oss_client
        {
            Some(client) => client,
            None => {
                warn!("OSS客户端未配置，跳过图片上传");
                return Ok(parse_result);
            }
        };
        // 2. 扫描 MinerU 输出目录图片：优先 auto/images，再兼容 images
        let mut local_image_paths: Vec<PathBuf> = Vec::new();
        if let Some(output_path) = parse_result.output_dir.as_ref() {
            let collected = Self::collect_local_images_from_output(output_path).await?;
            info!("扫描到 {} 个图片文件用于上传", collected.len());
            local_image_paths.extend(collected);
        }

        // 3. 更新任务状态
        self.update_task_stage_safe(task_id, ProcessingStage::UploadingImages)
            .await;

        // 4. 上传图片到OSS，生成文件名->URL 的映射列表
        let mut image_results: Vec<ImageInfo> = Vec::new();
        for local_path in local_image_paths {
            let original_filename = match local_path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => {
                    warn!("无法获取文件名，跳过: {}", local_path.display());
                    continue;
                }
            };

            // 计算文件SHA-256哈希作为对象键名称，确保相同图片去重
            let hash_hex = self
                .compute_file_sha256_hex(&local_path)
                .await
                .map_err(|e| {
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

            // 使用新的函数生成对象键，支持bucket_dir子目录
            let object_key = self
                .generate_image_oss_object_key(task_id, &hash_hex, &ext_suffix)
                .await?;

            // 直接构造URL上传
            let oss_url = oss_client
                .upload_file(local_path.to_string_lossy().as_ref(), &object_key)
                .await
                .map_err(|e| {
                    AppError::Oss(format!(
                        "上传图片失败: {original_filename} -> {object_key}: {e}"
                    ))
                })?;

            // 收集文件尺寸与 MIME
            let metadata = fs::metadata(&local_path).await.map_err(|e| {
                AppError::File(format!("读取文件信息失败: {}: {}", local_path.display(), e))
            })?;
            let file_size = metadata.len();
            let mime_type = oss_client::detect_mime_type(local_path.to_string_lossy().as_ref());

            image_results.push(ImageInfo::with_full_info(
                local_path.to_string_lossy().to_string(),
                original_filename,
                object_key,
                oss_url,
                file_size,
                mime_type,
            ));
        }
        info!("成功上传 {} 个图片到OSS", image_results.len());
        debug!("image_results: {:?}", image_results);

        // 5. 更新任务状态
        self.update_task_stage_safe(task_id, ProcessingStage::ReplacingImagePaths)
            .await;

        // 6. 替换Markdown中的图片路径
        let updated_content = self
            .replace_image_paths_in_markdown(&parse_result.markdown_content, &image_results)
            .await?;

        // 7. 创建新的解析结果
        let mut final_result = parse_result.clone();
        final_result.markdown_content = updated_content;

        info!(
            "图片路径替换完成，内容长度: {} 字符，替换了 {} 个图片路径",
            final_result.markdown_content.len(),
            image_results.len()
        );
        Ok(final_result)
    }

    /// 递归查找 `images` 目录，并收集其下所有图片文件
    async fn collect_local_images_from_output(output_path: &str) -> AnyhowResult<Vec<PathBuf>> {
        debug!("扫描输出目录下名为 'images' 的目录: {}", output_path);
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
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name == "images" {
                                images_dirs.push(path.clone());
                            }
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
        info!("替换Markdown中的 {} 个图片路径", image_results.len());

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

        info!("创建了 {} 个文件名映射", filename_to_oss_url.len());

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
            "图片路径替换完成，处理了 {} 个图片，替换了 {} 个路径",
            image_results.len(),
            replacements_count
        );
        Ok(output)
    }

    /// 计算文件的 SHA-256 哈希（hex）
    async fn compute_file_sha256_hex(&self, path: &std::path::Path) -> AnyhowResult<String> {
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
        Ok(format!("{digest:x}"))
    }

    /// 查找匹配的OSS URL（通过文件名匹配）
    fn find_oss_url_for_filename(
        &self,
        image_path: &str,
        filename_to_oss_url: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        // 从 Markdown 图片路径中提取文件名进行匹配
        if let Some(filename) = Path::new(image_path).file_name() {
            if let Some(filename_str) = filename.to_str() {
                if let Some(oss_url) = filename_to_oss_url.get(filename_str) {
                    return Some(oss_url.clone());
                }
            }
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
        info!("从URL解析文档: {} ", url);

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
        debug!("从URL提取的文件名: {}", filename);
        debug!("URL (去掉查询参数): {}", url_without_query);

        // 创建基于 taskId 的临时文件路径
        let file_path = self.create_temp_file_for_task("./temp", task_id, &filename)?;
        debug!("创建的临时文件路径: {}", file_path);

        // 下载文件到指定路径
        self.download_file_to_path(url, &file_path)
            .await
            .with_context(|| format!("下载文件失败: {url}"))?;
        debug!("文件下载完成: {}", file_path);

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
            warn!("更新任务本地路径失败: task_id={}, error={}", task_id, e);
        } else {
            debug!(
                "已更新任务本地路径: task_id={}, path={}",
                task_id, file_path
            );
        }

        // Update progress
        self.update_task_progress_safe(task_id, 30).await;

        // Parse document
        debug!("开始解析文档: {}", file_path);
        

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
            "创建临时文件 - 输入参数: temp_dir={}, task_id={}, filename={}",
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

        debug!("提取的文件扩展名: {}", extension);

        // 从文件名中移除扩展名，然后清理文件名
        let stem = Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");

        let clean_stem = stem
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect::<String>();

        debug!("清理后的文件名主体: {}", clean_stem);

        // 使用 taskId 作为文件名的一部分，确保唯一性和可追踪性
        let task_filename = format!("task_{task_id}_{clean_stem}.{extension}");
        let file_path = temp_path.join(task_filename);

        // 验证路径安全性（防止路径遍历）
        if !file_path.starts_with(temp_path) {
            return Err(AppError::File("文件路径不安全".to_string()));
        }

        let final_path = file_path.to_string_lossy().to_string();
        debug!("创建临时文件 - 最终路径: {}", final_path);

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
        debug!("检查解析器健康状态");

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
        processor.clear_cache().await;

        info!("Markdown处理器缓存已清理");
        Ok(())
    }

    /// 生成结构化文档（无任务ID，同步）- Enhanced with proper async patterns
    #[instrument(skip(self, markdown_content), fields(content_length = markdown_content.len()))]
    pub async fn generate_structured_document_simple(
        &self,
        markdown_content: &str,
    ) -> AnyhowResult<StructuredDocument> {
        debug!("生成简单结构化文档");

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
                    "简单结构化文档生成完成，章节数: {}",
                    structured_doc.total_sections
                );

                Ok(structured_doc)
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
            .map_err(|e| anyhow::anyhow!("创建任务失败: {}", e))?;

        Ok(task.id)
    }

    /// 创建URL下载任务
    pub async fn create_url_task(&self, url: &str, filename: &str) -> Result<String, AppError> {
        log::info!("创建URL下载任务: {url} -> {filename}");

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
        log::debug!("获取任务状态: {task_id}");

        self.task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))
    }

    /// 生成OSS对象键的统一函数
    ///
    /// 这个函数根据任务ID、资源类型和可选的bucket_dir生成标准的OSS对象键。
    /// 支持多种资源类型，便于统一管理和清理。
    ///
    /// # 参数
    /// * `task_id` - 任务ID
    /// * `resource_type` - 资源类型（如 "processed_markdown", "parsed_images"）
    /// * `resource_path` - 资源路径（如 "task_id.md" 或 "sha256/hash.ext"）
    ///
    /// # 返回
    /// * `Ok(String)` - 生成的OSS对象键
    /// * `Err(AnyhowResult)` - 如果获取任务信息失败
    ///
    /// # 示例
    /// ```rust
    /// // 生成Markdown文件的对象键
    /// let md_key = service.generate_unified_oss_object_key("task_123", "processed_markdown", "task_123.md").await?;
    /// // 无bucket_dir: "processed_markdown/task_123/task_123.md"
    /// // 有bucket_dir: "my_bucket/processed_markdown/task_123/task_123.md"
    ///
    /// // 生成图片文件的对象键
    /// let img_key = service.generate_unified_oss_object_key("task_123", "parsed_images", "sha256/abc123.jpg").await?;
    /// // 无bucket_dir: "parsed_images/task_123/sha256/abc123.jpg"
    /// // 有bucket_dir: "my_bucket/parsed_images/task_123/sha256/abc123.jpg"
    ///
    /// // 生成其他资源类型的对象键
    /// let other_key = service.generate_unified_oss_object_key("task_123", "temp_files", "cache.dat").await?;
    /// // 无bucket_dir: "temp_files/task_123/cache.dat"
    /// // 有bucket_dir: "my_bucket/temp_files/task_123/cache.dat"
    /// ```
    async fn generate_unified_oss_object_key(
        &self,
        task_id: &str,
        resource_type: &str,
        resource_path: &str,
    ) -> AnyhowResult<String> {
        let bucket_dir = {
            let task_opt = self.task_service.get_task(task_id).await?;
            if let Some(task) = task_opt {
                task.bucket_dir
                    .as_ref()
                    .map(|dir| dir.trim_matches('/').to_string())
            } else {
                None
            }
        };

        let object_key = if let Some(dir) = bucket_dir {
            if dir.is_empty() {
                format!("{resource_type}/{task_id}/{resource_path}")
            } else {
                format!("{dir}/{resource_type}/{task_id}/{resource_path}")
            }
        } else {
            format!("{resource_type}/{task_id}/{resource_path}")
        };

        Ok(object_key)
    }

    /// 生成OSS对象键：[bucket_dir/]processed_markdown/<task_id>/<task_id>.md
    ///
    /// 这个函数根据任务ID和可选的bucket_dir生成标准的OSS对象键。
    /// 生成的格式为：[bucket_dir/]processed_markdown/<task_id>/<task_id>.md
    ///
    /// # 参数
    /// * `task_id` - 任务ID
    ///
    /// # 返回
    /// * `Ok(String)` - 生成的OSS对象键
    /// * `Err(AnyhowResult)` - 如果获取任务信息失败
    ///
    /// # 示例
    /// ```rust
    /// // 无bucket_dir的情况
    /// let object_key = service.generate_oss_object_key("task_123").await?;
    /// // 结果: "processed_markdown/task_123/task_123.md"
    ///
    /// // 有bucket_dir的情况
    /// let object_key = service.generate_oss_object_key("task_456").await?;
    /// // 结果: "my_bucket/processed_markdown/task_456/task_456.md"
    /// ```
    async fn generate_oss_object_key(&self, task_id: &str) -> AnyhowResult<String> {
        self.generate_unified_oss_object_key(
            task_id,
            "processed_markdown",
            &format!("{task_id}.md"),
        )
        .await
    }

    /// 生成图片的OSS对象键：[bucket_dir/]parsed_images/sha256/<hash>.<ext>
    ///
    /// 这个函数根据任务ID、图片哈希和扩展名生成标准的OSS对象键。
    /// 生成的格式为：[bucket_dir/]parsed_images/<task_id>/sha256/<hash>.<ext>
    ///
    /// # 参数
    /// * `task_id` - 任务ID
    /// * `hash_hex` - 图片文件的SHA-256哈希值
    /// * `ext_suffix` - 文件扩展名（包含点号，如 ".jpg"）
    ///
    /// # 返回
    /// * `Ok(String)` - 生成的OSS对象键
    /// * `Err(AnyhowResult)` - 如果获取任务信息失败
    ///
    /// # 示例
    /// ```rust
    /// // 无bucket_dir的情况
    /// let object_key = service.generate_image_oss_object_key("task_123", "abc123", ".jpg").await?;
    /// // 结果: "parsed_images/task_123/sha256/abc123.jpg"
    ///
    /// // 有bucket_dir的情况
    /// let object_key = service.generate_image_oss_object_key("task_456", "def456", ".png").await?;
    /// // 结果: "my_bucket/parsed_images/task_456/sha256/def456.png"
    /// ```
    async fn generate_image_oss_object_key(
        &self,
        task_id: &str,
        hash_hex: &str,
        ext_suffix: &str,
    ) -> AnyhowResult<String> {
        self.generate_unified_oss_object_key(
            task_id,
            "parsed_images",
            &format!("sha256/{hash_hex}{ext_suffix}"),
        )
        .await
    }

    /// 将处理后的Markdown内容上传到OSS
    async fn upload_processed_markdown_to_oss(
        &self,
        task_id: &str,
        markdown_content: &str,
    ) -> AnyhowResult<(String, String)> {
        info!("开始上传处理后的Markdown内容到OSS: {}", task_id);

        // 检查OSS客户端是否可用
        let oss_client = match &self.oss_client {
            Some(client) => client,
            None => {
                return Err(anyhow::anyhow!("OSS客户端未配置，无法上传Markdown内容"));
            }
        };

        // 使用提取的函数生成OSS对象键
        let object_key = self.generate_oss_object_key(task_id).await?;

        // 将Markdown内容转换为字节
        let content_bytes = markdown_content.as_bytes();

        // 上传到OSS
        let upload_result = oss_client
            .upload_content(content_bytes, &object_key, Some("text/markdown"))
            .await
            .map_err(|e| anyhow::anyhow!("上传Markdown内容到OSS失败: {}: {}", object_key, e))?;

        info!("Markdown内容上传成功: {} -> {}", object_key, upload_result);

        Ok((upload_result, object_key))
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
}
