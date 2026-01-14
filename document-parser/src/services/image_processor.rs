use crate::error::AppError;
use crate::services::OssService;
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, warn};

/// 图片处理服务配置
#[derive(Debug, Clone)]
pub struct ImageProcessorConfig {
    /// 是否启用图片上传
    pub enable_upload: bool,
    /// 支持的图片格式
    pub supported_formats: Vec<String>,
    /// 最大图片大小（字节）
    pub max_image_size: usize,
    /// 图片存储路径前缀
    pub image_path_prefix: String,
}

impl Default for ImageProcessorConfig {
    fn default() -> Self {
        Self {
            enable_upload: true,
            supported_formats: vec![
                "jpg".to_string(),
                "jpeg".to_string(),
                "png".to_string(),
                "gif".to_string(),
                "webp".to_string(),
                "bmp".to_string(),
            ],
            max_image_size: 10 * 1024 * 1024, // 10MB
            image_path_prefix: "images/".to_string(),
        }
    }
}

/// 图片上传结果
#[derive(Debug, Clone)]
pub struct ImageUploadResult {
    /// 原始图片路径
    pub original_path: String,
    /// OSS图片URL
    pub oss_url: String,
    /// 图片文件名
    pub filename: String,
    /// 上传状态
    pub success: bool,
    /// 错误信息（如果有）
    pub error_message: Option<String>,
}

/// 图片处理服务
pub struct ImageProcessor {
    config: ImageProcessorConfig,
    oss_service: Option<Arc<OssService>>,
    upload_cache: Arc<Mutex<HashMap<String, String>>>, // 本地路径 -> OSS URL 映射
}

impl ImageProcessor {
    /// 创建新的图片处理服务
    pub fn new(config: ImageProcessorConfig, oss_service: Option<Arc<OssService>>) -> Self {
        Self {
            config,
            oss_service,
            upload_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 批量上传图片到OSS
    #[instrument(skip(self, image_paths))]
    pub async fn batch_upload_images(
        &self,
        image_paths: Vec<String>,
    ) -> Result<Vec<ImageUploadResult>> {
        let mut results = Vec::new();

        for image_path in image_paths {
            let result = self.upload_single_image(&image_path).await;
            results.push(result);
        }

        Ok(results)
    }

    /// 上传单个图片到OSS
    #[instrument(skip(self))]
    async fn upload_single_image(&self, image_path: &str) -> ImageUploadResult {
        // 检查缓存
        if let Some(cached_url) = self.upload_cache.lock().await.get(image_path) {
            return ImageUploadResult {
                original_path: image_path.to_string(),
                oss_url: cached_url.clone(),
                filename: self.extract_filename(image_path),
                success: true,
                error_message: None,
            };
        }

        // 检查文件是否存在
        if !Path::new(image_path).exists() {
            return ImageUploadResult {
                original_path: image_path.to_string(),
                oss_url: String::new(),
                filename: self.extract_filename(image_path),
                success: false,
                error_message: Some("Image file not found".to_string()),
            };
        }

        // 检查文件大小
        if let Ok(metadata) = fs::metadata(image_path).await {
            if metadata.len() > self.config.max_image_size as u64 {
                return ImageUploadResult {
                    original_path: image_path.to_string(),
                    oss_url: String::new(),
                    filename: self.extract_filename(image_path),
                    success: false,
                    error_message: Some("Image file too large".to_string()),
                };
            }
        }

        // 检查文件格式
        if let Some(extension) = Path::new(image_path).extension() {
            let ext = extension.to_string_lossy().to_lowercase();
            if !self.config.supported_formats.contains(&ext) {
                return ImageUploadResult {
                    original_path: image_path.to_string(),
                    oss_url: String::new(),
                    filename: self.extract_filename(image_path),
                    success: false,
                    error_message: Some(format!("Unsupported image format: {ext}")),
                };
            }
        }

        // 上传到OSS
        match self.upload_to_oss(image_path).await {
            Ok(oss_url) => {
                // 缓存结果
                self.upload_cache
                    .lock()
                    .await
                    .insert(image_path.to_string(), oss_url.clone());

                ImageUploadResult {
                    original_path: image_path.to_string(),
                    oss_url,
                    filename: self.extract_filename(image_path),
                    success: true,
                    error_message: None,
                }
            }
            Err(e) => ImageUploadResult {
                original_path: image_path.to_string(),
                oss_url: String::new(),
                filename: self.extract_filename(image_path),
                success: false,
                error_message: Some(e.to_string()),
            },
        }
    }

    /// 上传图片到OSS（复用现有OSS服务）
    #[instrument(skip(self))]
    async fn upload_to_oss(&self, image_path: &str) -> Result<String> {
        let oss_service = self
            .oss_service
            .as_ref()
            .ok_or_else(|| AppError::Oss("OSS service not initialized".to_string()))?;

        // 使用现有的OSS服务上传图片
        let image_info = oss_service
            .upload_image(image_path)
            .await
            .with_context(|| format!("Failed to upload image to OSS: {image_path}"))?;

        info!(
            "Successfully uploaded image to OSS: {} -> {}",
            image_path, image_info.oss_url
        );
        Ok(image_info.oss_url)
    }

    /// 提取文件名
    fn extract_filename(&self, path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string()
    }

    /// 替换Markdown中的图片路径
    #[instrument(skip(self, markdown_content))]
    pub async fn replace_markdown_images(&self, markdown_content: &str) -> Result<String> {
        // 正则表达式匹配Markdown图片语法
        let image_regex = Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap();
        let mut result = markdown_content.to_string();
        let mut replacements = Vec::new();

        // 收集所有需要替换的图片
        for cap in image_regex.captures_iter(markdown_content) {
            let alt_text = &cap[1];
            let image_path = &cap[2];

            // 跳过已经是URL的图片
            if image_path.starts_with("http://") || image_path.starts_with("https://") {
                continue;
            }

            // 检查缓存
            if let Some(oss_url) = self.upload_cache.lock().await.get(image_path) {
                replacements.push((image_path.to_string(), oss_url.clone()));
            } else {
                // 尝试上传图片
                let upload_result = self.upload_single_image(image_path).await;
                if upload_result.success {
                    replacements.push((image_path.to_string(), upload_result.oss_url));
                } else {
                    warn!(
                        "Failed to upload image: {} - {}",
                        image_path,
                        upload_result.error_message.unwrap_or_default()
                    );
                }
            }
        }

        // 执行替换
        for (old_path, new_url) in replacements {
            result = result.replace(&old_path, &new_url);
        }

        Ok(result)
    }

    /// 获取上传缓存统计
    pub async fn get_cache_stats(&self) -> (usize, usize) {
        let cache = self.upload_cache.lock().await;
        let total = cache.len();
        let successful = cache.values().filter(|url| !url.is_empty()).count();
        (total, successful)
    }

    /// 清空上传缓存
    pub async fn clear_cache(&self) {
        self.upload_cache.lock().await.clear();
    }

    /// 从Markdown内容中提取所有图片路径
    pub fn extract_image_paths(markdown_content: &str) -> Vec<String> {
        let image_regex = Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap();
        let mut image_paths = Vec::new();

        for cap in image_regex.captures_iter(markdown_content) {
            let image_path = &cap[2];

            // 跳过已经是URL的图片
            if !image_path.starts_with("http://") && !image_path.starts_with("https://") {
                image_paths.push(image_path.to_string());
            }
        }

        debug!("提取到的图片路径: {:?}", image_paths);

        image_paths
    }

    /// 验证图片文件
    pub async fn validate_image_file(&self, image_path: &str) -> Result<bool> {
        let path = Path::new(image_path);

        // 检查文件是否存在
        if !path.exists() {
            return Ok(false);
        }

        // 检查文件大小
        if let Ok(metadata) = fs::metadata(image_path).await {
            if metadata.len() > self.config.max_image_size as u64 {
                return Ok(false);
            }
        }

        // 检查文件格式
        if let Some(extension) = path.extension() {
            let ext = extension.to_string_lossy().to_lowercase();
            if !self.config.supported_formats.contains(&ext) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// 批量处理Markdown文件中的图片
    #[instrument(skip(self, markdown_files))]
    pub async fn process_markdown_files(
        &self,
        markdown_files: Vec<(String, String)>,
    ) -> Result<Vec<(String, String)>> {
        let mut results = Vec::new();

        for (file_path, content) in markdown_files {
            match self.replace_markdown_images(&content).await {
                Ok(processed_content) => {
                    results.push((file_path, processed_content));
                }
                Err(e) => {
                    warn!("Failed to process markdown file {}: {}", file_path, e);
                    // 如果处理失败，保留原内容
                    results.push((file_path, content));
                }
            }
        }

        Ok(results)
    }

    /// 从目录中提取所有图片路径
    pub async fn extract_images_from_directory(&self, directory_path: &str) -> Result<Vec<String>> {
        let mut image_paths = Vec::new();

        let mut entries = fs::read_dir(directory_path)
            .await
            .with_context(|| format!("Failed to read directory: {directory_path}"))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_file() {
                if let Some(extension) = path.extension() {
                    let ext = extension.to_string_lossy().to_lowercase();
                    if self.config.supported_formats.contains(&ext) {
                        if let Some(path_str) = path.to_str() {
                            image_paths.push(path_str.to_string());
                        }
                    }
                }
            }
        }

        Ok(image_paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_extract_image_paths() {
        let markdown = r#"
        # Test Document

        ![Image 1](images/test1.jpg)
        ![Image 2](images/test2.png)
        ![External Image](https://example.com/image.jpg)
        ![Local Image](./local/image.gif)
        "#;

        let paths = ImageProcessor::extract_image_paths(markdown);
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(&"images/test1.jpg".to_string()));
        assert!(paths.contains(&"images/test2.png".to_string()));
        assert!(paths.contains(&"./local/image.gif".to_string()));
        assert!(!paths.contains(&"https://example.com/image.jpg".to_string()));
    }

    #[tokio::test]
    async fn test_extract_filename() {
        let config = ImageProcessorConfig::default();
        let processor = ImageProcessor::new(config, None);

        assert_eq!(processor.extract_filename("path/to/image.jpg"), "image.jpg");
        assert_eq!(processor.extract_filename("image.png"), "image.png");
        assert_eq!(processor.extract_filename(""), "unknown");
    }

    #[tokio::test]
    async fn test_validate_image_file() {
        let config = ImageProcessorConfig::default();
        let processor = ImageProcessor::new(config, None);

        // 创建临时目录和测试文件
        let temp_dir = tempdir().unwrap();
        let test_file_path = temp_dir.path().join("test.txt");
        fs::write(&test_file_path, "not an image").unwrap();

        // 测试无效文件
        let result = processor
            .validate_image_file(test_file_path.to_str().unwrap())
            .await
            .unwrap();
        assert!(!result);

        // 清理
        temp_dir.close().unwrap();
    }
}
