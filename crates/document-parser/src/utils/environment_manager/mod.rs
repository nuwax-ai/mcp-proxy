//! 环境管理器：Python/uv/CUDA/MinerU/MarkItDown 环境的检测、安装与诊断。
//!
//! 按职责拆分为多个子模块，本模块通过 re-export 保持对外统一的 API 面：
//! 所有 `utils::environment_manager::xxx` 路径与拆分前完全一致。
//! EnvironmentManager 的字段与进度发送/版本解析等基础方法保留在本模块，
//! 子模块通过 `use super::*` 直接访问（祖先私有项对后代可见）。

use crate::error::AppError;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::{sleep, timeout};
use tracing::{debug, error, info, instrument, warn};

pub mod dependency;
pub mod detection;
pub mod directory;
pub mod installer;
pub mod paths;
pub mod report;
pub mod status;
pub mod types;
pub mod uv;
pub mod venv;

pub use dependency::*;
pub use directory::*;
pub use status::*;
pub use types::*;
pub use uv::*;

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

    /// 为指定目录创建环境管理器（不读全局 cwd，便于测试注入 / 并行隔离）
    pub fn for_directory<P: AsRef<Path>>(base_dir: P) -> Result<Self, AppError> {
        let base_dir = base_dir.as_ref();
        let python_path = Self::get_venv_python_path(&base_dir.join("venv"));
        Ok(Self {
            python_path: python_path.to_string_lossy().to_string(),
            base_dir: base_dir.to_string_lossy().to_string(),
            progress_sender: None,
            timeout_duration: Duration::from_secs(300), // 5分钟默认超时
            retry_config: RetryConfig::default(),
            environment_cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::from_secs(300), // 5分钟缓存
        })
    }

    /// 为当前目录创建环境管理器（推荐使用；生产入口，读进程 cwd 一次）
    pub fn for_current_directory() -> Result<Self, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;
        Self::for_directory(current_dir)
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

    /// 为指定目录创建带进度跟踪的环境管理器（不读全局 cwd）
    pub fn for_directory_with_progress<P: AsRef<Path>>(
        base_dir: P,
        progress_sender: mpsc::UnboundedSender<InstallProgress>,
    ) -> Result<Self, AppError> {
        Ok(Self::for_directory(base_dir)?.with_progress_sender(progress_sender))
    }

    /// 为当前目录创建带进度跟踪的环境管理器
    pub fn for_current_directory_with_progress(
        progress_sender: mpsc::UnboundedSender<InstallProgress>,
    ) -> Result<Self, AppError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| AppError::Environment(format!("无法获取当前目录: {e}")))?;
        Self::for_directory_with_progress(current_dir, progress_sender)
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

    /// 发送安装进度（便捷封装；实际发送统一走 send_progress_raw）
    async fn send_progress(
        &self,
        package: &str,
        stage: InstallStage,
        progress: f32,
        message: &str,
    ) {
        self.send_progress_raw(InstallProgress {
            package: package.to_string(),
            stage,
            progress,
            message: message.to_string(),
            estimated_time_remaining: None,
            bytes_downloaded: None,
            total_bytes: None,
        })
        .await;
    }

    /// 进度发送的唯一通道（try_lock + send；retry_with_backoff 与 send_progress 共用）
    async fn send_progress_raw(&self, progress_info: InstallProgress) {
        if let Some(sender) = &self.progress_sender
            && let Ok(sender) = sender.try_lock()
        {
            let _ = sender.send(progress_info);
        }
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
}
