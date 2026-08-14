//! 环境报告与最终验证：详细状态报告、综合环境报告、摘要、虚拟环境自动激活。

use super::*;

impl EnvironmentManager {
    /// 获取详细的环境状态报告
    pub async fn get_detailed_status_report(&self) -> Result<String, AppError> {
        let status = self.check_environment().await?;
        Ok(status.format_diagnostic_report())
    }

    /// 验证环境完整性
    #[instrument(skip(self))]
    pub async fn validate_environment(&self) -> Result<bool, AppError> {
        let status = self.check_environment().await?;

        let is_valid = status.is_ready();
        let health_score = status.health_score();

        if !is_valid {
            warn!(
                "Environment verification failed, health score: {}/100",
                health_score
            );
            for issue in status.get_critical_issues() {
                error!(
                    "Key questions [{}]: {} - {}",
                    issue.component, issue.message, issue.suggestion
                );
            }
        } else {
            info!(
                "Environmental verification passed, health score: {}/100",
                health_score
            );
        }

        Ok(is_valid)
    }

    /// 自动激活虚拟环境（如果存在且未激活）
    #[instrument(skip(self))]
    pub async fn auto_activate_virtual_environment(&self) -> Result<(), AppError> {
        let venv_path = Path::new(&self.base_dir).join("venv");

        // 检查虚拟环境是否存在
        if !venv_path.exists() {
            debug!(
                "The virtual environment does not exist: {}",
                venv_path.display()
            );
            return Ok(());
        }

        // 检查虚拟环境是否已经激活
        if let Ok(virtual_env) = std::env::var("VIRTUAL_ENV")
            && virtual_env == venv_path.to_string_lossy()
        {
            debug!(
                "The virtual environment has been activated: {}",
                virtual_env
            );
            return Ok(());
        }

        // 检查Python可执行文件是否存在
        let python_exe = Self::get_venv_python_path(&venv_path);
        if !python_exe.exists() {
            debug!(
                "The Python executable file in the virtual environment does not exist: {}",
                python_exe.display()
            );
            return Ok(());
        }

        // 设置环境变量以模拟虚拟环境激活
        info!(
            "Automatically activate virtual environment: {}",
            venv_path.display()
        );

        // 计算虚拟环境bin目录路径
        let venv_bin_path = if cfg!(windows) {
            venv_path.join("Scripts").to_string_lossy().to_string()
        } else {
            venv_path.join("bin").to_string_lossy().to_string()
        };

        // 设置环境变量
        unsafe {
            std::env::set_var("VIRTUAL_ENV", venv_path.to_string_lossy().to_string());

            // 更新PATH环境变量，将虚拟环境的bin目录放在前面
            let current_path = std::env::var("PATH").unwrap_or_default();
            let new_path = if cfg!(windows) {
                format!("{venv_bin_path};{current_path}")
            } else {
                format!("{venv_bin_path}:{current_path}")
            };

            std::env::set_var("PATH", new_path);

            // 设置Python相关环境变量
            std::env::set_var("PYTHONPATH", venv_path.to_string_lossy().to_string());
        }

        info!(
            "The virtual environment has been automatically activated, Python path: {}",
            python_exe.display()
        );
        debug!(
            "VIRTUAL_ENV: {}",
            std::env::var("VIRTUAL_ENV").unwrap_or_default()
        );
        debug!("PATH prefix: {}", venv_bin_path);

        Ok(())
    }

    /// 生成详细环境报告
    #[instrument(skip(self))]
    pub async fn generate_environment_report(&self) -> Result<String, AppError> {
        let status = self.check_environment().await?;

        let mut report = String::new();

        // 标题和概览
        report.push_str("=== 环境检查报告 ===\n");
        report.push_str(&format!("检查时间: {:?}\n", status.last_checked));
        report.push_str(&format!("检查耗时: {:?}\n", status.check_duration));
        report.push_str(&format!("健康评分: {}/100\n", status.health_score()));
        report.push_str(&format!(
            "环境状态: {}\n\n",
            if status.is_ready() {
                "就绪"
            } else {
                "未就绪"
            }
        ));

        // 组件状态
        report.push_str("=== 组件状态 ===\n");
        report.push_str(&format!(
            "Python: {} ({:?})\n",
            if status.python_available {
                "✓"
            } else {
                "✗"
            },
            status.python_version.as_deref().unwrap_or("未知")
        ));

        if status.virtual_env_active {
            report.push_str(&format!(
                "  虚拟环境: ✓ ({:?})\n",
                status.virtual_env_path.as_deref().unwrap_or("未知路径")
            ));
        }

        report.push_str(&format!(
            "uv工具: {} ({:?})\n",
            if status.uv_available { "✓" } else { "✗" },
            status.uv_version.as_deref().unwrap_or("未安装")
        ));

        report.push_str(&format!(
            "CUDA: {} ({:?})\n",
            if status.cuda_available { "✓" } else { "✗" },
            status.cuda_version.as_deref().unwrap_or("不可用")
        ));

        if !status.cuda_devices.is_empty() {
            report.push_str("  CUDA设备:\n");
            for device in &status.cuda_devices {
                report.push_str(&format!(
                    "    - GPU {}: {} ({}MB 可用)\n",
                    device.id,
                    device.name,
                    device.memory_free / 1024 / 1024
                ));
            }
        }

        report.push_str(&format!(
            "MinerU: {} ({:?})\n",
            if status.mineru_available {
                "✓"
            } else {
                "✗"
            },
            status.mineru_version.as_deref().unwrap_or("未安装")
        ));

        report.push_str(&format!(
            "MarkItDown: {} ({:?})\n",
            if status.markitdown_available {
                "✓"
            } else {
                "✗"
            },
            status.markitdown_version.as_deref().unwrap_or("未安装")
        ));

        // 问题列表
        if !status.issues.is_empty() {
            report.push_str("\n=== 问题列表 ===\n");
            for issue in &status.issues {
                let severity_icon = match issue.severity {
                    IssueSeverity::Critical => "🔴",
                    IssueSeverity::High => "🟠",
                    IssueSeverity::Medium => "🟡",
                    IssueSeverity::Low => "🔵",
                };
                report.push_str(&format!(
                    "{} [{}] {}: {}\n",
                    severity_icon, issue.component, issue.message, issue.suggestion
                ));
                if issue.auto_fixable {
                    report.push_str("   ↳ 可自动修复\n");
                }
            }
        }

        // 警告列表
        if !status.warnings.is_empty() {
            report.push_str("\n=== 警告列表 ===\n");
            for warning in &status.warnings {
                report.push_str(&format!(
                    "⚠️  [{}] {}\n",
                    warning.component, warning.message
                ));
                report.push_str(&format!("   影响: {}\n", warning.impact));
            }
        }

        // 建议
        report.push_str("\n=== 建议 ===\n");
        if status.is_ready() {
            report.push_str("✅ 环境配置良好，可以正常使用文档解析服务\n");
        } else {
            let auto_fixable = status.get_auto_fixable_issues();
            if !auto_fixable.is_empty() {
                report.push_str("🔧 可以运行自动修复来解决以下问题:\n");
                for issue in auto_fixable {
                    report.push_str(&format!("   - {}: {}\n", issue.component, issue.suggestion));
                }
            }

            let critical_issues = status.get_critical_issues();
            if !critical_issues.is_empty() {
                report.push_str("❌ 需要手动解决以下关键问题:\n");
                for issue in critical_issues {
                    if !issue.auto_fixable {
                        report
                            .push_str(&format!("   - {}: {}\n", issue.component, issue.suggestion));
                    }
                }
            }
        }

        Ok(report)
    }

    /// 获取环境摘要信息
    pub async fn get_environment_summary(&self) -> Result<String, AppError> {
        let status = self.check_environment().await?;

        Ok(format!(
            "环境状态: {} | 健康评分: {}/100 | Python: {} | MinerU: {} | MarkItDown: {} | CUDA: {}",
            if status.is_ready() {
                "就绪"
            } else {
                "未就绪"
            },
            status.health_score(),
            if status.python_available {
                "✓"
            } else {
                "✗"
            },
            if status.mineru_available {
                "✓"
            } else {
                "✗"
            },
            if status.markitdown_available {
                "✓"
            } else {
                "✗"
            },
            if status.cuda_available { "✓" } else { "✗" }
        ))
    }
}
