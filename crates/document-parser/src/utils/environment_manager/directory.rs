//! 当前目录设置校验与清理：目录/venv 路径诊断、清理选项执行、目录验证报告。

use super::*;

/// 目录验证结果
#[derive(Debug, Clone)]
pub struct DirectoryValidationResult {
    pub is_valid: bool,
    pub current_directory: std::path::PathBuf,
    pub venv_path: std::path::PathBuf,
    pub issues: Vec<DirectoryValidationIssue>,
    pub warnings: Vec<DirectoryValidationWarning>,
    pub cleanup_options: Vec<CleanupOption>,
    pub recommendations: Vec<String>,
}

/// 目录验证问题
#[derive(Debug, Clone)]
pub struct DirectoryValidationIssue {
    pub issue_type: DirectoryIssueType,
    pub message: String,
    pub severity: ValidationSeverity,
    pub auto_fixable: bool,
    pub fix_suggestion: String,
}

/// 目录验证警告
#[derive(Debug, Clone)]
pub struct DirectoryValidationWarning {
    pub warning_type: DirectoryWarningType,
    pub message: String,
    pub impact: String,
}

/// 清理选项
#[derive(Debug, Clone)]
pub struct CleanupOption {
    pub option_type: CleanupType,
    pub description: String,
    pub risk_level: CleanupRisk,
    pub command: String,
}

/// 目录问题类型
#[derive(Debug, Clone, PartialEq)]
pub enum DirectoryIssueType {
    PermissionDenied,
    InsufficientSpace,
    PathConflict,
    PathTooLong,
}

/// 目录警告类型
#[derive(Debug, Clone, PartialEq)]
pub enum DirectoryWarningType {
    ExistingVenv,
    CorruptedVenv,
    PathWithSpaces,
}

/// 验证严重程度
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// 清理类型
#[derive(Debug, Clone, PartialEq)]
pub enum CleanupType {
    RemoveConflictingFile,
    RemoveCorruptedVenv,
    CreateBackup,
}

/// 清理风险级别
#[derive(Debug, Clone, PartialEq)]
pub enum CleanupRisk {
    Low,
    Medium,
    High,
}

impl EnvironmentManager {
    /// 检查并报告虚拟环境状态
    pub async fn check_virtual_environment_status(&self) -> Result<VirtualEnvStatus, AppError> {
        let status = self.check_environment().await?;
        Ok(status.get_virtual_env_status())
    }

    /// 诊断虚拟环境路径问题
    pub async fn diagnose_venv_path_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let venv_path = Path::new(&self.base_dir).join("venv");
        let base_dir = Path::new(&self.base_dir);

        // 检查基础目录
        if !base_dir.exists() {
            issues.push(format!("基础目录不存在: {}", base_dir.display()));
        } else if !base_dir.is_dir() {
            issues.push(format!("基础路径不是目录: {}", base_dir.display()));
        } else {
            // 检查写入权限
            if let Err(e) = self.check_directory_writable(base_dir).await {
                issues.push(format!(
                    "基础目录无写入权限: {} ({})",
                    base_dir.display(),
                    e
                ));
            }
        }

        // 检查虚拟环境路径
        if venv_path.exists() {
            if !venv_path.is_dir() {
                issues.push(format!(
                    "虚拟环境路径存在但不是目录: {}",
                    venv_path.display()
                ));
            } else {
                // 检查虚拟环境完整性
                let python_exe = Self::get_venv_python_path(&venv_path);

                if !python_exe.exists() {
                    issues.push(format!(
                        "虚拟环境不完整，缺少Python可执行文件: {}",
                        python_exe.display()
                    ));
                }
            }
        }

        // 检查路径长度（Windows）
        if cfg!(windows) && venv_path.to_string_lossy().len() > 260 {
            issues.push(format!(
                "虚拟环境路径过长 ({} 字符)，Windows限制为260字符",
                venv_path.to_string_lossy().len()
            ));
        }

        issues
    }

    /// 生成虚拟环境问题的恢复建议
    pub async fn get_venv_recovery_suggestions(&self) -> Vec<String> {
        let mut suggestions = Vec::new();
        let issues = self.diagnose_venv_path_issues().await;

        if issues.is_empty() {
            suggestions.push("虚拟环境路径检查通过，可以尝试创建虚拟环境".to_string());
            return suggestions;
        }

        suggestions.push("检测到以下虚拟环境路径问题:".to_string());
        for issue in &issues {
            suggestions.push(format!("  - {issue}"));
        }

        suggestions.push("".to_string());
        suggestions.push("建议的解决方案:".to_string());

        // 基于问题类型提供具体建议
        for issue in &issues {
            if issue.contains("不存在") {
                suggestions.push("1. 确保在正确的项目目录中运行命令".to_string());
                suggestions.push("2. 检查当前工作目录: pwd (Unix) 或 cd (Windows)".to_string());
            } else if issue.contains("权限") {
                suggestions.push("1. 检查目录权限: ls -la (Unix)".to_string());
                suggestions.push("2. 使用管理员权限运行命令".to_string());
                if cfg!(unix) {
                    suggestions.push("3. 修改目录权限: chmod 755 .".to_string());
                    suggestions.push("4. 修改目录所有者: chown $USER .".to_string());
                }
            } else if issue.contains("不是目录") {
                suggestions
                    .push("1. 删除同名文件: rm venv (Unix) 或 del venv (Windows)".to_string());
                suggestions.push("2. 重新创建虚拟环境".to_string());
            } else if issue.contains("不完整") {
                suggestions.push("1. 删除损坏的虚拟环境: rm -rf ./venv".to_string());
                suggestions.push("2. 重新运行 document-parser uv-init".to_string());
            } else if issue.contains("路径过长") {
                suggestions.push("1. 移动项目到路径较短的目录".to_string());
                suggestions.push("2. 使用较短的目录名称".to_string());
            } else if issue.contains("磁盘空间") {
                suggestions.push("1. 清理磁盘空间，至少保留500MB可用空间".to_string());
                suggestions.push("2. 删除不需要的文件和目录".to_string());
            }
        }

        suggestions.push("".to_string());
        suggestions.push("如果问题仍然存在，请尝试:".to_string());
        suggestions.push("1. 重启终端或命令提示符".to_string());
        suggestions.push("2. 检查防病毒软件是否阻止文件操作".to_string());
        suggestions.push("3. 在不同的目录中尝试创建虚拟环境".to_string());

        suggestions
    }

    /// 尝试自动修复常见的虚拟环境路径问题
    pub async fn auto_fix_venv_path_issues(&self) -> Result<Vec<String>, AppError> {
        let mut fixed_issues = Vec::new();
        let venv_path = Path::new(&self.base_dir).join("venv");

        // 尝试清理损坏的虚拟环境
        if venv_path.exists() && !venv_path.is_dir() {
            match std::fs::remove_file(&venv_path) {
                Ok(_) => {
                    fixed_issues.push(format!(
                        "已删除阻碍虚拟环境创建的文件: {}",
                        venv_path.display()
                    ));
                }
                Err(e) => {
                    return Err(AppError::permission_error(
                        format!("无法删除阻碍文件: {e}"),
                        &venv_path,
                    ));
                }
            }
        }

        // 尝试清理损坏的虚拟环境目录
        if venv_path.exists() && venv_path.is_dir() {
            let python_exe = Self::get_venv_python_path(&venv_path);

            if !python_exe.exists() {
                match self.cleanup_corrupted_venv(&venv_path).await {
                    Ok(_) => {
                        fixed_issues.push(format!("已清理损坏的虚拟环境: {}", venv_path.display()));
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
        }

        Ok(fixed_issues)
    }

    /// 验证当前目录设置（任务12的核心功能）
    #[instrument(skip(self))]
    pub async fn validate_current_directory_setup(
        &self,
    ) -> Result<DirectoryValidationResult, AppError> {
        let current_dir = Path::new(&self.base_dir);
        let venv_path = current_dir.join("venv");

        info!(
            "Start verifying current directory settings: {}",
            current_dir.display()
        );

        let mut result = DirectoryValidationResult {
            is_valid: true,
            current_directory: current_dir.to_path_buf(),
            venv_path: venv_path.clone(),
            issues: Vec::new(),
            warnings: Vec::new(),
            cleanup_options: Vec::new(),
            recommendations: Vec::new(),
        };

        // 1. 检查当前目录是否可写
        if let Err(e) = self.check_directory_writable(current_dir).await {
            result.is_valid = false;
            result.issues.push(DirectoryValidationIssue {
                issue_type: DirectoryIssueType::PermissionDenied,
                message: format!("当前目录不可写: {e}"),
                severity: ValidationSeverity::Critical,
                auto_fixable: false,
                fix_suggestion: "检查目录权限，确保当前用户有写入权限".to_string(),
            });
        }

        // 3. 检查虚拟环境路径冲突
        if venv_path.exists() {
            if venv_path.is_file() {
                result.is_valid = false;
                result.issues.push(DirectoryValidationIssue {
                    issue_type: DirectoryIssueType::PathConflict,
                    message: "虚拟环境路径被文件占用".to_string(),
                    severity: ValidationSeverity::High,
                    auto_fixable: true,
                    fix_suggestion: "删除冲突的文件".to_string(),
                });

                result.cleanup_options.push(CleanupOption {
                    option_type: CleanupType::RemoveConflictingFile,
                    description: format!("删除冲突文件: {}", venv_path.display()),
                    risk_level: CleanupRisk::Low,
                    command: format!("rm {}", venv_path.display()),
                });
            } else if venv_path.is_dir() {
                // 检查虚拟环境是否损坏
                let python_exe = Self::get_venv_python_path(&venv_path);
                if !python_exe.exists() {
                    result.warnings.push(DirectoryValidationWarning {
                        warning_type: DirectoryWarningType::CorruptedVenv,
                        message: "检测到损坏的虚拟环境".to_string(),
                        impact: "虚拟环境无法正常使用".to_string(),
                    });

                    result.cleanup_options.push(CleanupOption {
                        option_type: CleanupType::RemoveCorruptedVenv,
                        description: format!("清理损坏的虚拟环境: {}", venv_path.display()),
                        risk_level: CleanupRisk::Medium,
                        command: format!("rm -rf {}", venv_path.display()),
                    });
                } else {
                    // 虚拟环境存在且看起来完整，进行更深入的验证
                    match self.test_virtual_environment_activation(&venv_path).await {
                        Ok(true) => {
                            result.warnings.push(DirectoryValidationWarning {
                                warning_type: DirectoryWarningType::ExistingVenv,
                                message: "检测到现有的虚拟环境".to_string(),
                                impact: "将使用现有虚拟环境，可能需要更新依赖".to_string(),
                            });
                        }
                        Ok(false) => {
                            result.warnings.push(DirectoryValidationWarning {
                                warning_type: DirectoryWarningType::CorruptedVenv,
                                message: "现有虚拟环境无法激活".to_string(),
                                impact: "虚拟环境可能已损坏".to_string(),
                            });

                            result.cleanup_options.push(CleanupOption {
                                option_type: CleanupType::RemoveCorruptedVenv,
                                description: format!(
                                    "清理无法激活的虚拟环境: {}",
                                    venv_path.display()
                                ),
                                risk_level: CleanupRisk::Medium,
                                command: format!("rm -rf {}", venv_path.display()),
                            });
                        }
                        Err(e) => {
                            result.warnings.push(DirectoryValidationWarning {
                                warning_type: DirectoryWarningType::CorruptedVenv,
                                message: format!("虚拟环境测试失败: {e}"),
                                impact: "无法确定虚拟环境状态".to_string(),
                            });
                        }
                    }
                }
            }
        }

        // 4. 检查路径长度（Windows特有问题）
        if cfg!(windows) && venv_path.to_string_lossy().len() > 260 {
            result.is_valid = false;
            result.issues.push(DirectoryValidationIssue {
                issue_type: DirectoryIssueType::PathTooLong,
                message: format!(
                    "虚拟环境路径过长 ({} 字符)，Windows限制为260字符",
                    venv_path.to_string_lossy().len()
                ),
                severity: ValidationSeverity::High,
                auto_fixable: false,
                fix_suggestion: "移动项目到路径较短的目录".to_string(),
            });
        }

        // 5. 检查特殊字符和编码问题
        let path_str = venv_path.to_string_lossy();
        if path_str.contains(' ') {
            result.warnings.push(DirectoryValidationWarning {
                warning_type: DirectoryWarningType::PathWithSpaces,
                message: "路径包含空格".to_string(),
                impact: "某些工具可能无法正确处理包含空格的路径".to_string(),
            });
        }

        // 6. 生成推荐建议
        self.generate_directory_recommendations(&mut result);

        info!(
            "Directory verification completed: valid={}, issues={}, warnings={}",
            result.is_valid,
            result.issues.len(),
            result.warnings.len()
        );

        Ok(result)
    }

    /// 生成目录验证推荐建议
    fn generate_directory_recommendations(&self, result: &mut DirectoryValidationResult) {
        if result.is_valid && result.warnings.is_empty() {
            result
                .recommendations
                .push("当前目录设置良好，可以安全创建虚拟环境".to_string());
            return;
        }

        if !result.is_valid {
            result
                .recommendations
                .push("请先解决关键问题后再创建虚拟环境".to_string());
        }

        // 基于问题类型生成具体建议
        for issue in &result.issues {
            match issue.issue_type {
                DirectoryIssueType::PermissionDenied => {
                    if cfg!(unix) {
                        result
                            .recommendations
                            .push("使用 'chmod 755 .' 修改目录权限".to_string());
                        result
                            .recommendations
                            .push("使用 'chown $USER .' 修改目录所有者".to_string());
                    } else if cfg!(windows) {
                        result
                            .recommendations
                            .push("以管理员身份运行命令".to_string());
                        result
                            .recommendations
                            .push("检查Windows用户账户控制(UAC)设置".to_string());
                    }
                }
                DirectoryIssueType::InsufficientSpace => {
                    result
                        .recommendations
                        .push("清理不需要的文件释放磁盘空间".to_string());
                    result
                        .recommendations
                        .push("考虑移动项目到有更多可用空间的磁盘".to_string());
                }
                DirectoryIssueType::PathConflict => {
                    result
                        .recommendations
                        .push("删除或重命名冲突的文件/目录".to_string());
                }
                DirectoryIssueType::PathTooLong => {
                    result
                        .recommendations
                        .push("移动项目到路径较短的目录".to_string());
                    result
                        .recommendations
                        .push("使用较短的目录名称".to_string());
                }
            }
        }

        // 基于清理选项生成建议
        if !result.cleanup_options.is_empty() {
            result
                .recommendations
                .push("可以使用以下清理选项解决问题:".to_string());
            for option in &result.cleanup_options {
                result.recommendations.push(format!(
                    "  - {} (风险: {:?})",
                    option.description, option.risk_level
                ));
            }
        }
    }

    /// 执行自动清理选项
    pub async fn execute_cleanup_option(
        &self,
        option_type: CleanupType,
    ) -> Result<String, AppError> {
        let venv_path = Path::new(&self.base_dir).join("venv");

        match option_type {
            CleanupType::RemoveConflictingFile => {
                if venv_path.exists() && venv_path.is_file() {
                    std::fs::remove_file(&venv_path).map_err(|e| {
                        AppError::permission_error(format!("删除冲突文件失败: {e}"), &venv_path)
                    })?;
                    Ok(format!("成功删除冲突文件: {}", venv_path.display()))
                } else {
                    Err(AppError::path_error(
                        "冲突文件不存在".to_string(),
                        &venv_path,
                    ))
                }
            }
            CleanupType::RemoveCorruptedVenv => {
                if venv_path.exists() && venv_path.is_dir() {
                    self.cleanup_corrupted_venv(&venv_path).await?;
                    Ok(format!("成功清理损坏的虚拟环境: {}", venv_path.display()))
                } else {
                    Err(AppError::path_error(
                        "虚拟环境目录不存在".to_string(),
                        &venv_path,
                    ))
                }
            }
            CleanupType::CreateBackup => {
                if venv_path.exists() {
                    let backup_path = Path::new(&self.base_dir).join("venv.backup");
                    std::fs::rename(&venv_path, &backup_path).map_err(|e| {
                        AppError::permission_error(format!("创建备份失败: {e}"), &venv_path)
                    })?;
                    Ok(format!("成功备份虚拟环境到: {}", backup_path.display()))
                } else {
                    Err(AppError::path_error(
                        "虚拟环境不存在，无需备份".to_string(),
                        &venv_path,
                    ))
                }
            }
        }
    }

    /// 检查当前目录是否适合创建虚拟环境（公共接口）
    pub async fn check_current_directory_readiness(
        &self,
    ) -> Result<DirectoryValidationResult, AppError> {
        self.validate_current_directory_setup().await
    }

    /// 获取目录验证报告的格式化字符串
    pub async fn get_directory_validation_report(&self) -> Result<String, AppError> {
        let result = self.validate_current_directory_setup().await?;
        Ok(self.format_directory_validation_report(&result))
    }

    /// 格式化目录验证报告
    fn format_directory_validation_report(&self, result: &DirectoryValidationResult) -> String {
        let mut report = String::new();

        report.push_str("=== 当前目录验证报告 ===\n");
        report.push_str(&format!("目录: {}\n", result.current_directory.display()));
        report.push_str(&format!("虚拟环境路径: {}\n", result.venv_path.display()));
        report.push_str(&format!(
            "验证状态: {}\n\n",
            if result.is_valid {
                "✓ 通过"
            } else {
                "✗ 失败"
            }
        ));

        if !result.issues.is_empty() {
            report.push_str("=== 发现的问题 ===\n");
            for (i, issue) in result.issues.iter().enumerate() {
                report.push_str(&format!(
                    "{}. [{}] {}\n",
                    i + 1,
                    format!("{:?}", issue.severity).to_uppercase(),
                    issue.message
                ));
                report.push_str(&format!("   建议: {}\n", issue.fix_suggestion));
                if issue.auto_fixable {
                    report.push_str("   状态: 可自动修复\n");
                }
                report.push('\n');
            }
        }

        if !result.warnings.is_empty() {
            report.push_str("=== 警告信息 ===\n");
            for (i, warning) in result.warnings.iter().enumerate() {
                report.push_str(&format!("{}. {}\n", i + 1, warning.message));
                report.push_str(&format!("   影响: {}\n\n", warning.impact));
            }
        }

        if !result.cleanup_options.is_empty() {
            report.push_str("=== 可用的清理选项 ===\n");
            for (i, option) in result.cleanup_options.iter().enumerate() {
                report.push_str(&format!(
                    "{}. {} (风险: {:?})\n",
                    i + 1,
                    option.description,
                    option.risk_level
                ));
                report.push_str(&format!("   命令: {}\n\n", option.command));
            }
        }

        if !result.recommendations.is_empty() {
            report.push_str("=== 推荐操作 ===\n");
            for (i, recommendation) in result.recommendations.iter().enumerate() {
                report.push_str(&format!("{}. {}\n", i + 1, recommendation));
            }
        }

        report
    }
}
