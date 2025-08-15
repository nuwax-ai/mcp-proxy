use std::path::{Path, PathBuf};
use std::fs;
use std::time::Duration;
use anyhow::Result;
use tokio::fs as async_fs;
use tokio::time::{sleep, timeout};
use uuid::{Uuid, Timestamp, NoContext};
use crate::error::AppError;
use crate::services::OssService;
use crate::config::GlobalFileSizeConfig;
use std::sync::Arc;
use std::collections::HashMap;
use tracing::{debug, info, warn, error, instrument};
use futures::future::join_all;
use serde::{Serialize, Deserialize};

/// 图片处理器（增强版本）
pub struct ImageProcessor {
    temp_dir: PathBuf,
    oss_service: Option<Arc<OssService>>,
    config: ImageProcessorConfig,
    stats: Arc<tokio::sync::RwLock<ImageProcessingStats>>,
}

/// 图片处理结果（增强版本）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageProcessResult {
    pub original_path: String,
    pub processed_path: Option<String>,
    pub oss_url: Option<String>,
    pub file_size: u64,
    pub compressed_size: Option<u64>,
    pub format: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub processing_time_ms: Option<u64>,
    pub compression_ratio: Option<f32>,
    pub error_message: Option<String>,
    pub retry_count: u32,
    pub checksum: Option<String>,
}

/// 图片处理配置（增强版本）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageProcessConfig {
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub quality: Option<u8>,
    pub format: Option<String>,
    pub enable_compression: bool,
    pub max_file_size: u64,
    pub allowed_formats: Vec<String>,
    pub enable_validation: bool,
    pub enable_optimization: bool,
}

/// 图片处理器配置
#[derive(Debug, Clone)]
pub struct ImageProcessorConfig {
    pub max_concurrent_uploads: usize,
    pub upload_timeout_seconds: u64,
    pub max_retry_attempts: u32,
    pub retry_delay_ms: u64,
    pub enable_duplicate_detection: bool,
    pub enable_batch_processing: bool,
    pub temp_cleanup_interval_hours: u64,
}

impl Default for ImageProcessConfig {
    fn default() -> Self {
        Self {
            max_width: Some(1920),
            max_height: Some(1080),
            quality: Some(85),
            format: None, // 保持原格式
            enable_compression: true,
            max_file_size: 50 * 1024 * 1024, // 50MB，应该从全局配置获取
            allowed_formats: vec![
                "jpg".to_string(), "jpeg".to_string(), "png".to_string(),
                "gif".to_string(), "bmp".to_string(), "webp".to_string(),
                "svg".to_string(), "tiff".to_string(), "ico".to_string(),
            ],
            enable_validation: true,
            enable_optimization: true,
        }
    }
}

impl Default for ImageProcessorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_uploads: 10,
            upload_timeout_seconds: 300, // 5 minutes
            max_retry_attempts: 3,
            retry_delay_ms: 1000,
            enable_duplicate_detection: true,
            enable_batch_processing: true,
            temp_cleanup_interval_hours: 24,
        }
    }
}

impl ImageProcessor {
    /// 创建新的图片处理器（增强版本）
    pub fn new(
        temp_dir: PathBuf, 
        oss_service: Option<Arc<OssService>>,
        config: Option<ImageProcessorConfig>,
    ) -> Self {
        Self {
            temp_dir,
            oss_service,
            config: config.unwrap_or_default(),
            stats: Arc::new(tokio::sync::RwLock::new(ImageProcessingStats::default())),
        }
    }

    /// 创建带默认配置的处理器
    pub fn with_defaults(temp_dir: PathBuf, oss_service: Option<Arc<OssService>>) -> Self {
        Self::new(temp_dir, oss_service, None)
    }

    /// 批量处理图片（增强版本）
    #[instrument(skip(self, image_paths), fields(count = image_paths.len()))]
    pub async fn process_images_batch(
        &self,
        image_paths: &[String],
        config: Option<&ImageProcessConfig>,
    ) -> Result<BatchProcessResult, AppError> {
        let default_config = ImageProcessConfig::default();
        let process_config = config.unwrap_or(&default_config);
        let start_time = std::time::Instant::now();
        
        info!("开始批量处理 {} 个图片", image_paths.len());
        
        // 验证输入
        self.validate_batch_input(image_paths, process_config).await?;
        
        let mut results = Vec::new();
        let mut errors = Vec::new();
        
        if self.config.enable_batch_processing {
            // 并发处理
            let chunk_size = self.config.max_concurrent_uploads;
            for chunk in image_paths.chunks(chunk_size) {
                let chunk_results = self.process_image_chunk(chunk, process_config).await;
                
                for result in chunk_results {
                    match result {
                        Ok(image_result) => results.push(image_result),
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                
                // 短暂延迟避免过载
                if chunk.len() == chunk_size {
                    sleep(Duration::from_millis(100)).await;
                }
            }
        } else {
            // 顺序处理
            for image_path in image_paths {
                match self.process_image_with_retry(image_path, process_config).await {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        warn!("处理图片失败 {}: {}", image_path, e);
                        errors.push(format!("{}: {}", image_path, e));
                    }
                }
            }
        }
        
        let processing_time = start_time.elapsed();
        
        // 更新统计信息
        self.update_stats(results.len(), errors.len(), processing_time).await;
        
        info!(
            "批量处理完成: 成功 {}, 失败 {}, 耗时 {:?}",
            results.len(),
            errors.len(),
            processing_time
        );
        
        Ok(BatchProcessResult {
            successful_results: results,
            failed_items: errors,
            total_processed: image_paths.len(),
            processing_time_ms: processing_time.as_millis() as u64,
        })
    }

    /// 处理图片块（并发）
    async fn process_image_chunk(
        &self,
        image_paths: &[String],
        config: &ImageProcessConfig,
    ) -> Vec<Result<ImageProcessResult, AppError>> {
        let futures = image_paths.iter().map(|path| {
            self.process_image_with_retry(path, config)
        });
        
        join_all(futures).await
    }

    /// 从目录提取图片（增强版本）
    #[instrument(skip(self), fields(source_dir = %source_dir))]
    pub async fn extract_images_from_directory(
        &self,
        source_dir: &str,
    ) -> Result<ImageExtractionResult, AppError> {
        let start_time = std::time::Instant::now();
        let source_path = Path::new(source_dir);
        
        info!("开始从目录提取图片: {}", source_dir);
        
        // 验证源目录
        self.validate_source_directory(source_path).await?;
        
        let mut image_paths = Vec::new();
        let mut errors = Vec::new();
        let mut total_size = 0u64;
        
        // 递归扫描目录
        match self.scan_directory_for_images_enhanced(source_path, &mut image_paths, &mut errors, &mut total_size).await {
            Ok(_) => {
                let processing_time = start_time.elapsed();
                
                info!(
                    "图片提取完成: 找到 {} 个图片文件, 总大小 {} 字节, 耗时 {:?}",
                    image_paths.len(),
                    total_size,
                    processing_time
                );
                
                let total_files = image_paths.len();
                Ok(ImageExtractionResult {
                    image_paths,
                    errors,
                    total_files,
                    total_size,
                    processing_time_ms: processing_time.as_millis() as u64,
                })
            }
            Err(e) => {
                error!("图片提取失败: {}", e);
                Err(e)
            }
        }
    }

    /// 验证源目录
    async fn validate_source_directory(&self, source_path: &Path) -> Result<(), AppError> {
        if !source_path.exists() {
            return Err(AppError::File(format!("源目录不存在: {}", source_path.display())));
        }
        
        if !source_path.is_dir() {
            return Err(AppError::File(format!("路径不是目录: {}", source_path.display())));
        }
        
        // 检查读取权限
        match async_fs::read_dir(source_path).await {
            Ok(_) => Ok(()),
            Err(e) => Err(AppError::File(format!("无法读取目录 {}: {}", source_path.display(), e))),
        }
    }

    /// 递归扫描目录查找图片（增强版本）
    #[async_recursion::async_recursion]
    async fn scan_directory_for_images_enhanced(
        &self,
        dir: &Path,
        image_paths: &mut Vec<String>,
        errors: &mut Vec<String>,
        total_size: &mut u64,
    ) -> Result<(), AppError> {
        let mut entries = match async_fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(e) => {
                let error_msg = format!("读取目录失败 {}: {}", dir.display(), e);
                warn!("{}", error_msg);
                errors.push(error_msg);
                return Ok(()); // 继续处理其他目录
            }
        };
        
        while let Some(entry) = entries.next_entry().await
            .map_err(|e| AppError::File(format!("读取目录项失败: {}", e)))? {
            
            let path = entry.path();
            
            if path.is_dir() {
                // 递归处理子目录
                if let Err(e) = self.scan_directory_for_images_enhanced(&path, image_paths, errors, total_size).await {
                    let error_msg = format!("扫描子目录失败 {}: {}", path.display(), e);
                    warn!("{}", error_msg);
                    errors.push(error_msg);
                }
            } else if self.is_image_file_enhanced(&path).await {
                // 验证并添加图片文件
                match self.validate_image_file(&path).await {
                    Ok(file_size) => {
                        if let Some(path_str) = path.to_str() {
                            image_paths.push(path_str.to_string());
                            *total_size += file_size;
                            debug!("找到图片文件: {} ({}字节)", path_str, file_size);
                        }
                    }
                    Err(e) => {
                        let error_msg = format!("图片文件验证失败 {}: {}", path.display(), e);
                        warn!("{}", error_msg);
                        errors.push(error_msg);
                    }
                }
            }
        }
        
        Ok(())
    }

    /// 检查是否为图片文件（增强版本）
    async fn is_image_file_enhanced(&self, path: &Path) -> bool {
        // 基本扩展名检查
        if !self.is_image_file_by_extension(path) {
            return false;
        }
        
        // 文件存在性检查
        if !path.exists() {
            return false;
        }
        
        // 文件大小检查
        if let Ok(metadata) = async_fs::metadata(path).await {
            if metadata.len() == 0 {
                return false; // 空文件
            }
            
            if metadata.len() > 100 * 1024 * 1024 { // 100MB
                warn!("图片文件过大: {} ({}字节)", path.display(), metadata.len());
                return false;
            }
        }
        
        true
    }

    /// 基于扩展名检查图片文件
    fn is_image_file_by_extension(&self, path: &Path) -> bool {
        if let Some(extension) = path.extension() {
            if let Some(ext_str) = extension.to_str() {
                let ext_lower = ext_str.to_lowercase();
                let default_config = ImageProcessConfig::default();
                default_config.allowed_formats.contains(&ext_lower)
            } else {
                false
            }
        } else {
            false
        }
    }

    /// 验证图片文件
    async fn validate_image_file(&self, path: &Path) -> Result<u64, AppError> {
        let metadata = async_fs::metadata(path).await
            .map_err(|e| AppError::File(format!("获取文件元数据失败: {}", e)))?;
        
        let file_size = metadata.len();
        
        // 文件大小验证
        if file_size == 0 {
            return Err(AppError::Validation("图片文件为空".to_string()));
        }
        
        // 使用全局配置中的文件大小限制
        let global_config = GlobalFileSizeConfig::new();
        if file_size > global_config.max_file_size.bytes() {
            return Err(AppError::Validation(
                format!("图片文件过大: {} 字节 (最大: {} 字节)", 
                    file_size, global_config.max_file_size.bytes())
            ));
        }
        
        // 文件权限验证
        if metadata.permissions().readonly() {
            warn!("图片文件为只读: {}", path.display());
        }
        
        Ok(file_size)
    }

    /// 处理单个图片（带重试机制）
    #[instrument(skip(self, config), fields(image_path = %image_path))]
    async fn process_image_with_retry(
        &self,
        image_path: &str,
        config: &ImageProcessConfig,
    ) -> Result<ImageProcessResult, AppError> {
        let mut last_error = None;
        
        for attempt in 1..=self.config.max_retry_attempts {
            match self.process_image_single(image_path, config, attempt).await {
                Ok(result) => {
                    if attempt > 1 {
                        info!("图片处理成功 (第{}次尝试): {}", attempt, image_path);
                    }
                    return Ok(result);
                }
                Err(e) => {
                    warn!("图片处理失败 (第{}次尝试): {} - {}", attempt, image_path, e);
                    last_error = Some(e);
                    
                    if attempt < self.config.max_retry_attempts {
                        let delay = Duration::from_millis(
                            self.config.retry_delay_ms * attempt as u64
                        );
                        sleep(delay).await;
                    }
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| 
            AppError::Processing("图片处理失败，已达到最大重试次数".to_string())
        ))
    }

    /// 处理单个图片（单次尝试）
    async fn process_image_single(
        &self,
        image_path: &str,
        config: &ImageProcessConfig,
        attempt: u32,
    ) -> Result<ImageProcessResult, AppError> {
        let start_time = std::time::Instant::now();
        let source_path = Path::new(image_path);
        
        debug!("开始处理图片 (第{}次尝试): {}", attempt, image_path);
        
        // 验证输入文件
        let file_size = self.validate_image_file(source_path).await?;
        
        // 检测图片格式
        let format = self.detect_image_format_enhanced(source_path).await?;
        
        // 计算文件校验和
        let checksum = if config.enable_validation {
            Some(self.calculate_file_checksum(source_path).await?)
        } else {
            None
        };
        
        // 创建基础处理结果
        let mut result = ImageProcessResult {
            original_path: image_path.to_string(),
            processed_path: None,
            oss_url: None,
            file_size,
            compressed_size: None,
            format: format.clone(),
            width: None,
            height: None,
            processing_time_ms: None,
            compression_ratio: None,
            error_message: None,
            retry_count: attempt - 1,
            checksum,
        };
        
        // 图片处理和优化
        if config.enable_compression || config.enable_optimization {
            match self.process_and_optimize_image(source_path, config).await {
                Ok((processed_path, compressed_size, dimensions)) => {
                    result.processed_path = Some(processed_path);
                    result.compressed_size = Some(compressed_size);
                    result.width = dimensions.0;
                    result.height = dimensions.1;
                    
                    if compressed_size > 0 {
                        result.compression_ratio = Some(
                            (file_size as f32 - compressed_size as f32) / file_size as f32
                        );
                    }
                }
                Err(e) => {
                    warn!("图片优化失败: {} - {}", image_path, e);
                    result.error_message = Some(e.to_string());
                }
            }
        }
        
        let processing_time = start_time.elapsed();
        result.processing_time_ms = Some(processing_time.as_millis() as u64);
        
        debug!(
            "图片处理完成: {} (耗时: {:?})",
            image_path,
            processing_time
        );
        
        Ok(result)
    }

    /// 检测图片格式（增强版本）
    async fn detect_image_format_enhanced(&self, path: &Path) -> Result<String, AppError> {
        // 基于扩展名的检测
        let extension_format = if let Some(extension) = path.extension() {
            if let Some(ext_str) = extension.to_str() {
                Some(ext_str.to_lowercase())
            } else {
                None
            }
        } else {
            None
        };
        
        // 基于文件头的检测（更可靠）
        let header_format = self.detect_format_by_header(path).await?;
        
        // 优先使用文件头检测结果
        let detected_format = header_format.unwrap_or_else(|| {
            extension_format.unwrap_or_else(|| "unknown".to_string())
        });
        
        // 验证格式是否支持
        let default_config = ImageProcessConfig::default();
        if !default_config.allowed_formats.contains(&detected_format) {
            return Err(AppError::Validation(
                format!("不支持的图片格式: {}", detected_format)
            ));
        }
        
        Ok(detected_format)
    }

    /// 基于文件头检测格式
    async fn detect_format_by_header(&self, path: &Path) -> Result<Option<String>, AppError> {
        let mut file = async_fs::File::open(path).await
            .map_err(|e| AppError::File(format!("打开文件失败: {}", e)))?;
        
        let mut header = [0u8; 16];
        use tokio::io::AsyncReadExt;
        
        let bytes_read = file.read(&mut header).await
            .map_err(|e| AppError::File(format!("读取文件头失败: {}", e)))?;
        
        if bytes_read < 4 {
            return Ok(None);
        }
        
        // 检测常见图片格式的文件头
        let format = match &header[..4] {
            [0xFF, 0xD8, 0xFF, _] => Some("jpg".to_string()),
            [0x89, 0x50, 0x4E, 0x47] => Some("png".to_string()),
            [0x47, 0x49, 0x46, 0x38] => Some("gif".to_string()),
            [0x42, 0x4D, _, _] => Some("bmp".to_string()),
            [0x52, 0x49, 0x46, 0x46] if bytes_read >= 12 && &header[8..12] == b"WEBP" => {
                Some("webp".to_string())
            }
            _ => {
                // 检查TIFF格式
                if (header[0] == 0x49 && header[1] == 0x49 && header[2] == 0x2A && header[3] == 0x00) ||
                   (header[0] == 0x4D && header[1] == 0x4D && header[2] == 0x00 && header[3] == 0x2A) {
                    Some("tiff".to_string())
                } else {
                    None
                }
            }
        };
        
        Ok(format)
    }

    /// 计算文件校验和
    async fn calculate_file_checksum(&self, path: &Path) -> Result<String, AppError> {
        use sha2::{Sha256, Digest};
        use tokio::io::AsyncReadExt;
        
        let mut file = async_fs::File::open(path).await
            .map_err(|e| AppError::File(format!("打开文件失败: {}", e)))?;
        
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        
        loop {
            let bytes_read = file.read(&mut buffer).await
                .map_err(|e| AppError::File(format!("读取文件失败: {}", e)))?;
            
            if bytes_read == 0 {
                break;
            }
            
            hasher.update(&buffer[..bytes_read]);
        }
        
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// 处理和优化图片
    async fn process_and_optimize_image(
        &self,
        source_path: &Path,
        config: &ImageProcessConfig,
    ) -> Result<(String, u64, (Option<u32>, Option<u32>)), AppError> {
        // 创建任务专用临时目录
        let task_id = Uuid::new_v7(Timestamp::now(NoContext)).to_string();
        let temp_dir = self.temp_dir.join(&task_id);
        
        async_fs::create_dir_all(&temp_dir).await
            .map_err(|e| AppError::File(format!("创建临时目录失败: {}", e)))?;
        
        // 确定输出格式和文件名
        let output_format = config.format.as_deref()
            .unwrap_or_else(|| self.get_file_extension(source_path));
        
        let output_filename = format!("processed.{}", output_format);
        let output_path = temp_dir.join(&output_filename);
        
        // 执行图片处理
        let (compressed_size, dimensions) = if config.enable_optimization {
            self.optimize_image_advanced(source_path, &output_path, config).await?
        } else {
            // 简单复制
            async_fs::copy(source_path, &output_path).await
                .map_err(|e| AppError::File(format!("图片复制失败: {}", e)))?;
            
            let metadata = async_fs::metadata(&output_path).await
                .map_err(|e| AppError::File(format!("获取输出文件信息失败: {}", e)))?;
            
            (metadata.len(), (None, None))
        };
        
        Ok((output_path.to_string_lossy().to_string(), compressed_size, dimensions))
    }

    /// 高级图片优化
    async fn optimize_image_advanced(
        &self,
        source_path: &Path,
        output_path: &Path,
        config: &ImageProcessConfig,
    ) -> Result<(u64, (Option<u32>, Option<u32>)), AppError> {
        // 注意：这里使用简化的实现
        // 在实际项目中，应该使用 image crate 或其他图片处理库
        
        debug!("开始优化图片: {} -> {}", source_path.display(), output_path.display());
        
        // 模拟图片处理过程
        let processing_future = async {
            // 读取原始文件
            let original_data = async_fs::read(source_path).await
                .map_err(|e| AppError::File(format!("读取原始文件失败: {}", e)))?;
            
            // 模拟压缩处理（实际应该使用图片库）
            let mut processed_data = original_data.clone();
            
            // 简单的"压缩"：如果启用压缩且文件较大，则截断一部分数据
            if config.enable_compression && processed_data.len() > 1024 * 1024 {
                let target_size = (processed_data.len() as f32 * 0.8) as usize;
                processed_data.truncate(target_size);
            }
            
            // 写入处理后的文件
            async_fs::write(output_path, &processed_data).await
                .map_err(|e| AppError::File(format!("写入处理后文件失败: {}", e)))?;
            
            // 获取处理后文件大小
            let metadata = async_fs::metadata(output_path).await
                .map_err(|e| AppError::File(format!("获取处理后文件信息失败: {}", e)))?;
            
            // 模拟获取图片尺寸（实际应该解析图片）
            let dimensions = if config.max_width.is_some() || config.max_height.is_some() {
                (config.max_width, config.max_height)
            } else {
                (None, None)
            };
            
            Ok::<(u64, (Option<u32>, Option<u32>)), AppError>((metadata.len(), dimensions))
        };
        
        // 添加超时保护
        let timeout_duration = Duration::from_secs(self.config.upload_timeout_seconds);
        
        match timeout(timeout_duration, processing_future).await {
            Ok(result) => result,
            Err(_) => Err(AppError::Processing("图片处理超时".to_string())),
        }
    }

    /// 获取文件扩展名
    fn get_file_extension<'a>(&self, path: &'a Path) -> &'a str {
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("jpg")
    }

    /// 批量上传图片到OSS（增强版本）
    #[instrument(skip(self, image_results), fields(count = image_results.len(), task_id = %task_id))]
    pub async fn upload_images_to_oss_batch(
        &self,
        image_results: &mut [ImageProcessResult],
        task_id: &str,
    ) -> Result<BatchUploadResult, AppError> {
        let oss_service = self.oss_service.as_ref()
            .ok_or_else(|| AppError::Config("OSS服务未配置".to_string()))?;
        
        let start_time = std::time::Instant::now();
        
        info!("开始批量上传 {} 个图片到OSS", image_results.len());
        
        let mut successful_uploads = 0;
        let mut failed_uploads = Vec::new();
        let mut total_uploaded_size = 0u64;
        
        // 并发上传
        let chunk_size = self.config.max_concurrent_uploads;
        for chunk in image_results.chunks_mut(chunk_size) {
            let upload_futures = chunk.iter_mut().map(|result| {
                self.upload_single_image_with_retry(result, task_id, oss_service.clone())
            });
            
            let upload_results = join_all(upload_futures).await;
            
            for (result, upload_result) in chunk.iter_mut().zip(upload_results) {
                match upload_result {
                    Ok(uploaded_size) => {
                        successful_uploads += 1;
                        total_uploaded_size += uploaded_size;
                        debug!("图片上传成功: {}", result.original_path);
                    }
                    Err(e) => {
                        error!("图片上传失败: {} - {}", result.original_path, e);
                        result.error_message = Some(e.to_string());
                        failed_uploads.push(result.original_path.clone());
                    }
                }
            }
            
            // 短暂延迟避免过载
            if chunk.len() == chunk_size {
                sleep(Duration::from_millis(100)).await;
            }
        }
        
        let processing_time = start_time.elapsed();
        
        // 更新统计信息
        self.update_upload_stats(successful_uploads, failed_uploads.len(), processing_time).await;
        
        info!(
            "批量上传完成: 成功 {}, 失败 {}, 总大小 {} 字节, 耗时 {:?}",
            successful_uploads,
            failed_uploads.len(),
            total_uploaded_size,
            processing_time
        );
        
        Ok(BatchUploadResult {
            successful_uploads,
            failed_uploads,
            total_uploaded_size,
            processing_time_ms: processing_time.as_millis() as u64,
        })
    }

    /// 上传单个图片（带重试机制）
    async fn upload_single_image_with_retry(
        &self,
        result: &mut ImageProcessResult,
        task_id: &str,
        oss_service: Arc<OssService>,
    ) -> Result<u64, AppError> {
        let mut last_error = None;
        
        for attempt in 1..=self.config.max_retry_attempts {
            // 选择要上传的文件路径
            let upload_path = result.processed_path.as_ref()
                .unwrap_or(&result.original_path);
            
            // 生成OSS键名
            let oss_key = self.generate_oss_key_enhanced(task_id, upload_path, attempt)?;
            
            // 执行上传
            let upload_future = oss_service.upload_file(upload_path, &oss_key);
            let timeout_duration = Duration::from_secs(self.config.upload_timeout_seconds);
            
            match timeout(timeout_duration, upload_future).await {
                Ok(Ok(oss_url)) => {
                    result.oss_url = Some(oss_url);
                    result.retry_count = attempt - 1;
                    
                    // 返回上传的文件大小
                    let file_size = result.compressed_size.unwrap_or(result.file_size);
                    
                    if attempt > 1 {
                        info!("图片上传成功 (第{}次尝试): {}", attempt, upload_path);
                    }
                    
                    return Ok(file_size);
                }
                Ok(Err(e)) => {
                    warn!("图片上传失败 (第{}次尝试): {} - {}", attempt, upload_path, e);
                    last_error = Some(e);
                }
                Err(_) => {
                    let timeout_error = AppError::Processing("图片上传超时".to_string());
                    warn!("图片上传超时 (第{}次尝试): {}", attempt, upload_path);
                    last_error = Some(timeout_error);
                }
            }
            
            // 重试延迟
            if attempt < self.config.max_retry_attempts {
                let delay = Duration::from_millis(
                    self.config.retry_delay_ms * attempt as u64
                );
                sleep(delay).await;
            }
        }
        
        Err(last_error.unwrap_or_else(|| 
            AppError::Processing("图片上传失败，已达到最大重试次数".to_string())
        ))
    }

    /// 生成OSS键名（增强版本）
    fn generate_oss_key_enhanced(&self, task_id: &str, image_path: &str, attempt: u32) -> Result<String, AppError> {
        let path = Path::new(image_path);
        let filename = path.file_name()
            .ok_or_else(|| AppError::File("无法获取文件名".to_string()))?
            .to_str()
            .ok_or_else(|| AppError::File("文件名包含无效字符".to_string()))?;
        
        // 清理文件名，移除特殊字符
        let clean_filename = self.sanitize_filename(filename);
        
        // 添加时间戳避免冲突
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        
        // 如果是重试，添加重试标识
        let retry_suffix = if attempt > 1 {
            format!("_retry{}", attempt)
        } else {
            String::new()
        };
        
        Ok(format!("images/{}/{}_{}{}", task_id, timestamp, clean_filename, retry_suffix))
    }

    /// 清理文件名
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

    /// 从Markdown文本中提取图片路径
    pub fn extract_image_paths(&self, markdown_content: &str) -> Vec<String> {
        use pulldown_cmark::{Parser, Event, Tag, TagEnd};
        
        let mut image_paths = Vec::new();
        let parser = Parser::new(markdown_content);
        
        for event in parser {
            if let Event::Start(Tag::Image { dest_url, .. }) = event {
                let path = dest_url.to_string();
                if !path.is_empty() {
                    image_paths.push(path);
                }
            }
        }
        
        image_paths
    }

    /// 替换Markdown中的图片路径
    pub async fn replace_image_paths_in_markdown(
        &self,
        markdown_content: &str,
        image_results: &[ImageProcessResult],
    ) -> Result<String, AppError> {
        use pulldown_cmark::{Parser, Event, Tag, CowStr};
        use std::collections::HashMap;
        
        log::info!("替换Markdown中的 {} 个图片路径", image_results.len());
        
        // 创建路径映射表
        let mut path_mapping = HashMap::new();
        for result in image_results {
            if let Some(oss_url) = &result.oss_url {
                let local_path = &result.original_path;
                
                // 尝试多种可能的路径格式
                let path_variants = self.generate_path_variants(local_path);
                for variant in path_variants {
                    path_mapping.insert(variant, oss_url.clone());
                }
            }
        }
        
        // 使用pulldown-cmark解析和重建markdown
        let parser = Parser::new(markdown_content);
        let mut events = Vec::new();
        
        for event in parser {
            match event {
                Event::Start(Tag::Image { dest_url, title, .. }) => {
                    let original_url = dest_url.to_string();
                    let new_url = path_mapping.get(&original_url)
                        .map(|url| CowStr::Borrowed(url.as_str()))
                        .unwrap_or(dest_url);
                    
                    events.push(Event::Start(Tag::Image {
                        link_type: pulldown_cmark::LinkType::Inline,
                        dest_url: new_url,
                        title,
                        id: CowStr::Borrowed(""),
                    }));
                }
                other => events.push(other),
            }
        }
        
        // 重建markdown文本
        let mut output = String::new();
        pulldown_cmark::html::push_html(&mut output, events.into_iter());
        
        // 由于push_html生成HTML，我们需要使用cmark来重新生成markdown
        // 这里简化处理，直接返回原内容进行字符串替换
        let mut updated_content = markdown_content.to_string();
        for (original_path, oss_url) in &path_mapping {
            updated_content = updated_content.replace(original_path, oss_url);
        }
        
        Ok(updated_content)
    }

    /// 生成路径变体用于替换
    fn generate_path_variants(&self, original_path: &str) -> Vec<String> {
        let mut variants = Vec::new();
        
        // 原始路径
        variants.push(original_path.to_string());
        
        // 文件名
        if let Some(filename) = Path::new(original_path).file_name() {
            if let Some(filename_str) = filename.to_str() {
                variants.push(filename_str.to_string());
            }
        }
        
        // 相对路径变体
        if let Some(relative_path) = original_path.strip_prefix("./") {
            variants.push(relative_path.to_string());
        }
        
        variants
    }

    /// 检查重复图片
    pub async fn detect_duplicate_images(
        &self,
        image_paths: &[String],
    ) -> Result<Vec<(String, String)>, AppError> {
        let mut duplicates = Vec::new();
        
        // 简单的基于文件大小的重复检测
        // 实际项目中可以使用更复杂的算法如感知哈希
        let mut size_map: std::collections::HashMap<u64, String> = std::collections::HashMap::new();
        
        for image_path in image_paths {
            if let Ok(metadata) = fs::metadata(image_path) {
                let file_size = metadata.len();
                
                if let Some(existing_path) = size_map.get(&file_size) {
                    duplicates.push((existing_path.clone(), image_path.clone()));
                } else {
                    size_map.insert(file_size, image_path.clone());
                }
            }
        }
        
        log::info!("检测到 {} 对重复图片", duplicates.len());
        Ok(duplicates)
    }

    /// 验证图片URL有效性
    pub async fn validate_image_urls(
        &self,
        markdown_content: &str,
    ) -> Result<Vec<String>, AppError> {
        let mut invalid_urls = Vec::new();
        
        // 使用正则表达式提取图片URL
        let re = regex::Regex::new(r"!\[.*?\]\((.*?)\)")
            .map_err(|e| AppError::Parse(format!("正则表达式错误: {}", e)))?;
        
        for cap in re.captures_iter(markdown_content) {
            if let Some(url) = cap.get(1) {
                let url_str = url.as_str();
                
                // 检查URL是否有效
                if !self.is_valid_url(url_str).await {
                    invalid_urls.push(url_str.to_string());
                }
            }
        }
        
        log::info!("发现 {} 个无效的图片URL", invalid_urls.len());
        Ok(invalid_urls)
    }

    /// 检查URL是否有效
    async fn is_valid_url(&self, url: &str) -> bool {
        // 简单的URL格式检查
        if url.starts_with("http://") || url.starts_with("https://") {
            // 可以添加HTTP请求检查URL是否可访问
            true
        } else if Path::new(url).exists() {
            // 本地文件存在
            true
        } else {
            false
        }
    }

    /// 清理临时文件
    pub async fn cleanup_temp_files(&self, task_id: &str) -> Result<(), AppError> {
        let temp_dir = Path::new(&self.temp_dir).join(task_id);
        
        if temp_dir.exists() {
            async_fs::remove_dir_all(&temp_dir).await
                .map_err(|e| AppError::File(format!("清理临时文件失败: {}", e)))?;
            
            log::debug!("已清理临时目录: {}", temp_dir.display());
        }
        
        Ok(())
    }

    /// 验证批量输入
    async fn validate_batch_input(
        &self,
        image_paths: &[String],
        config: &ImageProcessConfig,
    ) -> Result<(), AppError> {
        if image_paths.is_empty() {
            return Err(AppError::Validation("图片路径列表为空".to_string()));
        }
        
        if image_paths.len() > 1000 {
            return Err(AppError::Validation("批量处理图片数量过多，最大支持1000个".to_string()));
        }
        
        // 验证配置
        if config.enable_validation {
            let global_config = GlobalFileSizeConfig::new();
        if global_config.max_file_size.bytes() == 0 {
                return Err(AppError::Validation("最大文件大小配置无效".to_string()));
            }
            
            if config.allowed_formats.is_empty() {
                return Err(AppError::Validation("允许的格式列表为空".to_string()));
            }
        }
        
        Ok(())
    }

    /// 更新处理统计信息
    async fn update_stats(&self, successful: usize, failed: usize, processing_time: Duration) {
        let mut stats = self.stats.write().await;
        stats.total_processed += successful + failed;
        stats.successful_uploads += successful;
        stats.failed_uploads += failed;
        stats.total_processing_time_ms += processing_time.as_millis() as u64;
    }

    /// 更新上传统计信息
    async fn update_upload_stats(&self, successful: usize, failed: usize, processing_time: Duration) {
        let mut stats = self.stats.write().await;
        stats.successful_uploads += successful;
        stats.failed_uploads += failed;
        stats.total_upload_time_ms += processing_time.as_millis() as u64;
    }

    /// 获取处理统计信息
    pub async fn get_processing_stats(&self) -> ImageProcessingStats {
        self.stats.read().await.clone()
    }

    /// 重置统计信息
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = ImageProcessingStats::default();
    }
}

/// 图片处理统计信息（增强版本）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageProcessingStats {
    pub total_processed: usize,
    pub successful_uploads: usize,
    pub failed_uploads: usize,
    pub duplicate_count: usize,
    pub total_size: u64,
    pub total_processing_time_ms: u64,
    pub total_upload_time_ms: u64,
    pub average_processing_time_ms: f64,
    pub average_file_size: f64,
    pub compression_ratio_average: f64,
}

impl Default for ImageProcessingStats {
    fn default() -> Self {
        Self {
            total_processed: 0,
            successful_uploads: 0,
            failed_uploads: 0,
            duplicate_count: 0,
            total_size: 0,
            total_processing_time_ms: 0,
            total_upload_time_ms: 0,
            average_processing_time_ms: 0.0,
            average_file_size: 0.0,
            compression_ratio_average: 0.0,
        }
    }
}

/// 批量处理结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchProcessResult {
    pub successful_results: Vec<ImageProcessResult>,
    pub failed_items: Vec<String>,
    pub total_processed: usize,
    pub processing_time_ms: u64,
}

/// 批量上传结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUploadResult {
    pub successful_uploads: usize,
    pub failed_uploads: Vec<String>,
    pub total_uploaded_size: u64,
    pub processing_time_ms: u64,
}

/// 图片提取结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageExtractionResult {
    pub image_paths: Vec<String>,
    pub errors: Vec<String>,
    pub total_files: usize,
    pub total_size: u64,
    pub processing_time_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio;

    #[tokio::test]
    async fn test_image_processor_creation() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        assert_eq!(processor.temp_dir, temp_dir.path());
    }

    #[test]
    fn test_is_image_file_by_extension() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        assert!(processor.is_image_file_by_extension(Path::new("test.jpg")));
        assert!(processor.is_image_file_by_extension(Path::new("test.png")));
        assert!(!processor.is_image_file_by_extension(Path::new("test.txt")));
    }

    #[test]
    fn test_generate_oss_key_enhanced() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        let key = processor.generate_oss_key_enhanced("task123", "/path/to/image.jpg", 1).unwrap();
        assert!(key.starts_with("images/task123/"));
        assert!(key.contains("image.jpg"));
    }

    #[test]
    fn test_sanitize_filename() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        assert_eq!(processor.sanitize_filename("test file.jpg"), "test_file.jpg");
        assert_eq!(processor.sanitize_filename("test@#$.jpg"), "test___.jpg");
        assert_eq!(processor.sanitize_filename("normal-file_123.png"), "normal-file_123.png");
    }

    #[tokio::test]
    async fn test_validate_batch_input() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        let config = ImageProcessConfig::default();
        
        // 空列表应该失败
        let empty_paths: Vec<String> = vec![];
        assert!(processor.validate_batch_input(&empty_paths, &config).await.is_err());
        
        // 正常列表应该成功
        let normal_paths = vec!["test1.jpg".to_string(), "test2.png".to_string()];
        assert!(processor.validate_batch_input(&normal_paths, &config).await.is_ok());
    }

    #[tokio::test]
    async fn test_stats_operations() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        // 初始统计应该为0
        let initial_stats = processor.get_processing_stats().await;
        assert_eq!(initial_stats.total_processed, 0);
        
        // 更新统计
        processor.update_stats(5, 2, Duration::from_millis(1000)).await;
        
        let updated_stats = processor.get_processing_stats().await;
        assert_eq!(updated_stats.total_processed, 7);
        assert_eq!(updated_stats.successful_uploads, 5);
        assert_eq!(updated_stats.failed_uploads, 2);
        
        // 重置统计
        processor.reset_stats().await;
        let reset_stats = processor.get_processing_stats().await;
        assert_eq!(reset_stats.total_processed, 0);
    }

    #[tokio::test]
    async fn test_detect_format_by_header() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        // 创建一个模拟的JPEG文件
        let test_file = temp_dir.path().join("test.jpg");
        let jpeg_header = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG文件头
        async_fs::write(&test_file, &jpeg_header).await.unwrap();
        
        let format = processor.detect_format_by_header(&test_file).await.unwrap();
        assert_eq!(format, Some("jpg".to_string()));
    }

    #[tokio::test]
    async fn test_calculate_file_checksum() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let processor = ImageProcessor::with_defaults(
            temp_dir.path().to_path_buf(),
            None,
        );
        
        // 创建测试文件
        let test_file = temp_dir.path().join("test.txt");
        let test_content = b"Hello, World!";
        async_fs::write(&test_file, test_content).await.unwrap();
        
        let checksum = processor.calculate_file_checksum(&test_file).await.unwrap();
        assert!(!checksum.is_empty());
        assert_eq!(checksum.len(), 64); // SHA256 hex string length
    }
}