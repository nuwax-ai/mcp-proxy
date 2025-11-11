use crate::error::AppError;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::{sleep, timeout};
use tracing::{debug, error, info, instrument, warn};

/// 环境管理器
#[derive(Debug, Clone)]
pub struct EnvironmentManager {
    python_path: String,
    base_dir: String,
    progress_sender: Option<Arc<Mutex<mpsc::UnboundedSender<InstallProgress>>>>,
    timeout_duration: Duration,
    retry_config: RetryConfig,
    environment_cache: Arc<RwLock<Option<EnvironmentStatus>>>,
    cache_ttl: Duration,
}

/// 重试配置
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub backoff_multiplier: f64,
}

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

/// 环境问题详情
#[derive(Debug, Clone)]
pub struct EnvironmentIssue {
    pub component: String,
    pub severity: IssueSeverity,
    pub message: String,
    pub suggestion: String,
    pub auto_fixable: bool,
}

/// 环境警告详情
#[derive(Debug, Clone)]
pub struct EnvironmentWarning {
    pub component: String,
    pub message: String,
    pub impact: String,
}

/// 问题严重程度
#[derive(Debug, Clone, PartialEq)]
pub enum IssueSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// CUDA设备信息
#[derive(Debug, Clone)]
pub struct CudaDevice {
    pub id: u32,
    pub name: String,
    pub memory_total: u64,
    pub memory_free: u64,
    pub compute_capability: String,
}

/// 依赖安装进度
#[derive(Debug, Clone)]
pub struct InstallProgress {
    pub package: String,
    pub stage: InstallStage,
    pub progress: f32,
    pub message: String,
    pub estimated_time_remaining: Option<Duration>,
    pub bytes_downloaded: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// 安装阶段
#[derive(Debug, Clone)]
pub enum InstallStage {
    Preparing,
    Downloading,
    Installing,
    Configuring,
    Verifying,
    Completed,
    Failed(String),
    Retrying { attempt: u32, max_attempts: u32 },
}

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

/// CUDA环境信息
#[derive(Debug)]
pub struct CudaInfo {
    pub available: bool,
    pub version: Option<String>,
    pub devices: Vec<CudaDevice>,
}

/// Python包信息
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub version: String,
}

/// 包版本兼容性信息
#[derive(Debug, Clone)]
pub struct PackageCompatibility {
    pub package_name: String,
    pub current_version: String,
    pub minimum_version: String,
    pub recommended_version: Option<String>,
    pub is_compatible: bool,
    pub compatibility_issues: Vec<String>,
    pub upgrade_available: bool,
    pub upgrade_recommendation: Option<String>,
}

/// 依赖验证结果
#[derive(Debug, Clone)]
pub struct DependencyVerificationResult {
    pub mineru_status: DependencyStatus,
    pub markitdown_status: DependencyStatus,
    pub overall_compatible: bool,
    pub recommendations: Vec<String>,
    pub critical_issues: Vec<String>,
}

/// 依赖状态
#[derive(Debug, Clone)]
pub struct DependencyStatus {
    pub package_name: String,
    pub is_available: bool,
    pub is_functional: bool,
    pub version_info: Option<PackageInfo>,
    pub compatibility: Option<PackageCompatibility>,
    pub issues: Vec<String>,
    pub path: Option<String>,
}

/// 虚拟环境状态详细信息
#[derive(Debug, Clone)]
pub struct VirtualEnvStatus {
    pub is_active: bool,
    pub path: Option<String>,
    pub expected_path: Option<String>,
    pub python_executable: Option<String>,
    pub is_properly_configured: bool,
    pub activation_command: String,
}

/// 虚拟环境详细信息（跨平台）
#[derive(Debug, Clone)]
pub struct VirtualEnvInfo {
    pub path: std::path::PathBuf,
    pub python_executable: std::path::PathBuf,
    pub pip_executable: std::path::PathBuf,
    pub activation_script: std::path::PathBuf,
    pub is_valid: bool,
    pub platform: String,
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

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
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

impl EnvironmentManager {
    /// 创建新的环境管理器
    pub fn new(python_path: String, base_dir: String) -> Self {
        Self {
            python_path,
            base_dir,
            progress_sender: None,
            timeout_duration: Duration::from_secs(300), // 5分钟默认超时
            retry_config: RetryConfig::default(),
            environment_cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::from_secs(300), // 5分钟缓存
        }
    }

    /// 为当前目录创建环境管理器（推荐使用）
    pub fn for_current_directory() -> Result<Self, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;

        let python_path = Self::get_venv_python_path(&current_dir.join("venv"));

        Ok(Self {
            python_path: python_path.to_string_lossy().to_string(),
            base_dir: current_dir.to_string_lossy().to_string(),
            progress_sender: None,
            timeout_duration: Duration::from_secs(300), // 5分钟默认超时
            retry_config: RetryConfig::default(),
            environment_cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::from_secs(300), // 5分钟缓存
        })
    }

    /// 获取虚拟环境中的Python可执行文件路径（跨平台）
    pub fn get_venv_python_path(venv_path: &Path) -> std::path::PathBuf {
        if cfg!(windows) {
            // Windows: Scripts/python.exe
            venv_path.join("Scripts").join("python.exe")
        } else {
            // Unix-like: bin/python
            venv_path.join("bin").join("python")
        }
    }

    /// 获取虚拟环境中的可执行文件路径（跨平台）
    pub fn get_venv_executable_path(venv_path: &Path, executable_name: &str) -> std::path::PathBuf {
        if cfg!(windows) {
            // Windows: Scripts/{executable}.exe
            let exe_name = if executable_name.ends_with(".exe") {
                executable_name.to_string()
            } else {
                format!("{executable_name}.exe")
            };
            venv_path.join("Scripts").join(exe_name)
        } else {
            // Unix-like: bin/{executable}
            venv_path.join("bin").join(executable_name)
        }
    }

    /// 获取虚拟环境激活脚本路径（跨平台）
    pub fn get_venv_activation_script(venv_path: &Path) -> std::path::PathBuf {
        if cfg!(windows) {
            // Windows: Scripts/activate.bat or Scripts/Activate.ps1
            venv_path.join("Scripts").join("activate.bat")
        } else {
            // Unix-like: bin/activate
            venv_path.join("bin").join("activate")
        }
    }

    /// 获取系统Python可执行文件名（跨平台）
    pub fn get_system_python_executable() -> Vec<String> {
        if cfg!(windows) {
            // Windows: python.exe, python3.exe, py.exe
            vec![
                "python.exe".to_string(),
                "python3.exe".to_string(),
                "py.exe".to_string(),
            ]
        } else {
            // Unix-like: python3, python
            vec!["python3".to_string(), "python".to_string()]
        }
    }

    /// 检查可执行文件是否存在于PATH中（跨平台）
    pub async fn is_executable_in_path(executable: &str) -> bool {
        let which_cmd = if cfg!(windows) { "where" } else { "which" };

        match Command::new(which_cmd).arg(executable).output().await {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// 查找系统中可用的Python可执行文件（跨平台）
    async fn find_system_python(&self) -> Option<String> {
        let python_candidates = Self::get_system_python_executable();

        for candidate in python_candidates {
            if Self::is_executable_in_path(&candidate).await {
                debug!("找到系统Python: {}", candidate);
                return Some(candidate);
            }
        }

        debug!("未找到系统Python可执行文件");
        None
    }

    /// 测试虚拟环境激活（跨平台）
    pub async fn test_virtual_environment_activation(
        &self,
        venv_path: &Path,
    ) -> Result<bool, AppError> {
        let python_exe = Self::get_venv_python_path(venv_path);

        if !python_exe.exists() {
            return Ok(false);
        }

        // 测试Python可执行文件是否工作
        let test_cmd = Command::new(&python_exe)
            .arg("-c")
            .arg("import sys; print('VENV_TEST_SUCCESS'); print(sys.prefix)")
            .output();

        match timeout(Duration::from_secs(10), test_cmd).await {
            Ok(Ok(output)) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("VENV_TEST_SUCCESS") {
                    debug!("虚拟环境激活测试成功: {}", python_exe.display());
                    Ok(true)
                } else {
                    debug!("虚拟环境激活测试失败: 输出不正确");
                    Ok(false)
                }
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                debug!("虚拟环境激活测试失败: {}", stderr);
                Ok(false)
            }
            Ok(Err(e)) => {
                debug!("虚拟环境激活测试执行失败: {}", e);
                Ok(false)
            }
            Err(_) => {
                debug!("虚拟环境激活测试超时");
                Ok(false)
            }
        }
    }

    /// 获取虚拟环境信息（跨平台）
    pub async fn get_virtual_environment_info(
        &self,
        venv_path: &Path,
    ) -> Result<VirtualEnvInfo, AppError> {
        let python_exe = Self::get_venv_python_path(venv_path);
        let activation_script = Self::get_venv_activation_script(venv_path);
        let pip_exe = Self::get_venv_executable_path(venv_path, "pip");

        let is_valid = self.test_virtual_environment_activation(venv_path).await?;

        Ok(VirtualEnvInfo {
            path: venv_path.to_path_buf(),
            python_executable: python_exe,
            pip_executable: pip_exe,
            activation_script,
            is_valid,
            platform: if cfg!(windows) {
                "windows".to_string()
            } else {
                "unix".to_string()
            },
        })
    }

    /// 获取跨平台环境变量设置
    pub fn get_cross_platform_env_vars(
        &self,
        venv_path: &Path,
    ) -> std::collections::HashMap<String, String> {
        let mut env_vars = std::collections::HashMap::new();

        if cfg!(windows) {
            // Windows环境变量
            env_vars.insert(
                "VIRTUAL_ENV".to_string(),
                venv_path.to_string_lossy().to_string(),
            );
            env_vars.insert(
                "PATH".to_string(),
                format!(
                    "{};{}",
                    venv_path.join("Scripts").to_string_lossy(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        } else {
            // Unix-like环境变量
            env_vars.insert(
                "VIRTUAL_ENV".to_string(),
                venv_path.to_string_lossy().to_string(),
            );
            env_vars.insert(
                "PATH".to_string(),
                format!(
                    "{}:{}",
                    venv_path.join("bin").to_string_lossy(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        }

        env_vars
    }

    /// 创建带进度跟踪的环境管理器
    pub fn with_progress_tracking(
        python_path: String,
        base_dir: String,
        progress_sender: mpsc::UnboundedSender<InstallProgress>,
    ) -> Self {
        Self {
            python_path,
            base_dir,
            progress_sender: Some(Arc::new(Mutex::new(progress_sender))),
            timeout_duration: Duration::from_secs(300),
            retry_config: RetryConfig::default(),
            environment_cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::from_secs(300),
        }
    }

    /// 为当前目录创建带进度跟踪的环境管理器
    pub fn for_current_directory_with_progress(
        progress_sender: mpsc::UnboundedSender<InstallProgress>,
    ) -> Result<Self, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;

        let python_path = Self::get_venv_python_path(&current_dir.join("venv"));

        Ok(Self {
            python_path: python_path.to_string_lossy().to_string(),
            base_dir: current_dir.to_string_lossy().to_string(),
            progress_sender: Some(Arc::new(Mutex::new(progress_sender))),
            timeout_duration: Duration::from_secs(300),
            retry_config: RetryConfig::default(),
            environment_cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::from_secs(300),
        })
    }

    /// 添加进度发送器到现有环境管理器
    pub fn with_progress_sender(
        mut self,
        progress_sender: mpsc::UnboundedSender<InstallProgress>,
    ) -> Self {
        self.progress_sender = Some(Arc::new(Mutex::new(progress_sender)));
        self
    }

    /// 设置操作超时时间
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_duration = timeout;
        self
    }

    /// 设置重试配置
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    /// 设置缓存TTL
    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    /// 检查完整环境状态（带缓存支持）
    #[instrument(skip(self))]
    pub async fn check_environment(&self) -> Result<EnvironmentStatus, AppError> {
        // 检查缓存
        if let Some(cached_status) = self.get_cached_status().await {
            if !cached_status.is_cache_expired(self.cache_ttl) {
                debug!("使用缓存的环境状态");
                return Ok(cached_status);
            }
        }

        let start_time = std::time::SystemTime::now();
        let mut status = EnvironmentStatus::default();

        info!("开始环境检查");

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
                info!("Python环境检查通过: {:?}", status.python_version);

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
                error!("Python环境检查失败: {}", e);
            }
        }

        // 处理uv工具检查结果
        match uv_result {
            Ok(uv_info) => {
                status.uv_available = true;
                status.uv_version = Some(uv_info.version);
                info!("uv工具检查通过: {:?}", status.uv_version);
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
                warn!("uv工具检查失败: {}", e);
            }
        }

        // 处理CUDA环境检查结果
        match cuda_result {
            Ok(cuda_info) => {
                status.cuda_available = cuda_info.available;
                status.cuda_version = cuda_info.version;
                status.cuda_devices = cuda_info.devices;
                if status.cuda_available {
                    info!("CUDA环境检查通过: {:?}", status.cuda_version);
                } else {
                    let warning = EnvironmentWarning {
                        component: "CUDA".to_string(),
                        message: "CUDA环境不可用".to_string(),
                        impact: "PDF处理性能可能较慢".to_string(),
                    };
                    status.warnings.push(warning);
                    info!("CUDA环境不可用");
                }
            }
            Err(e) => {
                let warning = EnvironmentWarning {
                    component: "CUDA".to_string(),
                    message: format!("CUDA环境检查失败: {e}"),
                    impact: "将使用CPU进行PDF处理".to_string(),
                };
                status.warnings.push(warning);
                warn!("CUDA环境检查失败: {}", e);
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
                    info!("MinerU环境检查通过: {:?}", status.mineru_version);
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
                    warn!("MinerU环境检查失败: {}", e);
                }
            }

            match markitdown_result {
                Ok(markitdown_info) => {
                    status.markitdown_available = true;
                    status.markitdown_version = Some(markitdown_info.version);
                    info!("MarkItDown环境检查通过: {:?}", status.markitdown_version);
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
                    warn!("MarkItDown环境检查失败: {}", e);
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
            "环境检查完成，状态: ready={}, 健康评分: {}/100, 耗时: {:?}",
            status.is_ready(),
            status.health_score(),
            status.check_duration
        );
        Ok(status)
    }

    /// 获取详细的环境状态报告
    pub async fn get_detailed_status_report(&self) -> Result<String, AppError> {
        let status = self.check_environment().await?;
        Ok(status.format_diagnostic_report())
    }

    /// 获取增强的依赖验证报告
    pub async fn get_enhanced_dependency_report(&self) -> Result<String, AppError> {
        let verification_result = self.verify_dependency_compatibility().await?;

        let mut report = String::new();
        report.push_str("=== 增强依赖验证报告 ===\n\n");

        // 总体状态
        report.push_str(&format!(
            "总体兼容性: {}\n",
            if verification_result.overall_compatible {
                "✓ 兼容"
            } else {
                "✗ 不兼容"
            }
        ));
        report.push('\n');

        // MinerU状态
        report.push_str("=== MinerU 状态 ===\n");
        let mineru = &verification_result.mineru_status;
        report.push_str(&format!(
            "可用性: {}\n",
            if mineru.is_available {
                "✓ 可用"
            } else {
                "✗ 不可用"
            }
        ));
        report.push_str(&format!(
            "功能性: {}\n",
            if mineru.is_functional {
                "✓ 正常"
            } else {
                "✗ 异常"
            }
        ));

        if let Some(ref version_info) = mineru.version_info {
            report.push_str(&format!("版本: {}\n", version_info.version));
        }

        if let Some(ref path) = mineru.path {
            report.push_str(&format!("路径: {path}\n"));
        }

        if let Some(ref compat) = mineru.compatibility {
            report.push_str(&format!(
                "版本兼容性: {}\n",
                if compat.is_compatible {
                    "✓ 兼容"
                } else {
                    "✗ 不兼容"
                }
            ));
            report.push_str(&format!("当前版本: {}\n", compat.current_version));
            report.push_str(&format!("最低要求: {}\n", compat.minimum_version));

            if !compat.compatibility_issues.is_empty() {
                report.push_str("兼容性问题:\n");
                for issue in &compat.compatibility_issues {
                    report.push_str(&format!("  - {issue}\n"));
                }
            }

            if compat.upgrade_available {
                if let Some(ref rec) = compat.upgrade_recommendation {
                    report.push_str(&format!("升级建议: {rec}\n"));
                }
            }
        }

        if !mineru.issues.is_empty() {
            report.push_str("问题:\n");
            for issue in &mineru.issues {
                report.push_str(&format!("  - {issue}\n"));
            }
        }
        report.push('\n');

        // MarkItDown状态
        report.push_str("=== MarkItDown 状态 ===\n");
        let markitdown = &verification_result.markitdown_status;
        report.push_str(&format!(
            "可用性: {}\n",
            if markitdown.is_available {
                "✓ 可用"
            } else {
                "✗ 不可用"
            }
        ));
        report.push_str(&format!(
            "功能性: {}\n",
            if markitdown.is_functional {
                "✓ 正常"
            } else {
                "✗ 异常"
            }
        ));

        if let Some(ref version_info) = markitdown.version_info {
            report.push_str(&format!("版本: {}\n", version_info.version));
        }

        if let Some(ref path) = markitdown.path {
            report.push_str(&format!("路径: {path}\n"));
        }

        if let Some(ref compat) = markitdown.compatibility {
            report.push_str(&format!(
                "版本兼容性: {}\n",
                if compat.is_compatible {
                    "✓ 兼容"
                } else {
                    "✗ 不兼容"
                }
            ));
            report.push_str(&format!("当前版本: {}\n", compat.current_version));
            report.push_str(&format!("最低要求: {}\n", compat.minimum_version));

            if !compat.compatibility_issues.is_empty() {
                report.push_str("兼容性问题:\n");
                for issue in &compat.compatibility_issues {
                    report.push_str(&format!("  - {issue}\n"));
                }
            }

            if compat.upgrade_available {
                if let Some(ref rec) = compat.upgrade_recommendation {
                    report.push_str(&format!("升级建议: {rec}\n"));
                }
            }
        }

        if !markitdown.issues.is_empty() {
            report.push_str("问题:\n");
            for issue in &markitdown.issues {
                report.push_str(&format!("  - {issue}\n"));
            }
        }
        report.push('\n');

        // 关键问题
        if !verification_result.critical_issues.is_empty() {
            report.push_str("=== 关键问题 ===\n");
            for issue in &verification_result.critical_issues {
                report.push_str(&format!("⚠️  {issue}\n"));
            }
            report.push('\n');
        }

        // 推荐操作
        if !verification_result.recommendations.is_empty() {
            report.push_str("=== 推荐操作 ===\n");
            for (i, rec) in verification_result.recommendations.iter().enumerate() {
                report.push_str(&format!("{}. {}\n", i + 1, rec));
            }
        }

        Ok(report)
    }

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
                    self.get_activation_command()
                ),
                auto_fixable: true,
            };
            status.issues.push(issue);
        } else {
            // 检查虚拟环境路径是否符合预期
            if let Some(ref venv_path) = python_info.virtual_env_path {
                let expected_venv_path = std::env::current_dir().map(|dir| dir.join("venv")).ok();

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
            let expected_python_path = std::env::current_dir()
                .map(|dir| Self::get_venv_python_path(&dir.join("venv")))
                .ok();

            if let Some(expected_path) = expected_python_path {
                if !expected_path.exists() {
                    let issue = EnvironmentIssue {
                        component: "Virtual Environment".to_string(),
                        severity: IssueSeverity::Medium,
                        message: format!(
                            "预期的Python可执行文件不存在: {}",
                            expected_path.display()
                        ),
                        suggestion: "重新创建虚拟环境: 运行 'document-parser uv-init'".to_string(),
                        auto_fixable: true,
                    };
                    status.issues.push(issue);
                }
            }
        }
    }

    /// 获取虚拟环境激活命令
    fn get_activation_command(&self) -> String {
        if cfg!(windows) {
            ".\\venv\\Scripts\\activate.bat".to_string()
        } else {
            "source ./venv/bin/activate".to_string()
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

    /// 验证依赖版本兼容性
    pub async fn verify_dependency_compatibility(
        &self,
    ) -> Result<DependencyVerificationResult, AppError> {
        debug!("开始依赖版本兼容性验证");

        let (mineru_result, markitdown_result) = tokio::join!(
            self.verify_mineru_dependency(),
            self.verify_markitdown_dependency()
        );

        let mineru_status = mineru_result.unwrap_or_else(|e| DependencyStatus {
            package_name: "MinerU".to_string(),
            is_available: false,
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: vec![e.to_string()],
            path: None,
        });

        let markitdown_status = markitdown_result.unwrap_or_else(|e| DependencyStatus {
            package_name: "MarkItDown".to_string(),
            is_available: false,
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: vec![e.to_string()],
            path: None,
        });

        let overall_compatible = mineru_status.is_available
            && mineru_status.is_functional
            && markitdown_status.is_available
            && markitdown_status.is_functional
            && mineru_status
                .compatibility
                .as_ref()
                .is_none_or(|c| c.is_compatible)
            && markitdown_status
                .compatibility
                .as_ref()
                .is_none_or(|c| c.is_compatible);

        let mut recommendations = Vec::new();
        let mut critical_issues = Vec::new();

        // 收集推荐和关键问题
        if let Some(ref compat) = mineru_status.compatibility {
            if !compat.is_compatible {
                critical_issues.push(format!(
                    "MinerU版本不兼容: {} (最低要求: {})",
                    compat.current_version, compat.minimum_version
                ));
            }
            if compat.upgrade_available {
                if let Some(ref rec) = compat.upgrade_recommendation {
                    recommendations.push(rec.clone());
                }
            }
        }

        if let Some(ref compat) = markitdown_status.compatibility {
            if !compat.is_compatible {
                critical_issues.push(format!(
                    "MarkItDown版本不兼容: {} (最低要求: {})",
                    compat.current_version, compat.minimum_version
                ));
            }
            if compat.upgrade_available {
                if let Some(ref rec) = compat.upgrade_recommendation {
                    recommendations.push(rec.clone());
                }
            }
        }

        // 添加通用推荐
        if !mineru_status.is_available {
            recommendations.push("安装MinerU: uv pip install -U \"mineru[core]\"".to_string());
        }
        if !markitdown_status.is_available {
            recommendations.push("安装MarkItDown: uv pip install markitdown".to_string());
        }

        Ok(DependencyVerificationResult {
            mineru_status,
            markitdown_status,
            overall_compatible,
            recommendations,
            critical_issues,
        })
    }

    /// 验证MinerU依赖
    async fn verify_mineru_dependency(&self) -> Result<DependencyStatus, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;
        let venv_path = current_dir.join("venv");
        let mineru_path = Self::get_venv_executable_path(&venv_path, "mineru");

        let mut status = DependencyStatus {
            package_name: "MinerU".to_string(),
            is_available: mineru_path.exists(),
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: Vec::new(),
            path: Some(mineru_path.to_string_lossy().to_string()),
        };

        if !status.is_available {
            status.issues.push("MinerU命令不存在".to_string());
            return Ok(status);
        }

        // 检查功能性
        match self.check_mineru_environment().await {
            Ok(package_info) => {
                status.is_functional = true;
                status.version_info = Some(package_info.clone());

                // 验证版本兼容性
                status.compatibility = Some(
                    self.check_mineru_version_compatibility(&package_info.version)
                        .await,
                );
            }
            Err(e) => {
                status.issues.push(format!("MinerU功能检查失败: {e}"));
            }
        }

        Ok(status)
    }

    /// 验证MarkItDown依赖
    async fn verify_markitdown_dependency(&self) -> Result<DependencyStatus, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;
        let venv_path = current_dir.join("venv");
        let python_path = Self::get_venv_python_path(&venv_path);

        let mut status = DependencyStatus {
            package_name: "MarkItDown".to_string(),
            is_available: false,
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: Vec::new(),
            path: Some(python_path.to_string_lossy().to_string()),
        };

        // 检查功能性
        match self.check_markitdown_environment().await {
            Ok(package_info) => {
                status.is_available = true;
                status.is_functional = true;
                status.version_info = Some(package_info.clone());

                // 验证版本兼容性
                status.compatibility = Some(
                    self.check_markitdown_version_compatibility(&package_info.version)
                        .await,
                );
            }
            Err(e) => {
                status.issues.push(format!("MarkItDown检查失败: {e}"));
            }
        }

        Ok(status)
    }

    /// 检查MinerU版本兼容性
    async fn check_mineru_version_compatibility(
        &self,
        current_version: &str,
    ) -> PackageCompatibility {
        let minimum_version = "0.1.0"; // MinerU最低版本要求
        let recommended_version = "latest"; // 推荐版本

        let is_compatible = self.is_version_compatible(current_version, minimum_version);
        let upgrade_available = current_version != "latest" && current_version != "available";

        let mut compatibility_issues = Vec::new();
        let mut upgrade_recommendation = None;

        if !is_compatible {
            compatibility_issues.push(format!(
                "当前版本 {current_version} 低于最低要求版本 {minimum_version}"
            ));
        }

        if upgrade_available {
            upgrade_recommendation =
                Some("升级MinerU到最新版本: uv pip install -U \"mineru[core]\"".to_string());
        }

        // 检查特定版本的已知问题
        if current_version.contains("0.0.") {
            compatibility_issues.push("检测到早期版本，可能存在稳定性问题".to_string());
        }

        PackageCompatibility {
            package_name: "MinerU".to_string(),
            current_version: current_version.to_string(),
            minimum_version: minimum_version.to_string(),
            recommended_version: Some(recommended_version.to_string()),
            is_compatible,
            compatibility_issues,
            upgrade_available,
            upgrade_recommendation,
        }
    }

    /// 检查MarkItDown版本兼容性
    async fn check_markitdown_version_compatibility(
        &self,
        current_version: &str,
    ) -> PackageCompatibility {
        let minimum_version = "0.0.1"; // MarkItDown最低版本要求
        let recommended_version = "latest"; // 推荐版本

        let is_compatible = self.is_version_compatible(current_version, minimum_version);
        let upgrade_available = current_version != "latest" && current_version != "available";

        let mut compatibility_issues = Vec::new();
        let mut upgrade_recommendation = None;

        if !is_compatible {
            compatibility_issues.push(format!(
                "当前版本 {current_version} 低于最低要求版本 {minimum_version}"
            ));
        }

        if upgrade_available {
            upgrade_recommendation =
                Some("升级MarkItDown到最新版本: uv pip install -U markitdown".to_string());
        }

        PackageCompatibility {
            package_name: "MarkItDown".to_string(),
            current_version: current_version.to_string(),
            minimum_version: minimum_version.to_string(),
            recommended_version: Some(recommended_version.to_string()),
            is_compatible,
            compatibility_issues,
            upgrade_available,
            upgrade_recommendation,
        }
    }

    /// 比较版本号兼容性（简单的语义版本比较）
    fn is_version_compatible(&self, current: &str, minimum: &str) -> bool {
        // 处理特殊版本字符串
        if current == "available" || current == "latest" || current == "unknown" {
            return true; // 假设可用
        }

        // 简单的版本比较逻辑
        match (self.parse_version(current), self.parse_version(minimum)) {
            (Some(current_parts), Some(min_parts)) => {
                for i in 0..3 {
                    let current_part = current_parts.get(i).unwrap_or(&0);
                    let min_part = min_parts.get(i).unwrap_or(&0);

                    if current_part > min_part {
                        return true;
                    } else if current_part < min_part {
                        return false;
                    }
                }
                true // 版本相等
            }
            _ => true, // 无法解析版本时假设兼容
        }
    }

    /// 解析版本号为数字数组
    fn parse_version(&self, version: &str) -> Option<Vec<u32>> {
        // 提取版本号中的数字部分
        let version_clean = version
            .split_whitespace()
            .next()?
            .trim_start_matches('v')
            .split('-')
            .next()?;

        let parts: Result<Vec<u32>, _> = version_clean
            .split('.')
            .take(3)
            .map(|s| s.parse::<u32>())
            .collect();

        parts.ok()
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
                        info!("{} 在第{}次尝试后成功", operation_name, attempt);
                    }
                    return Ok(result);
                }
                Err(e) => {
                    last_error = Some(e);

                    if attempt < self.retry_config.max_attempts {
                        warn!(
                            "{} 第{}次尝试失败，{}秒后重试",
                            operation_name,
                            attempt,
                            delay.as_secs_f32()
                        );

                        // 发送重试进度
                        if let Some(sender) = &self.progress_sender {
                            let progress = InstallProgress {
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
                            };

                            if let Ok(sender) = sender.try_lock() {
                                let _ = sender.send(progress);
                            }
                        }

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
            "{} 在{}次尝试后仍然失败",
            operation_name, self.retry_config.max_attempts
        );
        Err(last_error.unwrap_or_else(|| AppError::Environment(format!("{operation_name} 失败"))))
    }

    /// 检查Python环境
    #[instrument(skip(self))]
    async fn check_python_environment(&self) -> Result<PythonInfo, AppError> {
        debug!("检查Python环境: {}", self.python_path);

        // 首先检查配置的Python路径是否存在
        let python_executable = if Path::new(&self.python_path).exists() {
            self.python_path.clone()
        } else {
            // 如果虚拟环境Python不存在，尝试使用系统Python
            debug!("虚拟环境Python路径不存在，尝试查找系统Python");
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
        if let Some(version_num) = self.extract_python_version(&version) {
            if version_num < (3, 8) {
                return Err(AppError::Environment(format!(
                    "Python版本过低: {version}，需要3.8或更高版本"
                )));
            }
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

        debug!("Python环境检查通过: {}", version);
        if virtual_env_active {
            debug!("检测到虚拟环境: {:?}", virtual_env_path);
        }

        Ok(PythonInfo {
            version: Some(version),
            path: python_executable,
            virtual_env_active,
            virtual_env_path,
        })
    }

    /// 提取Python版本号
    fn extract_python_version(&self, version_str: &str) -> Option<(u32, u32)> {
        // 解析类似 "Python 3.9.7" 的版本字符串
        let parts: Vec<&str> = version_str.split_whitespace().collect();
        if parts.len() >= 2 {
            let version_part = parts[1];
            let version_nums: Vec<&str> = version_part.split('.').collect();
            if version_nums.len() >= 2 {
                if let (Ok(major), Ok(minor)) = (
                    version_nums[0].parse::<u32>(),
                    version_nums[1].parse::<u32>(),
                ) {
                    return Some((major, minor));
                }
            }
        }
        None
    }

    /// 检查uv工具
    async fn check_uv_environment(&self) -> Result<UvInfo, AppError> {
        debug!("检查uv工具");

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

        debug!("uv工具检查通过: {}", version);

        Ok(UvInfo { version })
    }

    /// 检查CUDA环境
    pub async fn check_cuda_environment(&self) -> Result<CudaInfo, AppError> {
        debug!("检查CUDA环境");

        let nvidia_cmd = Command::new("nvidia-smi")
            .arg("--query-gpu=index,name,memory.total,memory.free,compute_cap")
            .arg("--format=csv,noheader,nounits")
            .output();

        let output = match timeout(Duration::from_secs(10), nvidia_cmd).await {
            Ok(Ok(output)) if output.status.success() => output,
            Ok(Ok(_)) => {
                debug!("nvidia-smi执行失败，CUDA不可用");
                return Ok(CudaInfo {
                    available: false,
                    version: None,
                    devices: Vec::new(),
                });
            }
            Ok(Err(_)) | Err(_) => {
                debug!("CUDA环境不可用");
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
            "CUDA环境检查完成: available={}, devices={}",
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
        if parts.len() >= 5 {
            if let (Ok(id), Ok(memory_total), Ok(memory_free)) = (
                parts[0].parse::<u32>(),
                parts[2].parse::<u64>(),
                parts[3].parse::<u64>(),
            ) {
                return Some(CudaDevice {
                    id,
                    name: parts[1].to_string(),
                    memory_total: memory_total * 1024 * 1024, // 转换为字节
                    memory_free: memory_free * 1024 * 1024,   // 转换为字节
                    compute_capability: parts[4].to_string(),
                });
            }
        }
        None
    }

    /// 获取CUDA版本
    async fn get_cuda_version(&self) -> Option<String> {
        let version_cmd = Command::new("nvidia-smi")
            .arg("--query-gpu=driver_version")
            .arg("--format=csv,noheader,nounits")
            .output();

        if let Ok(Ok(output)) = timeout(Duration::from_secs(5), version_cmd).await {
            if output.status.success() {
                let version_str = String::from_utf8_lossy(&output.stdout);
                return Some(version_str.trim().to_string());
            }
        }
        None
    }

    /// 检查MinerU环境
    async fn check_mineru_environment(&self) -> Result<PackageInfo, AppError> {
        debug!("检查MinerU环境");

        // 使用当前目录的虚拟环境中的mineru命令路径
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;
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
                warn!("MinerU版本获取失败，但命令可用: {}", stderr);
                "available".to_string()
            }
            Err(e) => {
                return Err(AppError::Environment(format!(
                    "MinerU版本检查执行失败: {e}. 请检查MinerU安装"
                )));
            }
        };

        // MinerU命令验证已通过，无需额外的模块导入测试

        debug!("MinerU环境检查通过，版本: {}", version);

        Ok(PackageInfo { version })
    }

    /// 检查MarkItDown环境
    async fn check_markitdown_environment(&self) -> Result<PackageInfo, AppError> {
        debug!("检查MarkItDown环境");

        // 优先使用虚拟环境中的Python
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;
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
            warn!("MarkItDown版本获取失败，但模块可用");
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
                debug!("MarkItDown功能验证成功");
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("MarkItDown功能验证失败: {}", stderr);
                return Err(AppError::Environment(format!(
                    "MarkItDown功能验证失败: {stderr}. 请重新安装MarkItDown"
                )));
            }
            Ok(Err(e)) => {
                warn!("MarkItDown功能测试执行失败: {}", e);
            }
            Err(_) => {
                warn!("MarkItDown功能测试超时");
            }
        }

        debug!("MarkItDown环境检查通过: {}", version);

        Ok(PackageInfo { version })
    }

    /// Python环境设置
    #[instrument(skip(self))]
    pub async fn setup_python_environment(&self) -> Result<(), AppError> {
        info!("开始Python环境设置");

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
                    info!("uv工具已可用且兼容: {}", version);
                    if let Some(recommendation) = compatibility.recommendation {
                        info!("uv升级建议: {}", recommendation);
                    }
                } else {
                    warn!(
                        "uv版本不兼容，重新安装: {}",
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
                warn!("uv版本不兼容: {} - {}", version, issue);
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
                warn!("uv执行失败，重新安装: {}", error);
                self.send_progress("环境设置", InstallStage::Installing, 20.0, "重新安装uv工具")
                    .await;
                self.install_uv_with_progress().await?;
            }
            UvAvailabilityStatus::NotInstalled { error: _ } => {
                info!("uv工具未安装，开始安装");
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
                    info!("Python环境设置完成");
                } else {
                    warn!("环境验证未完全通过，但安装过程已完成");
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
                warn!("环境验证过程出现问题: {}", e);
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
        info!("开始安装Python依赖");

        // 并行安装MinerU和MarkItDown
        let (mineru_result, markitdown_result) = tokio::join!(
            self.install_mineru_with_progress(),
            self.install_markitdown_with_progress()
        );

        mineru_result?;
        markitdown_result?;

        info!("Python依赖安装完成");
        Ok(())
    }

    /// 验证所有引擎
    #[instrument(skip(self))]
    pub async fn validate_engines(&self) -> Result<bool, AppError> {
        info!("验证解析引擎");

        // 清除缓存以确保获取最新状态
        self.clear_cache().await;

        // 等待一小段时间确保安装完成
        sleep(Duration::from_millis(500)).await;

        let status = self.check_environment().await?;
        let is_valid = status.is_ready();

        if !is_valid {
            let critical_issues = status.get_critical_issues();
            for issue in critical_issues {
                error!("关键问题: {} - {}", issue.component, issue.message);
            }
        }

        Ok(is_valid)
    }

    /// 发送安装进度
    async fn send_progress(
        &self,
        package: &str,
        stage: InstallStage,
        progress: f32,
        message: &str,
    ) {
        if let Some(sender) = &self.progress_sender {
            let progress_info = InstallProgress {
                package: package.to_string(),
                stage,
                progress,
                message: message.to_string(),
                estimated_time_remaining: None,
                bytes_downloaded: None,
                total_bytes: None,
            };

            if let Ok(sender) = sender.try_lock() {
                let _ = sender.send(progress_info);
            }
        }
    }

    /// 确保基础目录存在
    async fn ensure_base_directory(&self) -> Result<(), AppError> {
        if !Path::new(&self.base_dir).exists() {
            std::fs::create_dir_all(&self.base_dir)
                .map_err(|e| AppError::File(format!("创建基础目录失败: {e}")))?;
            info!("创建基础目录: {}", self.base_dir);
        }
        Ok(())
    }

    /// 检查uv是否可用（增强版本，带详细错误报告）
    pub async fn is_uv_available(&self) -> Result<UvAvailabilityStatus, AppError> {
        debug!("检查uv工具可用性");

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
                        debug!("uv工具可用: {}", version);
                        Ok(UvAvailabilityStatus::Available {
                            version,
                            compatibility,
                        })
                    }
                    Err(e) => {
                        warn!("uv版本不兼容: {}", e);
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
                debug!("uv命令执行失败: {}", error_msg);
                Ok(UvAvailabilityStatus::ExecutionFailed { error: error_msg })
            }
            Err(e) => {
                let error_msg = format!("无法执行uv命令: {e}");
                debug!("uv命令不存在或无法执行: {}", error_msg);
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

    /// 解析版本号为元组 (major, minor, patch)
    fn parse_version_tuple(&self, version_str: &str) -> Option<(u32, u32, u32)> {
        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.len() >= 2 {
            let major = parts[0].parse::<u32>().ok()?;
            let minor = parts[1].parse::<u32>().ok()?;
            let patch = if parts.len() >= 3 {
                parts[2].parse::<u32>().unwrap_or(0)
            } else {
                0
            };
            Some((major, minor, patch))
        } else {
            None
        }
    }

    /// 安装uv工具（增强版本，带进度跟踪和多种安装方法）
    pub async fn install_uv_with_progress(&self) -> Result<(), AppError> {
        info!("开始安装uv工具");

        self.send_progress("uv", InstallStage::Preparing, 0.0, "准备安装uv工具")
            .await;

        // 确定最佳安装方法
        let installation_method = self.determine_best_uv_installation_method().await;
        info!("选择安装方法: {:?}", installation_method);

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
                            info!("uv安装成功: {}", version);
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
                warn!("主要安装方法失败，尝试备用方法");
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

        info!("使用curl脚本安装uv成功");
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

        info!("使用PowerShell脚本安装uv成功");
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

        info!("使用pip安装uv成功");
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
            if self.is_command_available(manager).await {
                let install_cmd = Command::new(manager).args(&args).output();

                match timeout(Duration::from_secs(300), install_cmd).await {
                    Ok(Ok(output)) if output.status.success() => {
                        info!("使用{}安装uv成功", manager);
                        return Ok(());
                    }
                    Ok(Ok(output)) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        warn!("{}安装uv失败: {}", manager, stderr);
                    }
                    Ok(Err(e)) => {
                        warn!("{}命令执行失败: {}", manager, e);
                    }
                    Err(_) => {
                        warn!("{}安装uv超时", manager);
                    }
                }
            }
        }

        Err(AppError::Environment(
            "所有系统包管理器都无法安装uv".to_string(),
        ))
    }

    /// 检查命令是否可用
    async fn is_command_available(&self, command: &str) -> bool {
        Command::new("which")
            .arg(command)
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 尝试备用UV安装方法
    async fn try_fallback_uv_installation(&self) -> Result<(), AppError> {
        warn!("尝试备用uv安装方法");

        // 备用方法列表
        let fallback_methods = vec![
            UvInstallationMethod::PipInstall,
            UvInstallationMethod::CurlScript,
            UvInstallationMethod::SystemPackageManager,
        ];

        for method in fallback_methods {
            info!("尝试备用安装方法: {:?}", method);

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
                                info!("uv备用安装成功: {}", version);
                                return Ok(());
                            }
                        }
                        _ => continue,
                    }
                }
                Err(e) => {
                    warn!("备用安装方法失败: {}", e);
                    continue;
                }
            }
        }

        Err(AppError::Environment("所有uv安装方法都失败了".to_string()))
    }

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
    async fn check_directory_writable(&self, dir: &Path) -> Result<(), std::io::Error> {
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
    async fn cleanup_corrupted_venv(&self, venv_path: &Path) -> Result<(), AppError> {
        if !venv_path.exists() {
            return Ok(());
        }

        info!("尝试清理损坏的虚拟环境: {}", venv_path.display());

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
                info!("成功清理损坏的虚拟环境");
                Ok(())
            }
            Err(e) => Err(AppError::permission_error(
                format!("清理虚拟环境失败: {e}"),
                venv_path,
            )),
        }
    }

    /// 验证当前目录设置（任务12的核心功能）
    #[instrument(skip(self))]
    pub async fn validate_current_directory_setup(
        &self,
    ) -> Result<DirectoryValidationResult, AppError> {
        let current_dir = Path::new(&self.base_dir);
        let venv_path = current_dir.join("venv");

        info!("开始验证当前目录设置: {}", current_dir.display());

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
            "目录验证完成: valid={}, issues={}, warnings={}",
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

    /// 创建Python虚拟环境（带进度跟踪和增强错误处理）
    async fn create_python_venv_with_progress(&self) -> Result<(), AppError> {
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
                info!("Python虚拟环境已存在且完整: {}", venv_path.display());
                self.send_progress("虚拟环境", InstallStage::Completed, 100.0, "虚拟环境已存在")
                    .await;
                return Ok(());
            } else {
                warn!("检测到损坏的虚拟环境，尝试清理");
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

        info!("创建Python虚拟环境: {}", venv_path.display());
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
            error!("虚拟环境创建失败: {}", error);
            for suggestion in error.get_path_recovery_suggestions() {
                error!("恢复建议: {}", suggestion);
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
        info!("Python虚拟环境创建完成");
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
            warn!("虚拟环境中pip不存在，但这可能是正常的（使用uv管理包）");
        }

        // 尝试运行Python验证虚拟环境
        let test_cmd = Command::new(&python_exe)
            .arg("-c")
            .arg("import sys; print(sys.prefix)")
            .output();

        match timeout(Duration::from_secs(10), test_cmd).await {
            Ok(Ok(output)) if output.status.success() => {
                let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
                debug!("虚拟环境Python前缀: {}", prefix);

                // 验证Python前缀是否指向虚拟环境
                if !prefix.contains("venv") {
                    warn!("Python前缀可能不指向虚拟环境: {}", prefix);
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

    /// 安装MinerU（带进度跟踪）
    async fn install_mineru_with_progress(&self) -> Result<(), AppError> {
        info!("安装MinerU");

        self.send_progress("MinerU", InstallStage::Preparing, 0.0, "准备安装MinerU")
            .await;

        // 检测是否在中国大陆，如果是则使用国内镜像
        let is_china = self.is_china_region().await;

        // 检查CUDA环境状态，决定安装哪个版本的MinerU
        let cuda_status = self.check_cuda_environment().await;
        let mineru_package = match cuda_status {
            Ok(cuda_info) if cuda_info.available && !cuda_info.devices.is_empty() => {
                info!("检测到CUDA环境，安装mineru[all]以支持GPU加速");
                "mineru[all]"
            }
            _ => {
                info!("未检测到CUDA环境，安装mineru[core]（仅CPU版本）");
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
            info!("检测到中国大陆环境，使用阿里云镜像源");
            install_cmd
                .arg("-i")
                .arg("https://mirrors.aliyun.com/pypi/simple/")
                .arg("--trusted-host")
                .arg("mirrors.aliyun.com");
        }
        //install_cmd 命令打印
        info!("mineru安装命令={:?}", &install_cmd);

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
        if is_china {
            if let Err(e) = self.configure_mineru_model_source().await {
                warn!("配置MinerU模型源失败: {}", e);
                // 不阻断安装流程，只记录警告
            }
        }

        self.send_progress("MinerU", InstallStage::Verifying, 90.0, "验证MinerU安装")
            .await;

        // 验证安装
        match self.check_mineru_environment().await {
            Ok(_) => {
                self.send_progress("MinerU", InstallStage::Completed, 100.0, "MinerU安装完成")
                    .await;
                info!("MinerU安装完成");
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
        info!("安装MarkItDown");

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
            info!("检测到中国大陆环境，使用国内镜像源");
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
                info!("MarkItDown安装完成");
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
        if let Ok(tz) = std::env::var("TZ") {
            if tz.contains("Asia/Shanghai") || tz.contains("Asia/Beijing") {
                return true;
            }
        }

        // 检查语言环境
        if let Ok(lang) = std::env::var("LANG") {
            if lang.contains("zh_CN") {
                return true;
            }
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
        {
            if output.status.success() {
                return true;
            }
        }

        false
    }

    /// 配置MinerU模型源为ModelScope（中国大陆）
    async fn configure_mineru_model_source(&self) -> Result<(), AppError> {
        info!("配置MinerU使用ModelScope模型源");

        // 创建配置目录
        let home_dir = std::env::var("HOME")
            .map_err(|_| AppError::Environment("无法获取HOME目录".to_string()))?;
        let config_dir = format!("{home_dir}/.mineru");

        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            warn!("创建MinerU配置目录失败: {}", e);
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

        info!("MinerU模型源配置完成: {}", config_file);
        Ok(())
    }

    /// 验证环境完整性
    #[instrument(skip(self))]
    pub async fn validate_environment(&self) -> Result<bool, AppError> {
        let status = self.check_environment().await?;

        let is_valid = status.is_ready();
        let health_score = status.health_score();

        if !is_valid {
            warn!("环境验证失败，健康评分: {}/100", health_score);
            for issue in status.get_critical_issues() {
                error!(
                    "关键问题 [{}]: {} - {}",
                    issue.component, issue.message, issue.suggestion
                );
            }
        } else {
            info!("环境验证通过，健康评分: {}/100", health_score);
        }

        Ok(is_valid)
    }

    /// 自动激活虚拟环境（如果存在且未激活）
    #[instrument(skip(self))]
    pub async fn auto_activate_virtual_environment(&self) -> Result<(), AppError> {
        let venv_path = Path::new(&self.base_dir).join("venv");

        // 检查虚拟环境是否存在
        if !venv_path.exists() {
            debug!("虚拟环境不存在: {}", venv_path.display());
            return Ok(());
        }

        // 检查虚拟环境是否已经激活
        if let Ok(virtual_env) = std::env::var("VIRTUAL_ENV") {
            if virtual_env == venv_path.to_string_lossy() {
                debug!("虚拟环境已经激活: {}", virtual_env);
                return Ok(());
            }
        }

        // 检查Python可执行文件是否存在
        let python_exe = Self::get_venv_python_path(&venv_path);
        if !python_exe.exists() {
            debug!(
                "虚拟环境中的Python可执行文件不存在: {}",
                python_exe.display()
            );
            return Ok(());
        }

        // 设置环境变量以模拟虚拟环境激活
        info!("自动激活虚拟环境: {}", venv_path.display());

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

        info!("虚拟环境已自动激活，Python路径: {}", python_exe.display());
        debug!(
            "VIRTUAL_ENV: {}",
            std::env::var("VIRTUAL_ENV").unwrap_or_default()
        );
        debug!("PATH前缀: {}", venv_bin_path);

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_environment_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = EnvironmentManager::new(
            "/usr/bin/python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        // 基本创建测试
        assert_eq!(manager.python_path, "/usr/bin/python3");
        assert_eq!(manager.retry_config.max_attempts, 3);
        assert_eq!(manager.cache_ttl, Duration::from_secs(300));
    }

    #[tokio::test]
    async fn test_for_current_directory_factory() {
        // 测试当前目录工厂方法
        let manager = EnvironmentManager::for_current_directory();
        assert!(manager.is_ok());

        let manager = manager.unwrap();

        // 验证路径设置正确
        let current_dir = std::env::current_dir().unwrap();
        assert_eq!(manager.base_dir, current_dir.to_string_lossy().to_string());

        // 验证Python路径根据平台正确设置
        let expected_python_path =
            EnvironmentManager::get_venv_python_path(&current_dir.join("venv"));
        assert_eq!(
            manager.python_path,
            expected_python_path.to_string_lossy().to_string()
        );

        // 验证默认配置
        assert_eq!(manager.retry_config.max_attempts, 3);
        assert_eq!(manager.cache_ttl, Duration::from_secs(300));
        assert!(manager.progress_sender.is_none());
    }

    #[tokio::test]
    async fn test_for_current_directory_with_progress_factory() {
        let (tx, _rx) = mpsc::unbounded_channel();

        // 测试带进度跟踪的当前目录工厂方法
        let manager = EnvironmentManager::for_current_directory_with_progress(tx);
        assert!(manager.is_ok());

        let manager = manager.unwrap();

        // 验证路径设置正确
        let current_dir = std::env::current_dir().unwrap();
        assert_eq!(manager.base_dir, current_dir.to_string_lossy().to_string());

        // 验证Python路径根据平台正确设置
        let expected_python_path =
            EnvironmentManager::get_venv_python_path(&current_dir.join("venv"));
        assert_eq!(
            manager.python_path,
            expected_python_path.to_string_lossy().to_string()
        );

        // 验证进度发送器已设置
        assert!(manager.progress_sender.is_some());
    }

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

    #[tokio::test]
    async fn test_retry_config() {
        let temp_dir = TempDir::new().unwrap();
        let retry_config = RetryConfig {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 1.5,
        };

        let manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        )
        .with_retry_config(retry_config.clone());

        assert_eq!(manager.retry_config.max_attempts, 5);
        assert_eq!(manager.retry_config.backoff_multiplier, 1.5);
    }

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
    async fn test_cross_platform_path_functions() {
        use std::path::Path;

        let venv_path = Path::new("test_venv");

        // 测试Python路径生成
        let python_path = EnvironmentManager::get_venv_python_path(venv_path);
        if cfg!(windows) {
            assert_eq!(python_path, venv_path.join("Scripts").join("python.exe"));
        } else {
            assert_eq!(python_path, venv_path.join("bin").join("python"));
        }

        // 测试可执行文件路径生成
        let mineru_path = EnvironmentManager::get_venv_executable_path(venv_path, "mineru");
        if cfg!(windows) {
            assert_eq!(mineru_path, venv_path.join("Scripts").join("mineru.exe"));
        } else {
            assert_eq!(mineru_path, venv_path.join("bin").join("mineru"));
        }

        // 测试激活脚本路径
        let activation_script = EnvironmentManager::get_venv_activation_script(venv_path);
        if cfg!(windows) {
            assert_eq!(
                activation_script,
                venv_path.join("Scripts").join("activate.bat")
            );
        } else {
            assert_eq!(activation_script, venv_path.join("bin").join("activate"));
        }

        // 测试系统Python可执行文件列表
        let python_executables = EnvironmentManager::get_system_python_executable();
        assert!(!python_executables.is_empty());

        if cfg!(windows) {
            assert!(python_executables.contains(&"python.exe".to_string()));
            assert!(python_executables.contains(&"python3.exe".to_string()));
        } else {
            assert!(python_executables.contains(&"python3".to_string()));
            assert!(python_executables.contains(&"python".to_string()));
        }
    }

    #[tokio::test]
    async fn test_cross_platform_environment_variables() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        let venv_path = temp_dir.path().join("venv");
        let env_vars = manager.get_cross_platform_env_vars(&venv_path);

        // 验证VIRTUAL_ENV变量
        assert_eq!(
            env_vars.get("VIRTUAL_ENV").unwrap(),
            &venv_path.to_string_lossy().to_string()
        );

        // 验证PATH变量包含正确的路径
        let path_var = env_vars.get("PATH").unwrap();
        if cfg!(windows) {
            assert!(path_var.contains(&venv_path.join("Scripts").to_string_lossy().to_string()));
        } else {
            assert!(path_var.contains(&venv_path.join("bin").to_string_lossy().to_string()));
        }
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

    #[tokio::test]
    async fn test_virtual_environment_info() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        let venv_path = temp_dir.path().join("venv");

        // 测试虚拟环境信息获取（即使虚拟环境不存在）
        let venv_info_result = manager.get_virtual_environment_info(&venv_path).await;
        assert!(venv_info_result.is_ok());

        let venv_info = venv_info_result.unwrap();
        assert_eq!(venv_info.path, venv_path);
        assert!(!venv_info.is_valid); // 因为虚拟环境不存在

        // 验证平台特定路径
        if cfg!(windows) {
            assert_eq!(
                venv_info.python_executable,
                venv_path.join("Scripts").join("python.exe")
            );
            assert_eq!(
                venv_info.pip_executable,
                venv_path.join("Scripts").join("pip.exe")
            );
            assert_eq!(
                venv_info.activation_script,
                venv_path.join("Scripts").join("activate.bat")
            );
            assert_eq!(venv_info.platform, "windows");
        } else {
            assert_eq!(
                venv_info.python_executable,
                venv_path.join("bin").join("python")
            );
            assert_eq!(venv_info.pip_executable, venv_path.join("bin").join("pip"));
            assert_eq!(
                venv_info.activation_script,
                venv_path.join("bin").join("activate")
            );
            assert_eq!(venv_info.platform, "unix");
        }
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
