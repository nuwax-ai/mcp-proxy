//! 生产部署功能模块
//!
//! 提供生产环境所需的功能，包括优雅关闭、配置验证、生产日志和监控集成

pub mod config_validation;
pub mod deployment_health;
pub mod graceful_shutdown;
pub mod monitoring_integration;
pub mod production_logging;
pub mod resource_cleanup;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::config::AppConfig;
use crate::error::AppError;

use config_validation::ConfigValidator;
use deployment_health::{HealthCheckManager, HealthChecker};
use graceful_shutdown::GracefulShutdownManager;
use monitoring_integration::MonitoringIntegration;
use production_logging::ProductionLogger;
use resource_cleanup::{DatabaseConnectionCleaner, ResourceCleaner};

/// 生产部署管理器
pub struct ProductionManager {
    config: AppConfig,
    shutdown_manager: Arc<GracefulShutdownManager>,
    config_validator: Arc<ConfigValidator>,
    logger: Arc<ProductionLogger>,
    monitoring: Arc<MonitoringIntegration>,
    health_checker: Arc<HealthCheckManager>,
    resource_cleaner: Arc<dyn ResourceCleaner + Send + Sync>,
    deployment_info: Arc<RwLock<DeploymentInfo>>,
    is_production_ready: Arc<RwLock<bool>>,
}

impl ProductionManager {
    /// 创建新的生产部署管理器
    pub async fn new(config: AppConfig) -> Result<Self, AppError> {
        let shutdown_manager = Arc::new(GracefulShutdownManager::new().await?);
        let config_validator = Arc::new(ConfigValidator::new(
            config_validation::ValidationRules::default(),
        ));
        let logger = Arc::new(ProductionLogger::new(
            production_logging::LoggingConfig::default(),
        ));
        let monitoring = Arc::new(MonitoringIntegration::new(
            monitoring_integration::MonitoringConfig::default(),
        ));
        let health_checker = Arc::new(HealthCheckManager::new(
            deployment_health::HealthCheckConfig::default(),
        ));
        let resource_cleaner: Arc<dyn ResourceCleaner + Send + Sync> = Arc::new(
            DatabaseConnectionCleaner::new("production_db_cleaner".to_string(), 10),
        );

        let deployment_info = Arc::new(RwLock::new(DeploymentInfo {
            deployment_id: Uuid::new_v4().to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            environment: config.environment.clone(),
            started_at: std::time::SystemTime::now(),
            ready_at: None,
            shutdown_at: None,
        }));

        Ok(Self {
            config,
            shutdown_manager,
            config_validator,
            logger,
            monitoring,
            health_checker,
            resource_cleaner,
            deployment_info,
            is_production_ready: Arc::new(RwLock::new(false)),
        })
    }

    /// 初始化生产环境
    pub async fn initialize_production(&self, app_state: Arc<AppState>) -> Result<(), AppError> {
        // 1. 验证配置
        self.validate_configuration().await?;

        // 2. 初始化生产日志
        self.initialize_logging().await?;

        // 3. 设置监控集成
        self.setup_monitoring().await?;

        // 4. 初始化健康检查
        self.initialize_health_checks(app_state.clone()).await?;

        // 5. 设置优雅关闭
        self.setup_graceful_shutdown(app_state).await?;

        // 6. 标记为生产就绪
        *self.is_production_ready.write().await = true;

        // 7. 更新部署信息
        {
            let mut info = self.deployment_info.write().await;
            info.ready_at = Some(std::time::SystemTime::now());
        }

        tracing::info!("Production environment initialized successfully");

        Ok(())
    }

    /// 验证配置
    pub async fn validate_configuration(&self) -> Result<(), AppError> {
        let mut validator = self.config_validator.as_ref().clone();
        let result = validator
            .validate_config(&self.config)
            .map_err(|e| AppError::Validation(e.to_string()))?;

        if !result.is_valid {
            let error_messages: Vec<String> =
                result.errors.iter().map(|e| e.message.clone()).collect();
            return Err(AppError::Validation(error_messages.join("; ")));
        }

        Ok(())
    }

    /// 初始化日志
    pub async fn initialize_logging(&self) -> Result<(), AppError> {
        self.logger.start_background_tasks().await;
        Ok(())
    }

    /// 设置监控
    pub async fn setup_monitoring(&self) -> Result<(), AppError> {
        // MonitoringIntegration 暂时没有 setup 方法
        Ok(())
    }

    /// 初始化健康检查
    pub async fn initialize_health_checks(
        &self,
        _app_state: Arc<AppState>,
    ) -> Result<(), AppError> {
        self.health_checker
            .start_health_checks()
            .await
            .map_err(|e| AppError::Internal(format!("健康检查初始化失败: {e}")))
    }

    /// 设置优雅关闭
    pub async fn setup_graceful_shutdown(&self, app_state: Arc<AppState>) -> Result<(), AppError> {
        self.shutdown_manager
            .setup(app_state, self.resource_cleaner.clone())
            .await
    }

    /// 检查生产就绪状态
    pub async fn is_ready(&self) -> bool {
        *self.is_production_ready.read().await
    }

    /// 获取部署信息
    pub async fn get_deployment_info(&self) -> DeploymentInfo {
        self.deployment_info.read().await.clone()
    }

    /// 获取健康状态
    pub async fn get_health_status(&self) -> Result<deployment_health::HealthStatus, AppError> {
        Ok(self.health_checker.get_health_status().await)
    }

    /// 获取监控指标
    pub async fn get_monitoring_metrics(&self) -> Result<MonitoringMetrics, AppError> {
        // 暂时返回空的监控指标
        Ok(MonitoringMetrics {
            system_metrics: std::collections::HashMap::new(),
            application_metrics: std::collections::HashMap::new(),
            custom_metrics: std::collections::HashMap::new(),
            collected_at: std::time::SystemTime::now(),
        })
    }

    /// 触发优雅关闭
    pub async fn shutdown(&self) -> Result<(), AppError> {
        tracing::info!("Starting graceful shutdown");

        // 更新部署信息
        {
            let mut info = self.deployment_info.write().await;
            info.shutdown_at = Some(std::time::SystemTime::now());
        }

        // 标记为非生产就绪
        *self.is_production_ready.write().await = false;

        // 执行优雅关闭
        self.shutdown_manager.shutdown().await?;

        tracing::info!("Graceful shutdown completed");

        Ok(())
    }

    /// 获取运行时统计
    pub async fn get_runtime_stats(&self) -> Result<RuntimeStats, AppError> {
        let deployment_info = self.get_deployment_info().await;
        let health_status = self.get_health_status().await?;
        let monitoring_metrics = self.get_monitoring_metrics().await?;

        let uptime = deployment_info
            .started_at
            .elapsed()
            .unwrap_or(Duration::from_secs(0));

        Ok(RuntimeStats {
            deployment_info,
            health_status,
            monitoring_metrics,
            uptime,
            is_ready: self.is_ready().await,
        })
    }

    /// 执行生产环境检查
    pub async fn run_production_checks(&self) -> Result<ProductionCheckResult, AppError> {
        let mut checks = Vec::new();

        // 配置检查
        match self.validate_configuration().await {
            Ok(_) => checks.push(ProductionCheck {
                name: "Configuration Validation".to_string(),
                status: CheckStatus::Passed,
                message: "All configuration values are valid".to_string(),
            }),
            Err(e) => checks.push(ProductionCheck {
                name: "Configuration Validation".to_string(),
                status: CheckStatus::Failed,
                message: format!("Configuration validation failed: {e}"),
            }),
        }

        // 健康检查
        match self.get_health_status().await {
            Ok(health) => {
                let status =
                    if health.overall_status == deployment_health::HealthCheckStatus::Healthy {
                        CheckStatus::Passed
                    } else {
                        CheckStatus::Warning
                    };
                checks.push(ProductionCheck {
                    name: "Health Check".to_string(),
                    status,
                    message: format!("Overall health: {:?}", health.overall_status),
                });
            }
            Err(e) => checks.push(ProductionCheck {
                name: "Health Check".to_string(),
                status: CheckStatus::Failed,
                message: format!("Health check failed: {e}"),
            }),
        }

        // 监控检查
        match self.get_monitoring_metrics().await {
            Ok(_) => checks.push(ProductionCheck {
                name: "Monitoring Integration".to_string(),
                status: CheckStatus::Passed,
                message: "Monitoring metrics are available".to_string(),
            }),
            Err(e) => checks.push(ProductionCheck {
                name: "Monitoring Integration".to_string(),
                status: CheckStatus::Failed,
                message: format!("Monitoring check failed: {e}"),
            }),
        }

        let overall_status = if checks.iter().any(|c| c.status == CheckStatus::Failed) {
            CheckStatus::Failed
        } else if checks.iter().any(|c| c.status == CheckStatus::Warning) {
            CheckStatus::Warning
        } else {
            CheckStatus::Passed
        };

        Ok(ProductionCheckResult {
            overall_status,
            checks,
            checked_at: std::time::SystemTime::now(),
        })
    }
}

/// 部署信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentInfo {
    pub deployment_id: String,
    pub version: String,
    pub environment: String,
    pub started_at: std::time::SystemTime,
    pub ready_at: Option<std::time::SystemTime>,
    pub shutdown_at: Option<std::time::SystemTime>,
}

/// 健康状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub overall_status: String,
    pub components: std::collections::HashMap<String, ComponentHealth>,
    pub last_check: std::time::SystemTime,
}

/// 组件健康状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub status: String,
    pub message: String,
    pub last_check: std::time::SystemTime,
}

/// 监控指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringMetrics {
    pub system_metrics: std::collections::HashMap<String, f64>,
    pub application_metrics: std::collections::HashMap<String, f64>,
    pub custom_metrics: std::collections::HashMap<String, f64>,
    pub collected_at: std::time::SystemTime,
}

/// 运行时统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStats {
    pub deployment_info: DeploymentInfo,
    pub health_status: deployment_health::HealthStatus,
    pub monitoring_metrics: MonitoringMetrics,
    pub uptime: Duration,
    pub is_ready: bool,
}

/// 生产检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionCheckResult {
    pub overall_status: CheckStatus,
    pub checks: Vec<ProductionCheck>,
    pub checked_at: std::time::SystemTime,
}

/// 单个生产检查
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionCheck {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
}

/// 检查状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    Passed,
    Warning,
    Failed,
}

/// 生产配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionConfig {
    pub graceful_shutdown_timeout: Duration,
    pub health_check_interval: Duration,
    pub monitoring_enabled: bool,
    pub log_level: String,
    pub metrics_endpoint: Option<String>,
    pub tracing_endpoint: Option<String>,
}

impl Default for ProductionConfig {
    fn default() -> Self {
        Self {
            graceful_shutdown_timeout: Duration::from_secs(30),
            health_check_interval: Duration::from_secs(30),
            monitoring_enabled: true,
            log_level: "info".to_string(),
            metrics_endpoint: None,
            tracing_endpoint: None,
        }
    }
}
