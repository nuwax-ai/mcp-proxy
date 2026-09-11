//! `MinerUParser` 的子进程执行族方法（从 mineru_parser.rs 拆出）：MinerU 命令
//! 构建与执行、输出流监控、命令路径解析。纯代码搬移，无行为变化。

use crate::error::AppError;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
#[cfg(unix)]
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, error, info, trace, warn};

use super::{CancellationToken, ParseProgress, ParseStage, StderrKind, classify_mineru_stderr};
use crate::parsers::managed_process::{self, kill_tree};

impl super::MinerUParser {
    /// 执行MinerU命令
    pub(super) async fn execute_mineru_command<F>(
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
            "MinerU command execution - input file: {}, output directory: {}",
            file_path,
            output_dir.display()
        );

        // 验证输入文件是否存在
        if !std::path::Path::new(file_path).exists() {
            error!("MinerU input file does not exist: {}", file_path);
            return Err(AppError::MinerU(format!("输入文件不存在: {file_path}")));
        }

        // 获取文件绝对路径（dunce：Windows 的 std canonicalize 返回 \\?\ 扩展
        // 长度路径，mineru CLI 不认——实测 Win53 PDF 解析从未通过的根因）
        let absolute_file_path = dunce::canonicalize(std::path::Path::new(file_path))
            .map_err(|e| AppError::MinerU(format!("无法获取文件绝对路径: {e}")))?
            .to_string_lossy()
            .to_string();
        debug!("MinerU input file absolute path: {}", absolute_file_path);

        // 自动检测并使用虚拟环境中的 mineru 命令
        let mineru_command = self.get_mineru_command_path()?;
        let mut cmd = Command::new(&mineru_command);
        cmd.arg("-p")
            .arg(&absolute_file_path)
            .arg("-o")
            .arg(output_dir);

        // 后端类型：mineru 3.4 合法值为 pipeline/vlm-engine/hybrid-engine/vlm-http-client/hybrid-http-client。
        // 统一传 -b（不再对 pipeline 做特殊处理）
        if !self.config.backend.is_empty() {
            cmd.arg("-b").arg(&self.config.backend);
            debug!("MinerU sets the backend type: {}", self.config.backend);
        }

        // vllm 显存占用比例(仅 hybrid-engine/vlm-engine 等走 vllm 的后端)。
        // mineru 3.4 默认约 0.5(占一半显存)；多 GPU 进程共存时调低避免 OOM。pipeline 后端不走 vllm，无需设置。
        if self.config.backend != "pipeline" && self.config.gpu_memory_utilization > 0.0 {
            cmd.arg("--gpu-memory-utilization")
                .arg(self.config.gpu_memory_utilization.to_string());
            debug!(
                "MinerU sets gpu_memory_utilization: {}",
                self.config.gpu_memory_utilization
            );
        }

        // 设备：mineru 3.4 不再支持 -d CLI 参数，改用环境变量 MINERU_DEVICE_MODE。
        // 有 CUDA 且 config 未显式指定非 cpu 设备时自动用 cuda；否则用 config 里的设备
        let cuda_available = crate::config::is_cuda_available();
        let device = if cuda_available && self.config.device == "cpu" {
            "cuda"
        } else {
            self.config.device.as_str()
        };
        cmd.env("MINERU_DEVICE_MODE", device);
        debug!("MinerU sets device (MINERU_DEVICE_MODE): {}", device);

        // 显存：mineru 3.4 不再支持 --vram CLI 参数，改用环境变量 MINERU_VIRTUAL_VRAM_SIZE。
        // vram=0 表示不限制，交由 mineru/vllm 自动管理
        if self.config.vram > 0 {
            cmd.env("MINERU_VIRTUAL_VRAM_SIZE", self.config.vram.to_string());
            debug!(
                "MinerU sets VRAM limit (MINERU_VIRTUAL_VRAM_SIZE): {}GB",
                self.config.vram
            );
        }

        // 模型源：mineru 3.4 的 main CLI 不再支持 --source 参数，统一用环境变量
        if self.is_china_region().await {
            cmd.env("MINERU_MODEL_SOURCE", "modelscope");
            debug!("MinerU sets model source (MINERU_MODEL_SOURCE): modelscope");
        }

        // stdio 策略（平台雷区详见 managed_process 模块头）：Windows 文件重定向
        // ——tokio 管道是 OVERLAPPED 句柄，mineru.exe 把继承句柄传给其本地 api
        // 子进程（uvicorn）后 I/O 失败 → 退出码 120（Win53 实测）；Unix 管道
        // （实时流监控）
        #[cfg(windows)]
        let stderr_log_path = {
            let task_dir = output_dir.parent().unwrap_or(output_dir);
            let out_path = task_dir.join("stdout.log");
            let err_path = task_dir.join("stderr.log");
            managed_process::redirect_stdio_to_files(&mut cmd, &out_path, &err_path)
                .map_err(|e| AppError::MinerU(format!("创建子进程日志文件失败: {e}")))?;
            err_path
        };
        #[cfg(unix)]
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        info!(
            "MinerU command parameters: {} -p {} -o {}",
            mineru_command,
            absolute_file_path,
            output_dir.display()
        );
        info!("Execute MinerU command: {:?}", cmd);

        // 进程树级击杀脚手架（Unix 进程组 / Windows 原生 + 树杀守卫——
        // Win53 曾僵尸泄漏，详见 managed_process 模块头）
        let mut child = managed_process::spawn_managed(cmd).map_err(|e| {
            error!("Failed to start child process: {}", e);
            AppError::MinerU(format!("启动MinerU进程失败: {e}"))
        })?;

        info!("MinerU process has been started, PID: {:?}", child.id());

        // 输出监控（Windows 走文件重定向，无实时流——结束后读文件兜底）
        let mut pump = managed_process::attach_output_pump(&mut child);

        // 监控进程和输出
        let timeout_seconds = if self.config.timeout == 0 {
            3600
        } else {
            self.config.timeout
        };
        let timeout_duration = Duration::from_secs(timeout_seconds as u64);
        info!(
            "MinerU parsing timeout setting: {} seconds ({})",
            timeout_seconds,
            if self.config.timeout == 0 {
                "using global config"
            } else {
                "using MinerU config"
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
                            kill_tree(&mut child).await;
                            return Err(AppError::MinerU("解析已取消".to_string()));
                        }
                    }

                    // 处理输出
                    Some((source, line)) = pump.recv() => {
                        if source == "stderr" {
                            stderr_output.push_str(&line);
                            stderr_output.push('\n');
                            // 按内容分级记录（tqdm 进度条/loguru INFO 等不再误报 ERROR）
                            match classify_mineru_stderr(&line) {
                                StderrKind::Error => error!("MinerU stderr: {}", line),
                                StderrKind::Warning => warn!("MinerU stderr: {}", line),
                                StderrKind::Progress => trace!("MinerU stderr: {}", line),
                                StderrKind::Info => debug!("MinerU stderr: {}", line),
                            }
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
                                // Windows 文件重定向：结束后读 stderr.log 兜底填充
                                #[cfg(windows)]
                                if let Ok(content) = std::fs::read_to_string(&stderr_log_path) {
                                    stderr_output = content;
                                }
                                if status.success() {
                                    info!("The MinerU process completed successfully with exit code: {}", status.code().unwrap_or(0));
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
        pump.abort();

        match process_result {
            Ok(result) => result,
            Err(_) => {
                error!(
                    "MinerU execution timeout ({} seconds), terminating process",
                    timeout_seconds
                );
                kill_tree(&mut child).await;

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

    /// 获取MinerU命令路径
    pub(super) fn get_mineru_command_path(&self) -> Result<String, AppError> {
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
            debug!("Found the MinerU command: {}", mineru_path.display());
            Ok(mineru_path.to_string_lossy().to_string())
        } else {
            // 如果虚拟环境中没有mineru命令，尝试使用系统PATH中的mineru
            debug!(
                "The mineru command was not found in the virtual environment, try using mineru in the system PATH"
            );
            Ok("mineru".to_string())
        }
    }
}
