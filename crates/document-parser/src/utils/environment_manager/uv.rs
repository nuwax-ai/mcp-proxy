//! uv 工具：可用性/版本兼容性检测与多通道安装（curl/PowerShell/pip/系统包管理器）。

use super::*;

/// UV工具可用性状态
#[derive(Debug, Clone)]
pub enum UvAvailabilityStatus {
    /// UV可用且版本兼容
    Available {
        version: String,
        compatibility: UvVersionCompatibility,
    },
    /// UV已安装但版本不兼容
    IncompatibleVersion { version: String, issue: String },
    /// UV命令执行失败
    ExecutionFailed { error: String },
    /// UV未安装
    NotInstalled { error: String },
}

/// UV版本兼容性信息
#[derive(Debug, Clone)]
pub struct UvVersionCompatibility {
    pub is_compatible: bool,
    pub minimum_version: String,
    pub current_version: String,
    pub recommendation: Option<String>,
}

/// UV安装方法
#[derive(Debug, Clone)]
pub enum UvInstallationMethod {
    /// 使用curl脚本安装（推荐）
    CurlScript,
    /// 使用PowerShell脚本安装（Windows）
    PowerShellScript,
    /// 使用pip安装
    PipInstall,
    /// 使用系统包管理器
    SystemPackageManager,
}

impl EnvironmentManager {
    /// 检查uv是否可用（增强版本，带详细错误报告）
    pub async fn is_uv_available(&self) -> Result<UvAvailabilityStatus, AppError> {
        debug!("Check uv tool availability");

        let uv_cmd = Command::new("uv").arg("--version").output();

        let output = timeout(Duration::from_secs(10), uv_cmd)
            .await
            .map_err(|_| AppError::Environment("uv版本检查超时".to_string()))?;

        match output {
            Ok(output) if output.status.success() => {
                let version_output = String::from_utf8_lossy(&output.stdout);
                let version = version_output.trim().to_string();

                // 检查版本兼容性
                match self.check_uv_version_compatibility(&version) {
                    Ok(compatibility) => {
                        debug!("uv tool available: {}", version);
                        Ok(UvAvailabilityStatus::Available {
                            version,
                            compatibility,
                        })
                    }
                    Err(e) => {
                        warn!("Incompatible uv version: {}", e);
                        Ok(UvAvailabilityStatus::IncompatibleVersion {
                            version,
                            issue: e.to_string(),
                        })
                    }
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let error_msg = if stderr.is_empty() {
                    "uv命令执行失败，未知错误".to_string()
                } else {
                    format!("uv命令执行失败: {stderr}")
                };
                debug!("uv command execution failed: {}", error_msg);
                Ok(UvAvailabilityStatus::ExecutionFailed { error: error_msg })
            }
            Err(e) => {
                let error_msg = format!("无法执行uv命令: {e}");
                debug!(
                    "The uv command does not exist or cannot be executed: {}",
                    error_msg
                );
                Ok(UvAvailabilityStatus::NotInstalled { error: error_msg })
            }
        }
    }

    /// 检查UV版本兼容性
    fn check_uv_version_compatibility(
        &self,
        version_str: &str,
    ) -> Result<UvVersionCompatibility, AppError> {
        // UV最低版本要求：0.1.0
        let minimum_version = "0.1.0";

        // 解析版本号
        let current_version = self
            .extract_uv_version(version_str)
            .ok_or_else(|| AppError::Environment(format!("无法解析uv版本: {version_str}")))?;

        let min_version = self.parse_version_tuple(minimum_version).ok_or_else(|| {
            AppError::Environment(format!("无法解析最低版本要求: {minimum_version}"))
        })?;

        let is_compatible = current_version >= min_version;

        let recommendation = if !is_compatible {
            Some(format!(
                "请升级uv到{minimum_version}或更高版本，运行: curl -LsSf https://astral.sh/uv/install.sh | sh"
            ))
        } else if current_version.0 == 0 && current_version.1 < 5 {
            // 如果版本低于0.5.0，建议升级以获得更好的性能
            Some("建议升级到uv 0.5.0+以获得更好的性能和稳定性".to_string())
        } else {
            None
        };

        Ok(UvVersionCompatibility {
            is_compatible,
            minimum_version: minimum_version.to_string(),
            current_version: version_str.to_string(),
            recommendation,
        })
    }

    /// 提取UV版本号
    fn extract_uv_version(&self, version_str: &str) -> Option<(u32, u32, u32)> {
        // 解析类似 "uv 0.4.15" 或 "0.4.15" 的版本字符串
        let version_part = if version_str.starts_with("uv ") {
            version_str.strip_prefix("uv ").unwrap_or(version_str)
        } else {
            version_str
        };

        self.parse_version_tuple(version_part)
    }

    /// 安装uv工具（增强版本，带进度跟踪和多种安装方法）
    pub async fn install_uv_with_progress(&self) -> Result<(), AppError> {
        info!("Start installing uv tools");

        self.send_progress("uv", InstallStage::Preparing, 0.0, "准备安装uv工具")
            .await;

        // 确定最佳安装方法
        let installation_method = self.determine_best_uv_installation_method().await;
        info!("Select installation method: {:?}", installation_method);

        // 尝试安装
        let install_result = match installation_method {
            UvInstallationMethod::CurlScript => self.install_uv_with_curl_script().await,
            UvInstallationMethod::PowerShellScript => {
                self.install_uv_with_powershell_script().await
            }
            UvInstallationMethod::PipInstall => self.install_uv_with_pip().await,
            UvInstallationMethod::SystemPackageManager => {
                self.install_uv_with_system_package_manager().await
            }
        };

        match install_result {
            Ok(_) => {
                self.send_progress("uv", InstallStage::Verifying, 90.0, "验证uv安装")
                    .await;

                // 验证安装
                match self.is_uv_available().await? {
                    UvAvailabilityStatus::Available {
                        version,
                        compatibility,
                    } => {
                        if compatibility.is_compatible {
                            self.send_progress(
                                "uv",
                                InstallStage::Completed,
                                100.0,
                                &format!("uv安装完成: {version}"),
                            )
                            .await;
                            info!("UV installation successful: {}", version);
                            Ok(())
                        } else {
                            let error_msg = format!(
                                "uv版本不兼容: {}",
                                compatibility.recommendation.unwrap_or_default()
                            );
                            self.send_progress(
                                "uv",
                                InstallStage::Failed(error_msg.clone()),
                                0.0,
                                "版本不兼容",
                            )
                            .await;
                            Err(AppError::Environment(error_msg))
                        }
                    }
                    UvAvailabilityStatus::IncompatibleVersion { version, issue } => {
                        let error_msg = format!("uv版本不兼容: {version} - {issue}");
                        self.send_progress(
                            "uv",
                            InstallStage::Failed(error_msg.clone()),
                            0.0,
                            "版本不兼容",
                        )
                        .await;
                        Err(AppError::Environment(error_msg))
                    }
                    UvAvailabilityStatus::ExecutionFailed { error } => {
                        let error_msg = format!("uv安装后执行失败: {error}");
                        self.send_progress(
                            "uv",
                            InstallStage::Failed(error_msg.clone()),
                            0.0,
                            "执行失败",
                        )
                        .await;
                        Err(AppError::Environment(error_msg))
                    }
                    UvAvailabilityStatus::NotInstalled { error } => {
                        let error_msg = format!("uv安装后仍不可用: {error}");
                        self.send_progress(
                            "uv",
                            InstallStage::Failed(error_msg.clone()),
                            0.0,
                            "安装失败",
                        )
                        .await;
                        Err(AppError::Environment(error_msg))
                    }
                }
            }
            Err(e) => {
                let error_msg = format!("uv安装失败: {e}");
                self.send_progress(
                    "uv",
                    InstallStage::Failed(error_msg.clone()),
                    0.0,
                    "安装失败",
                )
                .await;

                // 如果主要方法失败，尝试备用方法
                warn!("Primary installation method failed, try alternate method");
                self.try_fallback_uv_installation().await
            }
        }
    }

    /// 确定最佳UV安装方法
    async fn determine_best_uv_installation_method(&self) -> UvInstallationMethod {
        if cfg!(target_os = "windows") {
            // Windows优先使用PowerShell脚本
            if self.is_powershell_available().await {
                UvInstallationMethod::PowerShellScript
            } else {
                UvInstallationMethod::CurlScript
            }
        } else {
            // Unix系统优先使用curl脚本
            if self.is_curl_available().await {
                UvInstallationMethod::CurlScript
            } else if self.is_pip_available().await {
                UvInstallationMethod::PipInstall
            } else {
                UvInstallationMethod::SystemPackageManager
            }
        }
    }

    /// 检查PowerShell是否可用
    async fn is_powershell_available(&self) -> bool {
        Command::new("powershell")
            .arg("-Command")
            .arg("Get-Host")
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 检查curl是否可用
    async fn is_curl_available(&self) -> bool {
        Command::new("curl")
            .arg("--version")
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 检查pip是否可用
    async fn is_pip_available(&self) -> bool {
        Command::new("pip")
            .arg("--version")
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 使用curl脚本安装UV
    async fn install_uv_with_curl_script(&self) -> Result<(), AppError> {
        self.send_progress("uv", InstallStage::Downloading, 10.0, "下载uv安装脚本")
            .await;

        let install_cmd = Command::new("sh")
            .arg("-c")
            .arg("curl -LsSf https://astral.sh/uv/install.sh | sh")
            .output();

        self.send_progress("uv", InstallStage::Installing, 50.0, "执行curl安装脚本")
            .await;

        let output = timeout(Duration::from_secs(300), install_cmd)
            .await
            .map_err(|_| AppError::Environment("uv curl安装超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("curl安装uv失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Environment(format!("curl安装uv失败: {stderr}")));
        }

        info!("Successfully installed uv using curl script");
        Ok(())
    }

    /// 使用PowerShell脚本安装UV
    async fn install_uv_with_powershell_script(&self) -> Result<(), AppError> {
        self.send_progress(
            "uv",
            InstallStage::Downloading,
            10.0,
            "下载uv PowerShell脚本",
        )
        .await;

        let install_cmd = Command::new("powershell")
            .arg("-ExecutionPolicy")
            .arg("ByPass")
            .arg("-c")
            .arg("irm https://astral.sh/uv/install.ps1 | iex")
            .output();

        self.send_progress(
            "uv",
            InstallStage::Installing,
            50.0,
            "执行PowerShell安装脚本",
        )
        .await;

        let output = timeout(Duration::from_secs(300), install_cmd)
            .await
            .map_err(|_| AppError::Environment("uv PowerShell安装超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("PowerShell安装uv失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Environment(format!(
                "PowerShell安装uv失败: {stderr}"
            )));
        }

        info!("Successfully installed uv using PowerShell script");
        Ok(())
    }

    /// 使用pip安装UV
    async fn install_uv_with_pip(&self) -> Result<(), AppError> {
        self.send_progress("uv", InstallStage::Installing, 30.0, "使用pip安装uv")
            .await;

        let install_cmd = Command::new("pip").arg("install").arg("uv").output();

        let output = timeout(Duration::from_secs(180), install_cmd)
            .await
            .map_err(|_| AppError::Environment("uv pip安装超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("pip安装uv失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Environment(format!("pip安装uv失败: {stderr}")));
        }

        info!("Successfully installed uv using pip");
        Ok(())
    }

    /// 使用系统包管理器安装UV
    async fn install_uv_with_system_package_manager(&self) -> Result<(), AppError> {
        self.send_progress(
            "uv",
            InstallStage::Installing,
            30.0,
            "使用系统包管理器安装uv",
        )
        .await;

        // 尝试不同的包管理器
        let package_managers = if cfg!(target_os = "macos") {
            vec![("brew", vec!["install", "uv"])]
        } else if cfg!(target_os = "linux") {
            vec![
                ("apt", vec!["install", "-y", "uv"]),
                ("yum", vec!["install", "-y", "uv"]),
                ("dnf", vec!["install", "-y", "uv"]),
                ("pacman", vec!["-S", "--noconfirm", "uv"]),
            ]
        } else {
            vec![]
        };

        for (manager, args) in package_managers {
            if Self::is_executable_in_path(manager).await {
                let install_cmd = Command::new(manager).args(&args).output();

                match timeout(Duration::from_secs(300), install_cmd).await {
                    Ok(Ok(output)) if output.status.success() => {
                        info!("Use {} to install uv successfully", manager);
                        return Ok(());
                    }
                    Ok(Ok(output)) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        warn!("{} failed to install uv: {}", manager, stderr);
                    }
                    Ok(Err(e)) => {
                        warn!("{} command execution failed: {}", manager, e);
                    }
                    Err(_) => {
                        warn!("{} installation uv timeout", manager);
                    }
                }
            }
        }

        Err(AppError::Environment(
            "所有系统包管理器都无法安装uv".to_string(),
        ))
    }

    /// 尝试备用UV安装方法
    async fn try_fallback_uv_installation(&self) -> Result<(), AppError> {
        warn!("Try alternative uv installation methods");

        // 备用方法列表
        let fallback_methods = vec![
            UvInstallationMethod::PipInstall,
            UvInstallationMethod::CurlScript,
            UvInstallationMethod::SystemPackageManager,
        ];

        for method in fallback_methods {
            info!("Try alternative installation method: {:?}", method);

            let result = match method {
                UvInstallationMethod::PipInstall => self.install_uv_with_pip().await,
                UvInstallationMethod::CurlScript => self.install_uv_with_curl_script().await,
                UvInstallationMethod::SystemPackageManager => {
                    self.install_uv_with_system_package_manager().await
                }
                UvInstallationMethod::PowerShellScript => {
                    self.install_uv_with_powershell_script().await
                }
            };

            match result {
                Ok(_) => {
                    // 验证安装
                    match self.is_uv_available().await? {
                        UvAvailabilityStatus::Available {
                            version,
                            compatibility,
                        } => {
                            if compatibility.is_compatible {
                                self.send_progress(
                                    "uv",
                                    InstallStage::Completed,
                                    100.0,
                                    &format!("uv备用安装成功: {version}"),
                                )
                                .await;
                                info!("UV backup installation successful: {}", version);
                                return Ok(());
                            }
                        }
                        _ => continue,
                    }
                }
                Err(e) => {
                    warn!("Alternate installation method failed: {}", e);
                    continue;
                }
            }
        }

        Err(AppError::Environment("所有uv安装方法都失败了".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_uv_availability_check() {
        let temp_dir = TempDir::new().unwrap();
        let env_manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        // 测试UV可用性检查
        let result = env_manager.is_uv_available().await;
        assert!(result.is_ok());

        // 检查返回的状态类型
        match result.unwrap() {
            UvAvailabilityStatus::Available {
                version,
                compatibility,
            } => {
                assert!(!version.is_empty());
                assert!(!compatibility.minimum_version.is_empty());
                assert!(!compatibility.current_version.is_empty());
            }
            UvAvailabilityStatus::IncompatibleVersion { version, issue } => {
                assert!(!version.is_empty());
                assert!(!issue.is_empty());
            }
            UvAvailabilityStatus::ExecutionFailed { error } => {
                assert!(!error.is_empty());
            }
            UvAvailabilityStatus::NotInstalled { error } => {
                assert!(!error.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn test_uv_version_parsing() {
        let temp_dir = TempDir::new().unwrap();
        let env_manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        // 测试版本解析
        assert_eq!(
            env_manager.extract_uv_version("uv 0.4.15"),
            Some((0, 4, 15))
        );
        assert_eq!(env_manager.extract_uv_version("0.4.15"), Some((0, 4, 15)));
        assert_eq!(env_manager.extract_uv_version("1.0.0"), Some((1, 0, 0)));
        assert_eq!(env_manager.extract_uv_version("invalid"), None);

        // 测试版本兼容性检查
        let compatibility = env_manager.check_uv_version_compatibility("uv 0.4.15");
        assert!(compatibility.is_ok());

        let compat = compatibility.unwrap();
        assert!(compat.is_compatible); // 0.4.15 >= 0.1.0
        assert_eq!(compat.minimum_version, "0.1.0");
        assert_eq!(compat.current_version, "uv 0.4.15");
    }

    #[tokio::test]
    async fn test_uv_installation_method_detection() {
        let temp_dir = TempDir::new().unwrap();
        let env_manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        // 测试安装方法检测
        let method = env_manager.determine_best_uv_installation_method().await;

        // 确保返回了一个有效的安装方法
        match method {
            UvInstallationMethod::CurlScript
            | UvInstallationMethod::PowerShellScript
            | UvInstallationMethod::PipInstall
            | UvInstallationMethod::SystemPackageManager => {
                // 所有方法都是有效的
            }
        }
    }
}
