//! 服务器与日志配置及其校验。

use super::*;

/// 服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
}

impl ServerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.port == 0 {
            return Err(ConfigError::Validation {
                field: "server.port".to_string(),
                message: "端口号不能为0".to_string(),
            });
        }

        if self.host.is_empty() {
            return Err(ConfigError::Validation {
                field: "server.host".to_string(),
                message: "主机地址不能为空".to_string(),
            });
        }

        // 验证主机地址格式
        if !self
            .host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == ':' || c == '-')
        {
            return Err(ConfigError::Validation {
                field: "server.host".to_string(),
                message: "主机地址包含无效字符".to_string(),
            });
        }

        Ok(())
    }
}

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub level: String,
    pub path: String,
    /// The number of log files to retain (default: 20)
    #[serde(default = "default_retain_days")]
    pub retain_days: u32,
}

/// Default log files to retain
fn default_retain_days() -> u32 {
    20
}

impl LogConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        // 验证日志级别
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.level.to_lowercase().as_str()) {
            return Err(ConfigError::Validation {
                field: "log.level".to_string(),
                message: format!(
                    "无效的日志级别: {}，支持的级别: {:?}",
                    self.level, valid_levels
                ),
            });
        }

        if self.path.is_empty() {
            return Err(ConfigError::Validation {
                field: "log.path".to_string(),
                message: "日志路径不能为空".to_string(),
            });
        }

        // 验证路径是否可以创建
        let path = Path::new(&self.path);
        if let Some(parent) = path.parent()
            && parent.exists()
            && !parent.is_dir()
        {
            return Err(ConfigError::InvalidPath {
                path: self.path.clone(),
                message: "父目录不是一个有效的目录".to_string(),
            });
        }

        Ok(())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8077,
            host: "0.0.0.0".to_string(),
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            path: "logs".to_string(),
            retain_days: default_retain_days(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_validation() {
        let mut config = ServerConfig {
            port: 8080,
            host: "localhost".to_string(),
        };

        assert!(config.validate().is_ok());

        // 测试无效端口
        config.port = 0;
        assert!(config.validate().is_err());

        // 测试空主机
        config.port = 8080;
        config.host = "".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_log_config_validation() {
        let mut config = LogConfig {
            level: "info".to_string(),
            path: "/tmp/test.log".to_string(),
            retain_days: 20,
        };

        assert!(config.validate().is_ok());

        // 测试无效日志级别
        config.level = "invalid".to_string();
        assert!(config.validate().is_err());

        // 测试空路径
        config.level = "info".to_string();
        config.path = "".to_string();
        assert!(config.validate().is_err());
    }
}
