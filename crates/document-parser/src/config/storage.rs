//! 存储域：sled 本地存储、阿里云 OSS 与外部集成配置及校验。

use super::*;

/// 存储配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageConfig {
    pub sled: SledConfig,
    pub oss: OssConfig,
}

impl StorageConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.sled.validate()?;
        self.oss.validate()?;
        Ok(())
    }
}

/// Sled数据库配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SledConfig {
    pub path: String,
    pub cache_capacity: usize,
}

impl SledConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.path.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.sled.path".to_string(),
                message: "数据库路径不能为空".to_string(),
            });
        }

        if self.cache_capacity == 0 {
            return Err(ConfigError::Validation {
                field: "storage.sled.cache_capacity".to_string(),
                message: "缓存容量不能为0".to_string(),
            });
        }

        Ok(())
    }
}

/// OSS配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OssConfig {
    pub endpoint: String,
    // pub bucket: String,
    /// 公有存储桶名称 (必填;为空时启动期 validate 报错,需在 config.yml 配置)
    pub public_bucket: String,
    /// 私有存储桶名称 (必填;为空时启动期 validate 报错,需在 config.yml 配置)
    pub private_bucket: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    /// 上传文件的统一子目录前缀
    #[serde(default = "default_upload_directory")]
    pub upload_directory: String,
    /// 区域 (默认: oss-rg-china-mainland)
    pub region: String,
}

/// 默认上传目录
fn default_upload_directory() -> String {
    "document_parser".to_string()
}

impl OssConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.endpoint.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.oss.endpoint".to_string(),
                message: "OSS端点不能为空".to_string(),
            });
        }

        if self.public_bucket.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.oss.public_bucket".to_string(),
                message: "OSS存储桶名称不能为空".to_string(),
            });
        }

        if self.private_bucket.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.oss.private_bucket".to_string(),
                message: "OSS存储桶名称不能为空".to_string(),
            });
        }
        // 注意：region 现在是可选的，可以为 None
        // 注意：access_key_id 和 access_key_secret 可以为空，因为它们可能通过环境变量设置

        Ok(())
    }

    /// 检查OSS配置是否完整（环境变量是否已设置）
    pub fn is_configured(&self) -> bool {
        !self.access_key_id.is_empty() && !self.access_key_secret.is_empty()
    }
}

/// 外部集成配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIntegrationConfig {
    pub webhook_url: String,
    pub api_key: String,
    pub timeout: u32,
}

impl ExternalIntegrationConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.timeout == 0 {
            return Err(ConfigError::Validation {
                field: "external_integration.timeout".to_string(),
                message: "超时时间不能为0".to_string(),
            });
        }

        // webhook_url 和 api_key 可以为空，因为外部集成是可选的
        if !self.webhook_url.is_empty() {
            // 简单的URL格式验证
            if !self.webhook_url.starts_with("http://") && !self.webhook_url.starts_with("https://")
            {
                return Err(ConfigError::Validation {
                    field: "external_integration.webhook_url".to_string(),
                    message: "Webhook URL必须以http://或https://开头".to_string(),
                });
            }
        }

        Ok(())
    }
}

impl Default for SledConfig {
    fn default() -> Self {
        Self {
            path: "data/document_parser".to_string(),
            cache_capacity: 104_857_600,
        }
    }
}

impl Default for OssConfig {
    fn default() -> Self {
        Self {
            endpoint: "oss-rg-china-mainland.aliyuncs.com".to_string(),
            public_bucket: String::new(),
            private_bucket: String::new(),
            access_key_id: String::new(),
            access_key_secret: String::new(),
            region: "oss-rg-china-mainland".to_string(),
            upload_directory: "document_parser".to_string(),
        }
    }
}

impl Default for ExternalIntegrationConfig {
    fn default() -> Self {
        Self {
            webhook_url: String::new(),
            api_key: String::new(),
            timeout: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_integration_config_validation() {
        let config = ExternalIntegrationConfig {
            webhook_url: "https://example.com/webhook".to_string(),
            api_key: "test-key".to_string(),
            timeout: 30,
        };

        assert!(config.validate().is_ok());

        // 测试无效URL
        let mut invalid_config = config.clone();
        invalid_config.webhook_url = "invalid-url".to_string();
        assert!(invalid_config.validate().is_err());

        // 测试零超时
        invalid_config.webhook_url = "https://example.com/webhook".to_string();
        invalid_config.timeout = 0;
        assert!(invalid_config.validate().is_err());
    }
}
