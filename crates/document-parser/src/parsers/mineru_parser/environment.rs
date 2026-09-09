//! `MinerUParser` 的环境校验族方法（从 mineru_parser.rs 拆出）：解析环境
//! 验证、环境就绪等待与地区检测。纯代码搬移，无行为变化。

use crate::error::AppError;
use crate::utils::environment_manager::EnvironmentManager;
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::process::Command;
use tokio::time::sleep;
use tracing::{info, warn};

impl super::MinerUParser {
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
                info!("MinerU version book: {}", version_str.trim());
            }
            _ => {
                info!("Unable to get MinerU version information, but the command is available");
            }
        }

        // MinerU 会自动检测和使用可用的 GPU，无需手动检查

        info!("MinerU environment verification passed");
        Ok(())
    }

    /// 等待环境依赖安装完成
    pub(super) async fn wait_for_environment_ready(&self) -> Result<(), AppError> {
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
                        info!(
                            "MinerU dependency is ready, version: {:?}",
                            status.mineru_version
                        );
                        return Ok(());
                    } else {
                        let elapsed = start_time.elapsed();
                        if elapsed >= max_wait_time {
                            return Err(AppError::MinerU(
                                "等待MinerU依赖安装超时，请检查安装状态".to_string(),
                            ));
                        }

                        info!(
                            "Waiting for MinerU dependency installation to complete... (Waiting: {:?})",
                            elapsed
                        );
                        sleep(check_interval).await;
                    }
                }
                Err(e) => {
                    warn!("Failed to check environment status: {}", e);
                    sleep(check_interval).await;
                }
            }
        }
    }

    /// 检测是否在中国大陆地区,默认为true
    pub(super) async fn is_china_region(&self) -> bool {
        true
    }
}
