//! 环境检测：check_environment 聚合编排、原子检测器（Python/uv/CUDA/MinerU/MarkItDown）、
//! 重试封装与缓存、状态校验与安装建议文案。

use super::*;

/// Python环境信息
#[derive(Debug)]
struct PythonInfo {
    version: Option<String>,
    path: String,
    virtual_env_active: bool,
    virtual_env_path: Option<String>,
}

/// uv工具信息
#[derive(Debug)]
struct UvInfo {
    version: String,
}

impl EnvironmentManager {
    /// 检查完整环境状态（带缓存支持）
    #[instrument(skip(self))]
    pub async fn check_environment(&self) -> Result<EnvironmentStatus, AppError> {
        // 检查缓存
        if let Some(cached_status) = self.get_cached_status().await
            && !cached_status.is_cache_expired(self.cache_ttl)
        {
            debug!("Using cached environment state");
            return Ok(cached_status);
        }

        let start_time = std::time::SystemTime::now();
        let mut status = EnvironmentStatus::default();

        info!("Start environment check");

        // 并行检查各个环境组件
        let (python_result, uv_result, cuda_result) = tokio::join!(
            self.check_python_environment_with_retry(),
            self.check_uv_environment_with_retry(),
            self.check_cuda_environment_with_retry()
        );

        // 处理Python环境检查结果
        match python_result {
            Ok(python_info) => {
                status.python_available = true;
                status.python_version = python_info.version.clone();
                status.python_path = Some(python_info.path.clone());
                status.virtual_env_active = python_info.virtual_env_active;
                status.virtual_env_path = python_info.virtual_env_path.clone();
                info!(
                    "Python environment check passed: {:?}",
                    status.python_version
                );

                // 增强虚拟环境状态验证
                self.validate_virtual_environment_status(&mut status, &python_info);
            }
            Err(e) => {
                let issue = EnvironmentIssue {
                    component: "Python".to_string(),
                    severity: IssueSeverity::Critical,
                    message: format!("Python环境检查失败: {e}"),
                    suggestion: self.get_python_installation_suggestion(&e.to_string()),
                    auto_fixable: false,
                };
                status.issues.push(issue);
                error!("Python environment check failed: {}", e);
            }
        }

        // 处理uv工具检查结果
        match uv_result {
            Ok(uv_info) => {
                status.uv_available = true;
                status.uv_version = Some(uv_info.version);
                info!("UV tool inspection passed: {:?}", status.uv_version);
            }
            Err(e) => {
                let issue = EnvironmentIssue {
                    component: "UV".to_string(),
                    severity: IssueSeverity::High,
                    message: format!("uv工具检查失败: {e}"),
                    suggestion: self.get_uv_installation_suggestion(&e.to_string()),
                    auto_fixable: true,
                };
                status.issues.push(issue);
                warn!("uv tool check failed: {}", e);
            }
        }

        // 处理CUDA环境检查结果
        match cuda_result {
            Ok(cuda_info) => {
                status.cuda_available = cuda_info.available;
                status.cuda_version = cuda_info.version;
                status.cuda_devices = cuda_info.devices;
                if status.cuda_available {
                    info!("CUDA environment check passed: {:?}", status.cuda_version);
                } else {
                    let warning = EnvironmentWarning {
                        component: "CUDA".to_string(),
                        message: "CUDA环境不可用".to_string(),
                        impact: "PDF处理性能可能较慢".to_string(),
                    };
                    status.warnings.push(warning);
                    info!("CUDA environment is not available");
                }
            }
            Err(e) => {
                let warning = EnvironmentWarning {
                    component: "CUDA".to_string(),
                    message: format!("CUDA环境检查失败: {e}"),
                    impact: "将使用CPU进行PDF处理".to_string(),
                };
                status.warnings.push(warning);
                warn!("CUDA environment check failed: {}", e);
            }
        }

        // 如果Python可用，检查Python包
        if status.python_available {
            let (mineru_result, markitdown_result) = tokio::join!(
                self.check_mineru_environment_with_retry(),
                self.check_markitdown_environment_with_retry()
            );

            match mineru_result {
                Ok(mineru_info) => {
                    status.mineru_available = true;
                    status.mineru_version = Some(mineru_info.version);
                    info!(
                        "MinerU environment check passed: {:?}",
                        status.mineru_version
                    );
                }
                Err(e) => {
                    let issue = EnvironmentIssue {
                        component: "MinerU".to_string(),
                        severity: IssueSeverity::Critical,
                        message: format!("MinerU环境检查失败: {e}"),
                        suggestion: self.get_mineru_installation_suggestion(&e.to_string()),
                        auto_fixable: true,
                    };
                    status.issues.push(issue);
                    warn!("MinerU environment check failed: {}", e);
                }
            }

            match markitdown_result {
                Ok(markitdown_info) => {
                    status.markitdown_available = true;
                    status.markitdown_version = Some(markitdown_info.version);
                    info!(
                        "MarkItDown environment check passed: {:?}",
                        status.markitdown_version
                    );
                }
                Err(e) => {
                    let issue = EnvironmentIssue {
                        component: "MarkItDown".to_string(),
                        severity: IssueSeverity::Critical,
                        message: format!("MarkItDown环境检查失败: {e}"),
                        suggestion: self.get_markitdown_installation_suggestion(&e.to_string()),
                        auto_fixable: true,
                    };
                    status.issues.push(issue);
                    warn!("MarkItDown environment check failed: {}", e);
                }
            }
        } else {
            // Python不可用时，添加相关问题
            let mineru_issue = EnvironmentIssue {
                component: "MinerU".to_string(),
                severity: IssueSeverity::Critical,
                message: "无法检查MinerU：Python环境不可用".to_string(),
                suggestion: "首先修复Python环境问题".to_string(),
                auto_fixable: false,
            };
            let markitdown_issue = EnvironmentIssue {
                component: "MarkItDown".to_string(),
                severity: IssueSeverity::Critical,
                message: "无法检查MarkItDown：Python环境不可用".to_string(),
                suggestion: "首先修复Python环境问题".to_string(),
                auto_fixable: false,
            };
            status.issues.push(mineru_issue);
            status.issues.push(markitdown_issue);
        }

        // 设置检查时间和持续时间
        status.last_checked = start_time;
        status.check_duration = start_time.elapsed().unwrap_or(Duration::from_secs(0));

        // 更新缓存
        self.update_cache(status.clone()).await;

        info!(
            "Environmental check completed, status: ready={}, health score: {}/100, time taken: {:?}",
            status.is_ready(),
            status.health_score(),
            status.check_duration
        );
        Ok(status)
    }

    /// 获取缓存的环境状态
    async fn get_cached_status(&self) -> Option<EnvironmentStatus> {
        self.environment_cache.read().await.clone()
    }

    /// 更新环境状态缓存
    async fn update_cache(&self, status: EnvironmentStatus) {
        *self.environment_cache.write().await = Some(status);
    }

    /// 清除环境状态缓存
    pub async fn clear_cache(&self) {
        *self.environment_cache.write().await = None;
    }

    /// 验证虚拟环境状态并添加相关问题和警告
    fn validate_virtual_environment_status(
        &self,
        status: &mut EnvironmentStatus,
        python_info: &PythonInfo,
    ) {
        if !python_info.virtual_env_active {
            let issue = EnvironmentIssue {
                component: "Virtual Environment".to_string(),
                severity: IssueSeverity::High,
                message: "虚拟环境未激活".to_string(),
                suggestion: format!(
                    "创建并激活虚拟环境: 运行 'document-parser uv-init' 或手动运行 '{}'",
                    EnvironmentStatus::default().get_activation_command()
                ),
                auto_fixable: true,
            };
            status.issues.push(issue);
        } else {
            // 检查虚拟环境路径是否符合预期
            if let Some(ref venv_path) = python_info.virtual_env_path {
                let expected_venv_path = Some(Path::new(&self.base_dir).join("venv"));

                let is_expected_location = expected_venv_path
                    .as_ref()
                    .map(|expected| venv_path.contains(&expected.to_string_lossy().to_string()))
                    .unwrap_or(false);

                if !is_expected_location {
                    let warning = EnvironmentWarning {
                        component: "Virtual Environment".to_string(),
                        message: format!("虚拟环境位于非预期位置: {venv_path}"),
                        impact: "可能影响依赖管理和路径解析".to_string(),
                    };
                    status.warnings.push(warning);
                }
            }

            // 检查虚拟环境中的Python可执行文件
            let expected_python_path = Some(Self::get_venv_python_path(
                &Path::new(&self.base_dir).join("venv"),
            ));

            if let Some(expected_path) = expected_python_path
                && !expected_path.exists()
            {
                let issue = EnvironmentIssue {
                    component: "Virtual Environment".to_string(),
                    severity: IssueSeverity::Medium,
                    message: format!("预期的Python可执行文件不存在: {}", expected_path.display()),
                    suggestion: "重新创建虚拟环境: 运行 'document-parser uv-init'".to_string(),
                    auto_fixable: true,
                };
                status.issues.push(issue);
            }
        }
    }

    /// 获取Python安装建议
    fn get_python_installation_suggestion(&self, error_message: &str) -> String {
        if error_message.contains("command not found") || error_message.contains("not found") {
            "Python未安装。请安装Python 3.8+: https://www.python.org/downloads/".to_string()
        } else if error_message.contains("版本过低") {
            "Python版本过低。请升级到Python 3.8或更高版本".to_string()
        } else if error_message.contains("超时") {
            "Python命令执行超时。检查系统负载或Python安装是否正常".to_string()
        } else {
            format!("Python环境问题: {error_message}。请检查Python安装并确保可以正常执行")
        }
    }

    /// 获取UV安装建议
    fn get_uv_installation_suggestion(&self, error_message: &str) -> String {
        if error_message.contains("command not found") || error_message.contains("not found") {
            "UV工具未安装。安装命令: curl -LsSf https://astral.sh/uv/install.sh | sh".to_string()
        } else if error_message.contains("版本") {
            "UV版本不兼容。请更新到最新版本: curl -LsSf https://astral.sh/uv/install.sh | sh"
                .to_string()
        } else {
            format!("UV工具问题: {error_message}。请重新安装UV工具")
        }
    }

    /// 获取MinerU安装建议
    fn get_mineru_installation_suggestion(&self, error_message: &str) -> String {
        if error_message.contains("command not found") || error_message.contains("not found") {
            "MinerU未安装。在虚拟环境中安装: uv pip install magic-pdf[full]".to_string()
        } else if error_message.contains("模块") || error_message.contains("module") {
            "MinerU模块缺失。重新安装: uv pip install --force-reinstall magic-pdf[full]".to_string()
        } else if error_message.contains("版本") {
            "MinerU版本问题。更新到最新版本: uv pip install -U magic-pdf[full]".to_string()
        } else {
            format!("MinerU问题: {error_message}。请检查安装或重新安装")
        }
    }

    /// 获取MarkItDown安装建议
    fn get_markitdown_installation_suggestion(&self, error_message: &str) -> String {
        if error_message.contains("模块") || error_message.contains("module") {
            "MarkItDown模块未找到。在虚拟环境中安装: uv pip install markitdown".to_string()
        } else if error_message.contains("版本") {
            "MarkItDown版本问题。更新到最新版本: uv pip install -U markitdown".to_string()
        } else {
            format!("MarkItDown问题: {error_message}。请检查安装或重新安装")
        }
    }

    /// 带重试的Python环境检查
    async fn check_python_environment_with_retry(&self) -> Result<PythonInfo, AppError> {
        self.retry_with_backoff("Python环境检查", || self.check_python_environment())
            .await
    }

    /// 带重试的uv环境检查
    async fn check_uv_environment_with_retry(&self) -> Result<UvInfo, AppError> {
        self.retry_with_backoff("uv环境检查", || self.check_uv_environment())
            .await
    }

    /// 带重试的CUDA环境检查
    async fn check_cuda_environment_with_retry(&self) -> Result<CudaInfo, AppError> {
        self.retry_with_backoff("CUDA环境检查", || self.check_cuda_environment())
            .await
    }

    /// 带重试的MinerU环境检查
    async fn check_mineru_environment_with_retry(&self) -> Result<PackageInfo, AppError> {
        self.retry_with_backoff("MinerU环境检查", || self.check_mineru_environment())
            .await
    }

    /// 带重试的MarkItDown环境检查
    async fn check_markitdown_environment_with_retry(&self) -> Result<PackageInfo, AppError> {
        self.retry_with_backoff("MarkItDown环境检查", || {
            self.check_markitdown_environment()
        })
        .await
    }

    /// 通用重试机制
    async fn retry_with_backoff<T, F, Fut>(
        &self,
        operation_name: &str,
        mut operation: F,
    ) -> Result<T, AppError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let mut last_error = None;
        let mut delay = self.retry_config.base_delay;

        for attempt in 1..=self.retry_config.max_attempts {
            match operation().await {
                Ok(result) => {
                    if attempt > 1 {
                        info!("{} succeeded after {} attempt", operation_name, attempt);
                    }
                    return Ok(result);
                }
                Err(e) => {
                    last_error = Some(e);

                    if attempt < self.retry_config.max_attempts {
                        warn!(
                            "{} The {} attempt failed, try again in {} seconds",
                            operation_name,
                            attempt,
                            delay.as_secs_f32()
                        );

                        // 发送重试进度（与 send_progress 共用 send_progress_raw 单一通道）
                        self.send_progress_raw(InstallProgress {
                            package: operation_name.to_string(),
                            stage: InstallStage::Retrying {
                                attempt,
                                max_attempts: self.retry_config.max_attempts,
                            },
                            progress: (attempt as f32 / self.retry_config.max_attempts as f32)
                                * 100.0,
                            message: format!(
                                "重试中... ({}/{})",
                                attempt, self.retry_config.max_attempts
                            ),
                            estimated_time_remaining: Some(
                                delay * (self.retry_config.max_attempts - attempt),
                            ),
                            bytes_downloaded: None,
                            total_bytes: None,
                        })
                        .await;

                        sleep(delay).await;
                        delay = std::cmp::min(
                            Duration::from_secs_f64(
                                delay.as_secs_f64() * self.retry_config.backoff_multiplier,
                            ),
                            self.retry_config.max_delay,
                        );
                    }
                }
            }
        }

        error!(
            "{} Still failed after {} attempts",
            operation_name, self.retry_config.max_attempts
        );
        Err(last_error.unwrap_or_else(|| AppError::Environment(format!("{operation_name} 失败"))))
    }

    /// 查找系统中可用的Python可执行文件（跨平台）
    async fn find_system_python(&self) -> Option<String> {
        let python_candidates = Self::get_system_python_executable();

        for candidate in python_candidates {
            if Self::is_executable_in_path(&candidate).await {
                debug!("Found system Python: {}", candidate);
                return Some(candidate);
            }
        }

        debug!("System Python executable not found");
        None
    }

    /// 检查Python环境
    #[instrument(skip(self))]
    async fn check_python_environment(&self) -> Result<PythonInfo, AppError> {
        debug!("Check Python environment: {}", self.python_path);

        // 首先检查配置的Python路径是否存在
        let python_executable = if Path::new(&self.python_path).exists() {
            self.python_path.clone()
        } else {
            // 如果虚拟环境Python不存在，尝试使用系统Python
            debug!(
                "The virtual environment Python path does not exist, try to find the system Python"
            );
            self.find_system_python().await.unwrap_or_else(|| {
                // 如果找不到系统Python，使用平台默认值
                if cfg!(windows) {
                    "python.exe".to_string()
                } else {
                    "python3".to_string()
                }
            })
        };

        // 检查Python版本（带超时）
        let version_cmd = Command::new(&python_executable).arg("--version").output();

        let output = timeout(self.timeout_duration, version_cmd)
            .await
            .map_err(|_| {
                AppError::Environment(format!(
                    "Python版本检查超时: {}",
                    self.timeout_duration.as_secs()
                ))
            })?
            .map_err(|e| AppError::Environment(format!("无法执行Python命令: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Environment(format!(
                "Python命令执行失败: {stderr}"
            )));
        }

        let version_output = String::from_utf8_lossy(&output.stdout);
        let version = version_output.trim().to_string();

        // 验证Python版本是否符合要求（3.8+）
        if let Some(version_num) = self.extract_python_version(&version)
            && version_num < (3, 8)
        {
            return Err(AppError::Environment(format!(
                "Python版本过低: {version}，需要3.8或更高版本"
            )));
        }

        // 检查虚拟环境（带超时）
        let venv_cmd = Command::new(&python_executable)
            .arg("-c")
            .arg("import sys; print(hasattr(sys, 'real_prefix') or (hasattr(sys, 'base_prefix') and sys.base_prefix != sys.prefix)); print(getattr(sys, 'prefix', ''))")
            .output();

        let venv_output = timeout(self.timeout_duration, venv_cmd)
            .await
            .map_err(|_| AppError::Environment("虚拟环境检查超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("无法检查虚拟环境: {e}")))?;

        let venv_info = String::from_utf8_lossy(&venv_output.stdout);
        let lines: Vec<&str> = venv_info.trim().split('\n').collect();

        let virtual_env_active = lines
            .first()
            .and_then(|line| line.parse::<bool>().ok())
            .unwrap_or(false);

        let virtual_env_path = if virtual_env_active {
            lines.get(1).map(|s| s.to_string())
        } else {
            None
        };

        debug!("Python environment check passed: {}", version);
        if virtual_env_active {
            debug!("Virtual environment detected: {:?}", virtual_env_path);
        }

        Ok(PythonInfo {
            version: Some(version),
            path: python_executable,
            virtual_env_active,
            virtual_env_path,
        })
    }

    /// 提取Python版本号（解析收敛到 parse_version_tuple）
    fn extract_python_version(&self, version_str: &str) -> Option<(u32, u32)> {
        // 解析类似 "Python 3.9.7" 的版本字符串
        let parts: Vec<&str> = version_str.split_whitespace().collect();
        if parts.len() >= 2 {
            return self
                .parse_version_tuple(parts[1])
                .map(|(major, minor, _)| (major, minor));
        }
        None
    }

    /// 检查uv工具
    async fn check_uv_environment(&self) -> Result<UvInfo, AppError> {
        debug!("Check uv tools");

        let uv_cmd = Command::new("uv").arg("--version").output();

        let output = timeout(self.timeout_duration, uv_cmd)
            .await
            .map_err(|_| AppError::Environment("uv版本检查超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("无法执行uv命令: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Environment(format!("uv命令执行失败: {stderr}")));
        }

        let version_output = String::from_utf8_lossy(&output.stdout);
        let version = version_output.trim().to_string();

        debug!("UV tool inspection passed: {}", version);

        Ok(UvInfo { version })
    }

    /// 检查CUDA环境
    pub async fn check_cuda_environment(&self) -> Result<CudaInfo, AppError> {
        debug!("Check CUDA environment");

        let nvidia_cmd = Command::new("nvidia-smi")
            .arg("--query-gpu=index,name,memory.total,memory.free,compute_cap")
            .arg("--format=csv,noheader,nounits")
            .output();

        let output = match timeout(Duration::from_secs(10), nvidia_cmd).await {
            Ok(Ok(output)) if output.status.success() => output,
            Ok(Ok(_)) => {
                debug!("nvidia-smi execution failed, CUDA is not available");
                return Ok(CudaInfo {
                    available: false,
                    version: None,
                    devices: Vec::new(),
                });
            }
            Ok(Err(_)) | Err(_) => {
                debug!("CUDA environment is not available");
                return Ok(CudaInfo {
                    available: false,
                    version: None,
                    devices: Vec::new(),
                });
            }
        };

        // 解析CUDA设备信息
        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut devices = Vec::new();

        for line in output_str.lines() {
            if let Some(device) = self.parse_cuda_device_info(line) {
                devices.push(device);
            }
        }

        // 获取CUDA版本
        let version = self.get_cuda_version().await;

        debug!(
            "CUDA environment check completed: available={}, devices={}",
            !devices.is_empty(),
            devices.len()
        );

        Ok(CudaInfo {
            available: !devices.is_empty(),
            version,
            devices,
        })
    }

    /// 解析CUDA设备信息
    fn parse_cuda_device_info(&self, line: &str) -> Option<CudaDevice> {
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() >= 5
            && let (Ok(id), Ok(memory_total), Ok(memory_free)) = (
                parts[0].parse::<u32>(),
                parts[2].parse::<u64>(),
                parts[3].parse::<u64>(),
            )
        {
            return Some(CudaDevice {
                id,
                name: parts[1].to_string(),
                memory_total: memory_total * 1024 * 1024, // 转换为字节
                memory_free: memory_free * 1024 * 1024,   // 转换为字节
                compute_capability: parts[4].to_string(),
            });
        }
        None
    }

    /// 获取CUDA版本
    async fn get_cuda_version(&self) -> Option<String> {
        let version_cmd = Command::new("nvidia-smi")
            .arg("--query-gpu=driver_version")
            .arg("--format=csv,noheader,nounits")
            .output();

        if let Ok(Ok(output)) = timeout(Duration::from_secs(5), version_cmd).await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            return Some(version_str.trim().to_string());
        }
        None
    }

    /// 检查MinerU环境
    pub(super) async fn check_mineru_environment(&self) -> Result<PackageInfo, AppError> {
        debug!("Check MinerU environment");

        // 使用 base_dir 虚拟环境中的 mineru 命令路径
        let current_dir = Path::new(&self.base_dir).to_path_buf();
        let venv_path = current_dir.join("venv");
        let mineru_path = Self::get_venv_executable_path(&venv_path, "mineru");

        // 首先检查mineru可执行文件是否存在
        if !mineru_path.exists() {
            return Err(AppError::Environment(format!(
                "MinerU命令不存在: {}. 请运行 'uv pip install -U \"mineru[core]\"' 安装MinerU",
                mineru_path.display()
            )));
        }

        // 检查mineru命令是否可执行
        let help_cmd = Command::new(&mineru_path).arg("--help").output();

        let help_output = timeout(self.timeout_duration, help_cmd)
            .await
            .map_err(|_| AppError::Environment("MinerU帮助命令检查超时".to_string()))?
            .map_err(|e| {
                AppError::Environment(format!(
                    "无法执行MinerU帮助命令: {e}. 请确保已正确安装MinerU"
                ))
            })?;

        if !help_output.status.success() {
            let stderr = String::from_utf8_lossy(&help_output.stderr);
            return Err(AppError::Environment(format!(
                "MinerU帮助命令执行失败: {stderr}. 请检查MinerU安装"
            )));
        }

        // 验证mineru命令功能性 - 测试基本功能
        let test_cmd = Command::new(&mineru_path).arg("--version").output();

        let version_output = timeout(Duration::from_secs(30), test_cmd)
            .await
            .map_err(|_| AppError::Environment("MinerU版本检查超时".to_string()))?;

        let version = match version_output {
            Ok(output) if output.status.success() => {
                let version_str = String::from_utf8_lossy(&output.stdout);
                let version = version_str.trim().to_string();
                if version.is_empty() {
                    // 如果版本输出为空，尝试从stderr获取
                    let stderr_str = String::from_utf8_lossy(&output.stderr);
                    if !stderr_str.is_empty() {
                        stderr_str.trim().to_string()
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    version
                }
            }
            Ok(output) => {
                // 版本命令失败，但帮助命令成功，说明mineru可用但版本获取有问题
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(
                    "MinerU version acquisition failed, but the command is available: {}",
                    stderr
                );
                "available".to_string()
            }
            Err(e) => {
                return Err(AppError::Environment(format!(
                    "MinerU版本检查执行失败: {e}. 请检查MinerU安装"
                )));
            }
        };

        // MinerU命令验证已通过，无需额外的模块导入测试

        debug!("MinerU environment check passed, version: {}", version);

        Ok(PackageInfo { version })
    }

    /// 检查MarkItDown环境
    pub(super) async fn check_markitdown_environment(&self) -> Result<PackageInfo, AppError> {
        debug!("Check MarkItDown environment");

        // 优先使用虚拟环境中的 Python
        let current_dir = Path::new(&self.base_dir).to_path_buf();
        let venv_path = current_dir.join("venv");
        let python_executable = if venv_path.exists() {
            Self::get_venv_python_path(&venv_path)
        } else if Path::new(&self.python_path).exists() {
            std::path::PathBuf::from(&self.python_path)
        } else {
            // 回退到系统Python
            let system_python = self.find_system_python().await.unwrap_or_else(|| {
                if cfg!(windows) {
                    "python.exe".to_string()
                } else {
                    "python3".to_string()
                }
            });
            std::path::PathBuf::from(system_python)
        };

        // 首先测试MarkItDown模块导入
        let import_test_cmd = Command::new(&python_executable)
            .arg("-c")
            .arg("import markitdown; print('MarkItDown模块导入成功')")
            .output();

        let import_output = timeout(self.timeout_duration, import_test_cmd)
            .await
            .map_err(|_| AppError::Environment("MarkItDown模块导入测试超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("无法测试MarkItDown模块导入: {e}")))?;

        if !import_output.status.success() {
            let stderr = String::from_utf8_lossy(&import_output.stderr);
            return Err(AppError::Environment(format!(
                "MarkItDown模块导入失败: {stderr}. 请运行 'uv pip install markitdown' 安装MarkItDown"
            )));
        }

        // 获取版本信息
        let version_cmd = Command::new(&python_executable)
            .arg("-c")
            .arg("import markitdown; print(markitdown.__version__)")
            .output();

        let version_output = timeout(self.timeout_duration, version_cmd)
            .await
            .map_err(|_| AppError::Environment("MarkItDown版本检查超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("无法获取MarkItDown版本: {e}")))?;

        let version = if version_output.status.success() {
            let version_str = String::from_utf8_lossy(&version_output.stdout);
            version_str.trim().to_string()
        } else {
            // 如果版本获取失败但导入成功，使用默认版本
            warn!("MarkItDown version acquisition failed, but the module is available");
            "available".to_string()
        };

        // 功能性验证 - 测试MarkItDown基本功能
        let functionality_test_cmd = Command::new(&python_executable)
            .arg("-c")
            .arg(
                r#"
import markitdown
from markitdown import MarkItDown
md = MarkItDown()
# 测试基本功能是否可用
print('MarkItDown功能验证成功')
"#,
            )
            .output();

        let func_result = timeout(Duration::from_secs(15), functionality_test_cmd).await;
        match func_result {
            Ok(Ok(output)) if output.status.success() => {
                debug!("MarkItDown function verification successful");
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("MarkItDown function verification failed: {}", stderr);
                return Err(AppError::Environment(format!(
                    "MarkItDown功能验证失败: {stderr}. 请重新安装MarkItDown"
                )));
            }
            Ok(Err(e)) => {
                warn!("MarkItDown functional test execution failed: {}", e);
            }
            Err(_) => {
                warn!("MarkItDown function test timeout");
            }
        }

        debug!("MarkItDown environment check passed: {}", version);

        Ok(PackageInfo { version })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_environment_check() {
        let temp_dir = TempDir::new().unwrap();
        let manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        // 环境检查不应该失败（即使某些工具不可用）
        let result = manager.check_environment().await;
        assert!(result.is_ok());

        let status = result.unwrap();
        assert!(status.health_score() <= 100);
    }

    #[tokio::test]
    async fn test_system_python_detection() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        // 测试系统Python查找
        let system_python = manager.find_system_python().await;
        // 注意：这个测试可能在某些环境中失败，如果系统没有安装Python
        // 但我们至少可以验证函数不会panic
        if let Some(python_exe) = system_python {
            assert!(!python_exe.is_empty());
            // 验证返回的是我们期望的可执行文件名之一
            let expected_names = EnvironmentManager::get_system_python_executable();
            assert!(expected_names.contains(&python_exe));
        }
    }
}
