//! `MinerUParser` 的子进程执行族方法（从 mineru_parser.rs 拆出）：MinerU 命令
//! 构建与执行、输出流监控、命令路径解析。纯代码搬移，无行为变化。

use crate::error::AppError;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, error, info, trace, warn};

use super::{CancellationToken, ParseProgress, ParseStage, StderrKind, classify_mineru_stderr};

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

        // Windows 的 tokio 管道是 OVERLAPPED 句柄——mineru.exe 会把继承的句柄
        // 传给其本地 api 子进程（uvicorn），跨进程继承后 I/O 失败 → api 健康
        // 检查超时 → 退出码 120（Win53 实测；python/Git Bash 同步句柄直跑全过）。
        // 对策：Windows 下 stdout/stderr 重定向到任务目录日志文件（同步句柄，
        // 等价于已验证成功的 `>` 重定向），结束后读取分类
        #[cfg(windows)]
        {
            let task_dir = output_dir.parent().unwrap_or(output_dir);
            let _ = std::fs::create_dir_all(task_dir);
            let out_log = std::fs::File::create(task_dir.join("stdout.log"))
                .map_err(|e| AppError::MinerU(format!("创建 stdout.log 失败: {e}")))?;
            let err_log = std::fs::File::create(task_dir.join("stderr.log"))
                .map_err(|e| AppError::MinerU(format!("创建 stderr.log 失败: {e}")))?;
            cmd.stdout(Stdio::from(out_log))
                .stderr(Stdio::from(err_log));
        }
        #[cfg(unix)]
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        info!(
            "MinerU command parameters: {} -p {} -o {}",
            mineru_command,
            absolute_file_path,
            output_dir.display()
        );
        info!("Execute MinerU command: {:?}", cmd);

        // 进程树级击杀：Unix 进程组（process-wrap；孙进程一并终止——Win53 曾
        // 僵尸泄漏）。Windows 原生 creation_flags + KillOnDrop（flags 本身无
        // 害——python 逐一复刻验证；真正的雷是上面的 OVERLAPPED 管道）；
        // 树杀由 kill_tree 的 taskkill /T 承担
        #[cfg(unix)]
        let mut child = {
            let mut wrapped = process_wrap::tokio::CommandWrap::from(cmd);
            wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
            wrapped.wrap(process_wrap::tokio::KillOnDrop);
            wrapped.spawn().map_err(|e| {
                error!("Failed to start child process: {}", e);
                AppError::MinerU(format!("启动MinerU进程失败: {e}"))
            })?
        };
        #[cfg(windows)]
        let mut child = {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
            cmd.kill_on_drop(true);
            cmd.spawn().map_err(|e| {
                error!("Failed to start child process: {}", e);
                AppError::MinerU(format!("启动MinerU进程失败: {e}"))
            })?
        };

        info!("MinerU process has been started, PID: {:?}", child.id());

        // 输出监控（Windows 走文件重定向，无实时流——结束后读文件兜底）
        #[cfg(unix)]
        let (tx, mut rx) = mpsc::channel(100);
        #[cfg(unix)]
        let (stdout_task, stderr_task); // 声明提外：unix JoinHandle / windows DummyTask
        #[cfg(unix)]
        {
            let stdout = child_stdout(&mut child).unwrap();
            let stderr = child_stderr(&mut child).unwrap();

            let stdout_reader = BufReader::new(stdout);
            let stderr_reader = BufReader::new(stderr);

            let tx_clone = tx.clone();

            // 监控stdout
            stdout_task = tokio::spawn(async move {
                let mut lines = stdout_reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(("stdout".to_string(), line)).await;
                }
            });

            // 监控stderr
            stderr_task = tokio::spawn(async move {
                let mut lines = stderr_reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_clone.send(("stderr".to_string(), line)).await;
                }
            });
        }
        #[cfg(windows)]
        let mut rx = DummyRx;
        #[cfg(windows)]
        let (stdout_task, stderr_task) = (DummyTask, DummyTask);

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
                    Some((source, line)) = rx.recv() => {
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
                                {
                                    let task_dir = output_dir.parent().unwrap_or(output_dir);
                                    if let Ok(content) = std::fs::read_to_string(task_dir.join("stderr.log")) {
                                        stderr_output = content;
                                    }
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
        stdout_task.abort();
        stderr_task.abort();

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

/// 进程树击杀回归：ProcessGroup 包装下 kill 直接子进程时，孙进程必须一并终止
/// （此前 Child::kill 只杀直接子进程——Win53 实测 mineru 孙进程僵尸泄漏）。
/// Unix 用 /proc 探活；非 Unix 平台跳过（Windows JobObject 行为靠实机验证）。
#[cfg(all(test, unix))]
mod tree_kill_tests {
    use super::kill_tree;
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::process::Command;

    #[tokio::test]
    async fn process_group_kill_terminates_grandchildren() {
        let marker = std::env::temp_dir().join("dp-treekill-test.pid");
        let marker_str = marker.display().to_string();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &format!("sleep 300 & echo $! > {marker_str}; wait")])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut wrapped = process_wrap::tokio::CommandWrap::from(cmd);
        wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
        let mut child = wrapped.spawn().expect("spawn sh");

        // 等 sh 写出孙进程 pid 并让 sleep 真正起跑
        let mut grandchild_pid = None;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(content) = std::fs::read_to_string(&marker)
                && let Ok(pid) = content.trim().parse::<u32>()
            {
                grandchild_pid = Some(pid);
                break;
            }
        }
        let grandchild_pid = grandchild_pid.expect("sh 应写出孙进程 pid");
        let _ = std::fs::remove_file(&marker);
        let alive = |pid: u32| unsafe { libc::kill(pid as i32, 0) == 0 };
        assert!(alive(grandchild_pid), "孙进程应已在运行（前置条件）");

        kill_tree(&mut child).await;

        // 组击杀后孙进程应消失（短暂宽限轮询）
        let mut gone = false;
        for _ in 0..30 {
            if !alive(grandchild_pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            gone,
            "kill 进程组后孙进程 {grandchild_pid} 仍存活（树击杀失效）"
        );
    }
}

/// Windows 文件重定向模式下的占位通道/任务（select 结构与 unix 保持同构）
#[cfg(windows)]
pub(crate) struct DummyRx;
#[cfg(windows)]
impl DummyRx {
    pub(crate) async fn recv(&mut self) -> Option<(String, String)> {
        // 永不产出：select 的输出臂在 Windows 下天然休眠
        std::future::pending().await
    }
}
#[cfg(windows)]
pub(crate) struct DummyTask;
#[cfg(windows)]
impl DummyTask {
    pub(crate) fn abort(&self) {}
}

/// 平台统一的子进程句柄：Unix 用 process-wrap 的 ChildWrapper（进程组语义）；
/// Windows 用原生 tokio Child（process-wrap 的 spawn 路径与 mineru 3.4.5 的
/// 本地 api 子进程不兼容——Win53 实测退出码 120，无论 JobObject 还是仅
/// CreationFlags 包装器；原生 creation_flags 与直跑成功路径等价）
#[cfg(unix)]
pub(crate) type ManagedChild = Box<dyn process_wrap::tokio::ChildWrapper>;
#[cfg(windows)]
pub(crate) type ManagedChild = tokio::process::Child;

pub(crate) fn child_stdout(child: &mut ManagedChild) -> Option<tokio::process::ChildStdout> {
    #[cfg(unix)]
    {
        child.stdout().take()
    }
    #[cfg(windows)]
    {
        child.stdout.take()
    }
}

pub(crate) fn child_stderr(child: &mut ManagedChild) -> Option<tokio::process::ChildStderr> {
    #[cfg(unix)]
    {
        child.stderr().take()
    }
    #[cfg(windows)]
    {
        child.stderr.take()
    }
}

/// 进程树击杀：Unix 在 ProcessGroup::leader 下 kill 即组杀；Windows 先
/// `taskkill /PID <pid> /T /F`（官方树杀，孙进程一并终止）再 child.kill()
pub(crate) async fn kill_tree(child: &mut ManagedChild) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Box::into_pin(child.kill()).await;
    }
    #[cfg(windows)]
    {
        let _ = child.kill().await;
    }
}
