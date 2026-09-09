use super::parser_trait::DocumentParser;
use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::models::{DocumentFormat, ParseResult, ParserEngine};
use crate::parsers::FormatDetector;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};
use uuid::{NoContext, Timestamp, Uuid};

mod environment;
mod execute;
/// 输出读取族方法（`impl MinerUParser` 的子模块拆分）。
mod output;

/// MinerU 子进程 stderr 输出行的分级（用于按内容选择日志级别，避免 tqdm/loguru 正常输出污染 ERROR 日志）。
///
/// 注意：逐行分级仅影响日志级别，**不丢错误信息**——所有 stderr 原文同时累积进 `stderr_output`，
/// 进程非零退出时会连同 exit code 一起作为 `AppError` 抛出（见调用处兜底逻辑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StderrKind {
    /// tqdm 进度条 / ANSI 转义噪音 → `trace!`（默认不采集，等价于静默）
    Progress,
    /// Python traceback / 异常 / loguru ERROR → `error!`
    Error,
    /// WARNING / DeprecationWarning 等 → `warn!`
    Warning,
    /// loguru INFO / uvicorn 关闭等正常输出 → `debug!`
    Info,
}

/// 按 MinerU stderr 行的内容判定日志级别。
///
/// MinerU（含底层 loguru、tqdm、uvicorn）把进度条、INFO 日志、关闭信息都写 stderr，
/// 若全部 `error!` 会淹没真实故障。本函数按特征字符串分级：
/// - `%|` / `it/s` / `s/it` / `█` / `\x1b[` → 进度条噪音
/// - `Traceback` / `Error:` / `ERROR` / `Exception` / `FATAL` → 真错误
/// - `WARNING` / `*Warning` → 警告
/// - 其余 → 普通 info
fn classify_mineru_stderr(line: &str) -> StderrKind {
    let l = line.trim_start();
    // tqdm 进度条（`N%|bars| N/M [.., X it/s]`）及其 ANSI 光标控制残留
    if l.contains("%|")
        || l.contains("it/s")
        || l.contains("s/it")
        || l.contains('\x1b')
        || l.contains('█')
    {
        return StderrKind::Progress;
    }
    // 真正的错误：Python 异常（`XError: ...`）、traceback 头、loguru/uvicorn ERROR 级别
    if l.contains("Traceback")
        || l.contains("Error:")
        || l.contains("ERROR")
        || l.contains("Exception")
        || l.contains("FATAL")
    {
        return StderrKind::Error;
    }
    // 警告
    if l.contains("WARNING") || l.contains("Warning") {
        return StderrKind::Warning;
    }
    StderrKind::Info
}

/// 解析进度信息
#[derive(Debug, Clone)]
pub struct ParseProgress {
    pub stage: ParseStage,
    pub progress: f32,
    pub message: String,
    pub elapsed_time: Duration,
}

/// 解析阶段
#[derive(Debug, Clone, PartialEq)]
pub enum ParseStage {
    Initializing,
    PreProcessing,
    Parsing,
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

/// 活跃任务注册守卫（RAII）
///
/// 创建时向 `active_tasks` 注册任务，`Drop` 时自动移除。确保 parse future
/// 被取消（timeout/连接中断）时 `active_tasks` 不泄漏条目。使用
/// `parking_lot::Mutex`（同步锁），使其能在 `Drop` 中安全调用。
///
/// 泛型 `T` 为各 parser 各自的 `CancellationToken` 类型（MinerU 与 MarkItDown
/// 各有独立定义），由调用处类型推导自动适配。
pub(crate) struct ActiveTaskGuard<T> {
    active_tasks: Arc<Mutex<std::collections::HashMap<String, T>>>,
    task_id: String,
}

impl<T> ActiveTaskGuard<T> {
    /// 注册任务并创建守卫
    pub(crate) fn new(
        active_tasks: Arc<Mutex<std::collections::HashMap<String, T>>>,
        task_id: String,
        token: T,
    ) -> Self {
        active_tasks.lock().insert(task_id.clone(), token);
        Self {
            active_tasks,
            task_id,
        }
    }
}

impl<T> Drop for ActiveTaskGuard<T> {
    fn drop(&mut self) {
        self.active_tasks.lock().remove(&self.task_id);
    }
}

// MinerUConfig 和 QualityLevel 现在在 crate::config 中定义
pub use crate::config::{MinerUConfig, QualityLevel};

/// MinerUParser PDF解析器
pub struct MinerUParser {
    config: MinerUConfig,
    active_tasks: Arc<Mutex<std::collections::HashMap<String, CancellationToken>>>,
}

impl MinerUParser {
    /// 创建新的MinerU解析器
    pub fn new(config: MinerUConfig) -> Self {
        Self {
            config,
            active_tasks: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// 创建带默认配置的解析器
    pub fn with_defaults(python_path: String, backend: String, device: Option<String>) -> Self {
        let config = MinerUConfig {
            python_path,
            backend,
            device: device.unwrap_or_else(|| "cpu".to_string()),
            ..Default::default()
        };
        Self::new(config)
    }

    /// 创建自动检测当前目录虚拟环境的解析器
    pub fn with_auto_venv_detection() -> Result<Self, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::MinerU(format!("无法获取当前目录: {e}")))?;

        let venv_path = current_dir.join("venv");
        let python_path = if cfg!(windows) {
            venv_path.join("Scripts").join("python.exe")
        } else {
            venv_path.join("bin").join("python")
        };

        // 尝试从全局配置获取MinerU配置，如果失败则使用默认值
        let (backend, device, vram, gpu_memory_utilization) =
            match std::panic::catch_unwind(crate::config::get_global_config) {
                Ok(global_config) => (
                    global_config.mineru.backend.clone(),
                    global_config.mineru.device.clone(),
                    global_config.mineru.vram,
                    global_config.mineru.gpu_memory_utilization,
                ),
                Err(_) => ("pipeline".to_string(), "cpu".to_string(), 0, 0.0),
            };

        let config = MinerUConfig {
            python_path: python_path.to_string_lossy().to_string(),
            backend,
            device,
            vram,
            gpu_memory_utilization,
            ..Default::default()
        };

        Ok(Self::new(config))
    }

    /// 获取配置
    pub fn config(&self) -> &MinerUConfig {
        &self.config
    }

    /// 带进度跟踪和取消支持的解析
    pub async fn parse_with_progress<F>(
        &self,
        file_path: &str,
        progress_callback: F,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<ParseResult, AppError>
    where
        F: Fn(ParseProgress) + Send + Sync + 'static,
    {
        let start_time = Instant::now();
        let task_id = Uuid::new_v7(Timestamp::now(NoContext)).to_string();

        // 注册取消令牌（RAII 守卫：future 取消时 Drop 自动从 active_tasks 移除，不泄漏）
        let token = cancellation_token.unwrap_or_default();
        let _task_guard =
            ActiveTaskGuard::new(self.active_tasks.clone(), task_id.clone(), token.clone());

        self.parse_internal_with_progress(
            file_path,
            &task_id,
            progress_callback,
            token.clone(),
            start_time,
        )
        .await
    }

    /// 取消指定任务
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), AppError> {
        // 锁内取出 token 克隆后立即释放锁，避免持锁跨 await
        let token = self.active_tasks.lock().get(task_id).cloned();
        if let Some(token) = token {
            token.cancel().await;
            info!("MinerU analysis task canceled: {}", task_id);
            Ok(())
        } else {
            Err(AppError::MinerU(format!("任务不存在: {task_id}")))
        }
    }

    /// 获取活跃任务数量
    pub fn get_active_task_count(&self) -> usize {
        self.active_tasks.lock().len()
    }

    /// 内部解析实现（带进度跟踪）
    async fn parse_internal_with_progress<F>(
        &self,
        file_path: &str,
        task_id: &str,
        progress_callback: F,
        cancellation_token: CancellationToken,
        start_time: Instant,
    ) -> Result<ParseResult, AppError>
    where
        F: Fn(ParseProgress) + Send + Sync + 'static,
    {
        // 初始化阶段
        progress_callback(ParseProgress {
            stage: ParseStage::Initializing,
            progress: 0.0,
            message: "初始化解析环境".to_string(),
            elapsed_time: start_time.elapsed(),
        });

        // 验证文件
        self.validate_input_file(file_path).await?;

        if cancellation_token.is_cancelled().await {
            return Err(AppError::MinerU("解析已取消".to_string()));
        }

        // 预处理阶段
        progress_callback(ParseProgress {
            stage: ParseStage::PreProcessing,
            progress: 10.0,
            message: "准备工作环境".to_string(),
            elapsed_time: start_time.elapsed(),
        });

        let work_dir = Path::new("temp/mineru").join(task_id);
        fs::create_dir_all(&work_dir)
            .await
            .map_err(|e| AppError::File(format!("创建工作目录失败: {e}")))?;

        let output_dir = work_dir.join("output");
        fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| AppError::File(format!("创建输出目录失败: {e}")))?;

        info!(
            "Use MinerU to parse PDF files: {} -> {}",
            file_path,
            work_dir.display()
        );

        if cancellation_token.is_cancelled().await {
            self.cleanup_work_dir(&work_dir).await;
            return Err(AppError::MinerU("解析已取消".to_string()));
        }

        // 解析阶段
        progress_callback(ParseProgress {
            stage: ParseStage::Parsing,
            progress: 20.0,
            message: "正在解析PDF文档".to_string(),
            elapsed_time: start_time.elapsed(),
        });

        let parse_result = self
            .execute_mineru_command(
                file_path,
                &output_dir,
                &progress_callback,
                &cancellation_token,
                start_time,
            )
            .await;

        if let Err(e) = &parse_result {
            error!("MinerU command execution failed: {}", e);
            error!("Working directory: {}", work_dir.display());
            error!("Output directory: {}", output_dir.display());
            error!("Input file: {}", file_path);
            error!("Task ID: {}", task_id);

            // 检查输入文件状态
            match fs::metadata(file_path).await {
                Ok(metadata) => {
                    debug!("Input file size: {} bytes", metadata.len());
                    debug!("Input file modification time: {:?}", metadata.modified());
                }
                Err(file_err) => {
                    error!("Unable to read input file metadata: {}", file_err);
                }
            }

            // 检查工作目录状态
            if work_dir.exists() {
                match fs::read_dir(&work_dir).await {
                    Ok(_) => {
                        debug!(
                            "The working directory exists, directory: {}",
                            &work_dir.display()
                        );
                    }
                    Err(dir_err) => {
                        error!("Unable to read working directory: {}", dir_err);
                    }
                }
            } else {
                warn!(
                    "The working directory does not exist, directory: {}",
                    &work_dir.display()
                );
            }

            // 检查输出目录是否存在以及内容
            if output_dir.exists() {
                match self.debug_output_directory(&output_dir).await {
                    Ok(debug_info) => {
                        error!("Output directory debugging information: {}", debug_info);
                    }
                    Err(debug_err) => {
                        error!(
                            "Unable to obtain output directory debugging information: {}",
                            debug_err
                        );
                    }
                }
            } else {
                error!(
                    "The output directory does not exist: {}",
                    output_dir.display()
                );
            }

            self.cleanup_work_dir(&work_dir).await;
            return Err(e.clone());
        }

        if cancellation_token.is_cancelled().await {
            self.cleanup_work_dir(&work_dir).await;
            return Err(AppError::MinerU("解析已取消".to_string()));
        }

        // 后处理阶段
        progress_callback(ParseProgress {
            stage: ParseStage::PostProcessing,
            progress: 80.0,
            message: "处理解析结果".to_string(),
            elapsed_time: start_time.elapsed(),
        });
        info!("The output directory of minerU: {}", output_dir.display());
        info!("Prepare to read the output of minerU, task_id: {}", task_id);

        let markdown_content = self.read_markdown_output(&output_dir).await?;

        // 完成阶段
        progress_callback(ParseProgress {
            stage: ParseStage::Finalizing,
            progress: 95.0,
            message: "生成最终结果".to_string(),
            elapsed_time: start_time.elapsed(),
        });

        let processing_time = start_time.elapsed();
        let word_count = markdown_content.split_whitespace().count();

        let mut result =
            ParseResult::new(markdown_content, DocumentFormat::PDF, ParserEngine::MinerU);

        // 记录 MinerU 的输出目录与任务工作目录，供后续逻辑复用
        result.output_dir = Some(
            output_dir
                .canonicalize()
                .unwrap_or(output_dir.clone())
                .to_string_lossy()
                .to_string(),
        );
        result.work_dir = Some(
            work_dir
                .canonicalize()
                .unwrap_or(work_dir.clone())
                .to_string_lossy()
                .to_string(),
        );

        result.set_processing_time(processing_time.as_secs_f64());
        result.set_error_count(0);

        // 注意：不在此处清理工作目录，交由上层在完成图片上传与路径替换后统一清理
        // 这样能够保证后续能够访问到 MinerU 的输出目录（例如 images/auto/images）

        progress_callback(ParseProgress {
            stage: ParseStage::Completed,
            progress: 100.0,
            message: format!("解析完成，耗时: {processing_time:?}，字数: {word_count}"),
            elapsed_time: processing_time,
        });

        info!(
            "MinerU analysis is completed, time consumption: {:?}, word count: {}",
            processing_time, word_count
        );

        Ok(result)
    }

    /// 验证输入文件
    async fn validate_input_file(&self, file_path: &str) -> Result<(), AppError> {
        let path = Path::new(file_path);

        if !path.exists() {
            return Err(AppError::File(format!("文件不存在: {file_path}")));
        }

        let metadata = fs::metadata(path)
            .await
            .map_err(|e| AppError::File(format!("无法读取文件元数据: {e}")))?;

        let file_size_bytes = metadata.len();
        let global_config = GlobalFileSizeConfig::default();
        if file_size_bytes > global_config.max_file_size.bytes() {
            return Err(AppError::File(format!(
                "文件大小超过限制: {}MB > {}MB",
                file_size_bytes / (1024 * 1024),
                global_config.max_file_size.bytes() / (1024 * 1024)
            )));
        }

        // 验证文件格式
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            if extension.to_lowercase() != "pdf" {
                return Err(AppError::UnsupportedFormat(format!(
                    "MinerU只支持PDF格式，当前文件: {extension}"
                )));
            }
        } else {
            return Err(AppError::UnsupportedFormat("无法确定文件格式".to_string()));
        }

        Ok(())
    }

    /// 获取解析统计信息
    pub async fn get_parse_statistics(
        &self,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        let mut stats = std::collections::HashMap::new();

        let active_count = self.get_active_task_count();
        stats.insert(
            "active_tasks".to_string(),
            serde_json::Value::Number(active_count.into()),
        );
        stats.insert(
            "config".to_string(),
            serde_json::json!({
                "backend": self.config.backend,
                "timeout": if self.config.timeout == 0 { 3600 } else { self.config.timeout },
                "quality_level": format!("{:?}", self.config.quality_level),
            }),
        );

        stats
    }
}

#[async_trait]
impl DocumentParser for MinerUParser {
    #[instrument(skip(self), fields(file_path = %file_path))]
    async fn parse(&self, file_path: &str) -> Result<ParseResult, AppError> {
        let detector = FormatDetector::new();
        let detection = detector.detect_format(file_path, None)?;
        let format = detection.format;

        if !self.supports_format(&format) {
            return Err(AppError::UnsupportedFormat(format!(
                "MinerU不支持格式: {format:?}"
            )));
        }
        // 解析PDF文件
        self.parse_with_progress(
            file_path,
            |progress| {
                info!("MinerU parsing progress: {:?}", progress);
            },
            None,
        )
        .await
    }

    fn supports_format(&self, format: &DocumentFormat) -> bool {
        matches!(format, DocumentFormat::PDF)
    }

    fn get_name(&self) -> &'static str {
        "MinerU"
    }

    fn get_description(&self) -> &'static str {
        "高精度PDF文档解析引擎，支持复杂布局和公式识别"
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

    fn create_test_config() -> MinerUConfig {
        MinerUConfig {
            backend: "local".to_string(),
            python_path: "python3".to_string(),
            max_concurrent: 3,
            queue_size: 100,
            timeout: 30,
            batch_size: 1,
            quality_level: QualityLevel::Fast,
            device: "cpu".to_string(),
            vram: 8, // 默认显存限制
            gpu_memory_utilization: 0.0,
        }
    }

    fn create_test_pdf() -> Result<NamedTempFile, std::io::Error> {
        let mut temp_file = NamedTempFile::new()?;
        // 创建一个简单的PDF文件头
        temp_file
            .write_all(b"%PDF-1.4\n1 0 obj\n<<\n/Type /Catalog\n/Pages 2 0 R\n>>\nendobj\n")?;
        temp_file.flush()?;
        Ok(temp_file)
    }

    #[test]
    fn test_mineru_config_default() {
        let config = MinerUConfig::default();
        if cfg!(windows) {
            assert_eq!(config.python_path, "./venv/Scripts/python.exe");
        } else {
            assert_eq!(config.python_path, "./venv/bin/python");
        }
        assert_eq!(config.backend, "pipeline");
        assert_eq!(config.timeout, 0);
        assert_eq!(config.quality_level, QualityLevel::Balanced);
        // device 平台感知:macOS 默认 mps,其他默认 cpu(见 default_device_for_platform)
        if cfg!(target_os = "macos") {
            assert_eq!(config.device, "mps");
        } else {
            assert_eq!(config.device, "cpu");
        }
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
    fn test_parse_progress() {
        let progress = ParseProgress {
            stage: ParseStage::Parsing,
            progress: 50.0,
            message: "测试进度".to_string(),
            elapsed_time: Duration::from_secs(10),
        };

        assert_eq!(progress.stage, ParseStage::Parsing);
        assert_eq!(progress.progress, 50.0);
        assert_eq!(progress.message, "测试进度");
    }

    #[tokio::test]
    async fn test_mineru_parser_creation() {
        let config = create_test_config();
        let parser = MinerUParser::new(config.clone());

        assert_eq!(parser.config().python_path, config.python_path);
        assert_eq!(parser.config().backend, config.backend);
        assert_eq!(parser.config().device, config.device);
        assert_eq!(parser.get_active_task_count(), 0);
    }

    #[tokio::test]
    async fn test_validate_input_file() {
        let config = create_test_config();
        let parser = MinerUParser::new(config);

        // 测试不存在的文件
        let result = parser.validate_input_file("nonexistent.pdf").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("文件不存在"));

        // 测试存在的PDF文件
        let temp_pdf = create_test_pdf().unwrap();
        let _result = parser
            .validate_input_file(temp_pdf.path().to_str().unwrap())
            .await;
        // 注意：这可能会失败，因为我们创建的不是真正的PDF文件
        // 但至少可以测试文件存在性检查
    }

    #[tokio::test]
    async fn test_file_size_validation() {
        let config = create_test_config();
        let parser = MinerUParser::new(config);

        // 创建一个大文件来测试文件大小限制
        let mut temp_file = NamedTempFile::with_suffix(".pdf").unwrap();
        let large_content = vec![0u8; 1024 * 1024 * 100]; // 100MB
        temp_file.write_all(&large_content).unwrap();
        temp_file.flush().unwrap();

        let _result = parser
            .validate_input_file(temp_file.path().to_str().unwrap())
            .await;
        // 注意：这个测试可能会通过，取决于全局文件大小配置
        // 主要是测试文件大小检查逻辑是否正常工作
    }

    #[tokio::test]
    async fn test_unsupported_format_validation() {
        let config = create_test_config();
        let parser = MinerUParser::new(config);

        // 创建一个非PDF文件
        let mut temp_file = NamedTempFile::with_suffix(".txt").unwrap();
        temp_file.write_all(b"This is not a PDF").unwrap();
        temp_file.flush().unwrap();

        let result = parser
            .validate_input_file(temp_file.path().to_str().unwrap())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("只支持PDF格式"));
    }

    #[tokio::test]
    async fn test_task_cancellation() {
        let config = create_test_config();
        let parser = MinerUParser::new(config);

        // 测试取消不存在的任务
        let result = parser.cancel_task("nonexistent_task").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("任务不存在"));
    }

    #[tokio::test]
    async fn test_cleanup_work_dir() {
        let config = create_test_config();
        let parser = MinerUParser::new(config);

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
    async fn test_get_parse_statistics() {
        let config = create_test_config();
        let parser = MinerUParser::new(config.clone());

        let stats = parser.get_parse_statistics().await;

        assert!(stats.contains_key("active_tasks"));
        assert!(stats.contains_key("config"));

        let config_stats = stats.get("config").unwrap();
        assert_eq!(config_stats["backend"], config.backend);
        assert_eq!(
            config_stats["timeout"],
            if config.timeout == 0 {
                3600
            } else {
                config.timeout
            }
        );
    }

    #[test]
    fn test_quality_level_variants() {
        assert_eq!(QualityLevel::Fast, QualityLevel::Fast);
        assert_ne!(QualityLevel::Fast, QualityLevel::Balanced);
        assert_ne!(QualityLevel::Balanced, QualityLevel::HighQuality);
    }

    #[test]
    fn test_parse_stage_variants() {
        let stages = vec![
            ParseStage::Initializing,
            ParseStage::PreProcessing,
            ParseStage::Parsing,
            ParseStage::PostProcessing,
            ParseStage::Finalizing,
            ParseStage::Completed,
            ParseStage::Failed,
            ParseStage::Cancelled,
        ];

        for stage in stages {
            // 测试Debug trait
            let debug_str = format!("{stage:?}");
            assert!(!debug_str.is_empty());
        }
    }

    #[tokio::test]
    async fn test_parser_trait_implementation() {
        // 初始化全局配置
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();

        let config = create_test_config();
        let parser = MinerUParser::new(config);

        // 测试支持的格式
        assert!(parser.supports_format(&DocumentFormat::PDF));
        assert!(!parser.supports_format(&DocumentFormat::Word));
        assert!(!parser.supports_format(&DocumentFormat::Excel));

        // 测试名称和描述
        assert_eq!(parser.get_name(), "MinerU");
        assert!(!parser.get_description().is_empty());

        // 测试不支持的格式解析 - 使用Word文件路径来触发格式检测失败
        let word_path = "/path/to/test.docx";
        let result = parser.parse(word_path).await;
        // 由于文件路径不存在，可能返回文件错误或其他错误
        if let Err(error) = result {
            let error_msg = error.to_string();
            // 验证错误信息包含预期的内容或文件相关错误
            assert!(
                error_msg.contains("MinerU不支持格式")
                    || error_msg.contains("not found")
                    || error_msg.contains("No such file")
                    || error_msg.contains("无法获取文件元数据"),
                "Expected format or file error, got: {error_msg}"
            );
        } else {
            // 如果解析成功，记录警告
            println!("Warning: MinerU parser succeeded with Word path");
        }
    }

    #[tokio::test]
    async fn test_with_defaults_constructor() {
        // 测试指定device的情况
        let parser = MinerUParser::with_defaults(
            "python3".to_string(),
            "cpu".to_string(),
            Some("cuda".to_string()),
        );
        assert_eq!(parser.config().python_path, "python3");
        assert_eq!(parser.config().backend, "cpu");
        assert_eq!(parser.config().device, "cuda");
        assert_eq!(parser.config().timeout, 0); // 默认值，0表示使用统一的超时配置

        // 测试device为None时使用默认值的情况
        let parser_default =
            MinerUParser::with_defaults("python3".to_string(), "cpu".to_string(), None);
        assert_eq!(parser_default.config().device, "cpu");
    }

    #[tokio::test]
    async fn test_progress_callback_integration() {
        let config = create_test_config();
        let parser = MinerUParser::new(config);

        let temp_pdf = create_test_pdf().unwrap();
        let progress_updates = Arc::new(Mutex::new(Vec::new()));
        let progress_updates_clone = progress_updates.clone();

        let progress_callback = move |progress: ParseProgress| {
            let updates = progress_updates_clone.clone();
            tokio::spawn(async move {
                let mut updates = updates.lock();
                updates.push(progress);
            });
        };

        // 注意：这个测试可能会失败，因为我们没有真正的MinerU环境
        // 但可以测试接口是否正确
        let _result = parser
            .parse_with_progress(temp_pdf.path().to_str().unwrap(), progress_callback, None)
            .await;

        // 验证至少收到了一些进度更新
        let updates = progress_updates.lock();
        if !updates.is_empty() {
            assert!(updates.iter().any(|p| p.stage == ParseStage::Initializing));
        }
    }

    #[test]
    fn test_classify_mineru_stderr_progress_bars() {
        // tqdm 进度条（各种形态）→ Progress
        assert_eq!(
            classify_mineru_stderr("Layout Predict: 100%|██████████| 1/1 [00:00<00:00, 1.49it/s]"),
            StderrKind::Progress
        );
        assert_eq!(
            classify_mineru_stderr("OCR-det ch:   0%|          | 0/6 [00:00<?, ?it/s]"),
            StderrKind::Progress
        );
        assert_eq!(
            classify_mineru_stderr("MFR Predict: 100%|██████████| 1/1 [00:00<00:02, 2.45s/it]"),
            StderrKind::Progress
        );
        // ANSI 光标控制残留（tqdm 原地刷新）
        assert_eq!(
            classify_mineru_stderr("\x1b[AITER: 100%|████| 1/1"),
            StderrKind::Progress
        );
    }

    #[test]
    fn test_classify_mineru_stderr_errors() {
        // Python traceback / 异常 → Error
        assert_eq!(
            classify_mineru_stderr("Traceback (most recent call last):"),
            StderrKind::Error
        );
        assert_eq!(
            classify_mineru_stderr("RuntimeError: Expected one of cpu, cuda, mps, meta device"),
            StderrKind::Error
        );
        assert_eq!(
            classify_mineru_stderr("ModuleNotFoundError: No module named 'vllm'"),
            StderrKind::Error
        );
        assert_eq!(
            classify_mineru_stderr("ValueError: invalid argument"),
            StderrKind::Error
        );
        // loguru / uvicorn ERROR 级别
        assert_eq!(
            classify_mineru_stderr("2026-07-06 | ERROR | mineru.cli - something failed"),
            StderrKind::Error
        );
    }

    #[test]
    fn test_classify_mineru_stderr_warnings() {
        assert_eq!(
            classify_mineru_stderr("UserWarning: torch.meshgrid is deprecated"),
            StderrKind::Warning
        );
        assert_eq!(
            classify_mineru_stderr("2026-07-06 | WARNING | mineru - low memory"),
            StderrKind::Warning
        );
        assert_eq!(
            classify_mineru_stderr("DeprecationWarning: use new API"),
            StderrKind::Warning
        );
    }

    #[test]
    fn test_classify_mineru_stderr_info() {
        // loguru INFO / uvicorn 关闭等正常输出 → Info（不再误报 ERROR）
        assert_eq!(
            classify_mineru_stderr("2026-07-06 | INFO | mineru.backend.pipeline - init done!"),
            StderrKind::Info
        );
        assert_eq!(
            classify_mineru_stderr("INFO:     Application shutdown complete."),
            StderrKind::Info
        );
        assert_eq!(
            classify_mineru_stderr("INFO:     Finished server process [72373]"),
            StderrKind::Info
        );
        assert_eq!(classify_mineru_stderr(""), StderrKind::Info);
    }
}
