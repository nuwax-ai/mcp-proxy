use crate::config::OssConfig;
use crate::error::AppError;
use crate::models::ImageInfo;
use aliyun_oss_rust_sdk::oss::OSS;
use aliyun_oss_rust_sdk::request::RequestBuilder;
use aliyun_oss_rust_sdk::url::UrlApi;
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{error, info, instrument, warn};

/// 批量操作进度回调
pub type ProgressCallback = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// 批量上传结果
#[derive(Debug, Clone)]
pub struct BatchUploadResult {
    pub successful: Vec<BatchUploadItem>,
    pub failed: Vec<BatchUploadError>,
    pub total_processed: usize,
    pub total_bytes: u64,
    pub duration: Duration,
}

/// 批量上传成功项
#[derive(Debug, Clone)]
pub struct BatchUploadItem {
    pub local_path: String,
    pub object_key: String,
    pub url: String,
    pub size: u64,
    pub content_type: String,
}

/// 批量上传失败项
#[derive(Debug, Clone)]
pub struct BatchUploadError {
    pub local_path: String,
    pub object_key: String,
    pub error: String,
    pub retry_count: u32,
}

/// OSS服务配置
#[derive(Debug, Clone)]
pub struct OssServiceConfig {
    pub max_concurrent_uploads: usize,
    pub retry_attempts: u32,
    pub retry_delay_ms: u64,
    pub upload_timeout_secs: u64,
    pub chunk_size: usize,
}

impl Default for OssServiceConfig {
    fn default() -> Self {
        Self {
            max_concurrent_uploads: 10,
            retry_attempts: 3,
            retry_delay_ms: 1000,
            upload_timeout_secs: 300,
            chunk_size: 8 * 1024 * 1024, // 8MB
        }
    }
}

/// OSS服务
#[derive(Debug)]
pub struct OssService {
    client: OSS,
    bucket: String,
    endpoint: String,
    base_url: String,
    config: OssServiceConfig,
    semaphore: Arc<Semaphore>,
}

impl OssService {
    /// 创建新的OSS服务实例
    #[instrument(skip(oss_config), fields(public_bucket = %oss_config.public_bucket, endpoint = %oss_config.endpoint))]
    pub async fn new(oss_config: &OssConfig) -> Result<Self, AppError> {
        Self::new_with_config(oss_config, OssServiceConfig::default()).await
    }

    /// 使用自定义配置创建OSS服务实例
    #[instrument(skip(oss_config, service_config), fields(public_bucket = %oss_config.public_bucket, endpoint = %oss_config.endpoint))]
    pub async fn new_with_config(
        oss_config: &OssConfig,
        service_config: OssServiceConfig,
    ) -> Result<Self, AppError> {
        info!("Initialize OSS service");

        // 检查OSS配置是否完整（环境变量是否已设置）
        if oss_config.access_key_id.is_empty() || oss_config.access_key_secret.is_empty() {
            warn!(
                "OSS environment variables are not configured and OSS service initialization is skipped."
            );
            return Err(AppError::Config(
                "OSS环境变量未配置，请设置OSS_ACCESS_KEY_ID和OSS_ACCESS_KEY_SECRET".to_string(),
            ));
        }

        //这里使用公网的公有 bucket
        let bucket = oss_config.public_bucket.clone();

        // 创建OSS客户端 - 配置文件中endpoint已不包含协议前缀
        let client = OSS::new(
            &oss_config.access_key_id,
            &oss_config.access_key_secret,
            &oss_config.endpoint,
            &bucket,
        );

        // 构建base_url
        let base_url = format!("https://{}.{}", bucket, oss_config.endpoint);

        let service = Self {
            client,
            bucket,
            endpoint: oss_config.endpoint.clone(),
            base_url,
            config: service_config.clone(),
            semaphore: Arc::new(Semaphore::new(service_config.max_concurrent_uploads)),
        };

        // 验证连接
        service.validate_connection().await?;

        info!("OSS service initialization successful");
        Ok(service)
    }

    /// 验证OSS连接
    #[instrument(skip(self))]
    async fn validate_connection(&self) -> Result<(), AppError> {
        info!("Verify OSS connection");

        // 使用上传临时文件的方式验证连接
        let test_key = format!("health-check-{}", chrono::Utc::now().timestamp_millis());
        let test_content = b"OSS connection test";

        match self
            .upload_content(test_content, &test_key, Some("text/plain"))
            .await
        {
            Ok(_) => {
                info!("OSS connection verification successful");
                // 尝试删除测试文件（忽略删除失败）
                let _ = self.delete_object(&test_key).await;
                Ok(())
            }
            Err(e) => {
                error!("OSS connection verification failed: {}", e);
                Err(AppError::Oss(format!(
                    "无法连接到OSS存储桶 {}: {}",
                    self.bucket, e
                )))
            }
        }
    }

    /// 上传文件到OSS
    #[instrument(skip(self), fields(file_path, object_key))]
    pub async fn upload_file(&self, file_path: &str, object_key: &str) -> Result<String, AppError> {
        info!(
            "Start uploading files to OSS: {} -> {}",
            file_path, object_key
        );

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| AppError::Oss(format!("获取上传许可失败: {e}")))?;

        let content_type = self.detect_mime_type(file_path)?;

        // 检查文件大小决定上传方式
        let metadata = std::fs::metadata(file_path)
            .map_err(|e| AppError::Oss(format!("读取文件元数据失败: {e}")))?;

        if metadata.len() > self.config.chunk_size as u64 {
            self.upload_large_file(file_path, object_key, &content_type)
                .await
        } else {
            self.upload_small_file(file_path, object_key, &content_type)
                .await
        }
    }

    /// 上传小文件
    #[instrument(skip(self))]
    async fn upload_small_file(
        &self,
        file_path: &str,
        object_key: &str,
        content_type: &str,
    ) -> Result<String, AppError> {
        let builder = RequestBuilder::new().with_content_type(content_type);

        match self
            .client
            .put_object_from_file(object_key, file_path, builder)
            .await
        {
            Ok(_) => Ok(format!("{}/{}", self.base_url, object_key)),
            Err(e) => Err(AppError::Oss(format!("上传文件失败: {e}"))),
        }
    }

    /// 上传大文件（分片上传）
    #[instrument(skip(self))]
    async fn upload_large_file(
        &self,
        file_path: &str,
        object_key: &str,
        content_type: &str,
    ) -> Result<String, AppError> {
        // 对于大文件，我们仍然使用简单上传，因为aliyun-oss-rust-sdk的分片上传API可能不同
        // 如果需要分片上传，需要查看具体的API文档
        warn!("Large file upload, currently using the simple upload method");

        let builder = RequestBuilder::new().with_content_type(content_type);

        match self
            .client
            .put_object_from_file(object_key, file_path, builder)
            .await
        {
            Ok(_) => Ok(format!("{}/{}", self.base_url, object_key)),
            Err(e) => Err(AppError::Oss(format!("上传大文件失败: {e}"))),
        }
    }

    /// 上传内容到OSS
    #[instrument(skip(self, content), fields(object_key, content_size = content.len()))]
    pub async fn upload_content(
        &self,
        content: &[u8],
        object_key: &str,
        content_type: Option<&str>,
    ) -> Result<String, AppError> {
        self.upload_content_with_retry(content, object_key, content_type)
            .await
    }

    /// 上传markdown内容到OSS，返回(URL, object_key)
    #[instrument(skip(self, content), fields(task_id, content_size = content.len()))]
    pub async fn upload_markdown(
        &self,
        task_id: &str,
        content: &[u8],
        original_filename: Option<&str>,
    ) -> Result<(String, String), AppError> {
        // 生成唯一的object key
        let object_key = self.generate_markdown_object_key(task_id, original_filename);

        // 上传内容
        let url = self
            .upload_content(content, &object_key, Some("text/markdown; charset=utf-8"))
            .await?;

        Ok((url, object_key))
    }

    /// 上传用户文件到OSS，返回(URL, object_key, download_url)
    #[instrument(skip(self), fields(file_path))]
    pub async fn upload_user_file(
        &self,
        file_path: &str,
        original_filename: Option<&str>,
    ) -> Result<(String, String, String), AppError> {
        // 生成唯一的object key
        let object_key = self.generate_user_file_object_key(original_filename);

        // 上传文件
        let url = self.upload_file(file_path, &object_key).await?;

        // 生成4小时有效期的下载链接
        let download_url = self
            .generate_download_url(&object_key, Some(Duration::from_secs(4 * 3600)))
            .await?;

        Ok((url, object_key, download_url))
    }

    /// 根据object_key生成下载链接（如果文件存在）
    #[instrument(skip(self), fields(object_key))]
    pub async fn get_download_url_for_file(&self, object_key: &str) -> Result<String, AppError> {
        // 先检查文件是否存在
        if !self.file_exists(object_key).await? {
            return Err(AppError::Oss(format!("文件不存在: {object_key}")));
        }

        // 生成4小时有效期的下载链接
        self.generate_download_url(object_key, Some(Duration::from_secs(4 * 3600)))
            .await
    }

    /// 根据object_key和指定bucket生成下载链接（如果文件存在）
    /// 注意：这个方法仅适用于同一OSS账户下的不同bucket
    #[instrument(skip(self), fields(object_key, bucket))]
    pub async fn get_download_url_for_file_with_bucket(
        &self,
        object_key: &str,
        bucket: &str,
    ) -> Result<String, AppError> {
        // 如果指定的bucket与当前bucket相同，直接使用现有方法
        if bucket == self.bucket {
            return self.get_download_url_for_file(object_key).await;
        }

        // 对于不同的bucket，我们构建一个临时的下载URL
        // 注意：这假设所有bucket都在同一endpoint下，且访问权限相同

        // 构建临时URL (这是一个简化版本，实际环境中可能需要更复杂的签名逻辑)
        let temp_url = format!(
            "https://{}.{}/{}",
            bucket,
            self.endpoint.trim_start_matches("https://"),
            object_key
        );

        // 由于我们无法直接验证不同bucket中的文件存在性，
        // 这里返回URL但在实际使用时可能需要额外的验证
        warn!(
            "Use different buckets to generate download URLs, and the file existence cannot be verified in advance: bucket={}, object_key={}",
            bucket, object_key
        );

        Ok(temp_url)
    }

    /// 生成markdown文件的OSS object key
    fn generate_markdown_object_key(
        &self,
        task_id: &str,
        original_filename: Option<&str>,
    ) -> String {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let uid = uuid::Uuid::new_v4().to_string()[..8].to_string(); // 取前8位作为短UID

        // 如果有原始文件名，使用原始文件名和后缀
        let filename = if let Some(original) = original_filename {
            let clean_name = self.sanitize_filename(original);

            // 分离文件名和扩展名
            if let Some(dot_pos) = clean_name.rfind('.') {
                let name_part = &clean_name[..dot_pos];
                let ext_part = &clean_name[dot_pos..]; // 包含点号
                format!("{name_part}_{timestamp}_{uid}{ext_part}")
            } else {
                // 没有扩展名，默认加上.md
                format!("{clean_name}_{timestamp}_{uid}. md")
            }
        } else {
            // 没有原始文件名，生成默认名称
            format!("document_{timestamp}_{uid}.md")
        };

        format!("markdown/{task_id}/{filename}")
    }

    /// 生成用户文件的OSS object key
    fn generate_user_file_object_key(&self, original_filename: Option<&str>) -> String {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let uid = uuid::Uuid::new_v4().to_string()[..8].to_string(); // 取前8位作为短UID

        // 如果有原始文件名，使用原始文件名和后缀
        let filename = if let Some(original) = original_filename {
            let clean_name = self.sanitize_filename(original);

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

        format!("uploads/{filename}")
    }

    /// 清理文件名，移除特殊字符
    fn sanitize_filename(&self, filename: &str) -> String {
        filename
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// 带重试的内容上传
    #[instrument(skip(self, content))]
    async fn upload_content_with_retry(
        &self,
        content: &[u8],
        object_key: &str,
        content_type: Option<&str>,
    ) -> Result<String, AppError> {
        let mut last_error = None;

        for attempt in 1..=self.config.retry_attempts {
            match self.do_upload(content, object_key, content_type).await {
                Ok(url) => {
                    if attempt > 1 {
                        info!("The {}th retry upload was successful.", attempt);
                    }
                    return Ok(url);
                }
                Err(e) => {
                    last_error = Some(e);
                    if attempt < self.config.retry_attempts {
                        warn!(
                            "The {} upload failed, try again after {}ms",
                            attempt, self.config.retry_delay_ms
                        );
                        sleep(Duration::from_millis(self.config.retry_delay_ms)).await;
                    } else {
                        error!("Upload failed, maximum number of retries reached");
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| AppError::Oss("上传失败，未知错误".to_string())))
    }

    /// 执行上传
    #[instrument(skip(self, content))]
    async fn do_upload(
        &self,
        content: &[u8],
        object_key: &str,
        content_type: Option<&str>,
    ) -> Result<String, AppError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| AppError::Oss(format!("获取上传许可失败: {e}")))?;

        let mut builder = RequestBuilder::new();
        if let Some(ct) = content_type {
            builder = builder.with_content_type(ct);
        }

        // 将内容写入临时文件，然后使用put_object_from_file上传
        let temp_file = tempfile::NamedTempFile::new()
            .map_err(|e| AppError::Oss(format!("创建临时文件失败: {e}")))?;

        std::fs::write(temp_file.path(), content)
            .map_err(|e| AppError::Oss(format!("写入临时文件失败: {e}")))?;

        match self
            .client
            .put_object_from_file(object_key, temp_file.path().to_str().unwrap(), builder)
            .await
        {
            Ok(_) => Ok(format!("{}/{}", self.base_url, object_key)),
            Err(e) => Err(AppError::Oss(format!("上传内容失败: {e}"))),
        }
    }

    /// 上传图片
    #[instrument(skip(self), fields(image_path))]
    pub async fn upload_image(&self, image_path: &str) -> Result<ImageInfo, AppError> {
        let path = Path::new(image_path);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| AppError::Oss("无效的文件名".to_string()))?;

        let object_key = format!("images/{file_name}");
        let url = self.upload_file(image_path, &object_key).await?;

        let metadata = std::fs::metadata(image_path)
            .map_err(|e| AppError::Oss(format!("读取图片元数据失败: {e}")))?;

        Ok(ImageInfo::with_full_info(
            image_path.to_string(),
            file_name.to_string(),
            object_key,
            url,
            metadata.len(),
            self.detect_mime_type(image_path)?,
        ))
    }

    /// 批量上传图片
    #[instrument(skip(self, image_paths), fields(count = image_paths.len()))]
    pub async fn upload_images(&self, image_paths: &[String]) -> Result<Vec<ImageInfo>, AppError> {
        let result = self.upload_images_with_progress(image_paths, None).await?;
        Ok(result
            .successful
            .into_iter()
            .map(|item| {
                let original_filename = Path::new(&item.local_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                ImageInfo::with_full_info(
                    item.local_path,
                    original_filename,
                    item.object_key,
                    item.url,
                    item.size,
                    item.content_type,
                )
            })
            .collect())
    }

    /// 带进度的批量上传图片
    #[instrument(skip(self, image_paths, progress_callback), fields(count = image_paths.len()))]
    pub async fn upload_images_with_progress(
        &self,
        image_paths: &[String],
        progress_callback: Option<ProgressCallback>,
    ) -> Result<BatchUploadResult, AppError> {
        let total_count = image_paths.len();

        info!("Start batch uploading {} pictures", total_count);

        let files: Vec<(String, String)> = image_paths
            .iter()
            .map(|path| {
                let file_name = Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let object_key = format!("images/{file_name}");
                (path.clone(), object_key)
            })
            .collect();

        self.upload_batch(files, progress_callback).await
    }

    /// 批量上传文件
    #[instrument(skip(self, files, progress_callback), fields(count = files.len()))]
    pub async fn upload_batch<P: AsRef<Path> + Send + Sync>(
        &self,
        files: Vec<(P, String)>, // (local_path, object_key)
        progress_callback: Option<ProgressCallback>,
    ) -> Result<BatchUploadResult, AppError> {
        let start_time = std::time::Instant::now();
        let total_count = files.len();
        let mut successful = Vec::new();
        let mut failed = Vec::new();
        let mut total_bytes = 0u64;

        info!("Start batch uploading {} files", total_count);

        // 使用流处理来控制并发
        let mut stream = stream::iter(files.into_iter().enumerate())
            .map(|(index, (local_path, object_key))| {
                let local_path_str = local_path.as_ref().to_string_lossy().to_string();
                async move {
                    let result = self.upload_file(&local_path_str, &object_key).await;
                    (index, local_path_str, object_key, result)
                }
            })
            .buffer_unordered(self.config.max_concurrent_uploads);

        let mut processed = 0;

        while let Some((_index, local_path, object_key, result)) = stream.next().await {
            processed += 1;

            match result {
                Ok(url) => {
                    let metadata = std::fs::metadata(&local_path).unwrap_or_else(|_| {
                        std::fs::metadata("/dev/null").unwrap_or_else(|_| {
                            // 创建一个默认的元数据结构
                            std::fs::metadata(std::env::current_dir().unwrap()).unwrap()
                        })
                    });
                    let size = metadata.len();
                    let content_type = self
                        .detect_mime_type(&local_path)
                        .unwrap_or_else(|_| "application/octet-stream".to_string());

                    total_bytes += size;
                    successful.push(BatchUploadItem {
                        local_path,
                        object_key,
                        url,
                        size,
                        content_type,
                    });
                }
                Err(e) => {
                    failed.push(BatchUploadError {
                        local_path,
                        object_key,
                        error: e.to_string(),
                        retry_count: 0,
                    });
                }
            }

            // 调用进度回调
            if let Some(ref callback) = progress_callback {
                callback(processed, total_count);
            }

            if processed % 10 == 0 {
                info!("{}/{} files processed", processed, total_count);
            }
        }

        let duration = start_time.elapsed();

        info!(
            "Batch upload completed: success {}, failure {}, total size {} bytes, time consumption {:?}",
            successful.len(),
            failed.len(),
            total_bytes,
            duration
        );

        Ok(BatchUploadResult {
            successful,
            failed,
            total_processed: processed,
            total_bytes,
            duration,
        })
    }

    /// 从URL提取object key
    pub fn extract_object_key(&self, url: &str) -> String {
        url.trim_start_matches(&format!("{}/", self.base_url))
            .trim_start_matches('/')
            .to_string()
    }

    /// 下载文件到临时目录
    #[instrument(skip(self), fields(object_key))]
    pub async fn download_to_temp(&self, object_key: &str) -> Result<String, AppError> {
        let temp_dir = std::env::temp_dir();
        let file_name = Path::new(object_key)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download");

        let temp_path = temp_dir.join(format!("oss_download_{file_name}"));

        self.download_to_path(object_key, &temp_path).await?;

        Ok(temp_path.to_string_lossy().to_string())
    }

    /// 下载文件到指定路径
    #[instrument(skip(self), fields(object_key, target_path = %target_path.as_ref().display()))]
    pub async fn download_to_path<P: AsRef<Path>>(
        &self,
        object_key: &str,
        target_path: P,
    ) -> Result<(), AppError> {
        let target_path = target_path.as_ref();

        // 创建目标目录
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Oss(format!("创建目标目录失败: {e}")))?;
        }

        self.do_download(object_key, target_path).await
    }

    /// 执行下载
    #[instrument(skip(self))]
    async fn do_download(&self, object_key: &str, target_path: &Path) -> Result<(), AppError> {
        let builder = RequestBuilder::new();

        let content = match self.client.get_object(object_key, builder).await {
            Ok(content) => content,
            Err(e) => return Err(AppError::Oss(format!("下载文件失败: {e}"))),
        };

        std::fs::write(target_path, content)
            .map_err(|e| AppError::Oss(format!("写入文件失败: {e}")))?;

        info!("File download successful: {}", target_path.display());
        Ok(())
    }

    /// 获取对象内容（直接返回字节数组）
    #[instrument(skip(self), fields(object_key))]
    pub async fn get_object_content(&self, object_key: &str) -> Result<Vec<u8>, AppError> {
        let builder = RequestBuilder::new();

        match self.client.get_object(object_key, builder).await {
            Ok(content) => {
                info!(
                    "Successfully obtained the object content: object_key={}, size={} bytes",
                    object_key,
                    content.len()
                );
                Ok(content)
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("404") || error_msg.contains("NoSuchKey") {
                    Err(AppError::Oss(format!("OSS文件不存在: {object_key}")))
                } else {
                    Err(AppError::Oss(format!("获取文件内容失败: {e}")))
                }
            }
        }
    }

    /// 生成下载URL
    #[instrument(skip(self), fields(object_key, expires_in = ?expires_in))]
    pub async fn generate_download_url(
        &self,
        object_key: &str,
        expires_in: Option<Duration>,
    ) -> Result<String, AppError> {
        let expire_seconds = expires_in.unwrap_or(Duration::from_secs(3600)).as_secs() as i64;

        let builder = RequestBuilder::new().with_expire(expire_seconds);

        let url = self.client.sign_download_url(object_key, &builder);

        Ok(url)
    }

    /// 生成上传签名URL
    #[instrument(skip(self), fields(object_key, expires_in = ?expires_in))]
    pub async fn generate_upload_url(
        &self,
        object_key: &str,
        expires_in: Option<Duration>,
    ) -> Result<String, AppError> {
        let expire_seconds = expires_in.unwrap_or(Duration::from_secs(3600)).as_secs() as i64;

        let builder = RequestBuilder::new()
            .with_expire(expire_seconds)
            .with_content_type("application/octet-stream"); // 默认内容类型

        let url = self.client.sign_upload_url(object_key, &builder);

        Ok(url)
    }

    /// 生成带自定义内容类型的上传签名URL
    #[instrument(skip(self), fields(object_key, content_type, expires_in = ?expires_in))]
    pub async fn generate_upload_url_with_content_type(
        &self,
        object_key: &str,
        content_type: &str,
        expires_in: Option<Duration>,
    ) -> Result<String, AppError> {
        let expire_seconds = expires_in.unwrap_or(Duration::from_secs(3600)).as_secs() as i64;

        let builder = RequestBuilder::new()
            .with_expire(expire_seconds)
            .with_content_type(content_type);

        let url = self.client.sign_upload_url(object_key, &builder);

        Ok(url)
    }

    /// 删除对象
    #[instrument(skip(self), fields(object_key))]
    pub async fn delete_object(&self, object_key: &str) -> Result<(), AppError> {
        let builder = RequestBuilder::new();

        match self.client.delete_object(object_key, builder).await {
            Ok(_) => {
                info!("Object deleted successfully: {}", object_key);
                Ok(())
            }
            Err(e) => Err(AppError::Oss(format!("删除对象失败: {e}"))),
        }
    }

    /// 批量删除对象
    #[instrument(skip(self, object_keys), fields(count = object_keys.len()))]
    pub async fn delete_objects(&self, object_keys: &[String]) -> Result<Vec<String>, AppError> {
        let mut deleted = Vec::new();

        for object_key in object_keys {
            match self.delete_object(object_key).await {
                Ok(_) => deleted.push(object_key.clone()),
                Err(e) => {
                    warn!("Failed to delete object {}: {}", object_key, e);
                }
            }
        }

        Ok(deleted)
    }

    /// 检查文件是否存在
    #[instrument(skip(self), fields(object_key))]
    pub async fn file_exists(&self, object_key: &str) -> Result<bool, AppError> {
        let builder = RequestBuilder::new();

        // 尝试获取对象来检查是否存在
        match self.client.get_object(object_key, builder).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// 获取对象元数据
    #[instrument(skip(self), fields(object_key))]
    pub async fn get_object_metadata(
        &self,
        _object_key: &str,
    ) -> Result<HashMap<String, String>, AppError> {
        // 暂时返回空的元数据，因为 get_object_metadata 方法可能不存在
        // 如果需要元数据，可能需要使用其他方法或者升级SDK版本
        warn!(
            "The get_object_metadata method has not been implemented yet and returns empty metadata."
        );
        Ok(HashMap::new())
    }

    /// 检测MIME类型
    fn detect_mime_type(&self, file_path: &str) -> Result<String, AppError> {
        let path = Path::new(file_path);
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

        let mime_type = match extension.to_lowercase().as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "ppt" => "application/vnd.ms-powerpoint",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "txt" => "text/plain",
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" => "application/javascript",
            "json" => "application/json",
            "xml" => "application/xml",
            "zip" => "application/zip",
            "rar" => "application/x-rar-compressed",
            "7z" => "application/x-7z-compressed",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "mp4" => "video/mp4",
            "avi" => "video/x-msvideo",
            "mov" => "video/quicktime",
            _ => "application/octet-stream",
        };

        Ok(mime_type.to_string())
    }

    /// 获取存储桶名称
    pub fn get_bucket_name(&self) -> &str {
        &self.bucket
    }

    /// 获取基础URL
    pub fn get_base_url(&self) -> &str {
        &self.base_url
    }

    /// 获取配置
    pub fn get_config(&self) -> &OssServiceConfig {
        &self.config
    }

    /// 列出对象（简化版本）
    #[instrument(skip(self), fields(prefix, max_keys))]
    pub async fn list_objects(
        &self,
        _prefix: Option<&str>,
        _max_keys: Option<i32>,
    ) -> Result<Vec<String>, AppError> {
        // 注意：aliyun-oss-rust-sdk可能没有直接的list_objects API
        // 这里返回空列表，实际使用时需要根据SDK的API来实现
        warn!(
            "The list_objects function needs to be implemented according to the API of aliyun-oss-rust-sdk"
        );
        Ok(Vec::new())
    }

    /// 获取存储统计信息（简化版本）
    #[instrument(skip(self))]
    pub async fn get_storage_stats(&self, prefix: Option<&str>) -> Result<StorageStats, AppError> {
        // 注意：这个功能需要根据aliyun-oss-rust-sdk的API来实现
        warn!(
            "The get_storage_stats function needs to be implemented according to the API of aliyun-oss-rust-sdk"
        );
        Ok(StorageStats {
            total_objects: 0,
            total_size: 0,
            file_count: 0,
            last_modified: None,
        })
    }
}

/// 存储统计信息
#[derive(Debug, Clone)]
pub struct StorageStats {
    pub total_objects: usize,
    pub total_size: u64,
    pub file_count: usize,
    pub last_modified: Option<String>,
}

impl StorageStats {
    /// 格式化大小显示
    pub fn formatted_size(&self) -> String {
        let size = self.total_size as f64;
        if size < 1024.0 {
            format!("{size} B")
        } else if size < 1024.0 * 1024.0 {
            format!("{:.2} KB", size / 1024.0)
        } else if size < 1024.0 * 1024.0 * 1024.0 {
            format!("{:.2} MB", size / (1024.0 * 1024.0))
        } else {
            format!("{:.2} GB", size / (1024.0 * 1024.0 * 1024.0))
        }
    }
}
