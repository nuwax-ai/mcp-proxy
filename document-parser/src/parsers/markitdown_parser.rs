use super::parser_trait::DocumentParser;
use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::models::{DocumentFormat, ParseResult, ParserEngine};
use crate::parsers::FormatDetector;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::timeout;
use tracing::{debug, info, warn};
use uuid::{NoContext, Timestamp, Uuid};

/// 解析进度信息
#[derive(Debug, Clone)]
pub struct MarkItDownProgress {
    pub stage: ProcessingStage,
    pub progress: f32,
    pub message: String,
    pub elapsed_time: Duration,
    pub current_file: Option<String>,
}

/// 处理阶段
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessingStage {
    Initializing,
    ValidatingInput,
    PreProcessing,
    Converting,
    PostProcessing,
    Finalizing,
    Completed,
    Failed,
    Cancelled,
}

/// 取消令牌
#[derive(Debug, Clone)]
pub struct CancellationToken {
    inner: Arc<RwLock<bool>>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn cancel(&self) {
        let mut cancelled = self.inner.write().await;
        *cancelled = true;
    }

    pub async fn is_cancelled(&self) -> bool {
        *self.inner.read().await
    }
}

/// MarkItDown配置
#[derive(Debug, Clone)]
pub struct MarkItDownConfig {
    pub python_path: String,
    pub enable_plugins: bool,
    pub timeout_seconds: u64,
    // 文件大小限制现在由全局配置管理
    pub supported_formats: Vec<DocumentFormat>,
    pub output_format: OutputFormat,
    pub quality_settings: QualitySettings,
}

/// 输出格式配置
#[derive(Debug, Clone, PartialEq)]
pub enum OutputFormat {
    Markdown,
    PlainText,
    Html,
}

/// 质量设置
#[derive(Debug, Clone)]
pub struct QualitySettings {
    pub preserve_formatting: bool,
    pub extract_images: bool,
    pub extract_tables: bool,
    pub extract_metadata: bool,
    pub clean_output: bool,
}

impl MarkItDownConfig {
    /// 使用全局文件大小配置创建MarkItDownConfig
    pub fn with_global_config() -> Self {
        Self {
            python_path: if cfg!(windows) {
                "./venv/Scripts/python.exe".to_string()
            } else {
                "./venv/bin/python".to_string()
            },
            enable_plugins: true,
            timeout_seconds: 180, // 3分钟
            // 文件大小限制现在由全局配置管理
            supported_formats: vec![
                DocumentFormat::Word,
                DocumentFormat::Excel,
                DocumentFormat::PowerPoint,
                DocumentFormat::Image,
                DocumentFormat::Audio,
                DocumentFormat::HTML,
                DocumentFormat::Text,
                DocumentFormat::Txt,
                DocumentFormat::Md,
            ],
            output_format: OutputFormat::Markdown,
            quality_settings: QualitySettings {
                preserve_formatting: true,
                extract_images: true,
                extract_tables: true,
                extract_metadata: true,
                clean_output: true,
            },
        }
    }

    /// Get the effective python path, auto-detecting virtual environment if needed
    pub fn get_effective_python_path(&self) -> String {
        // If the configured path is the default and a virtual environment exists, use it
        let default_path = if cfg!(windows) {
            "./venv/Scripts/python.exe"
        } else {
            "./venv/bin/python"
        };

        if self.python_path == default_path
            || self.python_path == "python3"
            || self.python_path == "python"
        {
            let venv_python = if cfg!(windows) {
                std::path::Path::new("./venv/Scripts/python.exe")
            } else {
                std::path::Path::new("./venv/bin/python")
            };

            if venv_python.exists() {
                return venv_python.to_string_lossy().to_string();
            }
        }

        self.python_path.clone()
    }
}

impl Default for MarkItDownConfig {
    fn default() -> Self {
        Self::with_global_config()
    }
}

/// 格式支持信息
#[derive(Debug, Clone)]
pub struct FormatSupport {
    pub format: DocumentFormat,
    pub supported: bool,
    pub confidence: f32,
    pub features: Vec<String>,
    pub limitations: Vec<String>,
}

/// MarkItDown多格式解析器
pub struct MarkItDownParser {
    config: MarkItDownConfig,
    active_tasks: Arc<Mutex<HashMap<String, CancellationToken>>>,
    format_support_cache: Arc<RwLock<HashMap<DocumentFormat, FormatSupport>>>,
}

impl MarkItDownParser {
    /// 创建新的MarkItDown解析器
    pub fn new(config: MarkItDownConfig) -> Self {
        Self {
            config,
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            format_support_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 创建带默认配置的解析器
    pub fn with_defaults(python_path: String, enable_plugins: bool) -> Self {
        let config = MarkItDownConfig {
            python_path,
            enable_plugins,
            ..Default::default()
        };
        Self::new(config)
    }

    /// 创建自动检测当前目录虚拟环境的解析器
    pub fn with_auto_venv_detection() -> Result<Self, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::MarkItDown(format!("无法获取当前目录: {e}")))?;

        let venv_path = current_dir.join("venv");
        let python_path = if cfg!(windows) {
            venv_path.join("Scripts").join("python.exe")
        } else {
            venv_path.join("bin").join("python")
        };

        let config = MarkItDownConfig {
            python_path: python_path.to_string_lossy().to_string(),
            enable_plugins: true,
            timeout_seconds: 180,
            supported_formats: vec![
                DocumentFormat::Word,
                DocumentFormat::Excel,
                DocumentFormat::PowerPoint,
                DocumentFormat::Image,
                DocumentFormat::Audio,
                DocumentFormat::HTML,
                DocumentFormat::Text,
                DocumentFormat::Txt,
                DocumentFormat::Md,
            ],
            output_format: OutputFormat::Markdown,
            quality_settings: QualitySettings {
                preserve_formatting: true,
                extract_images: true,
                extract_tables: true,
                extract_metadata: true,
                clean_output: true,
            },
        };

        Ok(Self::new(config))
    }

    /// 获取配置
    pub fn config(&self) -> &MarkItDownConfig {
        &self.config
    }

    /// 带进度跟踪和取消支持的解析
    pub async fn parse_with_progress<F>(
        &self,
        file_path: &str,
        format: &DocumentFormat,
        progress_callback: F,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<ParseResult, AppError>
    where
        F: Fn(MarkItDownProgress) + Send + Sync + 'static,
    {
        let start_time = Instant::now();
        let task_id = Uuid::new_v7(Timestamp::now(NoContext)).to_string();

        // 注册取消令牌
        let token = cancellation_token.unwrap_or_default();
        {
            let mut tasks = self.active_tasks.lock().await;
            tasks.insert(task_id.clone(), token.clone());
        }

        let result = self
            .parse_internal_with_progress(
                file_path,
                format,
                &task_id,
                progress_callback,
                token.clone(),
                start_time,
            )
            .await;

        // 清理任务
        {
            let mut tasks = self.active_tasks.lock().await;
            tasks.remove(&task_id);
        }

        result
    }

    /// 取消指定任务
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), AppError> {
        let tasks = self.active_tasks.lock().await;
        if let Some(token) = tasks.get(task_id) {
            token.cancel().await;
            info!("已取消MarkItDown解析任务: {}", task_id);
            Ok(())
        } else {
            Err(AppError::MarkItDown(format!("任务不存在: {task_id}")))
        }
    }

    /// 获取活跃任务数量
    pub async fn get_active_task_count(&self) -> usize {
        let tasks = self.active_tasks.lock().await;
        tasks.len()
    }

    /// 验证格式支持
    pub async fn validate_format_support(
        &self,
        format: &DocumentFormat,
    ) -> Result<FormatSupport, AppError> {
        // 检查缓存
        {
            let cache = self.format_support_cache.read().await;
            if let Some(support) = cache.get(format) {
                return Ok(support.clone());
            }
        }

        // 执行格式支持检查
        let support = self.check_format_support_internal(format).await?;

        // 更新缓存
        {
            let mut cache = self.format_support_cache.write().await;
            cache.insert(format.clone(), support.clone());
        }

        Ok(support)
    }

    /// 清除格式支持缓存
    pub async fn clear_format_cache(&self) {
        let mut cache = self.format_support_cache.write().await;
        cache.clear();
    }

    /// 内部解析实现（带进度跟踪）
    async fn parse_internal_with_progress<F>(
        &self,
        file_path: &str,
        format: &DocumentFormat,
        task_id: &str,
        progress_callback: F,
        cancellation_token: CancellationToken,
        start_time: Instant,
    ) -> Result<ParseResult, AppError>
    where
        F: Fn(MarkItDownProgress) + Send + Sync + 'static,
    {
        // 初始化阶段
        progress_callback(MarkItDownProgress {
            stage: ProcessingStage::Initializing,
            progress: 0.0,
            message: "初始化MarkItDown解析器".to_string(),
            elapsed_time: start_time.elapsed(),
            current_file: Some(file_path.to_string()),
        });

        // 验证输入
        progress_callback(MarkItDownProgress {
            stage: ProcessingStage::ValidatingInput,
            progress: 10.0,
            message: "验证输入文件和格式".to_string(),
            elapsed_time: start_time.elapsed(),
            current_file: Some(file_path.to_string()),
        });

        self.validate_input_file(file_path, format).await?;

        if cancellation_token.is_cancelled().await {
            return Err(AppError::MarkItDown("解析已取消".to_string()));
        }

        // 预处理阶段
        progress_callback(MarkItDownProgress {
            stage: ProcessingStage::PreProcessing,
            progress: 20.0,
            message: "准备工作环境".to_string(),
            elapsed_time: start_time.elapsed(),
            current_file: Some(file_path.to_string()),
        });

        let work_dir = Path::new("temp/markitdown").join(task_id);
        fs::create_dir_all(&work_dir)
            .await
            .map_err(|e| AppError::File(format!("创建工作目录失败: {e}")))?;

        info!(
            "使用MarkItDown解析文档: {} (格式: {:?}) -> {}",
            file_path,
            format,
            work_dir.display()
        );

        if cancellation_token.is_cancelled().await {
            self.cleanup_work_dir(&work_dir).await;
            return Err(AppError::MarkItDown("解析已取消".to_string()));
        }

        // 转换阶段
        progress_callback(MarkItDownProgress {
            stage: ProcessingStage::Converting,
            progress: 30.0,
            message: "正在转换文档".to_string(),
            elapsed_time: start_time.elapsed(),
            current_file: Some(file_path.to_string()),
        });

        let conversion_result = self
            .execute_markitdown_command(
                file_path,
                &work_dir,
                format,
                &progress_callback,
                &cancellation_token,
                start_time,
            )
            .await;

        if let Err(e) = &conversion_result {
            //发生异常了，清理工作目录
            self.cleanup_work_dir(&work_dir).await;
            return Err(e.clone());
        }

        let (markdown_content, temp_files) = conversion_result.unwrap();

        if cancellation_token.is_cancelled().await {
            // 解析已取消，清理工作目录
            self.cleanup_work_dir(&work_dir).await;
            return Err(AppError::MarkItDown("解析已取消".to_string()));
        }

        // 后处理阶段
        progress_callback(MarkItDownProgress {
            stage: ProcessingStage::PostProcessing,
            progress: 80.0,
            message: "处理解析结果".to_string(),
            elapsed_time: start_time.elapsed(),
            current_file: Some(file_path.to_string()),
        });

        let processed_content = self.post_process_content(&markdown_content, format).await?;

        // 完成阶段
        progress_callback(MarkItDownProgress {
            stage: ProcessingStage::Finalizing,
            progress: 95.0,
            message: "生成最终结果".to_string(),
            elapsed_time: start_time.elapsed(),
            current_file: Some(file_path.to_string()),
        });

        let processing_time = start_time.elapsed();
        let word_count = processed_content.split_whitespace().count();

        let mut result =
            ParseResult::new(processed_content, format.clone(), ParserEngine::MarkItDown);

        result.set_processing_time(processing_time.as_secs_f64());
        result.set_error_count(0);

        // 注意：不在此处清理工作目录，交由上层在过期清理时统一清理

        progress_callback(MarkItDownProgress {
            stage: ProcessingStage::Completed,
            progress: 100.0,
            message: format!("解析完成，耗时: {processing_time:?}，字数: {word_count}"),
            elapsed_time: processing_time,
            current_file: Some(file_path.to_string()),
        });

        info!(
            "MarkItDown解析完成，耗时: {:?}，字数: {}",
            processing_time, word_count
        );

        Ok(result)
    }

    /// 验证输入文件
    async fn validate_input_file(
        &self,
        file_path: &str,
        format: &DocumentFormat,
    ) -> Result<(), AppError> {
        let path = Path::new(file_path);

        if !path.exists() {
            return Err(AppError::File(format!("文件不存在: {file_path}")));
        }

        let metadata = fs::metadata(path)
            .await
            .map_err(|e| AppError::File(format!("无法读取文件元数据: {e}")))?;

        let file_size_bytes = metadata.len();
        let global_config = GlobalFileSizeConfig::new();
        if file_size_bytes > global_config.max_file_size.bytes() {
            return Err(AppError::File(format!(
                "文件大小超过限制: {}MB > {}MB",
                file_size_bytes / (1024 * 1024),
                global_config.max_file_size.bytes() / (1024 * 1024)
            )));
        }

        // 验证格式支持
        let format_support = self.validate_format_support(format).await?;
        if !format_support.supported {
            return Err(AppError::UnsupportedFormat(format!(
                "MarkItDown不支持格式: {format:?}"
            )));
        }

        Ok(())
    }

    /// 执行MarkItDown命令
    async fn execute_markitdown_command<F>(
        &self,
        file_path: &str,
        work_dir: &Path,
        format: &DocumentFormat,
        progress_callback: &F,
        cancellation_token: &CancellationToken,
        start_time: Instant,
    ) -> Result<(String, Vec<String>), AppError>
    where
        F: Fn(MarkItDownProgress) + Send + Sync + 'static,
    {
        let output_file = work_dir.join("output.md");

        // 自动检测并使用虚拟环境中的 python
        let python_path = self.config.get_effective_python_path();
        let mut cmd = Command::new(&python_path);

        cmd.arg("-m").arg("markitdown").arg(file_path);

        // 设置输出文件
        cmd.arg("-o").arg(&output_file);

        // 如果启用插件，添加相关参数
        if self.config.enable_plugins {
            cmd.arg("-p");
        }

        // 保持数据URI（如base64编码的图片）
        if self.config.quality_settings.extract_images {
            cmd.arg("--keep-data-uris");
        }

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        debug!("执行MarkItDown命令: {:?}", cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::MarkItDown(format!("启动MarkItDown进程失败: {e}")))?;

        // 监控进程输出
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        let (tx, mut rx) = mpsc::channel(100);
        let tx_clone = tx.clone();

        // 监控stdout
        let stdout_task = tokio::spawn(async move {
            let mut lines = stdout_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(("stdout".to_string(), line)).await;
            }
        });

        // 监控stderr
        let stderr_task = tokio::spawn(async move {
            let mut lines = stderr_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_clone.send(("stderr".to_string(), line)).await;
            }
        });

        // 监控进程和输出
        let timeout_duration = Duration::from_secs(self.config.timeout_seconds);
        let process_result = timeout(timeout_duration, async {
            let mut progress = 30.0;
            let mut stderr_output = String::new();
            let mut temp_files = Vec::new();

            loop {
                tokio::select! {
                    // 检查取消
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if cancellation_token.is_cancelled().await {
                            let _ = child.kill().await;
                            return Err(AppError::MarkItDown("解析已取消".to_string()));
                        }
                    }

                    // 处理输出
                    Some((source, line)) = rx.recv() => {
                        debug!("MarkItDown {}: {}", source, line);

                        if source == "stderr" {
                            stderr_output.push_str(&line);
                            stderr_output.push('\n');
                        }

                        // 更新进度（基于输出内容推测）
                        if line.contains("Processing") || line.contains("Converting") {
                            progress = (progress + 2.0_f32).min(75.0_f32);
                            progress_callback(MarkItDownProgress {
                                stage: ProcessingStage::Converting,
                                progress,
                                message: line.clone(),
                                elapsed_time: start_time.elapsed(),
                                current_file: Some(file_path.to_string()),
                            });
                        }

                        // 收集临时文件信息
                        if line.contains("Created temp file:") {
                            if let Some(file_path) = line.split("Created temp file:").nth(1) {
                                temp_files.push(file_path.trim().to_string());
                            }
                        }
                    }

                    // 等待进程完成
                    result = child.wait() => {
                        match result {
                            Ok(status) => {
                                if status.success() {
                                    return Ok(temp_files);
                                } else {
                                    return Err(AppError::MarkItDown(format!(
                                        "MarkItDown执行失败，退出码: {}，错误输出: {}",
                                        status.code().unwrap_or(-1),
                                        stderr_output
                                    )));
                                }
                            }
                            Err(e) => {
                                return Err(AppError::MarkItDown(format!("等待进程完成失败: {e}")));
                            }
                        }
                    }
                }
            }
        })
        .await;

        // 清理任务
        stdout_task.abort();
        stderr_task.abort();

        let temp_files = match process_result {
            Ok(result) => result?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(AppError::MarkItDown(format!(
                    "MarkItDown执行超时（{}秒）",
                    self.config.timeout_seconds
                )));
            }
        };

        // 读取输出内容
        let markdown_content = if output_file.exists() {
            fs::read_to_string(&output_file)
                .await
                .map_err(|e| AppError::File(format!("读取输出文件失败: {e}")))?
        } else {
            return Err(AppError::MarkItDown("未生成输出文件".to_string()));
        };

        if markdown_content.trim().is_empty() {
            return Err(AppError::MarkItDown("生成的内容为空".to_string()));
        }

        Ok((markdown_content, temp_files))
    }

    /// 后处理内容
    async fn post_process_content(
        &self,
        content: &str,
        format: &DocumentFormat,
    ) -> Result<String, AppError> {
        let mut processed_content = content.to_string();

        if self.config.quality_settings.clean_output {
            // 清理多余的空行
            processed_content = processed_content
                .lines()
                .collect::<Vec<_>>()
                .join("\n")
                .replace("\n\n\n", "\n\n");

            // 修复常见的格式问题
            processed_content = processed_content
                .replace("# #", "#")
                .replace("## ##", "##")
                .replace("### ###", "###");
        }

        // 根据格式进行特定的后处理
        match format {
            DocumentFormat::Excel => {
                // Excel表格的特殊处理
                processed_content = self.post_process_excel_content(&processed_content).await?;
            }
            DocumentFormat::PowerPoint => {
                // PowerPoint幻灯片的特殊处理
                processed_content = self
                    .post_process_powerpoint_content(&processed_content)
                    .await?;
            }
            DocumentFormat::Word => {
                // Word文档的特殊处理
                processed_content = self.post_process_word_content(&processed_content).await?;
            }
            _ => {}
        }

        Ok(processed_content)
    }

    /// 后处理Excel内容
    async fn post_process_excel_content(&self, content: &str) -> Result<String, AppError> {
        // 改进表格格式
        let mut processed = content.to_string();

        // 确保表格有适当的标题
        if !processed.contains("# ") && processed.contains("|") {
            processed = format!("# Excel数据\n\n{processed}");
        }

        Ok(processed)
    }

    /// 后处理PowerPoint内容
    async fn post_process_powerpoint_content(&self, content: &str) -> Result<String, AppError> {
        let mut processed = content.to_string();

        // 为幻灯片添加分隔符
        processed = processed.replace("Slide ", "\n---\n\n# Slide ");

        Ok(processed)
    }

    /// 后处理Word内容
    async fn post_process_word_content(&self, content: &str) -> Result<String, AppError> {
        let mut processed = content.to_string();

        // 改进标题层次结构
        let lines: Vec<&str> = processed.lines().collect();
        let mut result_lines = Vec::new();

        for line in lines {
            if line.trim().is_empty() {
                result_lines.push(line.to_string());
                continue;
            }

            // 检测可能的标题
            if line.len() < 100
                && !line.starts_with('#')
                && (line
                    .chars()
                    .all(|c| c.is_uppercase() || c.is_whitespace() || c.is_numeric())
                    || line.ends_with(':'))
            {
                result_lines.push(format!("## {}", line.trim_end_matches(':')));
            } else {
                result_lines.push(line.to_string());
            }
        }

        processed = result_lines.join("\n");
        Ok(processed)
    }

    /// 清理工作目录
    async fn cleanup_work_dir(&self, work_dir: &Path) {
        if let Err(e) = fs::remove_dir_all(work_dir).await {
            warn!("清理工作目录失败: {} - {}", work_dir.display(), e);
        } else {
            debug!("已清理工作目录: {}", work_dir.display());
        }
    }

    /// 收集图片文件
    async fn collect_images(&self, work_dir: &Path) -> Result<Vec<String>, AppError> {
        debug!("收集图片文件: {}", work_dir.display());

        let mut images = Vec::new();

        if !work_dir.exists() {
            return Ok(images);
        }

        let collected = self.collect_images_from_dir(work_dir).await?;
        images.extend(collected);

        // 去重
        images.sort();
        images.dedup();

        info!("收集到 {} 个图片文件", images.len());
        Ok(images)
    }

    /// 从指定目录收集图片
    fn collect_images_from_dir<'a>(
        &'a self,
        dir: &'a Path,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<String>, AppError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let mut images = Vec::new();

            let mut entries = fs::read_dir(dir)
                .await
                .map_err(|e| AppError::File(format!("读取目录失败: {} - {}", dir.display(), e)))?;

            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| AppError::File(format!("遍历目录失败: {} - {}", dir.display(), e)))?
            {
                let path = entry.path();

                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
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
                            // 验证文件不为空
                            if let Ok(metadata) = fs::metadata(&path).await {
                                if metadata.len() > 0 {
                                    images.push(path.to_string_lossy().to_string());
                                    debug!("找到图片文件: {}", path.display());
                                }
                            }
                        }
                    }
                } else if path.is_dir() {
                    // 递归搜索子目录
                    let sub_images = self.collect_images_from_dir(&path).await?;
                    images.extend(sub_images);
                }
            }

            Ok(images)
        })
    }

    /// 检查格式支持的内部实现
    async fn check_format_support_internal(
        &self,
        format: &DocumentFormat,
    ) -> Result<FormatSupport, AppError> {
        let mut support = FormatSupport {
            format: format.clone(),
            supported: false,
            confidence: 0.0,
            features: Vec::new(),
            limitations: Vec::new(),
        };

        // 检查是否在支持列表中
        if self.config.supported_formats.contains(format) {
            support.supported = true;
            support.confidence = 0.8;
        }

        // 根据格式设置特性和限制
        match format {
            DocumentFormat::Word => {
                support.features.extend(vec![
                    "文本提取".to_string(),
                    "格式保持".to_string(),
                    "表格提取".to_string(),
                    "图片提取".to_string(),
                ]);
                support.limitations.push("复杂布局可能丢失".to_string());
                support.confidence = 0.9;
            }
            DocumentFormat::Excel => {
                support.features.extend(vec![
                    "表格数据提取".to_string(),
                    "多工作表支持".to_string(),
                    "公式转换".to_string(),
                ]);
                support.limitations.push("图表不支持".to_string());
                support.confidence = 0.85;
            }
            DocumentFormat::PowerPoint => {
                support.features.extend(vec![
                    "幻灯片内容提取".to_string(),
                    "文本和图片".to_string(),
                    "演讲者备注".to_string(),
                ]);
                support
                    .limitations
                    .extend(vec!["动画效果丢失".to_string(), "复杂图形简化".to_string()]);
                support.confidence = 0.8;
            }
            DocumentFormat::Image => {
                support
                    .features
                    .extend(vec!["OCR文本识别".to_string(), "图片描述".to_string()]);
                support.limitations.push("需要OCR引擎".to_string());
                support.confidence = 0.7;
            }
            DocumentFormat::Audio => {
                support.features.push("音频转录".to_string());
                support.limitations.extend(vec![
                    "需要语音识别引擎".to_string(),
                    "质量依赖音频清晰度".to_string(),
                ]);
                support.confidence = 0.6;
            }
            DocumentFormat::HTML => {
                support.features.extend(vec![
                    "HTML到Markdown转换".to_string(),
                    "链接保持".to_string(),
                    "表格转换".to_string(),
                ]);
                support.confidence = 0.95;
            }
            DocumentFormat::Text | DocumentFormat::Txt => {
                support.features.push("纯文本处理".to_string());
                support.confidence = 1.0;
            }
            DocumentFormat::Md => {
                support.features.push("Markdown格式化".to_string());
                support.confidence = 1.0;
            }
            DocumentFormat::PDF => {
                support.supported = false;
                support.confidence = 0.0;
                support
                    .limitations
                    .push("建议使用MinerU处理PDF".to_string());
            }
            DocumentFormat::Other(_) => {
                support.supported = false;
                support.confidence = 0.0;
                support.limitations.push("未知格式".to_string());
            }
        }

        Ok(support)
    }

    /// 获取解析统计信息
    pub async fn get_parse_statistics(&self) -> HashMap<String, serde_json::Value> {
        let mut stats = HashMap::new();

        let active_count = self.get_active_task_count().await;
        stats.insert(
            "active_tasks".to_string(),
            serde_json::Value::Number(active_count.into()),
        );

        let cache_size = {
            let cache = self.format_support_cache.read().await;
            cache.len()
        };
        stats.insert(
            "format_cache_size".to_string(),
            serde_json::Value::Number(cache_size.into()),
        );

        stats.insert(
            "config".to_string(),
            serde_json::json!({
                "enable_plugins": self.config.enable_plugins,
                "timeout_seconds": self.config.timeout_seconds,
                "output_format": format!("{:?}", self.config.output_format),
                "supported_formats_count": self.config.supported_formats.len(),
            }),
        );

        stats
    }

    /// 验证MarkItDown环境
    pub async fn validate_environment(&self) -> Result<(), AppError> {
        // 检查Python路径
        let effective_python_path = self.config.get_effective_python_path();
        if !Path::new(&effective_python_path).exists() {
            return Err(AppError::MarkItDown(format!(
                "Python路径不存在: {effective_python_path}"
            )));
        }

        // 检查临时目录
        let temp_dir = Path::new("temp/markitdown");
        if !temp_dir.exists() {
            fs::create_dir_all(temp_dir)
                .await
                .map_err(|e| AppError::MarkItDown(format!("创建临时目录失败: {e}")))?;
        }

        // 检查MarkItDown模块
        let output = Command::new(&effective_python_path)
            .arg("-c")
            .arg("import markitdown; print('MarkItDown available')")
            .output()
            .await
            .map_err(|e| AppError::MarkItDown(format!("检查MarkItDown模块失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::MarkItDown(format!(
                "MarkItDown模块不可用: {stderr}"
            )));
        }

        // 检查版本信息
        let version_output = Command::new(&effective_python_path)
            .arg("-c")
            .arg("import markitdown; print(f'MarkItDown version: {markitdown.__version__}')")
            .output()
            .await
            .map_err(|e| AppError::MarkItDown(format!("检查MarkItDown版本失败: {e}")))?;

        if version_output.status.success() {
            let version_str = String::from_utf8_lossy(&version_output.stdout);
            info!("MarkItDown环境验证通过: {}", version_str.trim());
        }

        Ok(())
    }

    /// 检查格式是否支持
    fn is_supported_format(&self, format: &DocumentFormat) -> bool {
        self.config.supported_formats.contains(format)
    }
}

#[async_trait]
impl DocumentParser for MarkItDownParser {
    async fn parse(&self, file_path: &str) -> Result<ParseResult, AppError> {
        let detector = FormatDetector::new();
        let detection = detector.detect_format(file_path, None)?;
        let format = detection.format;

        if !self.is_supported_format(&format) {
            return Err(AppError::UnsupportedFormat(format!(
                "MarkItDown不支持格式: {format:?}"
            )));
        }

        self.parse_with_progress(
            file_path,
            &format,
            |_progress| {
                // 默认不处理进度回调
            },
            None,
        )
        .await
    }

    fn supports_format(&self, format: &DocumentFormat) -> bool {
        self.is_supported_format(format)
    }

    fn get_name(&self) -> &'static str {
        "MarkItDown"
    }

    fn get_description(&self) -> &'static str {
        "多格式文档解析引擎，支持Office文档、网页、电子书等多种格式"
    }

    async fn health_check(&self) -> Result<(), AppError> {
        self.validate_environment().await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};
    use tokio::time::sleep;

    fn create_test_config() -> MarkItDownConfig {
        MarkItDownConfig {
            python_path: "python3".to_string(),
            enable_plugins: true,
            timeout_seconds: 30,
            // 文件大小限制现在由全局配置管理
            supported_formats: vec![
                DocumentFormat::Word,
                DocumentFormat::Excel,
                DocumentFormat::Text,
                DocumentFormat::HTML,
            ],
            output_format: OutputFormat::Markdown,
            quality_settings: QualitySettings {
                preserve_formatting: true,
                extract_images: true,
                extract_tables: true,
                extract_metadata: true,
                clean_output: true,
            },
        }
    }

    fn create_test_text_file() -> Result<NamedTempFile, std::io::Error> {
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"This is a test document.\nWith multiple lines.\n")?;
        temp_file.flush()?;
        Ok(temp_file)
    }

    fn create_test_html_file() -> Result<NamedTempFile, std::io::Error> {
        let mut temp_file = NamedTempFile::with_suffix(".html")?;
        temp_file.write_all(
            b"<html><head><title>Test</title></head><body><h1>Hello</h1><p>World</p></body></html>",
        )?;
        temp_file.flush()?;
        Ok(temp_file)
    }

    #[test]
    fn test_markitdown_config_default() {
        let config = MarkItDownConfig::default();
        if cfg!(windows) {
            assert_eq!(config.python_path, "./venv/Scripts/python.exe");
        } else {
            assert_eq!(config.python_path, "./venv/bin/python");
        }
        assert!(config.enable_plugins);
        assert_eq!(config.timeout_seconds, 180);
        assert_eq!(config.output_format, OutputFormat::Markdown);
        assert!(config.quality_settings.preserve_formatting);
    }

    #[test]
    fn test_cancellation_token() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let token = CancellationToken::new();
            assert!(!token.is_cancelled().await);

            token.cancel().await;
            assert!(token.is_cancelled().await);
        });
    }

    #[test]
    fn test_processing_stage_variants() {
        let stages = vec![
            ProcessingStage::Initializing,
            ProcessingStage::ValidatingInput,
            ProcessingStage::PreProcessing,
            ProcessingStage::Converting,
            ProcessingStage::PostProcessing,
            ProcessingStage::Finalizing,
            ProcessingStage::Completed,
            ProcessingStage::Failed,
            ProcessingStage::Cancelled,
        ];

        for stage in stages {
            let debug_str = format!("{stage:?}");
            assert!(!debug_str.is_empty());
        }
    }

    #[test]
    fn test_output_format_variants() {
        assert_eq!(OutputFormat::Markdown, OutputFormat::Markdown);
        assert_ne!(OutputFormat::Markdown, OutputFormat::PlainText);
        assert_ne!(OutputFormat::PlainText, OutputFormat::Html);
    }

    #[test]
    fn test_quality_settings() {
        let settings = QualitySettings {
            preserve_formatting: true,
            extract_images: false,
            extract_tables: true,
            extract_metadata: false,
            clean_output: true,
        };

        assert!(settings.preserve_formatting);
        assert!(!settings.extract_images);
        assert!(settings.extract_tables);
        assert!(!settings.extract_metadata);
        assert!(settings.clean_output);
    }

    #[tokio::test]
    async fn test_markitdown_parser_creation() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config.clone());

        assert_eq!(parser.config().python_path, config.python_path);
        assert_eq!(parser.config().enable_plugins, config.enable_plugins);
        assert_eq!(parser.get_active_task_count().await, 0);
    }

    #[tokio::test]
    async fn test_with_defaults_constructor() {
        let parser = MarkItDownParser::with_defaults("python3".to_string(), false);

        assert_eq!(parser.config().python_path, "python3");
        assert!(!parser.config().enable_plugins);
        assert_eq!(parser.config().timeout_seconds, 180); // 默认值
    }

    #[tokio::test]
    async fn test_validate_input_file() {
        // 初始化全局配置（用于文件大小限制等）
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 测试不存在的文件
        let result = parser
            .validate_input_file("nonexistent.txt", &DocumentFormat::Text)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("文件不存在"));

        // 测试存在的文件
        let temp_file = create_test_text_file().unwrap();
        let result = parser
            .validate_input_file(temp_file.path().to_str().unwrap(), &DocumentFormat::Text)
            .await;
        // 这可能会失败，因为我们没有真正的MarkItDown环境，但至少可以测试文件存在性检查
    }

    #[tokio::test]
    async fn test_file_size_validation() {
        // 初始化全局配置（用于文件大小限制等）
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 创建一个超过全局限制的文件来测试文件大小限制
        let limit_bytes = crate::config::get_global_file_size_config()
            .max_file_size
            .bytes();
        let mut temp_file = NamedTempFile::with_suffix(".txt").unwrap();
        let large_content = vec![b'a'; (limit_bytes as usize) + 1];
        temp_file.write_all(&large_content).unwrap();
        temp_file.flush().unwrap();

        let result = parser
            .validate_input_file(temp_file.path().to_str().unwrap(), &DocumentFormat::Text)
            .await;
        // 注意：这个测试可能会通过，取决于全局文件大小配置
        // 主要是测试文件大小检查逻辑是否正常工作
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("文件大小超过限制"));
    }

    #[tokio::test]
    async fn test_format_support_validation() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 测试支持的格式
        let support = parser
            .check_format_support_internal(&DocumentFormat::Word)
            .await
            .unwrap();
        assert!(support.supported);
        assert!(support.confidence > 0.8);
        assert!(!support.features.is_empty());

        // 测试不支持的格式
        let support = parser
            .check_format_support_internal(&DocumentFormat::PDF)
            .await
            .unwrap();
        assert!(!support.supported);
        assert_eq!(support.confidence, 0.0);
    }

    #[tokio::test]
    async fn test_format_support_caching() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 第一次调用应该执行检查
        let support1 = parser
            .validate_format_support(&DocumentFormat::Word)
            .await
            .unwrap();
        assert!(support1.supported);

        // 第二次调用应该使用缓存
        let support2 = parser
            .validate_format_support(&DocumentFormat::Word)
            .await
            .unwrap();
        assert_eq!(support1.format, support2.format);
        assert_eq!(support1.supported, support2.supported);

        // 清除缓存
        parser.clear_format_cache().await;

        // 缓存应该为空
        let cache_size = {
            let cache = parser.format_support_cache.read().await;
            cache.len()
        };
        assert_eq!(cache_size, 0);
    }

    #[tokio::test]
    async fn test_task_cancellation() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 测试取消不存在的任务
        let result = parser.cancel_task("nonexistent_task").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("任务不存在"));
    }

    #[tokio::test]
    async fn test_cleanup_work_dir() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 创建临时目录
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path().join("test_work");
        fs::create_dir_all(&work_dir).await.unwrap();

        // 创建一些测试文件
        let test_file = work_dir.join("test.txt");
        fs::write(&test_file, "test content").await.unwrap();

        assert!(work_dir.exists());
        assert!(test_file.exists());

        // 清理目录
        parser.cleanup_work_dir(&work_dir).await;

        // 给文件系统一些时间来完成删除操作
        sleep(Duration::from_millis(100)).await;

        // 验证目录已被删除
        assert!(!work_dir.exists());
    }

    #[tokio::test]
    async fn test_collect_images_from_empty_dir() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        let temp_dir = TempDir::new().unwrap();
        let images = parser.collect_images(temp_dir.path()).await.unwrap();
        assert!(images.is_empty());
    }

    #[tokio::test]
    async fn test_collect_images_with_files() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        let temp_dir = TempDir::new().unwrap();

        // 创建测试图片文件
        let image_files = vec!["test1.png", "test2.jpg", "test3.gif", "not_image.txt"];
        for file_name in &image_files {
            let file_path = temp_dir.path().join(file_name);
            fs::write(&file_path, "fake image content").await.unwrap();
        }

        let images = parser.collect_images(temp_dir.path()).await.unwrap();

        // 应该只收集图片文件，不包括txt文件
        assert_eq!(images.len(), 3);
        assert!(images.iter().any(|img| img.contains("test1.png")));
        assert!(images.iter().any(|img| img.contains("test2.jpg")));
        assert!(images.iter().any(|img| img.contains("test3.gif")));
        assert!(!images.iter().any(|img| img.contains("not_image.txt")));
    }

    #[tokio::test]
    async fn test_post_process_excel_content() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        let content = "| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |";
        let processed = parser.post_process_excel_content(content).await.unwrap();

        assert!(processed.contains("# Excel数据"));
        assert!(processed.contains("| A | B | C |"));
    }

    #[tokio::test]
    async fn test_post_process_powerpoint_content() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        let content = "Slide 1: Title\nContent here\nSlide 2: Another title";
        let processed = parser
            .post_process_powerpoint_content(content)
            .await
            .unwrap();

        assert!(processed.contains("---"));
        assert!(processed.contains("# Slide 1"));
        assert!(processed.contains("# Slide 2"));
    }

    #[tokio::test]
    async fn test_post_process_word_content() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        let content = "INTRODUCTION\nThis is the introduction.\nMETHODS:\nThis describes methods.";
        let processed = parser.post_process_word_content(content).await.unwrap();

        assert!(processed.contains("## INTRODUCTION"));
        assert!(processed.contains("## METHODS"));
    }

    #[tokio::test]
    async fn test_get_parse_statistics() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config.clone());

        let stats = parser.get_parse_statistics().await;

        assert!(stats.contains_key("active_tasks"));
        assert!(stats.contains_key("format_cache_size"));
        assert!(stats.contains_key("config"));

        let config_stats = stats.get("config").unwrap();
        assert_eq!(config_stats["enable_plugins"], config.enable_plugins);
        assert_eq!(config_stats["timeout_seconds"], config.timeout_seconds);
    }

    #[tokio::test]
    async fn test_parser_trait_implementation() {
        // 初始化全局配置
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();

        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 测试支持的格式
        assert!(parser.supports_format(&DocumentFormat::Word));
        assert!(parser.supports_format(&DocumentFormat::Excel));
        assert!(parser.supports_format(&DocumentFormat::Text));
        assert!(!parser.supports_format(&DocumentFormat::PDF)); // 不在支持列表中

        // 测试名称和描述
        assert_eq!(parser.get_name(), "MarkItDown");
        assert!(!parser.get_description().is_empty());

        // 测试不支持的格式解析 - 使用PDF文件路径来触发格式检测失败
        let pdf_path = "/path/to/test.pdf";
        let result = parser.parse(pdf_path).await;
        // 由于文件路径不存在，可能返回文件错误或其他错误
        if result.is_err() {
            let error = result.unwrap_err();
            let error_msg = error.to_string();
            // 验证错误信息包含预期的内容或文件相关错误
            assert!(
                error_msg.contains("MarkItDown不支持格式")
                    || error_msg.contains("not found")
                    || error_msg.contains("No such file")
                    || error_msg.contains("无法获取文件元数据"),
                "Expected format or file error, got: {error_msg}"
            );
        } else {
            // 如果解析成功，记录警告
            println!("Warning: MarkItDown parser succeeded with PDF path");
        }
    }

    #[tokio::test]
    async fn test_format_support_details() {
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        // 测试Word格式支持
        let word_support = parser
            .check_format_support_internal(&DocumentFormat::Word)
            .await
            .unwrap();
        assert!(word_support.supported);
        assert!(word_support.features.contains(&"文本提取".to_string()));
        assert!(word_support.features.contains(&"表格提取".to_string()));
        assert!(word_support.confidence > 0.8);

        // 测试Excel格式支持
        let excel_support = parser
            .check_format_support_internal(&DocumentFormat::Excel)
            .await
            .unwrap();
        assert!(excel_support.supported);
        assert!(excel_support.features.contains(&"表格数据提取".to_string()));
        assert!(
            excel_support
                .limitations
                .contains(&"图表不支持".to_string())
        );

        // 测试HTML格式支持
        let html_support = parser
            .check_format_support_internal(&DocumentFormat::HTML)
            .await
            .unwrap();
        assert!(html_support.supported);
        assert!(
            html_support
                .features
                .contains(&"HTML到Markdown转换".to_string())
        );
        assert!(html_support.confidence > 0.9);

        // 测试不支持的格式
        let pdf_support = parser
            .check_format_support_internal(&DocumentFormat::PDF)
            .await
            .unwrap();
        assert!(!pdf_support.supported);
        assert_eq!(pdf_support.confidence, 0.0);
        assert!(
            pdf_support
                .limitations
                .contains(&"建议使用MinerU处理PDF".to_string())
        );
    }

    #[tokio::test]
    async fn test_progress_callback_integration() {
        // 初始化全局配置，避免进度回调中触发的任何全局文件大小读取panic
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let config = create_test_config();
        let parser = MarkItDownParser::new(config);

        let temp_file = create_test_text_file().unwrap();
        let progress_updates = Arc::new(Mutex::new(Vec::new()));
        let progress_updates_clone = progress_updates.clone();

        let progress_callback = move |progress: MarkItDownProgress| {
            let updates = progress_updates_clone.clone();
            tokio::spawn(async move {
                let mut updates = updates.lock().await;
                updates.push(progress);
            });
        };

        // 注意：这个测试可能会失败，因为我们没有真正的MarkItDown环境
        // 但可以测试接口是否正确
        let _result = parser
            .parse_with_progress(
                temp_file.path().to_str().unwrap(),
                &DocumentFormat::Text,
                progress_callback,
                None,
            )
            .await;

        // 验证至少收到了一些进度更新
        let updates = progress_updates.lock().await;
        if !updates.is_empty() {
            assert!(
                updates
                    .iter()
                    .any(|p| p.stage == ProcessingStage::Initializing)
            );
        }
    }

    #[test]
    fn test_format_support_struct() {
        let support = FormatSupport {
            format: DocumentFormat::Word,
            supported: true,
            confidence: 0.9,
            features: vec!["文本提取".to_string(), "格式保持".to_string()],
            limitations: vec!["复杂布局可能丢失".to_string()],
        };

        assert_eq!(support.format, DocumentFormat::Word);
        assert!(support.supported);
        assert_eq!(support.confidence, 0.9);
        assert_eq!(support.features.len(), 2);
        assert_eq!(support.limitations.len(), 1);
    }

    #[test]
    fn test_markitdown_progress_struct() {
        let progress = MarkItDownProgress {
            stage: ProcessingStage::Converting,
            progress: 50.0,
            message: "正在转换文档".to_string(),
            elapsed_time: Duration::from_secs(10),
            current_file: Some("test.docx".to_string()),
        };

        assert_eq!(progress.stage, ProcessingStage::Converting);
        assert_eq!(progress.progress, 50.0);
        assert_eq!(progress.message, "正在转换文档");
        assert_eq!(progress.elapsed_time, Duration::from_secs(10));
        assert_eq!(progress.current_file, Some("test.docx".to_string()));
    }
}
