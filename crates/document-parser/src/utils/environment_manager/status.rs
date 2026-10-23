//! 环境状态模型 EnvironmentStatus 及其诊断报告生成（纯查询/格式化，无 I/O）。

use super::*;

/// 环境检查结果
#[derive(Debug, Clone)]
pub struct EnvironmentStatus {
    pub python_available: bool,
    pub python_version: Option<String>,
    pub python_path: Option<String>,
    pub uv_available: bool,
    pub uv_version: Option<String>,
    pub cuda_available: bool,
    pub cuda_version: Option<String>,
    pub cuda_devices: Vec<CudaDevice>,
    pub mineru_available: bool,
    pub mineru_version: Option<String>,
    pub markitdown_available: bool,
    pub markitdown_version: Option<String>,
    pub virtual_env_active: bool,
    pub virtual_env_path: Option<String>,
    pub issues: Vec<EnvironmentIssue>,
    pub warnings: Vec<EnvironmentWarning>,
    pub last_checked: std::time::SystemTime,
    pub check_duration: Duration,
}

/// 诊断报告
#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    pub overall_status: String,
    pub health_score: u8,
    pub components: Vec<ComponentDiagnostic>,
    pub recommendations: Vec<String>,
    pub next_steps: Vec<String>,
}

/// 组件诊断信息
#[derive(Debug, Clone)]
pub struct ComponentDiagnostic {
    pub name: String,
    pub status: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub issues: Vec<String>,
    pub details: String,
}

impl Default for EnvironmentStatus {
    fn default() -> Self {
        Self {
            python_available: false,
            python_version: None,
            python_path: None,
            uv_available: false,
            uv_version: None,
            cuda_available: false,
            cuda_version: None,
            cuda_devices: Vec::new(),
            mineru_available: false,
            mineru_version: None,
            markitdown_available: false,
            markitdown_version: None,
            virtual_env_active: false,
            virtual_env_path: None,
            issues: Vec::new(),
            warnings: Vec::new(),
            last_checked: std::time::SystemTime::now(),
            check_duration: Duration::from_secs(0),
        }
    }
}

impl EnvironmentStatus {
    /// 检查环境是否就绪
    pub fn is_ready(&self) -> bool {
        self.python_available && self.mineru_available && self.markitdown_available
    }

    /// 获取问题列表
    pub fn get_issues(&self) -> &Vec<EnvironmentIssue> {
        &self.issues
    }

    /// 获取警告列表
    pub fn get_warnings(&self) -> &Vec<EnvironmentWarning> {
        &self.warnings
    }

    /// 获取关键问题（阻止系统运行的问题）
    pub fn get_critical_issues(&self) -> Vec<&EnvironmentIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Critical)
            .collect()
    }

    /// 获取可自动修复的问题
    pub fn get_auto_fixable_issues(&self) -> Vec<&EnvironmentIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.auto_fixable)
            .collect()
    }

    /// 检查是否有CUDA支持
    pub fn has_cuda_support(&self) -> bool {
        self.cuda_available && !self.cuda_devices.is_empty()
    }

    /// 获取推荐的CUDA设备
    pub fn get_recommended_cuda_device(&self) -> Option<&CudaDevice> {
        self.cuda_devices
            .iter()
            .max_by_key(|device| device.memory_free)
    }

    /// 检查缓存是否过期
    pub fn is_cache_expired(&self, ttl: Duration) -> bool {
        self.last_checked.elapsed().unwrap_or(Duration::MAX) > ttl
    }

    /// 获取虚拟环境状态详细信息
    pub fn get_virtual_env_status(&self) -> VirtualEnvStatus {
        VirtualEnvStatus {
            is_active: self.virtual_env_active,
            path: self.virtual_env_path.clone(),
            expected_path: Some("./venv".to_string()),
            python_executable: self.python_path.clone(),
            is_properly_configured: self.is_virtual_env_properly_configured(),
            activation_command: self.get_activation_command(),
        }
    }

    /// 检查虚拟环境是否正确配置
    pub fn is_virtual_env_properly_configured(&self) -> bool {
        if !self.virtual_env_active {
            return false;
        }

        // 检查虚拟环境路径是否符合预期（当前目录下的venv）
        if let Some(ref venv_path) = self.virtual_env_path {
            // 检查路径是否以当前目录的venv结尾
            venv_path.ends_with("venv")
                || venv_path.contains("/venv")
                || venv_path.contains("\\venv")
        } else {
            false
        }
    }

    /// 获取虚拟环境激活命令
    pub fn get_activation_command(&self) -> String {
        if cfg!(windows) {
            // Windows supports both batch and PowerShell activation
            ".\\venv\\Scripts\\activate.bat".to_string()
        } else {
            "source ./venv/bin/activate".to_string()
        }
    }

    /// 获取虚拟环境激活命令（PowerShell版本，仅Windows）
    pub fn get_powershell_activation_command(&self) -> Option<String> {
        if cfg!(windows) {
            Some(".\\venv\\Scripts\\Activate.ps1".to_string())
        } else {
            None
        }
    }

    /// 生成详细的诊断报告
    pub fn generate_diagnostic_report(&self) -> DiagnosticReport {
        let mut report = DiagnosticReport {
            overall_status: if self.is_ready() {
                "Ready"
            } else {
                "Not Ready"
            }
            .to_string(),
            health_score: self.health_score(),
            components: Vec::new(),
            recommendations: Vec::new(),
            next_steps: Vec::new(),
        };

        // Python组件诊断
        let python_component = ComponentDiagnostic {
            name: "Python".to_string(),
            status: if self.python_available {
                "Available"
            } else {
                "Missing"
            }
            .to_string(),
            version: self.python_version.clone(),
            path: self.python_path.clone(),
            issues: self.get_component_issues("Python"),
            details: if self.python_available {
                format!(
                    "Python {} is available at {:?}",
                    self.python_version.as_deref().unwrap_or("unknown"),
                    self.python_path.as_deref().unwrap_or("unknown")
                )
            } else {
                "Python is not available or not properly configured".to_string()
            },
        };
        report.components.push(python_component);

        // 虚拟环境组件诊断
        let venv_status = self.get_virtual_env_status();
        let venv_component = ComponentDiagnostic {
            name: "Virtual Environment".to_string(),
            status: if venv_status.is_active {
                "Active"
            } else {
                "Inactive"
            }
            .to_string(),
            version: None,
            path: venv_status.path.clone(),
            issues: self.get_component_issues("Virtual Environment"),
            details: if venv_status.is_active {
                format!(
                    "Virtual environment is active at {:?}",
                    venv_status.path.as_deref().unwrap_or("unknown")
                )
            } else {
                format!(
                    "Virtual environment is not active. Expected at ./venv. Use: {}",
                    venv_status.activation_command
                )
            },
        };
        report.components.push(venv_component);

        // UV工具诊断
        let uv_component = ComponentDiagnostic {
            name: "UV Tool".to_string(),
            status: if self.uv_available {
                "Available"
            } else {
                "Missing"
            }
            .to_string(),
            version: self.uv_version.clone(),
            path: None,
            issues: self.get_component_issues("UV"),
            details: if self.uv_available {
                format!(
                    "UV {} is available",
                    self.uv_version.as_deref().unwrap_or("unknown")
                )
            } else {
                "UV tool is not installed. Install with: curl -LsSf https://astral.sh/uv/install.sh | sh".to_string()
            },
        };
        report.components.push(uv_component);

        // MinerU组件诊断
        let mineru_component = ComponentDiagnostic {
            name: "MinerU".to_string(),
            status: if self.mineru_available {
                "Available"
            } else {
                "Missing"
            }
            .to_string(),
            version: self.mineru_version.clone(),
            path: None,
            issues: self.get_component_issues("MinerU"),
            details: if self.mineru_available {
                format!(
                    "MinerU {} is available",
                    self.mineru_version.as_deref().unwrap_or("unknown")
                )
            } else {
                "MinerU is not installed. Install with: uv pip install magic-pdf[full]".to_string()
            },
        };
        report.components.push(mineru_component);

        // MarkItDown组件诊断
        let markitdown_component = ComponentDiagnostic {
            name: "MarkItDown".to_string(),
            status: if self.markitdown_available {
                "Available"
            } else {
                "Missing"
            }
            .to_string(),
            version: self.markitdown_version.clone(),
            path: None,
            issues: self.get_component_issues("MarkItDown"),
            details: if self.markitdown_available {
                format!(
                    "MarkItDown {} is available",
                    self.markitdown_version.as_deref().unwrap_or("unknown")
                )
            } else {
                "MarkItDown is not installed. Install with: uv pip install markitdown".to_string()
            },
        };
        report.components.push(markitdown_component);

        // CUDA组件诊断（可选）
        let cuda_component = ComponentDiagnostic {
            name: "CUDA".to_string(),
            status: if self.cuda_available {
                "Available"
            } else {
                "Not Available"
            }
            .to_string(),
            version: self.cuda_version.clone(),
            path: None,
            issues: self.get_component_issues("CUDA"),
            details: if self.cuda_available {
                format!(
                    "CUDA {} is available with {} device(s)",
                    self.cuda_version.as_deref().unwrap_or("unknown"),
                    self.cuda_devices.len()
                )
            } else {
                "CUDA is not available. GPU acceleration will not be used.".to_string()
            },
        };
        report.components.push(cuda_component);

        // 生成推荐和下一步操作
        self.generate_recommendations(&mut report);

        report
    }

    /// 获取特定组件的问题
    fn get_component_issues(&self, component_name: &str) -> Vec<String> {
        self.issues
            .iter()
            .filter(|issue| issue.component == component_name)
            .map(|issue| format!("{}: {}", issue.message, issue.suggestion))
            .collect()
    }

    /// 生成推荐和下一步操作
    fn generate_recommendations(&self, report: &mut DiagnosticReport) {
        // 基于当前状态生成推荐
        if !self.python_available {
            report
                .recommendations
                .push("Install Python 3.8+ to enable document parsing functionality".to_string());
            report
                .next_steps
                .push("1. Install Python 3.8 or higher".to_string());
        }

        if !self.virtual_env_active {
            report.recommendations.push(
                "Create and activate a virtual environment for isolated dependency management"
                    .to_string(),
            );
            report
                .next_steps
                .push("2. Run 'document-parser uv-init' to set up the environment".to_string());
        }

        if !self.uv_available {
            report
                .recommendations
                .push("Install UV tool for fast Python package management".to_string());
            if !report
                .next_steps
                .iter()
                .any(|step| step.contains("uv-init"))
            {
                report.next_steps.push(
                    "2. Run 'document-parser uv-init' to install UV and set up dependencies"
                        .to_string(),
                );
            }
        }

        if !self.mineru_available {
            report
                .recommendations
                .push("Install MinerU for PDF document parsing capabilities".to_string());
            if !report
                .next_steps
                .iter()
                .any(|step| step.contains("uv-init"))
            {
                report
                    .next_steps
                    .push("3. Install MinerU with: uv pip install magic-pdf[full]".to_string());
            }
        }

        if !self.markitdown_available {
            report
                .recommendations
                .push("Install MarkItDown for multi-format document parsing".to_string());
            if !report
                .next_steps
                .iter()
                .any(|step| step.contains("uv-init"))
            {
                report
                    .next_steps
                    .push("4. Install MarkItDown with: uv pip install markitdown".to_string());
            }
        }

        if self.is_ready() {
            report.recommendations.push(
                "Environment is ready! You can start the document parsing server".to_string(),
            );
            report
                .next_steps
                .push("Run 'document-parser server' to start the service".to_string());
        }

        // 添加CUDA相关推荐
        if !self.cuda_available && self.python_available {
            report.recommendations.push(
                "Consider installing CUDA for improved PDF processing performance".to_string(),
            );
        }

        // 添加虚拟环境配置推荐
        if self.virtual_env_active && !self.is_virtual_env_properly_configured() {
            report.recommendations.push(
                "Virtual environment detected but may not be in the expected location (./venv)"
                    .to_string(),
            );
        }
    }

    /// 格式化诊断报告为可读字符串
    pub fn format_diagnostic_report(&self) -> String {
        let report = self.generate_diagnostic_report();
        let mut output = String::new();

        output.push_str("=== Environment Diagnostic Report ===\n");
        output.push_str(&format!("Overall Status: {}\n", report.overall_status));
        output.push_str(&format!("Health Score: {}/100\n\n", report.health_score));

        output.push_str("=== Components ===\n");
        for component in &report.components {
            output.push_str(&format!("• {}: {} ", component.name, component.status));
            if let Some(ref version) = component.version {
                output.push_str(&format!("({version})"));
            }
            output.push('\n');

            if let Some(ref path) = component.path {
                output.push_str(&format!("  Path: {path}\n"));
            }

            output.push_str(&format!("  Details: {}\n", component.details));

            if !component.issues.is_empty() {
                output.push_str("  Issues:\n");
                for issue in &component.issues {
                    output.push_str(&format!("    - {issue}\n"));
                }
            }
            output.push('\n');
        }

        if !report.recommendations.is_empty() {
            output.push_str("=== Recommendations ===\n");
            for (i, recommendation) in report.recommendations.iter().enumerate() {
                output.push_str(&format!("{}. {}\n", i + 1, recommendation));
            }
            output.push('\n');
        }

        if !report.next_steps.is_empty() {
            output.push_str("=== Next Steps ===\n");
            for step in &report.next_steps {
                output.push_str(&format!("{step}\n"));
            }
        }

        output
    }

    /// 生成环境健康评分 (0-100)
    pub fn health_score(&self) -> u8 {
        let mut score = 0u8;

        // 基础组件检查 (60分)
        if self.python_available {
            score += 20;
        }
        if self.mineru_available {
            score += 20;
        }
        if self.markitdown_available {
            score += 20;
        }

        // 工具支持 (20分)
        if self.uv_available {
            score += 10;
        }
        if self.virtual_env_active {
            score += 10;
        }

        // CUDA支持 (10分)
        if self.has_cuda_support() {
            score += 10;
        }

        // 扣除问题分数 (最多扣30分)
        let issue_penalty = self
            .issues
            .iter()
            .map(|issue| match issue.severity {
                IssueSeverity::Critical => 10,
                IssueSeverity::High => 5,
                IssueSeverity::Medium => 2,
                IssueSeverity::Low => 1,
            })
            .sum::<u8>()
            .min(30);

        score.saturating_sub(issue_penalty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_environment_status_health_score() {
        let mut status = EnvironmentStatus::default();

        // 初始状态应该得分很低
        assert_eq!(status.health_score(), 0);

        // 添加基础组件
        status.python_available = true;
        status.mineru_available = true;
        status.markitdown_available = true;
        assert_eq!(status.health_score(), 60);

        // 添加工具支持
        status.uv_available = true;
        status.virtual_env_active = true;
        assert_eq!(status.health_score(), 80);

        // 添加CUDA支持
        status.cuda_available = true;
        status.cuda_devices.push(CudaDevice {
            id: 0,
            name: "Test GPU".to_string(),
            memory_total: 8 * 1024 * 1024 * 1024,
            memory_free: 4 * 1024 * 1024 * 1024,
            compute_capability: "8.6".to_string(),
        });
        assert_eq!(status.health_score(), 90);
    }

    #[tokio::test]
    async fn test_virtual_environment_activation_commands() {
        let status = EnvironmentStatus::default();

        // 测试基本激活命令
        let activation_cmd = status.get_activation_command();
        if cfg!(windows) {
            assert_eq!(activation_cmd, ".\\venv\\Scripts\\activate.bat");
        } else {
            assert_eq!(activation_cmd, "source ./venv/bin/activate");
        }

        // 测试PowerShell激活命令（仅Windows）
        let powershell_cmd = status.get_powershell_activation_command();
        if cfg!(windows) {
            assert_eq!(
                powershell_cmd,
                Some(".\\venv\\Scripts\\Activate.ps1".to_string())
            );
        } else {
            assert_eq!(powershell_cmd, None);
        }
    }
}
