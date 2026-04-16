use crate::services::oss_service::OssService;
use crate::services::storage_service::StorageService;
use crate::utils::environment_manager::EnvironmentManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 健康检查状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
            HealthStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// 健康检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub component: String,
    pub status: HealthStatus,
    pub message: String,
    pub details: HashMap<String, String>,
    pub timestamp: u64,
    pub response_time_ms: u64,
}

impl HealthCheckResult {
    pub fn new(component: String, status: HealthStatus, message: String) -> Self {
        Self {
            component,
            status,
            message,
            details: HashMap::new(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            response_time_ms: 0,
        }
    }

    pub fn with_details(mut self, details: HashMap<String, String>) -> Self {
        self.details = details;
        self
    }

    pub fn with_response_time(mut self, response_time: Duration) -> Self {
        self.response_time_ms = response_time.as_millis() as u64;
        self
    }

    pub fn add_detail(&mut self, key: String, value: String) {
        self.details.insert(key, value);
    }

    pub fn is_healthy(&self) -> bool {
        self.status == HealthStatus::Healthy
    }

    pub fn is_degraded(&self) -> bool {
        self.status == HealthStatus::Degraded
    }

    pub fn is_unhealthy(&self) -> bool {
        self.status == HealthStatus::Unhealthy
    }
}

/// 系统健康状态汇总
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemHealthStatus {
    pub overall_status: HealthStatus,
    pub components: Vec<HealthCheckResult>,
    pub healthy_count: usize,
    pub degraded_count: usize,
    pub unhealthy_count: usize,
    pub unknown_count: usize,
    pub total_response_time_ms: u64,
    pub timestamp: u64,
}

impl SystemHealthStatus {
    pub fn new(components: Vec<HealthCheckResult>) -> Self {
        let healthy_count = components.iter().filter(|c| c.is_healthy()).count();
        let degraded_count = components.iter().filter(|c| c.is_degraded()).count();
        let unhealthy_count = components.iter().filter(|c| c.is_unhealthy()).count();
        let unknown_count = components.len() - healthy_count - degraded_count - unhealthy_count;

        let total_response_time_ms = components.iter().map(|c| c.response_time_ms).sum();

        // 确定整体状态
        let overall_status = if unhealthy_count > 0 {
            HealthStatus::Unhealthy
        } else if degraded_count > 0 {
            HealthStatus::Degraded
        } else if healthy_count > 0 {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unknown
        };

        Self {
            overall_status,
            components,
            healthy_count,
            degraded_count,
            unhealthy_count,
            unknown_count,
            total_response_time_ms,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.overall_status == HealthStatus::Healthy
    }

    pub fn get_component_status(&self, component: &str) -> Option<&HealthCheckResult> {
        self.components.iter().find(|c| c.component == component)
    }
}

/// 健康检查器trait
#[async_trait::async_trait]
pub trait HealthChecker: Send + Sync {
    async fn check_health(&self) -> HealthCheckResult;
    fn component_name(&self) -> &str;
    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// 环境健康检查器
#[derive(Debug)]
pub struct EnvironmentHealthChecker {
    environment_manager: Arc<EnvironmentManager>,
}

impl EnvironmentHealthChecker {
    pub fn new(environment_manager: Arc<EnvironmentManager>) -> Self {
        Self {
            environment_manager,
        }
    }
}

#[async_trait::async_trait]
impl HealthChecker for EnvironmentHealthChecker {
    async fn check_health(&self) -> HealthCheckResult {
        let start = Instant::now();

        match self.environment_manager.check_environment().await {
            Ok(status) => {
                let mut details = HashMap::new();
                details.insert(
                    "python_available".to_string(),
                    status.python_available.to_string(),
                );
                details.insert("uv_available".to_string(), status.uv_available.to_string());
                details.insert(
                    "cuda_available".to_string(),
                    status.cuda_available.to_string(),
                );
                details.insert(
                    "mineru_available".to_string(),
                    status.mineru_available.to_string(),
                );
                details.insert(
                    "markitdown_available".to_string(),
                    status.markitdown_available.to_string(),
                );

                if let Some(ref python_version) = status.python_version {
                    details.insert("python_version".to_string(), python_version.clone());
                }

                if let Some(ref cuda_version) = status.cuda_version {
                    details.insert("cuda_version".to_string(), cuda_version.clone());
                }

                let health_status = if status.is_ready() {
                    HealthStatus::Healthy
                } else if status.python_available && status.uv_available {
                    HealthStatus::Degraded
                } else {
                    HealthStatus::Unhealthy
                };

                let message = if status.is_ready() {
                    "All environments are ready".to_string()
                } else {
                    format!("Environment issues: {:?}", status.get_issues())
                };

                HealthCheckResult::new(self.component_name().to_string(), health_status, message)
                    .with_details(details)
                    .with_response_time(start.elapsed())
            }
            Err(e) => HealthCheckResult::new(
                self.component_name().to_string(),
                HealthStatus::Unhealthy,
                format!("Environment check failed: {e}"),
            )
            .with_response_time(start.elapsed()),
        }
    }

    fn component_name(&self) -> &str {
        "environment"
    }
}

/// 存储健康检查器
#[derive(Debug)]
pub struct StorageHealthChecker {
    storage_service: Arc<StorageService>,
}

impl StorageHealthChecker {
    pub fn new(storage_service: Arc<StorageService>) -> Self {
        Self { storage_service }
    }
}

#[async_trait::async_trait]
impl HealthChecker for StorageHealthChecker {
    async fn check_health(&self) -> HealthCheckResult {
        let start = Instant::now();

        // 测试基本的数据库操作
        match self.storage_service.get_stats().await {
            Ok(stats) => {
                let mut details = HashMap::new();
                details.insert("total_tasks".to_string(), stats.total_tasks.to_string());
                details.insert(
                    "total_size_bytes".to_string(),
                    stats.total_size_bytes.to_string(),
                );
                details.insert("index_count".to_string(), stats.index_count.to_string());
                details.insert(
                    "cache_hit_rate".to_string(),
                    stats.cache_hit_rate.to_string(),
                );

                // 检查数据库是否响应正常
                let health_status = if stats.total_tasks > 0 {
                    HealthStatus::Healthy
                } else {
                    HealthStatus::Unhealthy
                };

                HealthCheckResult::new(
                    self.component_name().to_string(),
                    health_status,
                    "Storage service is operational".to_string(),
                )
                .with_details(details)
                .with_response_time(start.elapsed())
            }
            Err(e) => HealthCheckResult::new(
                self.component_name().to_string(),
                HealthStatus::Unhealthy,
                format!("Storage check failed: {e}"),
            )
            .with_response_time(start.elapsed()),
        }
    }

    fn component_name(&self) -> &str {
        "storage"
    }
}

/// OSS健康检查器
#[derive(Debug)]
pub struct OssHealthChecker {
    oss_service: Arc<OssService>,
}

impl OssHealthChecker {
    pub fn new(oss_service: Arc<OssService>) -> Self {
        Self { oss_service }
    }
}

#[async_trait::async_trait]
impl HealthChecker for OssHealthChecker {
    async fn check_health(&self) -> HealthCheckResult {
        let start = Instant::now();

        // 测试OSS连接
        let test_key = "health-check-test".to_string();
        let test_content = b"health check test";

        match self
            .oss_service
            .upload_content(test_content, &test_key, None)
            .await
        {
            Ok(_) => {
                // 尝试删除测试文件
                let _ = self.oss_service.delete_object(&test_key).await;

                let mut details = HashMap::new();
                details.insert(
                    "bucket".to_string(),
                    self.oss_service.get_bucket_name().to_string(),
                );
                details.insert(
                    "base_url".to_string(),
                    self.oss_service.get_base_url().to_string(),
                );

                HealthCheckResult::new(
                    self.component_name().to_string(),
                    HealthStatus::Healthy,
                    "OSS service is operational".to_string(),
                )
                .with_details(details)
                .with_response_time(start.elapsed())
            }
            Err(e) => HealthCheckResult::new(
                self.component_name().to_string(),
                HealthStatus::Unhealthy,
                format!("OSS check failed: {e}"),
            )
            .with_response_time(start.elapsed()),
        }
    }

    fn component_name(&self) -> &str {
        "oss"
    }
}

/// 系统资源健康检查器
#[derive(Debug)]
pub struct SystemResourceChecker {
    memory_threshold_mb: u64,
    disk_threshold_percent: f64,
}

impl SystemResourceChecker {
    pub fn new(memory_threshold_mb: u64, disk_threshold_percent: f64) -> Self {
        Self {
            memory_threshold_mb,
            disk_threshold_percent,
        }
    }
}

#[async_trait::async_trait]
impl HealthChecker for SystemResourceChecker {
    async fn check_health(&self) -> HealthCheckResult {
        let start = Instant::now();

        let mut details = HashMap::new();
        let mut issues = Vec::new();

        // 检查内存使用情况
        if let Ok(memory_info) = Self::get_memory_info() {
            let used_mb = memory_info.used / 1024 / 1024;
            let total_mb = memory_info.total / 1024 / 1024;
            let usage_percent = (memory_info.used as f64 / memory_info.total as f64) * 100.0;

            details.insert("memory_used_mb".to_string(), used_mb.to_string());
            details.insert("memory_total_mb".to_string(), total_mb.to_string());
            details.insert(
                "memory_usage_percent".to_string(),
                format!("{usage_percent:.1}"),
            );

            if used_mb > self.memory_threshold_mb {
                issues.push(format!("High memory usage: {used_mb}MB"));
            }
        } else {
            issues.push("Failed to get memory information".to_string());
        }

        // 检查磁盘使用情况
        if let Ok(disk_info) = Self::get_disk_info(".") {
            let usage_percent = (disk_info.used as f64 / disk_info.total as f64) * 100.0;

            details.insert(
                "disk_used_gb".to_string(),
                (disk_info.used / 1024 / 1024 / 1024).to_string(),
            );
            details.insert(
                "disk_total_gb".to_string(),
                (disk_info.total / 1024 / 1024 / 1024).to_string(),
            );
            details.insert(
                "disk_usage_percent".to_string(),
                format!("{usage_percent:.1}"),
            );

            if usage_percent > self.disk_threshold_percent {
                issues.push(format!("High disk usage: {usage_percent:.1}%"));
            }
        } else {
            issues.push("Failed to get disk information".to_string());
        }

        // 检查CPU负载
        if let Ok(load_avg) = Self::get_load_average() {
            details.insert("load_1min".to_string(), format!("{:.2}", load_avg.0));
            details.insert("load_5min".to_string(), format!("{:.2}", load_avg.1));
            details.insert("load_15min".to_string(), format!("{:.2}", load_avg.2));

            // 简单的负载检查（假设4核CPU）
            if load_avg.0 > 4.0 {
                issues.push(format!("High CPU load: {:.2}", load_avg.0));
            }
        }

        let (status, message) = if issues.is_empty() {
            (
                HealthStatus::Healthy,
                "System resources are normal".to_string(),
            )
        } else if issues.len() == 1 {
            (
                HealthStatus::Degraded,
                format!("Resource issue: {}", issues[0]),
            )
        } else {
            (
                HealthStatus::Unhealthy,
                format!("Multiple resource issues: {}", issues.join(", ")),
            )
        };

        HealthCheckResult::new(self.component_name().to_string(), status, message)
            .with_details(details)
            .with_response_time(start.elapsed())
    }

    fn component_name(&self) -> &str {
        "system_resources"
    }
}

impl SystemResourceChecker {
    fn get_memory_info() -> Result<MemoryInfo, Box<dyn std::error::Error>> {
        #[cfg(target_os = "macos")]
        {
            let output = Command::new("vm_stat").output()?;
            let output_str = String::from_utf8(output.stdout)?;

            // 解析vm_stat输出
            let mut free_pages = 0u64;
            let mut active_pages = 0u64;
            let mut inactive_pages = 0u64;
            let mut wired_pages = 0u64;

            for line in output_str.lines() {
                if line.contains("Pages free:") {
                    free_pages = Self::extract_pages(line)?;
                } else if line.contains("Pages active:") {
                    active_pages = Self::extract_pages(line)?;
                } else if line.contains("Pages inactive:") {
                    inactive_pages = Self::extract_pages(line)?;
                } else if line.contains("Pages wired down:") {
                    wired_pages = Self::extract_pages(line)?;
                }
            }

            let page_size = 4096u64; // macOS页面大小
            let total = (free_pages + active_pages + inactive_pages + wired_pages) * page_size;
            let used = (active_pages + inactive_pages + wired_pages) * page_size;

            Ok(MemoryInfo { total, used })
        }

        #[cfg(target_os = "linux")]
        {
            let meminfo = std::fs::read_to_string("/proc/meminfo")?;
            let mut total = 0u64;
            let mut available = 0u64;

            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    total = Self::extract_kb_value(line)? * 1024;
                } else if line.starts_with("MemAvailable:") {
                    available = Self::extract_kb_value(line)? * 1024;
                }
            }

            let used = total - available;
            Ok(MemoryInfo { total, used })
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err("Unsupported platform for memory info".into())
        }
    }

    fn extract_pages(line: &str) -> Result<u64, Box<dyn std::error::Error>> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let page_str = parts[2].trim_end_matches('.');
            Ok(page_str.parse()?)
        } else {
            Err("Invalid vm_stat line format".into())
        }
    }

    #[allow(dead_code)]
    fn extract_kb_value(line: &str) -> Result<u64, Box<dyn std::error::Error>> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            Ok(parts[1].parse()?)
        } else {
            Err("Invalid meminfo line format".into())
        }
    }

    fn get_disk_info(path: &str) -> Result<DiskInfo, Box<dyn std::error::Error>> {
        let output = Command::new("df").arg("-k").arg(path).output()?;

        let output_str = String::from_utf8(output.stdout)?;
        let lines: Vec<&str> = output_str.lines().collect();

        if lines.len() >= 2 {
            let parts: Vec<&str> = lines[1].split_whitespace().collect();
            if parts.len() >= 4 {
                let total = parts[1].parse::<u64>()? * 1024; // 转换为字节
                let used = parts[2].parse::<u64>()? * 1024;
                return Ok(DiskInfo { total, used });
            }
        }

        Err("Failed to parse df output".into())
    }

    fn get_load_average() -> Result<(f64, f64, f64), Box<dyn std::error::Error>> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let loadavg = std::fs::read_to_string("/proc/loadavg").or_else(
                |_| -> Result<String, Box<dyn std::error::Error>> {
                    // macOS fallback
                    let output = Command::new("uptime").output()?;
                    Ok(String::from_utf8(output.stdout)?)
                },
            )?;

            let parts: Vec<&str> = loadavg.split_whitespace().collect();
            if parts.len() >= 3 {
                let load1 = parts[0].parse::<f64>()?;
                let load5 = parts[1].parse::<f64>()?;
                let load15 = parts[2].parse::<f64>()?;
                return Ok((load1, load5, load15));
            }
        }

        Err("Failed to get load average".into())
    }
}

#[derive(Debug)]
struct MemoryInfo {
    total: u64,
    used: u64,
}

#[derive(Debug)]
struct DiskInfo {
    total: u64,
    used: u64,
}

/// 健康检查配置
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    pub check_interval: Duration,
    pub timeout: Duration,
    pub enable_detailed_checks: bool,
    pub enable_system_metrics: bool,
    pub memory_threshold_mb: u64,
    pub disk_threshold_percent: f64,
    pub cpu_threshold_percent: f64,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(30),
            timeout: Duration::from_secs(10),
            enable_detailed_checks: true,
            enable_system_metrics: true,
            memory_threshold_mb: 1024, // 1GB
            disk_threshold_percent: 85.0,
            cpu_threshold_percent: 80.0,
        }
    }
}

/// 增强的健康检查管理器
pub struct EnhancedHealthCheckManager {
    checkers: Arc<RwLock<Vec<Arc<dyn HealthChecker>>>>,
    last_check: Arc<RwLock<Option<SystemHealthStatus>>>,
    config: HealthCheckConfig,
    metrics_registry: Option<Arc<crate::utils::metrics::MetricsRegistry>>,
    is_running: Arc<std::sync::atomic::AtomicBool>,
}

impl EnhancedHealthCheckManager {
    pub fn new(config: HealthCheckConfig) -> Self {
        Self {
            checkers: Arc::new(RwLock::new(Vec::new())),
            last_check: Arc::new(RwLock::new(None)),
            config,
            metrics_registry: None,
            is_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn with_metrics(mut self, registry: Arc<crate::utils::metrics::MetricsRegistry>) -> Self {
        self.metrics_registry = Some(registry);
        self
    }

    /// 注册健康检查器
    pub async fn register_checker(&self, checker: Arc<dyn HealthChecker>) {
        let component_name = checker.component_name().to_string();
        let mut checkers = self.checkers.write().await;
        checkers.push(checker);
        tracing::info!("Registered health checker: {}", component_name);
    }

    /// 执行所有健康检查
    #[tracing::instrument(skip(self))]
    pub async fn check_all(&self) -> SystemHealthStatus {
        let start_time = std::time::Instant::now();
        let checkers = self.checkers.read().await;

        tracing::debug!("Start executing {} health checks", checkers.len());

        // 并发执行所有健康检查
        let check_futures: Vec<_> = checkers
            .iter()
            .map(|checker| {
                let checker = checker.clone();
                let timeout_duration = self.config.timeout;
                async move {
                    let result =
                        tokio::time::timeout(timeout_duration, checker.check_health()).await;
                    match result {
                        Ok(health_result) => health_result,
                        Err(_) => {
                            tracing::warn!("Health check timeout: {}", checker.component_name());
                            HealthCheckResult::new(
                                checker.component_name().to_string(),
                                HealthStatus::Unhealthy,
                                "Health check timeout".to_string(),
                            )
                            .with_response_time(timeout_duration)
                        }
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(check_futures).await;

        let total_duration = start_time.elapsed();
        let status = SystemHealthStatus::new(results);

        // 更新指标
        if let Some(ref registry) = self.metrics_registry {
            self.update_health_metrics(registry, &status).await;
        }

        // 更新最后检查结果
        let mut last_check = self.last_check.write().await;
        *last_check = Some(status.clone());

        tracing::info!(
            overall_status = %status.overall_status,
            healthy_count = status.healthy_count,
            degraded_count = status.degraded_count,
            unhealthy_count = status.unhealthy_count,
            total_response_time_ms = status.total_response_time_ms,
            check_duration_ms = total_duration.as_millis(),
            "Health check completed"
        );

        status
    }

    /// 更新健康检查指标
    async fn update_health_metrics(
        &self,
        registry: &crate::utils::metrics::MetricsRegistry,
        status: &SystemHealthStatus,
    ) {
        // 更新健康检查计数器
        if let Some(counter) = registry.get_counter("health_checks_total").await {
            counter.inc();
        }

        // 更新组件状态指标
        for component in &status.components {
            let status_value = match component.status {
                HealthStatus::Healthy => 1,
                HealthStatus::Degraded => 2,
                HealthStatus::Unhealthy => 3,
                HealthStatus::Unknown => 0,
            };

            let mut labels = std::collections::HashMap::new();
            labels.insert("component".to_string(), component.component.clone());

            if let Some(gauge) = registry.get_gauge("health_check_status").await {
                gauge.set(status_value);
            }

            if let Some(histogram) = registry
                .get_histogram("health_check_duration_seconds")
                .await
            {
                histogram.observe(component.response_time_ms as f64 / 1000.0);
            }
        }

        // 更新整体状态
        let overall_status_value = match status.overall_status {
            HealthStatus::Healthy => 1,
            HealthStatus::Degraded => 2,
            HealthStatus::Unhealthy => 3,
            HealthStatus::Unknown => 0,
        };

        if let Some(gauge) = registry.get_gauge("health_check_overall_status").await {
            gauge.set(overall_status_value);
        }
    }

    /// 启动定期健康检查
    pub async fn start_periodic_checks(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self
            .is_running
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err("Health check manager is already running".into());
        }

        let checkers = self.checkers.clone();
        let last_check = self.last_check.clone();
        let config = self.config.clone();
        let metrics_registry = self.metrics_registry.clone();
        let is_running = self.is_running.clone();

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(config.check_interval);
            tracing::info!(
                "Start regular health check, interval: {:?}",
                config.check_interval
            );

            while is_running.load(std::sync::atomic::Ordering::SeqCst) {
                interval_timer.tick().await;

                let start_time = std::time::Instant::now();
                let checkers_guard = checkers.read().await;

                // 并发执行健康检查
                let check_futures: Vec<_> = checkers_guard
                    .iter()
                    .map(|checker| {
                        let checker = checker.clone();
                        let timeout_duration = config.timeout;
                        async move {
                            let result =
                                tokio::time::timeout(timeout_duration, checker.check_health())
                                    .await;
                            match result {
                                Ok(health_result) => health_result,
                                Err(_) => HealthCheckResult::new(
                                    checker.component_name().to_string(),
                                    HealthStatus::Unhealthy,
                                    "Health check timeout".to_string(),
                                )
                                .with_response_time(timeout_duration),
                            }
                        }
                    })
                    .collect();

                let results = futures::future::join_all(check_futures).await;
                drop(checkers_guard);

                let status = SystemHealthStatus::new(results);

                // 更新指标
                if let Some(ref registry) = metrics_registry {
                    if let Err(e) = Self::update_health_metrics_static(registry, &status).await {
                        tracing::warn!("Failed to update health check indicators: {}", e);
                    }
                }

                // 记录状态变化
                {
                    let mut last_check_guard = last_check.write().await;
                    if let Some(ref previous) = *last_check_guard {
                        if previous.overall_status != status.overall_status {
                            tracing::warn!(
                                previous_status = %previous.overall_status,
                                new_status = %status.overall_status,
                                "System health status changed"
                            );
                        }
                    }
                    *last_check_guard = Some(status);
                }

                let check_duration = start_time.elapsed();
                tracing::debug!(
                    "Regular health check completed, time taken: {:?}",
                    check_duration
                );
            }

            tracing::info!("Regular health checks have been stopped");
        });

        Ok(())
    }

    /// 静态方法更新健康检查指标
    async fn update_health_metrics_static(
        registry: &crate::utils::metrics::MetricsRegistry,
        status: &SystemHealthStatus,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 更新健康检查计数器
        if let Some(counter) = registry.get_counter("health_checks_total").await {
            counter.inc();
        }

        // 更新整体状态
        let overall_status_value = match status.overall_status {
            HealthStatus::Healthy => 1,
            HealthStatus::Degraded => 2,
            HealthStatus::Unhealthy => 3,
            HealthStatus::Unknown => 0,
        };

        if let Some(gauge) = registry.get_gauge("health_check_overall_status").await {
            gauge.set(overall_status_value);
        }

        Ok(())
    }

    /// 停止定期健康检查
    pub fn stop_periodic_checks(&self) {
        self.is_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("Stop regular health check-ups");
    }

    /// 获取最后的健康检查结果
    pub async fn get_last_check(&self) -> Option<SystemHealthStatus> {
        let last_check = self.last_check.read().await;
        last_check.clone()
    }

    /// 检查特定组件
    #[tracing::instrument(skip(self))]
    pub async fn check_component(&self, component_name: &str) -> Option<HealthCheckResult> {
        let checkers = self.checkers.read().await;

        for checker in checkers.iter() {
            if checker.component_name() == component_name {
                let result =
                    tokio::time::timeout(self.config.timeout, checker.check_health()).await;

                return match result {
                    Ok(health_result) => {
                        tracing::debug!(
                            component = component_name,
                            status = %health_result.status,
                            response_time_ms = health_result.response_time_ms,
                            "Component health check completed"
                        );
                        Some(health_result)
                    }
                    Err(_) => {
                        tracing::warn!("Component health check timeout: {}", component_name);
                        Some(
                            HealthCheckResult::new(
                                component_name.to_string(),
                                HealthStatus::Unhealthy,
                                "Health check timeout".to_string(),
                            )
                            .with_response_time(self.config.timeout),
                        )
                    }
                };
            }
        }

        None
    }

    /// 获取配置
    pub fn config(&self) -> &HealthCheckConfig {
        &self.config
    }

    /// 获取注册的检查器数量
    pub async fn get_checker_count(&self) -> usize {
        let checkers = self.checkers.read().await;
        checkers.len()
    }

    /// 是否正在运行
    pub fn is_running(&self) -> bool {
        self.is_running.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// 健康检查管理器（保持向后兼容）
pub struct HealthCheckManager {
    checkers: Arc<RwLock<Vec<Arc<dyn HealthChecker>>>>,
    last_check: Arc<RwLock<Option<SystemHealthStatus>>>,
    check_interval: Duration,
}

impl HealthCheckManager {
    pub fn new(check_interval: Duration) -> Self {
        Self {
            checkers: Arc::new(RwLock::new(Vec::new())),
            last_check: Arc::new(RwLock::new(None)),
            check_interval,
        }
    }

    /// 注册健康检查器
    pub async fn register_checker(&self, checker: Arc<dyn HealthChecker>) {
        let mut checkers = self.checkers.write().await;
        checkers.push(checker);
    }

    /// 执行所有健康检查
    pub async fn check_all(&self) -> SystemHealthStatus {
        let checkers = self.checkers.read().await;
        let mut results = Vec::new();

        for checker in checkers.iter() {
            let result = tokio::time::timeout(checker.timeout(), checker.check_health()).await;

            match result {
                Ok(health_result) => results.push(health_result),
                Err(_) => {
                    results.push(HealthCheckResult::new(
                        checker.component_name().to_string(),
                        HealthStatus::Unhealthy,
                        "Health check timeout".to_string(),
                    ));
                }
            }
        }

        let status = SystemHealthStatus::new(results);

        // 更新最后检查结果
        let mut last_check = self.last_check.write().await;
        *last_check = Some(status.clone());

        status
    }

    /// 获取最后的健康检查结果
    pub async fn get_last_check(&self) -> Option<SystemHealthStatus> {
        let last_check = self.last_check.read().await;
        last_check.clone()
    }

    /// 检查特定组件
    pub async fn check_component(&self, component_name: &str) -> Option<HealthCheckResult> {
        let checkers = self.checkers.read().await;

        for checker in checkers.iter() {
            if checker.component_name() == component_name {
                let result = tokio::time::timeout(checker.timeout(), checker.check_health()).await;

                return match result {
                    Ok(health_result) => Some(health_result),
                    Err(_) => Some(HealthCheckResult::new(
                        component_name.to_string(),
                        HealthStatus::Unhealthy,
                        "Health check timeout".to_string(),
                    )),
                };
            }
        }

        None
    }

    /// 启动定期健康检查
    pub async fn start_periodic_checks(&self) {
        let checkers = self.checkers.clone();
        let last_check = self.last_check.clone();
        let interval = self.check_interval;

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);

            loop {
                interval_timer.tick().await;

                let checkers_guard = checkers.read().await;
                let mut results = Vec::new();

                for checker in checkers_guard.iter() {
                    let result =
                        tokio::time::timeout(checker.timeout(), checker.check_health()).await;

                    match result {
                        Ok(health_result) => results.push(health_result),
                        Err(_) => {
                            results.push(HealthCheckResult::new(
                                checker.component_name().to_string(),
                                HealthStatus::Unhealthy,
                                "Health check timeout".to_string(),
                            ));
                        }
                    }
                }

                drop(checkers_guard);

                let status = SystemHealthStatus::new(results);
                let mut last_check_guard = last_check.write().await;
                *last_check_guard = Some(status);
            }
        });
    }

    /// 获取检查间隔
    pub fn get_check_interval(&self) -> Duration {
        self.check_interval
    }

    /// 获取注册的检查器数量
    pub async fn get_checker_count(&self) -> usize {
        let checkers = self.checkers.read().await;
        checkers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct MockHealthChecker {
        name: String,
        should_fail: Arc<AtomicBool>,
    }

    impl MockHealthChecker {
        fn new(name: String) -> Self {
            Self {
                name,
                should_fail: Arc::new(AtomicBool::new(false)),
            }
        }

        fn set_should_fail(&self, should_fail: bool) {
            self.should_fail.store(should_fail, Ordering::Relaxed);
        }
    }

    #[async_trait::async_trait]
    impl HealthChecker for MockHealthChecker {
        async fn check_health(&self) -> HealthCheckResult {
            let status = if self.should_fail.load(Ordering::Relaxed) {
                HealthStatus::Unhealthy
            } else {
                HealthStatus::Healthy
            };

            HealthCheckResult::new(self.name.clone(), status, "Mock health check".to_string())
        }

        fn component_name(&self) -> &str {
            &self.name
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(100)
        }
    }

    #[tokio::test]
    async fn test_health_check_result() {
        let mut result = HealthCheckResult::new(
            "test".to_string(),
            HealthStatus::Healthy,
            "Test message".to_string(),
        );

        assert!(result.is_healthy());
        assert!(!result.is_degraded());
        assert!(!result.is_unhealthy());

        result.add_detail("key".to_string(), "value".to_string());
        assert_eq!(result.details.get("key"), Some(&"value".to_string()));
    }

    #[tokio::test]
    async fn test_system_health_status() {
        let components = vec![
            HealthCheckResult::new(
                "component1".to_string(),
                HealthStatus::Healthy,
                "OK".to_string(),
            ),
            HealthCheckResult::new(
                "component2".to_string(),
                HealthStatus::Degraded,
                "Warning".to_string(),
            ),
            HealthCheckResult::new(
                "component3".to_string(),
                HealthStatus::Unhealthy,
                "Error".to_string(),
            ),
        ];

        let status = SystemHealthStatus::new(components);

        assert_eq!(status.overall_status, HealthStatus::Unhealthy);
        assert_eq!(status.healthy_count, 1);
        assert_eq!(status.degraded_count, 1);
        assert_eq!(status.unhealthy_count, 1);
        assert!(!status.is_healthy());
    }

    #[tokio::test]
    async fn test_health_check_manager() {
        let manager = HealthCheckManager::new(Duration::from_secs(60));

        let checker1 = Arc::new(MockHealthChecker::new("test1".to_string()));
        let checker2 = Arc::new(MockHealthChecker::new("test2".to_string()));

        manager.register_checker(checker1.clone()).await;
        manager.register_checker(checker2.clone()).await;

        assert_eq!(manager.get_checker_count().await, 2);

        // 测试所有检查器都健康
        let status = manager.check_all().await;
        assert_eq!(status.overall_status, HealthStatus::Healthy);
        assert_eq!(status.healthy_count, 2);

        // 设置一个检查器失败
        checker1.set_should_fail(true);
        let status = manager.check_all().await;
        assert_eq!(status.overall_status, HealthStatus::Unhealthy);
        assert_eq!(status.healthy_count, 1);
        assert_eq!(status.unhealthy_count, 1);

        // 测试单个组件检查
        let result = manager.check_component("test1").await;
        assert!(result.is_some());
        assert!(result.unwrap().is_unhealthy());
    }
}
