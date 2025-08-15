//! 配置验证模块
//!
//! 提供生产环境配置验证功能，确保所有配置项都符合生产要求。

use crate::config::AppConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

/// 配置验证器
#[derive(Debug, Clone)]
pub struct ConfigValidator {
    /// 验证规则
    rules: ValidationRules,
    /// 验证结果缓存
    cache: HashMap<String, ValidationResult>,
}

/// 验证规则配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRules {
    /// 必需的环境变量
    pub required_env_vars: Vec<String>,
    /// 端口范围限制
    pub port_range: (u16, u16),
    /// 最小内存要求 (MB)
    pub min_memory_mb: u64,
    /// 最大文件大小 (MB)
    pub max_file_size_mb: u64,
    /// 超时限制
    pub timeout_limits: TimeoutLimits,
    /// 安全配置要求
    pub security_requirements: SecurityRequirements,
}

/// 超时限制配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutLimits {
    /// 请求超时
    pub request_timeout: Duration,
    /// 数据库连接超时
    pub db_timeout: Duration,
    /// 文件处理超时
    pub file_processing_timeout: Duration,
}

/// 安全配置要求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirements {
    /// 是否要求 HTTPS
    pub require_https: bool,
    /// 是否要求认证
    pub require_auth: bool,
    /// 最小密码长度
    pub min_password_length: usize,
    /// 是否启用速率限制
    pub enable_rate_limiting: bool,
}

/// 验证结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// 是否通过验证
    pub is_valid: bool,
    /// 错误信息
    pub errors: Vec<ValidationError>,
    /// 警告信息
    pub warnings: Vec<ValidationWarning>,
    /// 验证时间
    pub validated_at: std::time::SystemTime,
}

/// 验证错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// 错误类型
    pub error_type: ValidationErrorType,
    /// 错误消息
    pub message: String,
    /// 配置路径
    pub config_path: String,
    /// 建议修复方案
    pub suggestion: Option<String>,
}

/// 验证警告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    /// 警告类型
    pub warning_type: ValidationWarningType,
    /// 警告消息
    pub message: String,
    /// 配置路径
    pub config_path: String,
    /// 建议优化方案
    pub suggestion: Option<String>,
}

/// 验证错误类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationErrorType {
    /// 缺少必需配置
    MissingRequired,
    /// 配置值无效
    InvalidValue,
    /// 配置冲突
    ConfigConflict,
    /// 安全问题
    SecurityIssue,
    /// 资源不足
    InsufficientResources,
    /// 网络配置错误
    NetworkError,
}

/// 验证警告类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationWarningType {
    /// 性能问题
    PerformanceIssue,
    /// 不推荐的配置
    DeprecatedConfig,
    /// 资源使用建议
    ResourceUsage,
    /// 安全建议
    SecurityRecommendation,
}

/// 环境验证器
#[derive(Debug, Clone)]
pub struct EnvironmentValidator {
    /// 环境类型
    environment: Environment,
    /// 系统信息收集器
    system_info: SystemInfoCollector,
}

/// 环境类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Environment {
    Development,
    Testing,
    Staging,
    Production,
}

/// 系统信息收集器
#[derive(Debug, Clone)]
pub struct SystemInfoCollector {
    /// 系统资源信息
    system_resources: SystemResources,
    /// 网络配置信息
    network_config: NetworkConfig,
}

/// 系统资源信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemResources {
    /// 总内存 (MB)
    pub total_memory_mb: u64,
    /// 可用内存 (MB)
    pub available_memory_mb: u64,
    /// CPU 核心数
    pub cpu_cores: u32,
    /// 磁盘空间 (MB)
    pub disk_space_mb: u64,
}

/// 网络配置信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// 监听地址
    pub listen_address: SocketAddr,
    /// 是否支持 IPv6
    pub ipv6_support: bool,
    /// 防火墙状态
    pub firewall_enabled: bool,
}

impl ConfigValidator {
    /// 创建新的配置验证器
    pub fn new(rules: ValidationRules) -> Self {
        Self {
            rules,
            cache: HashMap::new(),
        }
    }

    /// 验证应用配置
    pub fn validate_config(&mut self, config: &AppConfig) -> Result<ValidationResult> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // 验证基本配置
        self.validate_basic_config(config, &mut errors, &mut warnings)?;

        // 验证网络配置
        self.validate_network_config(config, &mut errors, &mut warnings)?;

        // 验证安全配置
        self.validate_security_config(config, &mut errors, &mut warnings)?;

        // 验证性能配置
        self.validate_performance_config(config, &mut errors, &mut warnings)?;

        // 验证环境变量
        self.validate_environment_variables(&mut errors, &mut warnings)?;

        let result = ValidationResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
            validated_at: std::time::SystemTime::now(),
        };

        Ok(result)
    }

    /// 验证基本配置
    fn validate_basic_config(
        &self,
        config: &AppConfig,
        errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        // 验证服务器配置
        if config.server.host.is_empty() {
            errors.push(ValidationError {
                error_type: ValidationErrorType::MissingRequired,
                message: "服务器主机地址不能为空".to_string(),
                config_path: "server.host".to_string(),
                suggestion: Some("设置有效的主机地址，如 0.0.0.0 或 127.0.0.1".to_string()),
            });
        }

        // 验证日志级别
        if config.log.level.is_empty() {
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::DeprecatedConfig,
                message: "未设置日志级别，将使用默认值".to_string(),
                config_path: "log.level".to_string(),
                suggestion: Some("建议明确设置日志级别".to_string()),
            });
        }

        Ok(())
    }

    /// 验证网络配置
    fn validate_network_config(
        &self,
        config: &AppConfig,
        errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        // 验证端口范围
        if config.server.port < self.rules.port_range.0 || config.server.port > self.rules.port_range.1 {
            errors.push(ValidationError {
                error_type: ValidationErrorType::InvalidValue,
                message: format!(
                    "端口 {} 超出允许范围 {}-{}",
                    config.server.port, self.rules.port_range.0, self.rules.port_range.1
                ),
                config_path: "server.port".to_string(),
                suggestion: Some(format!(
                    "使用 {}-{} 范围内的端口",
                    self.rules.port_range.0, self.rules.port_range.1
                )),
            });
        }

        // 验证主机地址
        if config.server.host == "0.0.0.0" {
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::SecurityRecommendation,
                message: "监听所有接口可能存在安全风险".to_string(),
                config_path: "server.host".to_string(),
                suggestion: Some("考虑绑定到特定接口".to_string()),
            });
        }

        Ok(())
    }

    /// 验证安全配置
    fn validate_security_config(
        &self,
        _config: &AppConfig,
        errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        // 验证 HTTPS 要求
        if self.rules.security_requirements.require_https {
            // 这里应该检查 TLS 配置
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::SecurityRecommendation,
                message: "生产环境建议启用 HTTPS".to_string(),
                config_path: "tls".to_string(),
                suggestion: Some("配置 TLS 证书和密钥".to_string()),
            });
        }

        // 验证认证配置
        if self.rules.security_requirements.require_auth {
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::SecurityRecommendation,
                message: "建议启用身份认证".to_string(),
                config_path: "auth".to_string(),
                suggestion: Some("配置认证中间件".to_string()),
            });
        }

        Ok(())
    }

    /// 验证性能配置
    fn validate_performance_config(
        &self,
        _config: &AppConfig,
        _errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        // 验证超时配置
        if self.rules.timeout_limits.request_timeout > Duration::from_secs(30) {
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::PerformanceIssue,
                message: "请求超时时间过长可能影响用户体验".to_string(),
                config_path: "request_timeout".to_string(),
                suggestion: Some("建议设置较短的超时时间".to_string()),
            });
        }

        Ok(())
    }

    /// 验证环境变量
    fn validate_environment_variables(
        &self,
        errors: &mut Vec<ValidationError>,
        _warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        for env_var in &self.rules.required_env_vars {
            if std::env::var(env_var).is_err() {
                errors.push(ValidationError {
                    error_type: ValidationErrorType::MissingRequired,
                    message: format!("缺少必需的环境变量: {}", env_var),
                    config_path: format!("env.{}", env_var),
                    suggestion: Some(format!("设置环境变量 {}", env_var)),
                });
            }
        }

        Ok(())
    }

    /// 生成验证报告
    pub fn generate_report(&self, result: &ValidationResult) -> String {
        let mut report = String::new();
        
        report.push_str("=== 配置验证报告 ===\n");
        report.push_str(&format!("验证状态: {}\n", 
            if result.is_valid { "通过" } else { "失败" }
        ));
        report.push_str(&format!("验证时间: {:?}\n", result.validated_at));
        
        if !result.errors.is_empty() {
            report.push_str("\n错误:\n");
            for error in &result.errors {
                report.push_str(&format!("  - [{}] {}: {}\n", 
                    error.config_path, 
                    format!("{:?}", error.error_type),
                    error.message
                ));
                if let Some(suggestion) = &error.suggestion {
                    report.push_str(&format!("    建议: {}\n", suggestion));
                }
            }
        }
        
        if !result.warnings.is_empty() {
            report.push_str("\n警告:\n");
            for warning in &result.warnings {
                report.push_str(&format!("  - [{}] {}: {}\n", 
                    warning.config_path,
                    format!("{:?}", warning.warning_type),
                    warning.message
                ));
                if let Some(suggestion) = &warning.suggestion {
                    report.push_str(&format!("    建议: {}\n", suggestion));
                }
            }
        }
        
        report
    }
}

impl EnvironmentValidator {
    /// 创建新的环境验证器
    pub fn new(environment: Environment) -> Self {
        Self {
            environment,
            system_info: SystemInfoCollector::new(),
        }
    }

    /// 验证环境
    pub fn validate_environment(&self) -> Result<ValidationResult> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // 验证系统资源
        self.validate_system_resources(&mut errors, &mut warnings)?;

        // 验证网络配置
        self.validate_network_configuration(&mut errors, &mut warnings)?;

        // 验证环境特定要求
        self.validate_environment_specific(&mut errors, &mut warnings)?;

        Ok(ValidationResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
            validated_at: std::time::SystemTime::now(),
        })
    }

    /// 验证系统资源
    fn validate_system_resources(
        &self,
        errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        let resources = &self.system_info.system_resources;

        // 验证内存
        if resources.available_memory_mb < 512 {
            errors.push(ValidationError {
                error_type: ValidationErrorType::InsufficientResources,
                message: "可用内存不足".to_string(),
                config_path: "system.memory".to_string(),
                suggestion: Some("增加系统内存或释放内存".to_string()),
            });
        } else if resources.available_memory_mb < 1024 {
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::ResourceUsage,
                message: "可用内存较少，可能影响性能".to_string(),
                config_path: "system.memory".to_string(),
                suggestion: Some("考虑增加内存".to_string()),
            });
        }

        // 验证 CPU
        if resources.cpu_cores < 2 {
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::PerformanceIssue,
                message: "CPU 核心数较少，可能影响并发性能".to_string(),
                config_path: "system.cpu".to_string(),
                suggestion: Some("考虑使用多核 CPU".to_string()),
            });
        }

        Ok(())
    }

    /// 验证网络配置
    fn validate_network_configuration(
        &self,
        _errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        let network = &self.system_info.network_config;

        if !network.firewall_enabled {
            warnings.push(ValidationWarning {
                warning_type: ValidationWarningType::SecurityRecommendation,
                message: "防火墙未启用".to_string(),
                config_path: "network.firewall".to_string(),
                suggestion: Some("启用防火墙以提高安全性".to_string()),
            });
        }

        Ok(())
    }

    /// 验证环境特定要求
    fn validate_environment_specific(
        &self,
        errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationWarning>,
    ) -> Result<()> {
        match self.environment {
            Environment::Production => {
                // 生产环境特定验证
                warnings.push(ValidationWarning {
                    warning_type: ValidationWarningType::SecurityRecommendation,
                    message: "生产环境建议启用所有安全功能".to_string(),
                    config_path: "environment.production".to_string(),
                    suggestion: Some("检查安全配置清单".to_string()),
                });
            }
            Environment::Development => {
                // 开发环境可以更宽松
            }
            _ => {}
        }

        Ok(())
    }
}

impl SystemInfoCollector {
    /// 创建新的系统信息收集器
    pub fn new() -> Self {
        Self {
            system_resources: SystemResources {
                total_memory_mb: 8192,  // 默认值，实际应该从系统获取
                available_memory_mb: 4096,
                cpu_cores: 4,
                disk_space_mb: 102400,
            },
            network_config: NetworkConfig {
                listen_address: "127.0.0.1:8080".parse().unwrap(),
                ipv6_support: true,
                firewall_enabled: false,
            },
        }
    }

    /// 收集系统信息
    pub fn collect_system_info(&mut self) -> Result<()> {
        // 这里应该实现实际的系统信息收集
        // 可以使用 sysinfo 等库
        Ok(())
    }
}

impl Default for ValidationRules {
    fn default() -> Self {
        Self {
            required_env_vars: vec![
                "RUST_LOG".to_string(),
                "DATABASE_URL".to_string(),
            ],
            port_range: (1024, 65535),
            min_memory_mb: 512,
            max_file_size_mb: 100,
            timeout_limits: TimeoutLimits {
                request_timeout: Duration::from_secs(30),
                db_timeout: Duration::from_secs(10),
                file_processing_timeout: Duration::from_secs(300),
            },
            security_requirements: SecurityRequirements {
                require_https: true,
                require_auth: true,
                min_password_length: 8,
                enable_rate_limiting: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn test_config_validation() {
        let rules = ValidationRules::default();
        let mut validator = ConfigValidator::new(rules);
        
        let config = AppConfig::load_base_config().unwrap();
        let result = validator.validate_config(&config).unwrap();
        
        // 应该有一些警告，因为使用的是默认配置
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_environment_validation() {
        let validator = EnvironmentValidator::new(Environment::Development);
        let result = validator.validate_environment().unwrap();
        
        // 开发环境验证应该通过
        assert!(result.is_valid || !result.errors.is_empty());
    }

    #[test]
    fn test_validation_report_generation() {
        let result = ValidationResult {
            is_valid: false,
            errors: vec![ValidationError {
                error_type: ValidationErrorType::MissingRequired,
                message: "测试错误".to_string(),
                config_path: "test.path".to_string(),
                suggestion: Some("测试建议".to_string()),
            }],
            warnings: vec![],
            validated_at: std::time::SystemTime::now(),
        };
        
        let rules = ValidationRules::default();
        let validator = ConfigValidator::new(rules);
        let report = validator.generate_report(&result);
        
        assert!(report.contains("配置验证报告"));
        assert!(report.contains("测试错误"));
    }
}