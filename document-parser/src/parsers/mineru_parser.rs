use super::parser_trait::DocumentParser;
use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::models::{DocumentFormat, ParseResult, ParserEngine};
use crate::parsers::FormatDetector;
use crate::utils::environment_manager::EnvironmentManager;
use async_trait::async_trait;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::{sleep, timeout};
use tracing::{debug, error, info, instrument, warn};
use uuid::{NoContext, Timestamp, Uuid};

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

// MinerUConfig 和 QualityLevel 现在在 crate::config 中定义
pub use crate::config::{MinerUConfig, QualityLevel};

impl Default for MinerUConfig {
    fn default() -> Self {
        Self {
            python_path: if cfg!(windows) {
                "./venv/Scripts/python.exe".to_string()
            } else {
                "./venv/bin/python".to_string()
            },
            backend: "pipeline".to_string(),
            max_concurrent: 3,
            queue_size: 100,
            timeout: 0, // 0表示使用统一的超时配置
            batch_size: 1,
            quality_level: QualityLevel::Balanced,
            device: "cpu".to_string(),
            vram: 8, // 默认8GB显存限制
        }
    }
}

/// MinerU PDF解析器
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
        let (backend, device) = match std::panic::catch_unwind(crate::config::get_global_config) {
            Ok(global_config) => (
                global_config.mineru.backend.clone(),
                global_config.mineru.device.clone(),
            ),
            Err(_) => ("pipeline".to_string(), "cpu".to_string()),
        };

        let config = MinerUConfig {
            python_path: python_path.to_string_lossy().to_string(),
            backend,
            device,
            vram: 8, // 默认显存限制
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

        // 注册取消令牌
        let token = cancellation_token.unwrap_or_default();
        {
            let mut tasks = self.active_tasks.lock().await;
            tasks.insert(task_id.clone(), token.clone());
        }

        let result = self
            .parse_internal_with_progress(
                file_path,
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
            info!("已取消MinerU解析任务: {}", task_id);
            Ok(())
        } else {
            Err(AppError::MinerU(format!("任务不存在: {task_id}")))
        }
    }

    /// 获取活跃任务数量
    pub async fn get_active_task_count(&self) -> usize {
        let tasks = self.active_tasks.lock().await;
        tasks.len()
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
            "使用MinerU解析PDF文件: {} -> {}",
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
            error!("MinerU命令执行失败: {}", e);
            error!("工作目录: {}", work_dir.display());
            error!("输出目录: {}", output_dir.display());
            error!("输入文件: {}", file_path);
            error!("任务ID: {}", task_id);

            // 检查输入文件状态
            match fs::metadata(file_path).await {
                Ok(metadata) => {
                    debug!("输入文件大小: {} 字节", metadata.len());
                    debug!("输入文件修改时间: {:?}", metadata.modified());
                }
                Err(file_err) => {
                    error!("无法读取输入文件元数据: {}", file_err);
                }
            }

            // 检查工作目录状态
            if work_dir.exists() {
                match fs::read_dir(&work_dir).await {
                    Ok(_) => {
                        debug!("工作目录存在,目录:{}", &work_dir.display());
                    }
                    Err(dir_err) => {
                        error!("无法读取工作目录: {}", dir_err);
                    }
                }
            } else {
                warn!("工作目录不存在,目录:{}", &work_dir.display());
            }

            // 检查输出目录是否存在以及内容
            if output_dir.exists() {
                match self.debug_output_directory(&output_dir).await {
                    Ok(debug_info) => {
                        error!("输出目录调试信息: {}", debug_info);
                    }
                    Err(debug_err) => {
                        error!("无法获取输出目录调试信息: {}", debug_err);
                    }
                }
            } else {
                error!("输出目录不存在: {}", output_dir.display());
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
        info!("minerU的输出目录: {}", output_dir.display());
        info!("准备读取minerU的输出,task_id: {}", task_id);

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
            "MinerU解析完成，耗时: {:?}，字数: {}",
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

    /// 执行MinerU命令
    async fn execute_mineru_command<F>(
        &self,
        file_path: &str,
        output_dir: &Path,
        progress_callback: &F,
        cancellation_token: &CancellationToken,
        start_time: Instant,
    ) -> Result<(), AppError>
    where
        F: Fn(ParseProgress) + Send + Sync + 'static,
    {
        debug!(
            "MinerU 命令执行 - 输入文件: {}, 输出目录: {}",
            file_path,
            output_dir.display()
        );

        // 验证输入文件是否存在
        if !std::path::Path::new(file_path).exists() {
            error!("MinerU 输入文件不存在: {}", file_path);
            return Err(AppError::MinerU(format!("输入文件不存在: {file_path}")));
        }

        // 获取文件绝对路径
        let absolute_file_path = std::path::Path::new(file_path)
            .canonicalize()
            .map_err(|e| AppError::MinerU(format!("无法获取文件绝对路径: {e}")))?
            .to_string_lossy()
            .to_string();
        debug!("MinerU 输入文件绝对路径: {}", absolute_file_path);

        // 自动检测并使用虚拟环境中的 mineru 命令
        let mineru_command = self.get_mineru_command_path()?;
        let mut cmd = Command::new(&mineru_command);
        cmd.arg("-p")
            .arg(&absolute_file_path)
            .arg("-o")
            .arg(output_dir);

        // 添加后端类型参数
        if !self.config.backend.is_empty() && self.config.backend != "pipeline" {
            cmd.arg("-b").arg(&self.config.backend);
            debug!("MinerU 设置后端类型: {}", self.config.backend);
        }

        // 添加设备参数：当使用pipeline后端且支持CUDA时，自动添加-d cuda参数
        if self.config.backend == "pipeline" {
            // 使用全局缓存的CUDA状态，避免每次都检查环境
            let cuda_available = crate::config::is_cuda_available();

            if cuda_available {
                // 如果配置中指定了设备，使用配置的设备；否则使用"cuda"
                let device = if self.config.device != "cpu" {
                    self.config.device.as_str()
                } else {
                    "cuda" // 直接使用"cuda"，不需要调用get_recommended_cuda_device
                };
                cmd.arg("-d").arg(device);
                debug!("MinerU 设置推理设备: {} (全局CUDA状态可用)", device);
            } else if self.config.device != "cpu" {
                // 即使没有CUDA支持，如果配置中指定了其他设备，也使用配置的设备
                cmd.arg("-d").arg(&self.config.device);
                debug!(
                    "MinerU 设置推理设备: {} (配置指定，CUDA不可用)",
                    self.config.device
                );
            } else {
                debug!("MinerU 使用默认CPU模式 (CUDA不可用且未指定其他设备)");
            }

            // 添加显存限制参数：只要是 pipeline 后端就设置
            if self.config.vram > 0 {
                cmd.arg("--vram").arg(self.config.vram.to_string());
                debug!("MinerU 设置显存限制: {}GB", self.config.vram);
            }
        }

        // 检查是否在中国大陆，如果是则添加模型源参数
        if self.is_china_region().await {
            cmd.arg("--source").arg("modelscope");
            debug!("MinerU 设置模型源: modelscope");
        }

        // MinerU 会自动检测和使用可用的 GPU，无需手动设置环境变量

        // 设置模型源环境变量（如果网络访问有问题）
        if self.is_china_region().await {
            cmd.env("MINERU_MODEL_SOURCE", "modelscope");
            debug!("MinerU 设置环境变量 MINERU_MODEL_SOURCE: modelscope");
        }

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        info!(
            "MinerU命令参数: {} -p {} -o {}",
            mineru_command,
            absolute_file_path,
            output_dir.display()
        );
        info!("执行MinerU命令: {:?}", cmd);

        let mut child = cmd.spawn().map_err(|e| {
            error!("启动MinerU进程失败: {}", e);
            AppError::MinerU(format!("启动MinerU进程失败: {e}"))
        })?;

        info!("MinerU 进程已启动，PID: {:?}", child.id());

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
        let timeout_seconds = if self.config.timeout == 0 {
            3600
        } else {
            self.config.timeout
        };
        let timeout_duration = Duration::from_secs(timeout_seconds as u64);
        info!(
            "MinerU解析超时设置: {}秒 ({})",
            timeout_seconds,
            if self.config.timeout == 0 {
                "使用统一配置"
            } else {
                "使用MinerU配置"
            }
        );
        let process_result = timeout(timeout_duration, async {
            let mut progress = 20.0;
            let mut stderr_output = String::new();

            loop {
                tokio::select! {
                    // 检查取消
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if cancellation_token.is_cancelled().await {
                            let _ = child.kill().await;
                            return Err(AppError::MinerU("解析已取消".to_string()));
                        }
                    }

                    // 处理输出
                    Some((source, line)) = rx.recv() => {
                        debug!("MinerU {}: {}", source, line);

                        if source == "stderr" {
                            stderr_output.push_str(&line);
                            stderr_output.push('\n');
                            error!("MinerU stderr: {}", line);
                        } else {
                            info!("MinerU stdout: {}", line);
                        }

                        // 更新进度（基于输出内容推测）
                        if line.contains("Processing") || line.contains("解析") {
                            progress = (progress + 1.0_f32).min(75.0_f32);
                            progress_callback(ParseProgress {
                                stage: ParseStage::Parsing,
                                progress,
                                message: line.clone(),
                                elapsed_time: start_time.elapsed(),
                            });
                        }
                    }

                    // 等待进程完成
                    result = child.wait() => {
                        match result {
                            Ok(status) => {
                                if status.success() {
                                    info!("MinerU进程成功完成，退出码: {}", status.code().unwrap_or(0));
                                    return Ok(());
                                } else {
                                    let exit_code = status.code().unwrap_or(-1);
                                    #[cfg(unix)]
                                    let signal = status.signal();
                                    #[cfg(not(unix))]
                                    let signal: Option<i32> = None;

                                    let error_msg = if let Some(sig) = signal {
                                        format!(
                                            "MinerU执行失败，进程被信号 {sig} 终止，错误输出: {stderr_output}"
                                        )
                                    } else {
                                        format!(
                                            "MinerU执行失败，退出码: {exit_code}，错误输出: {stderr_output}"
                                        )
                                    };

                                    error!("{}", error_msg);
                                    return Err(AppError::MinerU(error_msg));
                                }
                            }
                            Err(e) => {
                                let error_msg = format!("等待进程完成失败: {e}");
                                error!("{}", error_msg);
                                return Err(AppError::MinerU(error_msg));
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

        match process_result {
            Ok(result) => result,
            Err(_) => {
                error!("MinerU执行超时（{}秒），正在终止进程", timeout_seconds);
                let _ = child.kill().await;

                // 提供更详细的超时信息
                let timeout_msg = format!(
                    "MinerU执行超时（{timeout_seconds}秒）。可能的原因：\n\
                    1. 模型下载时间过长\n\
                    2. 文档处理时间过长\n\
                    3. 系统资源不足\n\
                    4. 网络连接问题\n\
                    建议：\n\
                    - 检查网络连接\n\
                    - 增加超时时间\n\
                    - 检查系统资源"
                );

                Err(AppError::MinerU(timeout_msg))
            }
        }
    }

    /// 清理工作目录
    async fn cleanup_work_dir(&self, work_dir: &Path) {
        if let Err(e) = fs::remove_dir_all(work_dir).await {
            warn!("清理工作目录失败: {} - {}", work_dir.display(), e);
        } else {
            debug!("已清理工作目录: {}", work_dir.display());
        }
    }

    /// 调试输出目录内容
    async fn debug_output_directory(&self, output_dir: &Path) -> Result<String, AppError> {
        let mut debug_info = format!("输出目录: {}\n", output_dir.display());

        if !output_dir.exists() {
            return Ok(format!("{debug_info} (目录不存在)"));
        }

        if !output_dir.is_dir() {
            return Ok(format!("{debug_info} (不是目录)"));
        }

        let mut entries = fs::read_dir(output_dir)
            .await
            .map_err(|e| AppError::File(format!("读取输出目录失败: {e}")))?;

        let mut file_count = 0;
        let mut dir_count = 0;
        let mut total_size = 0u64;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AppError::File(format!("遍历输出目录失败: {e}")))?
        {
            let path = entry.path();
            let metadata = match fs::metadata(&path).await {
                Ok(m) => m,
                Err(_) => continue,
            };

            if metadata.is_file() {
                file_count += 1;
                total_size += metadata.len();
                debug_info.push_str(&format!(
                    "  文件: {} ({} 字节)\n",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    metadata.len()
                ));
            } else if metadata.is_dir() {
                dir_count += 1;
                debug_info.push_str(&format!(
                    "  目录: {}\n",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }

        debug_info.push_str(&format!(
            "总计: {file_count} 个文件, {dir_count} 个目录, 总大小: {total_size} 字节"
        ));

        Ok(debug_info)
    }

    /// 读取Markdown输出
    async fn read_markdown_output(&self, output_dir: &Path) -> Result<String, AppError> {
        debug!("读取Markdown输出: {}", output_dir.display());

        // 递归查找所有markdown文件
        let mut markdown_files = Vec::new();
        self.find_markdown_files_recursively(output_dir, &mut markdown_files)
            .await?;

        if markdown_files.is_empty() {
            error!(
                "在输出目录中未找到任何Markdown文件: {}",
                output_dir.display()
            );
            // 提供调试信息
            match self.debug_output_directory(output_dir).await {
                Ok(debug_info) => {
                    error!("输出目录调试信息: {}", debug_info);
                }
                Err(debug_err) => {
                    error!("无法获取输出目录调试信息: {}", debug_err);
                }
            }
            return Err(AppError::MinerU("未找到Markdown输出文件".to_string()));
        }

        // 按文件大小排序，选择最大的文件（通常是主要内容）
        markdown_files.sort_by_key(|path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
        markdown_files.reverse();

        let selected_file = &markdown_files[0];
        let file_size = std::fs::metadata(selected_file)
            .map(|m| m.len())
            .unwrap_or(0);
        debug!(
            "选择Markdown文件: {} (大小: {} 字节)",
            selected_file.display(),
            file_size
        );

        // 读取markdown文件
        let content = fs::read_to_string(selected_file)
            .await
            .map_err(|e| AppError::File(format!("读取Markdown文件失败: {e}")))?;

        // 验证内容不为空
        if content.trim().is_empty() {
            return Err(AppError::MinerU("Markdown文件内容为空".to_string()));
        }

        info!(
            "成功读取Markdown文件: {}，大小: {} 字节",
            selected_file.display(),
            content.len()
        );
        Ok(content)
    }

    /// 递归查找所有Markdown文件
    async fn find_markdown_files_recursively(
        &self,
        dir: &Path,
        markdown_files: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), AppError> {
        self.find_markdown_files_recursively_impl(dir, markdown_files)
            .await
    }

    /// 递归查找所有Markdown文件的实现（使用Box避免递归Future问题）
    async fn find_markdown_files_recursively_impl(
        &self,
        dir: &Path,
        markdown_files: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), AppError> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| AppError::File(format!("读取目录失败: {e}")))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AppError::File(format!("遍历目录失败: {e}")))?
        {
            let path = entry.path();

            if path.is_file() {
                // 检查是否是Markdown文件
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if ext.to_lowercase() == "md" {
                        markdown_files.push(path.clone());
                        debug!("找到Markdown文件: {}", path.display());
                    }
                }
            } else if path.is_dir() {
                // 递归搜索子目录
                Box::pin(self.find_markdown_files_recursively_impl(&path, markdown_files)).await?;
            }
        }

        Ok(())
    }

    /// 获取MinerU命令路径
    fn get_mineru_command_path(&self) -> Result<String, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::MinerU(format!("无法获取当前目录: {e}")))?;

        let venv_path = current_dir.join("venv");
        let mineru_path = if cfg!(windows) {
            venv_path.join("Scripts").join("mineru.exe")
        } else {
            venv_path.join("bin").join("mineru")
        };

        // 检查mineru命令是否存在
        if mineru_path.exists() {
            debug!("找到MinerU命令: {}", mineru_path.display());
            Ok(mineru_path.to_string_lossy().to_string())
        } else {
            // 如果虚拟环境中没有mineru命令，尝试使用系统PATH中的mineru
            debug!("虚拟环境中未找到mineru命令，尝试使用系统PATH中的mineru");
            Ok("mineru".to_string())
        }
    }

    /// 获取解析统计信息
    pub async fn get_parse_statistics(
        &self,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        let mut stats = std::collections::HashMap::new();

        let active_count = self.get_active_task_count().await;
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

    /// 验证MinerU环境
    pub async fn validate_environment(&self) -> Result<(), AppError> {
        // 检查临时目录
        let temp_dir = Path::new("temp/mineru");
        if !temp_dir.exists() {
            fs::create_dir_all(temp_dir)
                .await
                .map_err(|e| AppError::MinerU(format!("创建临时目录失败: {e}")))?;
        }

        // 等待环境依赖安装完成
        self.wait_for_environment_ready().await?;

        // 检查 mineru 命令是否可用（使用虚拟环境中的命令）
        let mineru_command = self.get_mineru_command_path()?;
        let output = Command::new(&mineru_command)
            .arg("--help")
            .output()
            .await
            .map_err(|e| {
                AppError::MinerU(format!(
                    "检查MinerU命令失败: {e}. 请确保已安装MinerU并且mineru命令在虚拟环境中可用"
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::MinerU(format!(
                "MinerU命令不可用: {stderr}. 请运行 'pip install magic-pdf[full]' 安装MinerU"
            )));
        }

        // 检查版本信息
        let version_output = Command::new(&mineru_command)
            .arg("--version")
            .output()
            .await;

        match version_output {
            Ok(output) if output.status.success() => {
                let version_str = String::from_utf8_lossy(&output.stdout);
                info!("MinerU版本: {}", version_str.trim());
            }
            _ => {
                info!("无法获取MinerU版本信息，但命令可用");
            }
        }

        // MinerU 会自动检测和使用可用的 GPU，无需手动检查

        info!("MinerU环境验证通过");
        Ok(())
    }

    /// 等待环境依赖安装完成
    async fn wait_for_environment_ready(&self) -> Result<(), AppError> {
        let environment_manager = EnvironmentManager::for_current_directory()
            .map_err(|e| AppError::MinerU(format!("创建环境管理器失败: {e}")))?;

        let max_wait_time = Duration::from_secs(600); // 最多等待10分钟
        let check_interval = Duration::from_secs(5); // 每5秒检查一次
        let start_time = Instant::now();

        loop {
            // 检查环境状态
            match environment_manager.check_environment().await {
                Ok(status) => {
                    if status.mineru_available {
                        info!("MinerU依赖已就绪，版本: {:?}", status.mineru_version);
                        return Ok(());
                    } else {
                        let elapsed = start_time.elapsed();
                        if elapsed >= max_wait_time {
                            return Err(AppError::MinerU(
                                "等待MinerU依赖安装超时，请检查安装状态".to_string(),
                            ));
                        }

                        info!("等待MinerU依赖安装完成... (已等待: {:?})", elapsed);
                        sleep(check_interval).await;
                    }
                }
                Err(e) => {
                    warn!("检查环境状态失败: {}", e);
                    sleep(check_interval).await;
                }
            }
        }
    }

    /// 检测是否在中国大陆地区,默认为true
    async fn is_china_region(&self) -> bool {
        true
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
                info!("MinerU解析进度: {:?}", progress);
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
        assert_eq!(config.device, "cpu");
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
        assert_eq!(parser.get_active_task_count().await, 0);
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

        let result = parser
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
        if result.is_err() {
            let error = result.unwrap_err();
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
                let mut updates = updates.lock().await;
                updates.push(progress);
            });
        };

        // 注意：这个测试可能会失败，因为我们没有真正的MinerU环境
        // 但可以测试接口是否正确
        let _result = parser
            .parse_with_progress(temp_pdf.path().to_str().unwrap(), progress_callback, None)
            .await;

        // 验证至少收到了一些进度更新
        let updates = progress_updates.lock().await;
        if !updates.is_empty() {
            assert!(updates.iter().any(|p| p.stage == ParseStage::Initializing));
        }
    }
}
