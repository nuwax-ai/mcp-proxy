//! OSS配置管理模块

use crate::error::{OssError, Result};
use serde::{Deserialize, Serialize};

/// 默认配置常量
pub mod defaults {
    pub const ENDPOINT: &str = "oss-rg-china-mainland.aliyuncs.com";
    pub const PUBLIC_BUCKET: &str = "nuwa-packages";
    pub const PRIVATE_BUCKET: &str = "edu-nuwa-packages";
    pub const REGION: &str = "oss-rg-china-mainland";
    pub const UPLOAD_DIRECTORY: &str = "edu";
}

/// OSS配置结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OssConfig {
    /// OSS endpoint (默认: oss-rg-china-mainland.aliyuncs.com)
    pub endpoint: String,
    /// 存储桶名称 (默认: nuwa-packages)
    pub bucket: String,
    /// 访问密钥ID (必须通过环境变量设置)
    pub access_key_id: String,
    /// 访问密钥Secret (必须通过环境变量设置)
    pub access_key_secret: String,
    /// 区域 (默认: oss-rg-china-mainland)
    pub region: String,
    /// 上传目录前缀 (默认: edu)
    pub upload_directory: String,
}

impl OssConfig {
    /// 创建自定义配置（所有字段需显式提供）
    pub fn new(
        endpoint: String,
        bucket: String,
        access_key_id: String,
        access_key_secret: String,
        region: String,
        upload_directory: String,
    ) -> Self {
        Self {
            endpoint,
            bucket,
            access_key_id,
            access_key_secret,
            region,
            upload_directory,
        }
    }

    /// 验证配置有效性
    pub fn validate(&self) -> Result<()> {
        if self.access_key_id.is_empty() {
            return Err(OssError::Config("access_key_id 不能为空".to_string()));
        }
        if self.access_key_secret.is_empty() {
            return Err(OssError::Config("access_key_secret 不能为空".to_string()));
        }
        if self.endpoint.is_empty() {
            return Err(OssError::Config("endpoint 不能为空".to_string()));
        }
        if self.bucket.is_empty() {
            return Err(OssError::Config("bucket 不能为空".to_string()));
        }
        if self.region.is_empty() {
            return Err(OssError::Config("region 不能为空".to_string()));
        }
        Ok(())
    }

    /// 获取完整的OSS URL
    pub fn get_base_url(&self) -> String {
        format!("https://{}.{}", self.bucket, self.endpoint)
    }

    /// 获取带前缀的object key
    pub fn get_prefixed_key(&self, key: &str) -> String {
        if key.starts_with(&self.upload_directory) {
            key.to_string()
        } else {
            format!("{}/{}", self.upload_directory, key.trim_start_matches('/'))
        }
    }
}
