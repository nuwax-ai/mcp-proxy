//! 优雅关闭管理器
//!
//! 提供优雅关闭处理，确保所有资源得到正确清理
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};
use tokio::signal;
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio::time::timeout;

use super::resource_cleanup::ResourceCleaner;
use crate::app_state::AppState;
use crate::error::AppError;

/// 优雅关闭管理器
pub struct GracefulShutdownManager {
    is_shutting_down: Arc<AtomicBool>,
    shutdown_handlers: Arc<RwLock<HashMap<String, Box<dyn ShutdownHandler + Send + Sync>>>>,
    shutdown_timeout: Duration,
    shutdown_semaphore: Arc<Semaphore>,
    shutdown_stats: Arc<Mutex<ShutdownStats>>,
}

impl GracefulShutdownManager {
    /// 创建新的优雅关闭管理器
    pub async fn new() -> Result<Self, AppError> {
        Ok(Self {
            is_shutting_down: Arc::new(AtomicBool::new(false)),
            shutdown_handlers: Arc::new(RwLock::new(HashMap::new())),
            shutdown_timeout: Duration::from_secs(30),
            shutdown_semaphore: Arc::new(Semaphore::new(1)),
            shutdown_stats: Arc::new(Mutex::new(ShutdownStats::default())),
        })
    }

    /// 设置优雅关闭
    pub async fn setup(
        &self,
        app_state: Arc<AppState>,
        resource_cleaner: Arc<dyn ResourceCleaner + Send + Sync>,
    ) -> Result<(), AppError> {
        // 注册默认的关闭处理器
        self.register_default_handlers(app_state, resource_cleaner)
            .await?;

        // 设置信号处理
        self.setup_signal_handlers().await?;

        tracing::info!("Graceful shutdown setup completed");

        Ok(())
    }

    /// 注册关闭处理器
    pub async fn register_handler(
        &self,
        name: String,
        handler: Box<dyn ShutdownHandler + Send + Sync>,
    ) -> Result<(), AppError> {
        let mut handlers = self.shutdown_handlers.write().await;
        handlers.insert(name, handler);
        Ok(())
    }

    /// 移除关闭处理器
    pub async fn unregister_handler(&self, name: &str) -> Result<(), AppError> {
        let mut handlers = self.shutdown_handlers.write().await;
        handlers.remove(name);
        Ok(())
    }

    /// 检查是否正在关闭
    pub fn is_shutting_down(&self) -> bool {
        self.is_shutting_down.load(Ordering::Relaxed)
    }

    /// 执行优雅关闭
    pub async fn shutdown(&self) -> Result<(), AppError> {
        // 获取关闭信号量，确保只有一个关闭过程
        let _permit = self
            .shutdown_semaphore
            .acquire()
            .await
            .map_err(|_| AppError::Config("Shutdown failed".to_string()))?;

        if self.is_shutting_down.swap(true, Ordering::Relaxed) {
            tracing::warn!("Shutdown already in progress");
            return Ok(());
        }

        let start_time = SystemTime::now();
        let mut stats = self.shutdown_stats.lock().await;
        stats.shutdown_started_at = Some(start_time);

        tracing::info!("Starting graceful shutdown");

        // 执行关闭处理器
        let result = self.execute_shutdown_handlers().await;

        // 更新统计信息
        stats.shutdown_completed_at = Some(SystemTime::now());
        stats.shutdown_duration = SystemTime::now().duration_since(start_time).ok();
        stats.shutdown_successful = result.is_ok();

        match result {
            Ok(_) => {
                tracing::info!(
                    "Graceful shutdown completed successfully in {:?}",
                    start_time.elapsed()
                );
            }
            Err(ref e) => {
                tracing::error!(
                    "Graceful shutdown failed after {:?}: {}",
                    start_time.elapsed(),
                    e
                );
            }
        }

        result
    }

    /// 强制关闭
    pub async fn force_shutdown(&self) -> Result<(), AppError> {
        tracing::warn!("Force shutdown initiated");

        self.is_shutting_down.store(true, Ordering::Relaxed);

        // 强制执行关闭处理器（不等待超时）
        let handlers = self.shutdown_handlers.read().await;
        for (name, handler) in handlers.iter() {
            if let Err(e) = handler.force_shutdown().await {
                tracing::error!("Force shutdown handler '{}' failed: {}", name, e);
            }
        }

        tracing::warn!("Force shutdown completed");

        Ok(())
    }

    /// 获取关闭统计信息
    pub async fn get_shutdown_stats(&self) -> ShutdownStats {
        self.shutdown_stats.lock().await.clone()
    }

    /// 等待关闭完成
    pub async fn wait_for_shutdown(&self) -> Result<(), AppError> {
        while !self.is_shutting_down() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // 等待关闭完成
        loop {
            let stats = self.shutdown_stats.lock().await;
            if stats.shutdown_completed_at.is_some() {
                break;
            }
            drop(stats);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }

    // 私有方法

    async fn register_default_handlers(
        &self,
        app_state: Arc<AppState>,
        resource_cleaner: Arc<dyn ResourceCleaner + Send + Sync>,
    ) -> Result<(), AppError> {
        // 注册应用状态关闭处理器
        self.register_handler(
            "app_state".to_string(),
            Box::new(AppStateShutdownHandler::new(app_state)),
        )
        .await?;

        // 注册资源清理处理器
        self.register_handler(
            "resource_cleaner".to_string(),
            Box::new(ResourceCleanerShutdownHandler::new(resource_cleaner)),
        )
        .await?;

        // 注册数据库连接关闭处理器
        self.register_handler(
            "database".to_string(),
            Box::new(DatabaseShutdownHandler::new()),
        )
        .await?;

        // 注册HTTP服务器关闭处理器
        self.register_handler(
            "http_server".to_string(),
            Box::new(HttpServerShutdownHandler::new()),
        )
        .await?;

        Ok(())
    }

    async fn setup_signal_handlers(&self) -> Result<(), AppError> {
        #[cfg(unix)]
        {
            let shutdown_manager = Arc::new(self.clone());

            // 处理SIGTERM信号
            tokio::spawn(async move {
                let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
                    .expect("Failed to register SIGTERM handler");

                sigterm.recv().await;
                tracing::info!("Received SIGTERM, initiating graceful shutdown");

                if let Err(e) = shutdown_manager.shutdown().await {
                    tracing::error!("Graceful shutdown failed: {}", e);
                    std::process::exit(1);
                }
            });

            // 处理SIGINT信号 (Ctrl+C)
            let shutdown_manager = Arc::new(self.clone());
            tokio::spawn(async move {
                let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
                    .expect("Failed to register SIGINT handler");

                sigint.recv().await;
                tracing::info!("Received SIGINT, initiating graceful shutdown");

                if let Err(e) = shutdown_manager.shutdown().await {
                    tracing::error!("Graceful shutdown failed: {}", e);
                    std::process::exit(1);
                }
            });
        }

        #[cfg(not(unix))]
        {
            // Windows uses ctrl_c signal handling which is set up elsewhere
            tracing::debug!("Unix signal handlers not available on this platform");
        }

        Ok(())
    }

    async fn execute_shutdown_handlers(&self) -> Result<(), AppError> {
        let handlers = self.shutdown_handlers.read().await;
        let mut shutdown_results = Vec::new();

        // 按优先级顺序执行关闭处理器
        let ordered_handlers = self.get_ordered_handlers(&handlers).await;

        for (name, handler) in ordered_handlers {
            tracing::info!("Executing shutdown handler: {}", name);

            let result = timeout(self.shutdown_timeout, handler.shutdown()).await;

            match result {
                Ok(Ok(_)) => {
                    tracing::info!("Shutdown handler '{}' completed successfully", name);
                    shutdown_results.push((name.clone(), true));
                }
                Ok(Err(e)) => {
                    tracing::error!("Shutdown handler '{}' failed: {}", name, e);
                    shutdown_results.push((name.clone(), false));
                }
                Err(_) => {
                    tracing::error!("Shutdown handler '{}' timed out", name);
                    shutdown_results.push((name.clone(), false));

                    // 尝试强制关闭
                    if let Err(e) = handler.force_shutdown().await {
                        tracing::error!("Force shutdown for '{}' failed: {}", name, e);
                    }
                }
            }
        }

        // 检查是否有失败的处理器
        let failed_handlers: Vec<_> = shutdown_results
            .iter()
            .filter(|(_, success)| !success)
            .map(|(name, _)| name.clone())
            .collect();

        if !failed_handlers.is_empty() {
            return Err(AppError::Config(format!(
                "Shutdown handlers failed: {failed_handlers:?}"
            )));
        }

        Ok(())
    }

    async fn get_ordered_handlers<'a>(
        &self,
        handlers: &'a HashMap<String, Box<dyn ShutdownHandler + Send + Sync>>,
    ) -> Vec<(String, &'a Box<dyn ShutdownHandler + Send + Sync>)> {
        // 定义关闭顺序（优先级从高到低）
        let priority_order = vec!["http_server", "app_state", "database", "resource_cleaner"];

        let mut ordered = Vec::new();

        // 按优先级添加处理器
        for &priority_name in &priority_order {
            if let Some(handler) = handlers.get(priority_name) {
                ordered.push((priority_name.to_string(), handler));
            }
        }

        // 添加其他处理器
        for (name, handler) in handlers {
            if !priority_order.contains(&name.as_str()) {
                ordered.push((name.clone(), handler));
            }
        }

        ordered
    }
}

// 为了支持clone，我们需要实现Clone trait
impl Clone for GracefulShutdownManager {
    fn clone(&self) -> Self {
        Self {
            is_shutting_down: self.is_shutting_down.clone(),
            shutdown_handlers: self.shutdown_handlers.clone(),
            shutdown_timeout: self.shutdown_timeout,
            shutdown_semaphore: self.shutdown_semaphore.clone(),
            shutdown_stats: self.shutdown_stats.clone(),
        }
    }
}

/// 关闭处理器特征
#[async_trait::async_trait]
pub trait ShutdownHandler {
    /// 执行优雅关闭
    async fn shutdown(&self) -> Result<(), AppError>;

    /// 执行强制关闭
    async fn force_shutdown(&self) -> Result<(), AppError>;

    /// 获取处理器名称
    fn name(&self) -> &str;

    /// 获取关闭优先级（数字越小优先级越高）
    fn priority(&self) -> u32 {
        100
    }
}

/// 应用状态关闭处理器
struct AppStateShutdownHandler {
    app_state: Arc<AppState>,
}

impl AppStateShutdownHandler {
    fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }
}

#[async_trait::async_trait]
impl ShutdownHandler for AppStateShutdownHandler {
    async fn shutdown(&self) -> Result<(), AppError> {
        tracing::info!("Shutting down application state");

        // 停止接受新请求
        // 等待当前请求完成
        // 清理应用状态

        Ok(())
    }

    async fn force_shutdown(&self) -> Result<(), AppError> {
        tracing::warn!("Force shutting down application state");
        Ok(())
    }

    fn name(&self) -> &str {
        "app_state"
    }

    fn priority(&self) -> u32 {
        10
    }
}

/// 资源清理关闭处理器
struct ResourceCleanerShutdownHandler {
    resource_cleaner: Arc<dyn ResourceCleaner + Send + Sync>,
}

impl ResourceCleanerShutdownHandler {
    fn new(resource_cleaner: Arc<dyn ResourceCleaner + Send + Sync>) -> Self {
        Self { resource_cleaner }
    }
}

#[async_trait::async_trait]
impl ShutdownHandler for ResourceCleanerShutdownHandler {
    async fn shutdown(&self) -> Result<(), AppError> {
        tracing::info!("Executing resource cleanup");
        // ResourceCleaner trait methods return anyhow::Result, so we need to convert
        self.resource_cleaner
            .cleanup()
            .map_err(|e| AppError::Config(format!("Resource cleanup failed: {e}")))?;
        Ok(())
    }

    async fn force_shutdown(&self) -> Result<(), AppError> {
        tracing::warn!("Force executing resource cleanup");
        // ResourceCleaner trait methods return anyhow::Result, so we need to convert
        self.resource_cleaner
            .force_cleanup()
            .map_err(|e| AppError::Config(format!("Force resource cleanup failed: {e}")))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "resource_cleaner"
    }

    fn priority(&self) -> u32 {
        90
    }
}

/// 数据库关闭处理器
struct DatabaseShutdownHandler;

impl DatabaseShutdownHandler {
    fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ShutdownHandler for DatabaseShutdownHandler {
    async fn shutdown(&self) -> Result<(), AppError> {
        tracing::info!("Shutting down database connections");

        // 关闭数据库连接池
        // 等待当前事务完成

        Ok(())
    }

    async fn force_shutdown(&self) -> Result<(), AppError> {
        tracing::warn!("Force shutting down database connections");
        Ok(())
    }

    fn name(&self) -> &str {
        "database"
    }

    fn priority(&self) -> u32 {
        50
    }
}

/// HTTP服务器关闭处理器
struct HttpServerShutdownHandler;

impl HttpServerShutdownHandler {
    fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ShutdownHandler for HttpServerShutdownHandler {
    async fn shutdown(&self) -> Result<(), AppError> {
        tracing::info!("Shutting down HTTP server");

        // 停止接受新连接
        // 等待当前请求完成

        Ok(())
    }

    async fn force_shutdown(&self) -> Result<(), AppError> {
        tracing::warn!("Force shutting down HTTP server");
        Ok(())
    }

    fn name(&self) -> &str {
        "http_server"
    }

    fn priority(&self) -> u32 {
        5
    }
}

/// 关闭统计信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShutdownStats {
    pub shutdown_started_at: Option<SystemTime>,
    pub shutdown_completed_at: Option<SystemTime>,
    pub shutdown_duration: Option<Duration>,
    pub shutdown_successful: bool,
    pub handlers_executed: Vec<String>,
    pub failed_handlers: Vec<String>,
}

/// 关闭配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShutdownConfig {
    pub timeout: Duration,
    pub force_timeout: Duration,
    pub signal_handlers_enabled: bool,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            force_timeout: Duration::from_secs(5),
            signal_handlers_enabled: true,
        }
    }
}
