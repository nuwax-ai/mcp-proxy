//! 跨功能组共享的基础类型（重试配置、环境问题/警告、进度、CUDA、虚拟环境信息）。

use super::*;

/// 重试配置
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub backoff_multiplier: f64,
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

/// CUDA环境信息
#[derive(Debug)]
pub struct CudaInfo {
    pub available: bool,
    pub version: Option<String>,
    pub devices: Vec<CudaDevice>,
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
