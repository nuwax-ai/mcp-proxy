//! 虚拟环境生命周期：创建前置校验、目录可写检测、损坏 venv 清理、创建与结果验证。

use super::*;

impl EnvironmentManager {
    /// 验证虚拟环境创建的前置条件
    async fn validate_venv_creation_preconditions(&self, venv_path: &Path) -> Result<(), AppError> {
        let base_dir = Path::new(&self.base_dir);

        // 检查基础目录是否存在
        if !base_dir.exists() {
            return Err(AppError::path_error("基础目录不存在".to_string(), base_dir));
        }

        // 检查基础目录是否为目录
        if !base_dir.is_dir() {
            return Err(AppError::path_error(
                "基础路径不是目录".to_string(),
                base_dir,
            ));
        }

        // 检查基础目录写入权限
        if let Err(e) = self.check_directory_writable(base_dir).await {
            return Err(AppError::permission_error(
                format!("基础目录无写入权限: {e}"),
                base_dir,
            ));
        }

        // 检查虚拟环境路径是否已存在且为文件（而非目录）
        if venv_path.exists() && !venv_path.is_dir() {
            return Err(AppError::virtual_environment_path_error(
                "虚拟环境路径已存在但不是目录".to_string(),
                venv_path,
            ));
        }

        // 检查路径长度（Windows路径长度限制）
        if cfg!(windows) && venv_path.to_string_lossy().len() > 260 {
            return Err(AppError::virtual_environment_path_error(
                "虚拟环境路径过长，Windows系统限制为260字符".to_string(),
                venv_path,
            ));
        }

        Ok(())
    }

    /// 检查目录是否可写
    pub(super) async fn check_directory_writable(&self, dir: &Path) -> Result<(), std::io::Error> {
        let test_file = dir.join(".write_test");

        // 尝试创建测试文件
        match std::fs::File::create(&test_file) {
            Ok(_) => {
                // 清理测试文件
                let _ = std::fs::remove_file(&test_file);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 处理虚拟环境创建错误并提供恢复建议
    fn handle_venv_creation_error(&self, error: &str, venv_path: &Path) -> AppError {
        let error_lower = error.to_lowercase();

        if error_lower.contains("permission") || error_lower.contains("权限") {
            AppError::permission_error(
                format!("虚拟环境创建权限错误: {error}"),
                venv_path.parent().unwrap_or(venv_path),
            )
        } else if error_lower.contains("space")
            || error_lower.contains("空间")
            || error_lower.contains("disk")
        {
            AppError::virtual_environment_path_error(
                format!("磁盘空间不足导致虚拟环境创建失败: {error}"),
                venv_path,
            )
        } else if error_lower.contains("exists") || error_lower.contains("存在") {
            AppError::virtual_environment_path_error(
                format!("虚拟环境路径冲突: {error}"),
                venv_path,
            )
        } else if error_lower.contains("path") || error_lower.contains("路径") {
            AppError::path_error(format!("虚拟环境路径错误: {error}"), venv_path)
        } else if error_lower.contains("timeout") || error_lower.contains("超时") {
            AppError::Environment(format!("虚拟环境创建超时: {error}"))
        } else {
            AppError::virtual_environment_path_error(
                format!("虚拟环境创建失败: {error}"),
                venv_path,
            )
        }
    }

    /// 尝试清理损坏的虚拟环境
    pub(super) async fn cleanup_corrupted_venv(&self, venv_path: &Path) -> Result<(), AppError> {
        if !venv_path.exists() {
            return Ok(());
        }

        info!(
            "Try cleaning the corrupted virtual environment: {}",
            venv_path.display()
        );

        // 检查是否有权限删除
        if let Err(e) = self
            .check_directory_writable(venv_path.parent().unwrap_or(venv_path))
            .await
        {
            return Err(AppError::permission_error(
                format!("无权限清理虚拟环境: {e}"),
                venv_path,
            ));
        }

        // 尝试删除虚拟环境目录
        match std::fs::remove_dir_all(venv_path) {
            Ok(_) => {
                info!("Successfully cleans corrupted virtual environment");
                Ok(())
            }
            Err(e) => Err(AppError::permission_error(
                format!("清理虚拟环境失败: {e}"),
                venv_path,
            )),
        }
    }

    /// 创建Python虚拟环境（带进度跟踪和增强错误处理）
    pub(super) async fn create_python_venv_with_progress(&self) -> Result<(), AppError> {
        let venv_path = Path::new(&self.base_dir).join("venv");

        // 预检查：验证创建条件
        self.send_progress("虚拟环境", InstallStage::Preparing, 5.0, "验证创建条件")
            .await;
        if let Err(e) = self.validate_venv_creation_preconditions(&venv_path).await {
            self.send_progress(
                "虚拟环境",
                InstallStage::Failed(e.to_string()),
                0.0,
                "前置条件检查失败",
            )
            .await;
            return Err(e);
        }

        // 检查虚拟环境是否已存在
        if venv_path.exists() && venv_path.is_dir() {
            // 验证现有虚拟环境是否完整
            let python_exe = Self::get_venv_python_path(&venv_path);

            if python_exe.exists() {
                info!(
                    "The Python virtual environment exists and is complete: {}",
                    venv_path.display()
                );
                self.send_progress("虚拟环境", InstallStage::Completed, 100.0, "虚拟环境已存在")
                    .await;
                return Ok(());
            } else {
                warn!("Corrupted virtual environment detected, attempt to clean");
                self.send_progress(
                    "虚拟环境",
                    InstallStage::Preparing,
                    10.0,
                    "清理损坏的虚拟环境",
                )
                .await;
                self.cleanup_corrupted_venv(&venv_path).await?;
            }
        }

        info!(
            "Create a Python virtual environment: {}",
            venv_path.display()
        );
        self.send_progress(
            "虚拟环境",
            InstallStage::Preparing,
            15.0,
            "准备创建虚拟环境",
        )
        .await;

        // 使用 uv venv venv 在当前目录下创建名为venv的虚拟环境
        let create_cmd = Command::new("uv")
            .arg("venv")
            .arg("venv")
            .arg("--python")
            .arg("python3")
            .current_dir(&self.base_dir)
            .output();

        self.send_progress("虚拟环境", InstallStage::Installing, 50.0, "创建虚拟环境")
            .await;

        let output = timeout(Duration::from_secs(120), create_cmd)
            .await
            .map_err(|_| self.handle_venv_creation_error("虚拟环境创建超时", &venv_path))?
            .map_err(|e| {
                self.handle_venv_creation_error(&format!("命令执行失败: {e}"), &venv_path)
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let error_msg = if !stderr.is_empty() {
                stderr.to_string()
            } else if !stdout.is_empty() {
                stdout.to_string()
            } else {
                "未知错误".to_string()
            };

            self.send_progress(
                "虚拟环境",
                InstallStage::Failed(error_msg.clone()),
                0.0,
                "创建失败",
            )
            .await;

            let error = self.handle_venv_creation_error(&error_msg, &venv_path);

            // 记录详细的错误信息和恢复建议
            error!("Virtual environment creation failed: {}", error);
            for suggestion in error.get_path_recovery_suggestions() {
                error!("Recovery suggestion: {}", suggestion);
            }

            return Err(error);
        }

        self.send_progress("虚拟环境", InstallStage::Verifying, 90.0, "验证虚拟环境")
            .await;

        // 验证虚拟环境创建结果
        if let Err(e) = self.verify_venv_creation(&venv_path).await {
            self.send_progress(
                "虚拟环境",
                InstallStage::Failed(e.to_string()),
                0.0,
                "验证失败",
            )
            .await;
            return Err(e);
        }

        self.send_progress(
            "虚拟环境",
            InstallStage::Completed,
            100.0,
            "虚拟环境创建完成",
        )
        .await;
        info!("The Python virtual environment is created");
        Ok(())
    }

    /// 验证虚拟环境创建结果
    async fn verify_venv_creation(&self, venv_path: &Path) -> Result<(), AppError> {
        // 检查虚拟环境目录是否存在
        if !venv_path.exists() {
            return Err(AppError::virtual_environment_path_error(
                "虚拟环境创建后目录不存在".to_string(),
                venv_path,
            ));
        }

        if !venv_path.is_dir() {
            return Err(AppError::virtual_environment_path_error(
                "虚拟环境路径不是目录".to_string(),
                venv_path,
            ));
        }

        // 检查Python可执行文件
        let python_exe = Self::get_venv_python_path(venv_path);

        if !python_exe.exists() {
            return Err(AppError::virtual_environment_path_error(
                "虚拟环境中Python可执行文件不存在".to_string(),
                &python_exe,
            ));
        }

        // 检查pip是否可用
        let pip_exe = Self::get_venv_executable_path(venv_path, "pip");

        if !pip_exe.exists() {
            warn!(
                "pip does not exist in the virtual environment, but this may be normal (using uv management package)"
            );
        }

        // 尝试运行Python验证虚拟环境
        let test_cmd = Command::new(&python_exe)
            .arg("-c")
            .arg("import sys; print(sys.prefix)")
            .output();

        match timeout(Duration::from_secs(10), test_cmd).await {
            Ok(Ok(output)) if output.status.success() => {
                let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
                debug!("Virtual environment Python prefix: {}", prefix);

                // 验证Python前缀是否指向虚拟环境
                if !prefix.contains("venv") {
                    warn!(
                        "Python prefix may not point to virtual environment: {}",
                        prefix
                    );
                }
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(AppError::virtual_environment_path_error(
                    format!("虚拟环境Python测试失败: {stderr}"),
                    &python_exe,
                ));
            }
            Ok(Err(e)) => {
                return Err(AppError::virtual_environment_path_error(
                    format!("无法执行虚拟环境Python: {e}"),
                    &python_exe,
                ));
            }
            Err(_) => {
                return Err(AppError::virtual_environment_path_error(
                    "虚拟环境Python测试超时".to_string(),
                    &python_exe,
                ));
            }
        }

        Ok(())
    }
}
