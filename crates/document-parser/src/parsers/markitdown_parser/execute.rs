//! `MarkItDownParser` 的命令执行族方法（从 markitdown_parser.rs 拆出）：
//! 输入校验与 markitdown 子进程执行/输出流监控。纯代码搬移，无行为变化。

use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;

/// Windows 文件重定向的日志路径（output_file 旁）
#[cfg(windows)]
fn output_log_path(output_file: &std::path::Path) -> std::path::PathBuf {
    let mut p = output_file.as_os_str().to_os_string();
    p.push(".stdout.log");
    std::path::PathBuf::from(p)
}
#[cfg(windows)]
fn err_log_path(output_file: &std::path::Path) -> std::path::PathBuf {
    let mut p = output_file.as_os_str().to_os_string();
    p.push(".stderr.log");
    std::path::PathBuf::from(p)
}
use crate::models::DocumentFormat;
use crate::parsers::managed_process::{self, kill_tree};
use std::path::Path;
#[cfg(unix)]
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::debug;

use super::{CancellationToken, MarkItDownProgress, ProcessingStage};

impl super::MarkItDownParser {
    /// 验证输入文件
    pub(super) async fn validate_input_file(
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
    pub(super) async fn execute_markitdown_command<F>(
        &self,
        file_path: &str,
        work_dir: &Path,
        _format: &DocumentFormat,
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

        // stdio 策略（平台雷区详见 managed_process 模块头）：Windows 文件重定向
        //（OVERLAPPED 管道雷，与 mineru 同款），Unix 管道（实时流监控）
        #[cfg(windows)]
        {
            let out_path = output_log_path(&output_file);
            let err_path = err_log_path(&output_file);
            managed_process::redirect_stdio_to_files(&mut cmd, &out_path, &err_path)
                .map_err(|e| AppError::MarkItDown(format!("创建子进程日志文件失败: {e}")))?;
        }
        #[cfg(unix)]
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        debug!("Execute MarkItDown command: {:?}", cmd);

        // 进程树级击杀脚手架（Unix 进程组 / Windows 原生 + 树杀守卫）
        let mut child = managed_process::spawn_managed(cmd)
            .map_err(|e| AppError::MarkItDown(format!("启动MarkItDown进程失败: {e}")))?;

        // 输出监控（Windows 走文件重定向，无实时流——结束后读文件兜底）
        let mut pump = managed_process::attach_output_pump(&mut child);

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
                            kill_tree(&mut child).await;
                            return Err(AppError::MarkItDown("解析已取消".to_string()));
                        }
                    }

                    // 处理输出
                    Some((source, line)) = pump.recv() => {
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
                        if line.contains("Created temp file:")
                            && let Some(file_path) = line.split("Created temp file:").nth(1) {
                                temp_files.push(file_path.trim().to_string());
                            }
                    }

                    // 等待进程完成
                    result = child.wait() => {
                        match result {
                            Ok(status) => {
                                // Windows 文件重定向：结束后读日志兜底
                                #[cfg(windows)]
                                {
                                    if let Ok(content) = std::fs::read_to_string(err_log_path(&output_file)) {
                                        stderr_output = content;
                                    }
                                    if let Ok(content) = std::fs::read_to_string(output_log_path(&output_file)) {
                                        for line in content.lines() {
                                            if line.contains("Created temp file:")
                                                && let Some(fp) = line.split("Created temp file:").nth(1) {
                                                    temp_files.push(fp.trim().to_string());
                                                }
                                        }
                                    }
                                }
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
        pump.abort();

        let temp_files = match process_result {
            Ok(result) => result?,
            Err(_) => {
                kill_tree(&mut child).await;
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
}
