//! 存储域：sled 本地存储、阿里云 OSS 与外部集成配置及校验。

use super::*;

/// 存储配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageConfig {
    pub sled: SledConfig,
    pub oss: OssConfig,
    /// 自定义文件上传后端（nuwax 风格 REST API）全局默认；
    /// `base_url` 为空 = 未启用（走 OSS）。请求级字段可逐项覆盖。
    #[serde(default)]
    pub custom_upload: CustomUploadConfig,
}

impl StorageConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.sled.validate()?;
        self.oss.validate()?;
        self.custom_upload.validate()?;
        Ok(())
    }
}

/// 自定义文件上传后端配置（nuwax 风格）
///
/// 语义：`base_url` 非空时，未携带任何 `upload_*` 请求字段的请求也默认
/// 走自定义后端（运维显式 opt-in）；整段缺省/为空时行为与纯 OSS 完全一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomUploadConfig {
    /// 服务基地址，如 `https://agent.example.com`；为空 = 未启用
    #[serde(default)]
    pub base_url: String,
    /// API Key（Bearer）；可为空（部分部署不做鉴权）
    #[serde(default)]
    pub api_key: String,
    /// 上传接口路径，默认 nuwax 契约 [`oss_client::DEFAULT_UPLOAD_PATH`]
    #[serde(default = "default_custom_upload_path")]
    pub path: String,
}

fn default_custom_upload_path() -> String {
    oss_client::DEFAULT_UPLOAD_PATH.to_string()
}

impl Default for CustomUploadConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            path: default_custom_upload_path(),
        }
    }
}

impl CustomUploadConfig {
    /// 校验：仅在启用（base_url 非空）时检查；空段必须通过（存量部署兼容）
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.base_url.trim().is_empty() {
            return Ok(());
        }
        let field = "storage.custom_upload";
        Self::validate_base_url(&self.base_url).map_err(|message| ConfigError::Validation {
            field: format!("{field}.base_url"),
            message,
        })?;
        oss_client::validate_upload_path(&self.path).map_err(|message| {
            ConfigError::Validation {
                field: format!("{field}.path"),
                message,
            }
        })?;
        Ok(())
    }

    /// 校验 base_url：必须是合法 http(s) URL、包含主机名，且不得携带 query/fragment
    ///
    /// query/fragment 会被端点 URL 的字符串拼接吞进错误位置（整个 path 变成
    /// query/fragment，请求打到根路径），因此必须在此处拒绝。
    /// 该函数同时被 [`crate::services::upload_backend`] 的请求级解析复用。
    pub fn validate_base_url(base_url: &str) -> std::result::Result<(), String> {
        let trimmed = base_url.trim_end_matches('/');
        let parsed = url::Url::parse(trimmed)
            .map_err(|e| format!("自定义上传 base_url 无效（{trimmed}）: {e}"))?;
        if parsed.host_str().is_none() || !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!(
                "自定义上传 base_url 必须是 http(s) URL 且包含主机名: {trimmed}"
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(format!(
                "自定义上传 base_url 不得携带 query 或 fragment（{trimmed}），\
                 请只传 scheme://host[:port][/prefix]"
            ));
        }
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

/// config.yml 模板里 OSS bucket 的占位符值。OSS 密钥已配置（后端选了 OSS）
/// 而 bucket 仍是这些值时，异步上传要到运行期才炸 E010——启动期
/// [`crate::config::AppConfig::cross_validate`] 会拒绝这种组合。
pub const OSS_BUCKET_PLACEHOLDERS: &[&str] = &["your-public-bucket", "your-private-bucket"];

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

impl OssConfig {
    /// OSS 密钥是否实际生效：模板的 `${OSS_ACCESS_KEY_*}` 字面量或空值都算未配置
    /// （真实密钥经 `load_oss_config_from_env` 的环境变量覆盖注入）。
    pub fn keys_effective(&self) -> bool {
        !self.access_key_id.is_empty()
            && self.access_key_id != "${OSS_ACCESS_KEY_ID}"
            && !self.access_key_secret.is_empty()
            && self.access_key_secret != "${OSS_ACCESS_KEY_SECRET}"
    }
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

    #[test]
    fn test_custom_upload_config_validation() {
        // 空段（默认）必须通过——存量部署兼容
        assert!(CustomUploadConfig::default().validate().is_ok());

        // 合法配置
        let config = CustomUploadConfig {
            base_url: "https://agent.example.com".to_string(),
            api_key: "ak-xxx".to_string(),
            path: "/api/v1/file/upload".to_string(),
        };
        assert!(config.validate().is_ok());

        // path 不以 / 开头
        let mut invalid = config.clone();
        invalid.path = "api/v1/file/upload".to_string();
        assert!(invalid.validate().is_err());

        // path 含 query/fragment
        invalid.path = "/api?x=1".to_string();
        assert!(invalid.validate().is_err());
        invalid.path = "/api#f".to_string();
        assert!(invalid.validate().is_err());

        // base_url 非 http(s)
        invalid.path = "/api/v1/file/upload".to_string();
        invalid.base_url = "ftp://x.com".to_string();
        assert!(invalid.validate().is_err());

        // base_url 无 host
        invalid.base_url = "https://".to_string();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_storage_config_with_custom_upload_serde_default() {
        // 旧 config.yml 无 custom_upload 段 → 反序列化为默认（未启用）
        let yaml = r#"
sled:
  path: data/document_parser
  cache_capacity: 104857600
oss:
  endpoint: oss-rg-china-mainland.aliyuncs.com
  public_bucket: b
  private_bucket: b
  access_key_id: ""
  access_key_secret: ""
  region: oss-rg-china-mainland
  upload_directory: document_parser
"#;
        let config: StorageConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.custom_upload.base_url.is_empty());
        assert_eq!(config.custom_upload.path, "/api/v1/file/upload");
        assert!(config.validate().is_ok());
    }
}
