//! 环境安装编排：setup_python_environment 总编排、依赖包安装（MinerU/MarkItDown）、
//! 镜像源检测与模型源配置。

use super::*;

impl EnvironmentManager {
    /// Python环境设置
    #[instrument(skip(self))]
    pub async fn setup_python_environment(&self) -> Result<(), AppError> {
        info!("Start Python environment setup");

        // 发送开始进度
        self.send_progress("环境设置", InstallStage::Preparing, 0.0, "准备环境设置")
            .await;

        // 确保基础目录存在
        self.ensure_base_directory().await?;
        self.send_progress("环境设置", InstallStage::Preparing, 10.0, "准备工作目录")
            .await;

        // 检查并安装uv
        match self.is_uv_available().await? {
            UvAvailabilityStatus::Available {
                version,
                compatibility,
            } => {
                if compatibility.is_compatible {
                    info!("uv tool is available and compatible: {}", version);
                    if let Some(recommendation) = compatibility.recommendation {
                        info!("UV upgrade suggestion: {}", recommendation);
                    }
                } else {
                    warn!(
                        "The uv version is incompatible, reinstall: {}",
                        compatibility.recommendation.unwrap_or_default()
                    );
                    self.send_progress(
                        "环境设置",
                        InstallStage::Installing,
                        20.0,
                        "重新安装兼容版本的uv",
                    )
                    .await;
                    self.install_uv_with_progress().await?;
                }
            }
            UvAvailabilityStatus::IncompatibleVersion { version, issue } => {
                warn!("UV version is not compatible: {} - {}", version, issue);
                self.send_progress(
                    "环境设置",
                    InstallStage::Installing,
                    20.0,
                    "安装兼容版本的uv",
                )
                .await;
                self.install_uv_with_progress().await?;
            }
            UvAvailabilityStatus::ExecutionFailed { error } => {
                warn!("UV execution failed, reinstall: {}", error);
                self.send_progress("环境设置", InstallStage::Installing, 20.0, "重新安装uv工具")
                    .await;
                self.install_uv_with_progress().await?;
            }
            UvAvailabilityStatus::NotInstalled { error: _ } => {
                info!("The uv tool is not installed, start the installation");
                self.send_progress("环境设置", InstallStage::Installing, 20.0, "安装uv工具")
                    .await;
                self.install_uv_with_progress().await?;
            }
        }

        // 创建Python虚拟环境
        self.send_progress(
            "环境设置",
            InstallStage::Configuring,
            40.0,
            "创建Python虚拟环境",
        )
        .await;
        self.create_python_venv_with_progress().await?;

        // 安装依赖
        self.send_progress("环境设置", InstallStage::Installing, 60.0, "安装Python依赖")
            .await;
        self.install_dependencies().await?;

        // 验证安装（非阻塞）
        self.send_progress("环境设置", InstallStage::Verifying, 90.0, "验证环境")
            .await;
        match self.validate_engines().await {
            Ok(is_valid) => {
                if is_valid {
                    self.send_progress("环境设置", InstallStage::Completed, 100.0, "环境设置完成")
                        .await;
                    info!("Python environment setup completed");
                } else {
                    warn!(
                        "Environment verification did not fully pass, but the installation process was completed"
                    );
                    self.send_progress(
                        "环境设置",
                        InstallStage::Completed,
                        100.0,
                        "安装完成（部分验证待完善）",
                    )
                    .await;
                }
            }
            Err(e) => {
                warn!(
                    "There was a problem with the environment verification process: {}",
                    e
                );
                self.send_progress(
                    "环境设置",
                    InstallStage::Completed,
                    100.0,
                    "安装完成（验证待重试）",
                )
                .await;
            }
        }

        // 清除缓存以强制重新检查
        self.clear_cache().await;

        Ok(())
    }

    /// 安装依赖包
    #[instrument(skip(self))]
    pub async fn install_dependencies(&self) -> Result<(), AppError> {
        info!("Start installing Python dependencies");

        // 并行安装MinerU和MarkItDown
        let (mineru_result, markitdown_result) = tokio::join!(
            self.install_mineru_with_progress(),
            self.install_markitdown_with_progress()
        );

        mineru_result?;
        markitdown_result?;

        info!("Python dependency installation completed");
        Ok(())
    }

    /// 验证所有引擎
    #[instrument(skip(self))]
    pub async fn validate_engines(&self) -> Result<bool, AppError> {
        info!("Verify parsing engine");

        // 清除缓存以确保获取最新状态
        self.clear_cache().await;

        // 等待一小段时间确保安装完成
        sleep(Duration::from_millis(500)).await;

        let status = self.check_environment().await?;
        let is_valid = status.is_ready();

        if !is_valid {
            let critical_issues = status.get_critical_issues();
            for issue in critical_issues {
                error!("Key questions: {} - {}", issue.component, issue.message);
            }
        }

        Ok(is_valid)
    }

    /// 确保基础目录存在
    async fn ensure_base_directory(&self) -> Result<(), AppError> {
        if !Path::new(&self.base_dir).exists() {
            std::fs::create_dir_all(&self.base_dir)
                .map_err(|e| AppError::File(format!("创建基础目录失败: {e}")))?;
            info!("Create base directory: {}", self.base_dir);
        }
        Ok(())
    }

    /// 安装MinerU（带进度跟踪）
    async fn install_mineru_with_progress(&self) -> Result<(), AppError> {
        info!("Anso MinerU");

        self.send_progress("MinerU", InstallStage::Preparing, 0.0, "准备安装MinerU")
            .await;

        // 检测是否在中国大陆，如果是则使用国内镜像
        let is_china = self.is_china_region().await;

        // 检查CUDA环境状态，决定安装哪个版本的MinerU
        let cuda_status = self.check_cuda_environment().await;
        let mineru_package = match cuda_status {
            Ok(cuda_info) if cuda_info.available && !cuda_info.devices.is_empty() => {
                info!("CUDA environment detected, install mineru[all] to support GPU acceleration");
                "mineru[all]"
            }
            _ => {
                info!("CUDA environment not detected, install mineru[core] (CPU version only)");
                "mineru[core]"
            }
        };

        let venv_path = Path::new(&self.base_dir).join("venv");
        let python_path = Self::get_venv_python_path(&venv_path);

        let mut install_cmd = Command::new("uv");
        install_cmd
            .arg("pip")
            .arg("install")
            .arg("-U")
            .arg("--python")
            .arg(&python_path)
            .arg(mineru_package);

        // 如果在中国大陆，添加镜像配置
        if is_china {
            info!("Mainland China environment detected, using Alibaba Cloud mirror source");
            install_cmd
                .arg("-i")
                .arg("https://mirrors.aliyun.com/pypi/simple/")
                .arg("--trusted-host")
                .arg("mirrors.aliyun.com");
        }
        //install_cmd 命令打印
        info!("mineru installation command={:?}", &install_cmd);

        let install_cmd = install_cmd.output();

        self.send_progress("MinerU", InstallStage::Downloading, 20.0, "下载MinerU包")
            .await;

        let output = timeout(Duration::from_secs(900), install_cmd)
            .await
            .map_err(|_| AppError::Environment("MinerU安装超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("安装MinerU失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            self.send_progress(
                "MinerU",
                InstallStage::Failed(stderr.to_string()),
                0.0,
                "安装失败",
            )
            .await;
            return Err(AppError::Environment(format!("MinerU安装失败: {stderr}")));
        }

        self.send_progress("MinerU", InstallStage::Configuring, 80.0, "配置MinerU环境")
            .await;

        // 如果在中国大陆，配置模型源
        if is_china && let Err(e) = self.configure_mineru_model_source().await {
            warn!("Failed to configure MinerU model source: {}", e);
            // 不阻断安装流程，只记录警告
        }

        self.send_progress("MinerU", InstallStage::Verifying, 90.0, "验证MinerU安装")
            .await;

        // 验证安装
        match self.check_mineru_environment().await {
            Ok(_) => {
                self.send_progress("MinerU", InstallStage::Completed, 100.0, "MinerU安装完成")
                    .await;
                info!("MinerU installation completed");
                Ok(())
            }
            Err(e) => {
                self.send_progress(
                    "MinerU",
                    InstallStage::Failed(e.to_string()),
                    0.0,
                    "验证失败",
                )
                .await;
                Err(AppError::Environment(format!("MinerU安装验证失败: {e}")))
            }
        }
    }

    /// 安装MarkItDown（带进度跟踪）
    async fn install_markitdown_with_progress(&self) -> Result<(), AppError> {
        info!("InstallMarkItDown");

        self.send_progress(
            "MarkItDown",
            InstallStage::Preparing,
            0.0,
            "准备安装MarkItDown",
        )
        .await;

        // 检测是否在中国大陆，如果是则使用国内镜像
        let is_china = self.is_china_region().await;

        let venv_path = Path::new(&self.base_dir).join("venv");
        let python_path = Self::get_venv_python_path(&venv_path);

        let mut install_cmd = Command::new("uv");
        install_cmd
            .arg("pip")
            .arg("install")
            .arg("--python")
            .arg(&python_path)
            .arg("markitdown");

        // 如果在中国大陆，添加镜像配置
        if is_china {
            info!("Mainland China environment detected, using domestic mirror source");
            install_cmd
                .arg("-i")
                .arg("https://mirrors.aliyun.com/pypi/simple/")
                .arg("--trusted-host")
                .arg("mirrors.aliyun.com");
        }

        let install_cmd = install_cmd.output();

        self.send_progress(
            "MarkItDown",
            InstallStage::Downloading,
            20.0,
            "下载MarkItDown包",
        )
        .await;

        let output = timeout(Duration::from_secs(600), install_cmd)
            .await
            .map_err(|_| AppError::Environment("MarkItDown安装超时".to_string()))?
            .map_err(|e| AppError::Environment(format!("安装MarkItDown失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            self.send_progress(
                "MarkItDown",
                InstallStage::Failed(stderr.to_string()),
                0.0,
                "安装失败",
            )
            .await;
            return Err(AppError::Environment(format!(
                "MarkItDown安装失败: {stderr}"
            )));
        }

        self.send_progress(
            "MarkItDown",
            InstallStage::Verifying,
            90.0,
            "验证MarkItDown安装",
        )
        .await;

        // 验证安装
        match self.check_markitdown_environment().await {
            Ok(_) => {
                self.send_progress(
                    "MarkItDown",
                    InstallStage::Completed,
                    100.0,
                    "MarkItDown安装完成",
                )
                .await;
                info!("MarkItDown installation completed");
                Ok(())
            }
            Err(e) => {
                self.send_progress(
                    "MarkItDown",
                    InstallStage::Failed(e.to_string()),
                    0.0,
                    "验证失败",
                )
                .await;
                Err(AppError::Environment(format!(
                    "MarkItDown安装验证失败: {e}"
                )))
            }
        }
    }

    /// 检测是否在中国大陆地区
    async fn is_china_region(&self) -> bool {
        // 检查时区
        if let Ok(tz) = std::env::var("TZ")
            && (tz.contains("Asia/Shanghai") || tz.contains("Asia/Beijing"))
        {
            return true;
        }

        // 检查语言环境
        if let Ok(lang) = std::env::var("LANG")
            && lang.contains("zh_CN")
        {
            return true;
        }

        // 检查系统语言（macOS）
        if let Ok(output) = Command::new("defaults")
            .arg("read")
            .arg("-g")
            .arg("AppleLanguages")
            .output()
            .await
        {
            let output_str = String::from_utf8_lossy(&output.stdout);
            if output_str.contains("zh-Hans") || output_str.contains("zh-CN") {
                return true;
            }
        }

        // 尝试ping测试（简单的网络检测）
        if let Ok(output) = Command::new("ping")
            .arg("-c")
            .arg("1")
            .arg("-W")
            .arg("3000")
            .arg("baidu.com")
            .output()
            .await
            && output.status.success()
        {
            return true;
        }

        false
    }

    /// 配置MinerU模型源为ModelScope（中国大陆）
    async fn configure_mineru_model_source(&self) -> Result<(), AppError> {
        info!("Configure MinerU to use the ModelScope model source");

        // 创建配置目录
        let home_dir = std::env::var("HOME")
            .map_err(|_| AppError::Environment("无法获取HOME目录".to_string()))?;
        let config_dir = format!("{home_dir}/.mineru");

        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            warn!("Failed to create MinerU configuration directory: {}", e);
        }

        // 创建配置文件内容
        let config_content = r#"{
    "model_source": "modelscope",
    "default_source": "modelscope"
}"#;

        let config_file = format!("{config_dir}/config.json");
        if let Err(e) = std::fs::write(&config_file, config_content) {
            return Err(AppError::Environment(format!(
                "写入MinerU配置文件失败: {e}"
            )));
        }

        info!(
            "MinerU model source configuration completed: {}",
            config_file
        );
        Ok(())
    }
}
