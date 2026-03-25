//! 私有Bucket客户端实现

use aliyun_oss_rust_sdk::oss::OSS;
use aliyun_oss_rust_sdk::request::RequestBuilder;
use aliyun_oss_rust_sdk::url::UrlApi;
use chrono::Utc;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tracing::warn;

use crate::OssClientTrait;
use crate::config::OssConfig;
use crate::error::{OssError, Result};
use crate::utils::{self, detect_mime_type, sanitize_filename};

/// 私有OSS客户端（签名访问）
#[derive(Debug)]
pub struct PrivateOssClient {
    client: OSS,
    config: OssConfig,
}

impl PrivateOssClient {
    /// 创建新的私有OSS客户端
    pub fn new(config: OssConfig) -> Result<Self> {
        config.validate()?;

        let client = OSS::new(
            &config.access_key_id,
            &config.access_key_secret,
            &config.endpoint,
            &config.bucket,
        );

        Ok(Self { client, config })
    }

    /// 获取配置信息
    pub fn get_config(&self) -> &OssConfig {
        &self.config
    }

    /// 获取基础URL
    pub fn get_base_url(&self) -> String {
        self.config.get_base_url()
    }
}

#[async_trait::async_trait]
impl OssClientTrait for PrivateOssClient {
    fn get_config(&self) -> &OssConfig {
        &self.config
    }

    fn get_base_url(&self) -> String {
        self.config.get_base_url()
    }

    fn generate_upload_url(
        &self,
        object_key: &str,
        expires_in: std::time::Duration,
        content_type: Option<&str>,
    ) -> Result<String> {
        let prefixed_key = self.config.get_prefixed_key(object_key);

        let mut builder = RequestBuilder::new().with_expire(expires_in.as_secs() as i64);
        if let Some(ct) = content_type {
            builder = builder.with_content_type(ct);
        } else {
            builder = builder.with_content_type("application/octet-stream");
        }

        let url = self.client.sign_upload_url(&prefixed_key, &builder);
        let url = utils::replace_oss_domain(&url);
        Ok(url)
    }

    fn generate_download_url(
        &self,
        object_key: &str,
        expires_in: Option<std::time::Duration>,
    ) -> Result<String> {
        let prefixed_key = self.config.get_prefixed_key(object_key);
        let duration = expires_in.unwrap_or_else(|| Duration::from_secs(7 * 24 * 3600));
        let builder = RequestBuilder::new().with_expire(duration.as_secs() as i64);
        let url = self.client.sign_download_url(&prefixed_key, &builder);
        let url = utils::replace_oss_domain(&url);
        Ok(url)
    }

    async fn upload_file(&self, local_path: &str, object_key: &str) -> Result<String> {
        if !Path::new(local_path).exists() {
            return Err(OssError::file_not_found(format!(
                "本地文件不存在: {local_path}"
            )));
        }

        let content_type = detect_mime_type(local_path);
        let prefixed_key = self.config.get_prefixed_key(object_key);
        let builder = RequestBuilder::new().with_content_type(&content_type);

        let local_path_string = local_path.to_string();
        match self
            .client
            .put_object_from_file(&prefixed_key, &local_path_string, builder)
            .await
        {
            Ok(_) => {
                let url = format!("{}/{}", self.get_base_url(), prefixed_key);
                let url = utils::replace_oss_domain(&url);
                Ok(url)
            }
            Err(e) => Err(OssError::sdk(format!("上传文件失败: {e}"))),
        }
    }

    async fn upload_content(
        &self,
        content: &[u8],
        object_key: &str,
        content_type: Option<&str>,
    ) -> Result<String> {
        let prefixed_key = self.config.get_prefixed_key(object_key);
        let temp_file = tempfile::NamedTempFile::new().map_err(|e| OssError::io_error(e.to_string()))?;
        fs::write(temp_file.path(), content)
            .await
            .map_err(|e| OssError::io_error(e.to_string()))?;

        let mut builder = RequestBuilder::new();
        if let Some(ct) = content_type {
            builder = builder.with_content_type(ct);
        } else {
            builder = builder.with_content_type("application/octet-stream");
        }

        let temp_path_string = temp_file.path().to_str().unwrap().to_string();
        match self
            .client
            .put_object_from_file(&prefixed_key, &temp_path_string, builder)
            .await
        {
            Ok(_) => {
                let url = format!("{}/{}", self.get_base_url(), prefixed_key);
                let url = utils::replace_oss_domain(&url);
                Ok(url)
            }
            Err(e) => Err(OssError::sdk(format!("上传内容失败: {e}"))),
        }
    }

    async fn delete_file(&self, object_key: &str) -> Result<()> {
        let prefixed_key = self.config.get_prefixed_key(object_key);
        let builder = RequestBuilder::new();
        match self.client.delete_object(&prefixed_key, builder).await {
            Ok(_) => Ok(()),
            Err(e) => Err(OssError::sdk(format!("删除文件失败: {e}"))),
        }
    }

    async fn file_exists(&self, object_key: &str) -> Result<bool> {
        let prefixed_key = self.config.get_prefixed_key(object_key);
        let builder = RequestBuilder::new();
        // 使用 HEAD 请求检查对象是否存在，避免下载主体
        match self
            .client
            .get_object_metadata(&prefixed_key, builder)
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("检查文件存在性失败: {}", e);
                Ok(false)
            }
        }
    }

    async fn test_connection(&self) -> Result<()> {
        let test_key = format!("health-check-{}", chrono::Utc::now().timestamp_millis());
        let test_content = b"OSS connection test";

        match <PrivateOssClient as OssClientTrait>::upload_content(
            self,
            test_content,
            &test_key,
            Some("text/plain"),
        )
        .await
        {
            Ok(_) => {
                let _ = <PrivateOssClient as OssClientTrait>::delete_file(self, &test_key).await;
                Ok(())
            }
            Err(e) => Err(OssError::network(format!("无法连接到OSS: {e}"))),
        }
    }

    fn generate_object_key(&self, prefix: &str, filename: Option<&str>) -> String {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let uid = uuid::Uuid::new_v4().to_string()[..8].to_string();

        let filename = if let Some(original) = filename {
            let clean_name = sanitize_filename(original);
            if let Some(dot_pos) = clean_name.rfind('.') {
                let name_part = &clean_name[..dot_pos];
                let ext_part = &clean_name[dot_pos..];
                format!("{name_part}_{timestamp}_{uid}{ext_part}")
            } else {
                format!("{clean_name}_{timestamp}_{uid}.")
            }
        } else {
            format!("file_{timestamp}_{uid}")
        };

        format!("{prefix}/{filename}")
    }
}
