//! 公有Bucket客户端实现
//!
//! 专门用于处理公有bucket的公开访问服务，无需签名验证

use crate::OssClientTrait;
use crate::config::OssConfig;
use crate::error::{OssError, Result};
use crate::utils::{self, detect_mime_type, sanitize_filename};
use aliyun_oss_rust_sdk::oss::OSS;
use aliyun_oss_rust_sdk::request::RequestBuilder;
use aliyun_oss_rust_sdk::url::UrlApi;
use chrono::Utc;
use tracing::{info, warn};

/// 公有Bucket客户端
///
/// 专门用于处理公有bucket的公开访问服务
/// 所有操作都使用公有bucket，无需签名验证
#[derive(Debug)]
pub struct PublicOssClient {
    config: OssConfig,
}

impl PublicOssClient {
    /// 创建新的公有Bucket客户端
    pub fn new(config: OssConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    /// 获取配置信息
    pub fn get_config(&self) -> &OssConfig {
        &self.config
    }

    /// 获取公有bucket的基础URL
    pub fn get_base_url(&self) -> String {
        self.config.get_base_url()
    }

    /// 生成公有bucket的公开下载URL（无需签名，永久有效）
    ///
    /// # 参数
    /// * `object_key` - 对象键，如 "documents/manual.pdf"
    ///
    /// # 返回
    /// * 公开访问的下载URL，任何人都可以访问
    ///
    /// # 示例
    /// ```rust,no_run
    /// use oss_client::{PublicOssClient, OssConfig};
    ///
    /// let config = OssConfig::new(
    ///     "oss-rg-china-mainland.aliyuncs.com".to_string(),
    ///     "bucket".to_string(),
    ///     "".to_string(),
    ///     "".to_string(),
    ///     "oss-rg-china-mainland".to_string(),
    ///     "upload_directory".to_string(),
    /// );
    /// let client = PublicOssClient::new(config)?;
    /// let url = client.generate_public_download_url("documents/manual.pdf")?;
    /// println!("公开下载URL: {}", url);
    /// # Ok::<(), oss_client::OssError>(())
    /// ```
    pub fn generate_public_download_url(&self, object_key: &str) -> Result<String> {
        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 使用公有bucket生成公开URL
        let url = format!("{}/{}", self.get_base_url(), prefixed_key);
        let url = utils::replace_oss_domain(&url);
        info!("生成公有bucket公开下载URL: {}", url);
        Ok(url)
    }

    /// 生成公有bucket的公开访问URL（无需签名，永久有效）
    ///
    /// # 参数
    /// * `object_key` - 对象键，如 "images/logo.png"
    ///
    /// # 返回
    /// * 公开访问的URL，任何人都可以访问
    ///
    /// # 示例
    /// ```rust,no_run
    /// use oss_client::{PublicOssClient, OssConfig};
    ///
    /// let config = OssConfig::new(
    ///     "oss-rg-china-mainland.aliyuncs.com".to_string(),
    ///     "bucket".to_string(),
    ///     "".to_string(),
    ///     "".to_string(),
    ///     "oss-rg-china-mainland".to_string(),
    ///     "upload_directory".to_string(),
    /// );
    /// let client = PublicOssClient::new(config)?;
    /// let url = client.generate_public_access_url("images/logo.png")?;
    /// println!("公开访问URL: {}", url);
    /// # Ok::<(), oss_client::OssError>(())
    /// ```
    pub fn generate_public_access_url(&self, object_key: &str) -> Result<String> {
        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 使用公有bucket生成公开URL
        let url = format!("{}/{}", self.get_base_url(), prefixed_key);
        let url = utils::replace_oss_domain(&url);
        info!("生成公有bucket公开访问URL: {}", url);
        Ok(url)
    }

    /// 批量生成公有bucket的公开访问URL
    ///
    /// # 参数
    /// * `object_keys` - 对象键列表
    ///
    /// # 返回
    /// * 对象键到公开URL的映射
    ///
    /// # 示例
    /// ```rust,no_run
    /// use oss_client::{PublicOssClient, OssConfig};
    ///
    /// let config = OssConfig::new(
    ///     "oss-rg-china-mainland.aliyuncs.com".to_string(),
    ///     "bucket".to_string(),
    ///     "".to_string(),
    ///     "".to_string(),
    ///     "oss-rg-china-mainland".to_string(),
    ///     "upload_directory".to_string(),
    /// );
    /// let client = PublicOssClient::new(config)?;
    /// let keys = vec!["doc1.pdf", "doc2.pdf", "image.jpg"];
    /// let urls = client.generate_public_urls_batch(&keys)?;
    ///
    /// for (key, url) in urls {
    ///     println!("{}: {}", key, url);
    /// }
    /// # Ok::<(), oss_client::OssError>(())
    /// ```
    pub fn generate_public_urls_batch(
        &self,
        object_keys: &[&str],
    ) -> Result<std::collections::HashMap<String, String>> {
        let mut url_map = std::collections::HashMap::new();

        for &key in object_keys {
            let url = self.generate_public_access_url(key)?;
            url_map.insert(key.to_string(), url);
        }

        info!("批量生成公有bucket公开URL，共{}个", object_keys.len());
        Ok(url_map)
    }

    /// 获取公有bucket的基础信息
    ///
    /// # 返回
    /// * 包含bucket名称、endpoint等信息的字符串
    pub fn get_bucket_info(&self) -> String {
        format!(
            "Bucket: {} (Endpoint: {}, Region: {})",
            self.config.bucket, self.config.endpoint, self.config.region
        )
    }

    // 以下通用接口建议通过 OssClientTrait 使用

    /// 获取公有bucket中文件的元信息
    ///
    /// # 参数
    /// * `object_key` - 对象键
    ///
    /// # 返回
    /// * 文件元信息（如果存在）
    ///
    /// # 注意
    /// 这个方法通过HTTP HEAD请求获取文件元信息
    /// 由于是公有bucket，任何人都可以执行此操作
    pub async fn get_file_metadata(
        &self,
        object_key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>> {
        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 构建完整的URL
        let url = format!("{}/{}", self.get_base_url(), prefixed_key);
        let url = utils::replace_oss_domain(&url);

        // 使用HTTP HEAD请求获取文件元信息
        let client = reqwest::Client::new();
        let response = client
            .head(&url)
            .send()
            .await
            .map_err(|e| OssError::network(format!("获取文件元信息失败: {e}")))?;

        if response.status().is_success() {
            let headers = response.headers();
            let mut metadata = std::collections::HashMap::new();

            // 提取常用的元信息
            if let Some(content_length) = headers.get("content-length") {
                metadata.insert(
                    "content-length".to_string(),
                    content_length.to_str().unwrap_or("").to_string(),
                );
            }
            if let Some(content_type) = headers.get("content-type") {
                metadata.insert(
                    "content-type".to_string(),
                    content_type.to_str().unwrap_or("").to_string(),
                );
            }
            if let Some(last_modified) = headers.get("last-modified") {
                metadata.insert(
                    "last-modified".to_string(),
                    last_modified.to_str().unwrap_or("").to_string(),
                );
            }
            if let Some(etag) = headers.get("etag") {
                metadata.insert("etag".to_string(), etag.to_str().unwrap_or("").to_string());
            }

            info!(
                "获取公有bucket文件元信息: {} -> {}个字段",
                prefixed_key,
                metadata.len()
            );
            Ok(Some(metadata))
        } else {
            info!("公有bucket文件不存在: {}", prefixed_key);
            Ok(None)
        }
    }

    /// 生成唯一的object key
    ///
    /// # 参数
    /// * `prefix` - 前缀
    /// * `filename` - 原始文件名（可选）
    ///
    /// # 返回
    /// * 唯一的对象键
    pub fn generate_object_key(&self, prefix: &str, filename: Option<&str>) -> String {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let uid = uuid::Uuid::new_v4().to_string()[..8].to_string(); // 取前8位作为短UID

        // 如果有原始文件名，使用原始文件名和后缀
        let filename = if let Some(original) = filename {
            let clean_name = sanitize_filename(original);

            // 分离文件名和扩展名
            if let Some(dot_pos) = clean_name.rfind('.') {
                let name_part = &clean_name[..dot_pos];
                let ext_part = &clean_name[dot_pos..]; // 包含点号
                format!("{name_part}_{timestamp}_{uid}{ext_part}")
            } else {
                // 没有扩展名，保持原名
                format!("{clean_name}_{timestamp}_{uid}.")
            }
        } else {
            // 没有原始文件名，生成默认名称
            format!("file_{timestamp}_{uid}")
        };

        format!("{prefix}/{filename}")
    }
}

#[async_trait::async_trait]
impl OssClientTrait for PublicOssClient {
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
        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 创建请求构建器
        let mut builder = aliyun_oss_rust_sdk::request::RequestBuilder::new()
            .with_expire(expires_in.as_secs() as i64);

        // 设置Content-Type
        if let Some(ct) = content_type {
            builder = builder.with_content_type(ct);
        } else {
            builder = builder.with_content_type("application/octet-stream");
        }

        // 创建OSS客户端（使用公有bucket）
        let oss_client = aliyun_oss_rust_sdk::oss::OSS::new(
            &self.config.access_key_id,
            &self.config.access_key_secret,
            &self.config.endpoint,
            &self.config.bucket,
        );

        // 生成签名URL
        let url = oss_client.sign_upload_url(&prefixed_key, &builder);
        // 替换域名
        let url = utils::replace_oss_domain(&url);
        Ok(url)
    }

    fn generate_download_url(
        &self,
        object_key: &str,
        _expires_in: Option<std::time::Duration>,
    ) -> Result<String> {
        // 公有bucket的下载URL不需要签名，直接返回公开URL
        let prefixed_key = self.config.get_prefixed_key(object_key);
        let url = format!("{}/{}", self.get_base_url(), prefixed_key);
        let url = utils::replace_oss_domain(&url);
        Ok(url)
    }

    async fn upload_file(&self, local_path: &str, object_key: &str) -> Result<String> {
        // 检查文件是否存在
        if !std::path::Path::new(local_path).exists() {
            return Err(OssError::file_not_found(format!(
                "本地文件不存在: {local_path}"
            )));
        }

        // 检测MIME类型
        let content_type = detect_mime_type(local_path);

        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 创建OSS客户端（使用公有bucket）
        let oss_client = aliyun_oss_rust_sdk::oss::OSS::new(
            &self.config.access_key_id,
            &self.config.access_key_secret,
            &self.config.endpoint,
            &self.config.bucket,
        );

        // 创建请求构建器
        let builder = RequestBuilder::new().with_content_type(&content_type);

        // 执行上传
        let local_path_string = local_path.to_string();
        match oss_client
            .put_object_from_file(&prefixed_key, &local_path_string, builder)
            .await
        {
            Ok(_) => {
                let url = format!("{}/{}", self.get_base_url(), prefixed_key);
                let url = utils::replace_oss_domain(&url);
                Ok(url)
            }
            Err(e) => Err(OssError::sdk(format!("上传文件到公有bucket失败: {e}"))),
        }
    }

    async fn upload_content(
        &self,
        content: &[u8],
        object_key: &str,
        content_type: Option<&str>,
    ) -> Result<String> {
        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 创建临时文件
        let temp_file = tempfile::NamedTempFile::new().map_err(|e| OssError::io_error(e.to_string()))?;

        // 写入内容到临时文件
        tokio::fs::write(temp_file.path(), content)
            .await
            .map_err(|e| OssError::io_error(e.to_string()))?;

        // 创建OSS客户端（使用公有bucket）
        let oss_client = OSS::new(
            &self.config.access_key_id,
            &self.config.access_key_secret,
            &self.config.endpoint,
            &self.config.bucket,
        );

        // 创建请求构建器
        let mut builder = RequestBuilder::new();
        if let Some(ct) = content_type {
            builder = builder.with_content_type(ct);
        } else {
            builder = builder.with_content_type("application/octet-stream");
        }

        // 执行上传
        let temp_path_string = temp_file.path().to_str().unwrap().to_string();
        match oss_client
            .put_object_from_file(&prefixed_key, &temp_path_string, builder)
            .await
        {
            Ok(_) => {
                let url = format!("{}/{}", self.get_base_url(), prefixed_key);
                let url = utils::replace_oss_domain(&url);
                Ok(url)
            }
            Err(e) => Err(OssError::sdk(format!("上传内容到公有bucket失败: {e}"))),
        }
    }

    async fn delete_file(&self, object_key: &str) -> Result<()> {
        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 创建OSS客户端（使用公有bucket）
        let oss_client = OSS::new(
            &self.config.access_key_id,
            &self.config.access_key_secret,
            &self.config.endpoint,
            &self.config.bucket,
        );

        // 创建请求构建器
        let builder = RequestBuilder::new();

        // 执行删除
        match oss_client.delete_object(&prefixed_key, builder).await {
            Ok(_) => Ok(()),
            Err(e) => Err(OssError::sdk(format!("删除公有bucket文件失败: {e}"))),
        }
    }

    async fn file_exists(&self, object_key: &str) -> Result<bool> {
        // 获取带前缀的object key
        let prefixed_key = self.config.get_prefixed_key(object_key);

        // 创建OSS客户端（使用公有bucket）
        let oss_client = OSS::new(
            &self.config.access_key_id,
            &self.config.access_key_secret,
            &self.config.endpoint,
            &self.config.bucket,
        );

        // 创建请求构建器
        let builder = RequestBuilder::new();

        // 使用 HEAD 获取元信息来检查是否存在（避免下载主体）
        match oss_client.get_object_metadata(&prefixed_key, builder).await {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("检查文件存在性失败: {}", e);
                Ok(false)
            }
        }
    }

    async fn test_connection(&self) -> Result<()> {
        // 通过尝试上传一个小的测试文件来验证连接
        let test_key = format!("health-check-{}", chrono::Utc::now().timestamp_millis());
        let test_content = b"OSS connection test";

        match <PublicOssClient as OssClientTrait>::upload_content(
            self,
            test_content,
            &test_key,
            Some("text/plain"),
        )
        .await
        {
            Ok(_) => {
                // 尝试删除测试文件（忽略删除失败）
                let _ = <PublicOssClient as OssClientTrait>::delete_file(self, &test_key).await;
                Ok(())
            }
            Err(e) => Err(OssError::network(format!("无法连接到公有bucket: {e}"))),
        }
    }

    fn generate_object_key(&self, prefix: &str, filename: Option<&str>) -> String {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_client_creation() {
        let config = OssConfig::new(
            crate::config::defaults::ENDPOINT.to_string(),
            crate::config::defaults::PUBLIC_BUCKET.to_string(),
            "test_key_id".to_string(),
            "test_key_secret".to_string(),
            crate::config::defaults::REGION.to_string(),
            crate::config::defaults::UPLOAD_DIRECTORY.to_string(),
        );
        let client = PublicOssClient::new(config).unwrap();
        assert_eq!(client.get_config().bucket, "nuwa-packages");
    }

    #[test]
    fn test_generate_public_download_url() {
        let config = OssConfig::new(
            crate::config::defaults::ENDPOINT.to_string(),
            crate::config::defaults::PUBLIC_BUCKET.to_string(),
            "test_key_id".to_string(),
            "test_key_secret".to_string(),
            crate::config::defaults::REGION.to_string(),
            crate::config::defaults::UPLOAD_DIRECTORY.to_string(),
        );
        let client = PublicOssClient::new(config).unwrap();

        let url = client
            .generate_public_download_url("test/file.txt")
            .unwrap();
        // 验证URL包含正确的路径，但域名可能被替换为自定义域名
        assert!(url.contains("edu/test/file.txt"));
        // 由于 replace_oss_domain 可能替换域名，我们只验证路径部分
        // 公开URL不应该包含签名参数
        assert!(!url.contains("Expires="));
        assert!(!url.contains("Signature="));
    }

    #[test]
    fn test_generate_public_access_url() {
        let config = OssConfig::new(
            crate::config::defaults::ENDPOINT.to_string(),
            crate::config::defaults::PUBLIC_BUCKET.to_string(),
            "test_key_id".to_string(),
            "test_key_secret".to_string(),
            crate::config::defaults::REGION.to_string(),
            crate::config::defaults::UPLOAD_DIRECTORY.to_string(),
        );
        let client = PublicOssClient::new(config).unwrap();

        let url = client.generate_public_access_url("test/image.jpg").unwrap();
        // 验证URL包含正确的路径，但域名可能被替换为自定义域名
        assert!(url.contains("edu/test/image.jpg"));
        // 由于 replace_oss_domain 可能替换域名，我们只验证路径部分
        // 公开URL不应该包含签名参数
        assert!(!url.contains("Expires="));
        assert!(!url.contains("Signature="));
    }

    #[test]
    fn test_generate_public_urls_batch() {
        let config = OssConfig::new(
            crate::config::defaults::ENDPOINT.to_string(),
            crate::config::defaults::PUBLIC_BUCKET.to_string(),
            "test_key_id".to_string(),
            "test_key_secret".to_string(),
            crate::config::defaults::REGION.to_string(),
            crate::config::defaults::UPLOAD_DIRECTORY.to_string(),
        );
        let client = PublicOssClient::new(config).unwrap();

        let keys = vec!["doc1.pdf", "doc2.pdf", "image.jpg"];
        let urls = client.generate_public_urls_batch(&keys).unwrap();

        assert_eq!(urls.len(), 3);
        for (key, url) in urls {
            // 验证URL包含正确的路径，但域名可能被替换为自定义域名
            assert!(url.contains(&format!("edu/{key}")));
            // 由于 replace_oss_domain 可能替换域名，我们只验证路径部分
        }
    }

    #[test]
    fn test_get_bucket_info() {
        let config = OssConfig::new(
            crate::config::defaults::ENDPOINT.to_string(),
            crate::config::defaults::PUBLIC_BUCKET.to_string(),
            "test_key_id".to_string(),
            "test_key_secret".to_string(),
            crate::config::defaults::REGION.to_string(),
            crate::config::defaults::UPLOAD_DIRECTORY.to_string(),
        );
        let bucket = config.bucket.clone();
        let endpoint = config.endpoint.clone();
        let region = config.region.clone();
        let client = PublicOssClient::new(config).unwrap();

        let info = client.get_bucket_info();
        // 验证信息包含配置的bucket和endpoint，但不硬编码具体的值
        assert!(info.contains(&bucket));
        assert!(info.contains(&endpoint));
        assert!(info.contains(&region));
    }

    #[test]
    fn test_generate_object_key() {
        let config = OssConfig::new(
            crate::config::defaults::ENDPOINT.to_string(),
            crate::config::defaults::PUBLIC_BUCKET.to_string(),
            "test_key_id".to_string(),
            "test_key_secret".to_string(),
            crate::config::defaults::REGION.to_string(),
            crate::config::defaults::UPLOAD_DIRECTORY.to_string(),
        );
        let client = PublicOssClient::new(config).unwrap();

        // 测试生成带文件名的对象键
        let key1 = client.generate_object_key("documents", Some("manual.pdf"));
        assert!(key1.starts_with("documents/"));
        assert!(key1.contains("manual"));
        assert!(key1.ends_with(".pdf"));

        // 测试生成不带文件名的对象键
        let key2 = client.generate_object_key("images", None);
        assert!(key2.starts_with("images/"));
        assert!(key2.contains("file_"));
    }
}
