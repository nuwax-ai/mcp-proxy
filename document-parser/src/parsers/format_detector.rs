use crate::config::{FileSizePurpose, get_file_size_limit};
use crate::models::{DocumentFormat, ParserEngine};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tracing::{debug, warn};

/// 格式检测器
#[derive(Debug, Clone)]
pub struct FormatDetector {
    /// 自定义格式映射规则
    pub custom_mappings: HashMap<String, DocumentFormat>,
    /// 最大文件大小限制 (bytes)
    pub max_file_size: u64,
    /// 魔数检测缓冲区大小
    pub magic_buffer_size: usize,
    /// 安全检查配置
    pub security_config: SecurityConfig,
    /// 性能配置
    pub performance_config: PerformanceConfig,
}

/// 安全检查配置
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// 是否启用文件大小检查
    pub enable_size_check: bool,
    /// 是否启用恶意文件检测
    pub enable_malware_detection: bool,
    /// 是否启用文件名安全检查
    pub enable_filename_validation: bool,
    /// 允许的最大文件大小 (bytes)
    pub max_allowed_size: u64,
    /// 危险文件扩展名黑名单
    pub dangerous_extensions: Vec<String>,
}

/// 性能配置
#[derive(Debug, Clone)]
pub struct PerformanceConfig {
    /// 是否启用缓存
    pub enable_cache: bool,
    /// 缓存大小限制
    pub cache_size_limit: usize,
    /// 是否启用并行检测
    pub enable_parallel_detection: bool,
    /// 检测超时时间 (毫秒)
    pub detection_timeout_ms: u64,
}

/// 格式检测结果
#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub format: DocumentFormat,
    pub confidence: f32,
    pub detection_method: DetectionMethod,
    pub recommended_engine: ParserEngine,
    pub file_size: Option<u64>,
    pub mime_type: Option<String>,
    pub security_status: SecurityStatus,
    pub detection_time_ms: u64,
    pub fallback_methods: Vec<DetectionMethod>,
}

/// 安全状态
#[derive(Debug, Clone, PartialEq)]
pub enum SecurityStatus {
    Safe,
    Suspicious(String),
    Dangerous(String),
    Unknown,
}

/// 检测方法
#[derive(Debug, Clone, PartialEq)]
pub enum DetectionMethod {
    FileExtension,
    MimeType,
    MagicNumber,
    CustomMapping,
    ContentAnalysis,
    HybridDetection,
    FallbackDetection,
}

/// 魔数签名定义
#[derive(Debug, Clone)]
pub struct MagicSignature {
    pub signature: Vec<u8>,
    pub offset: usize,
    pub format: DocumentFormat,
    pub confidence: f32,
    pub description: String,
}

impl FormatDetector {
    /// 创建新的格式检测器
    pub fn new() -> Self {
        Self::with_global_config()
    }

    /// 使用全局配置创建格式检测器
    pub fn with_global_config() -> Self {
        // 安全地获取文件大小限制，如果全局配置未初始化则使用默认值
        let max_file_size = std::panic::catch_unwind(|| {
            get_file_size_limit(&FileSizePurpose::FormatDetector).bytes()
        })
        .unwrap_or(100 * 1024 * 1024);

        Self {
            custom_mappings: HashMap::new(),
            max_file_size,
            magic_buffer_size: 1024, // 1KB for magic number detection
            security_config: SecurityConfig::with_global_config(),
            performance_config: PerformanceConfig::default(),
        }
    }

    /// 验证文件安全性
    fn validate_file_security(&self, file_path: &str) -> Result<()> {
        if !self.security_config.enable_size_check
            && !self.security_config.enable_filename_validation
        {
            return Ok(());
        }

        let path = Path::new(file_path);

        // 文件名验证
        if self.security_config.enable_filename_validation {
            if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
                let ext_lower = extension.to_lowercase();
                if self
                    .security_config
                    .dangerous_extensions
                    .contains(&ext_lower)
                {
                    bail!("危险文件扩展名: {}", extension);
                }
            }

            // 检查文件名中的危险字符
            if let Some(filename) = path.file_name().and_then(|name| name.to_str()) {
                if filename.contains("..") || filename.contains("/") || filename.contains("\\") {
                    bail!("文件名包含危险字符: {}", filename);
                }
            }
        }

        // 文件大小检查
        if self.security_config.enable_size_check {
            if let Ok(metadata) = std::fs::metadata(file_path) {
                let file_size = metadata.len();
                if file_size > self.security_config.max_allowed_size {
                    bail!(
                        "文件大小超过限制: {} bytes (最大: {} bytes)",
                        file_size,
                        self.security_config.max_allowed_size
                    );
                }
            }
        }

        Ok(())
    }

    /// 评估安全状态
    fn assess_security_status(&self, format: &DocumentFormat, file_path: &str) -> SecurityStatus {
        let path = Path::new(file_path);

        // 检查文件扩展名
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            let ext_lower = extension.to_lowercase();
            if self
                .security_config
                .dangerous_extensions
                .contains(&ext_lower)
            {
                return SecurityStatus::Dangerous(format!("危险文件扩展名: {extension}"));
            }
        }

        // 根据格式评估安全性
        match format {
            DocumentFormat::PDF
            | DocumentFormat::Word
            | DocumentFormat::Excel
            | DocumentFormat::PowerPoint => SecurityStatus::Safe,
            DocumentFormat::Image | DocumentFormat::Audio => SecurityStatus::Safe,
            DocumentFormat::HTML => SecurityStatus::Suspicious("HTML文件可能包含脚本".to_string()),
            DocumentFormat::Text | DocumentFormat::Txt | DocumentFormat::Md => SecurityStatus::Safe,
            DocumentFormat::Other(_) => SecurityStatus::Unknown,
        }
    }

    /// 同步版本的魔数检测
    fn detect_by_magic_number_sync(&self, file_path: &str) -> Result<Option<DetectionResult>> {
        self.detect_by_magic_number(file_path)
    }

    /// 异步版本的魔数检测
    async fn detect_by_magic_number_async(
        &self,
        file_path: &str,
    ) -> Result<Option<DetectionResult>> {
        let mut file = tokio::fs::File::open(file_path).await?;
        let mut buffer = [0u8; 16]; // 读取前16字节
        let bytes_read = file.read(&mut buffer).await?;

        if bytes_read < 4 {
            return Ok(None);
        }

        let format = match &buffer[0..4] {
            [0x25, 0x50, 0x44, 0x46] => Some(DocumentFormat::PDF), // %PDF
            [0x50, 0x4B, 0x03, 0x04] | [0x50, 0x4B, 0x05, 0x06] => {
                // ZIP格式，可能是Office文档
                self.detect_office_format(file_path)?
            }
            [0xFF, 0xD8, 0xFF, _] => Some(DocumentFormat::Image), // JPEG
            [0x89, 0x50, 0x4E, 0x47] => Some(DocumentFormat::Image), // PNG
            [0x47, 0x49, 0x46, 0x38] => Some(DocumentFormat::Image), // GIF
            [0x42, 0x4D, _, _] => Some(DocumentFormat::Image),    // BMP
            [0x49, 0x44, 0x33, _] => Some(DocumentFormat::Audio), // MP3 with ID3
            [0xFF, 0xFB, _, _] | [0xFF, 0xF3, _, _] | [0xFF, 0xF2, _, _] => {
                Some(DocumentFormat::Audio)
            } // MP3
            [0x52, 0x49, 0x46, 0x46] => {
                // RIFF格式，可能是WAV
                if bytes_read >= 8 && &buffer[8..12] == b"WAVE" {
                    Some(DocumentFormat::Audio)
                } else {
                    None
                }
            }
            _ => None,
        };

        Ok(format.map(|f| DetectionResult {
            format: f.clone(),
            confidence: 0.95,
            detection_method: DetectionMethod::MagicNumber,
            recommended_engine: Self::select_engine_for_format(&f),
            file_size: None,
            mime_type: None,
            security_status: SecurityStatus::Safe,
            detection_time_ms: 0,
            fallback_methods: Vec::new(),
        }))
    }

    /// 内容分析检测
    fn detect_by_content_analysis(&self, file_path: &str) -> Result<Option<DetectionResult>> {
        let mut file = File::open(file_path)?;
        let mut buffer = [0u8; 512]; // 读取前512字节进行内容分析
        let bytes_read = file.read(&mut buffer)?;

        if bytes_read == 0 {
            return Ok(None);
        }

        // 检查是否为文本文件
        let text_ratio = buffer[..bytes_read]
            .iter()
            .filter(|&&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
            .count() as f32
            / bytes_read as f32;

        if text_ratio > 0.8 {
            // 进一步分析文本内容
            if let Ok(content) = String::from_utf8(buffer[..bytes_read].to_vec()) {
                let format = if content.starts_with("<!DOCTYPE html") || content.contains("<html") {
                    DocumentFormat::HTML
                } else if content.starts_with('#') || content.contains("##") {
                    DocumentFormat::Md
                } else {
                    DocumentFormat::Text
                };

                return Ok(Some(DetectionResult {
                    format: format.clone(),
                    confidence: 0.7,
                    detection_method: DetectionMethod::ContentAnalysis,
                    recommended_engine: Self::select_engine_for_format(&format),
                    file_size: None,
                    mime_type: None,
                    security_status: SecurityStatus::Safe,
                    detection_time_ms: 0,
                    fallback_methods: Vec::new(),
                }));
            }
        }

        Ok(None)
    }

    /// 创建带配置的格式检测器
    pub fn with_config(
        security_config: SecurityConfig,
        performance_config: PerformanceConfig,
    ) -> Self {
        Self {
            custom_mappings: HashMap::new(),
            max_file_size: security_config.max_allowed_size,
            magic_buffer_size: 1024,
            security_config,
            performance_config,
        }
    }

    /// 添加自定义格式映射
    pub fn add_custom_mapping(&mut self, extension: String, format: DocumentFormat) {
        self.custom_mappings
            .insert(extension.to_lowercase(), format);
    }

    /// 检测文件格式 (同步版本)
    pub fn detect_format(
        &self,
        file_path: &str,
        mime_type: Option<&str>,
    ) -> Result<DetectionResult> {
        let start_time = Instant::now();
        debug!("Start detecting file format: {}", file_path);

        // 1. 安全检查
        self.validate_file_security(file_path)?;

        // 2. 获取文件大小
        let file_size = std::fs::metadata(file_path)
            .context("无法获取文件元数据")?
            .len();

        if file_size > self.max_file_size {
            bail!("文件大小超过限制: {} bytes", file_size);
        }

        let mut fallback_methods = Vec::new();
        let mut best_result: Option<DetectionResult> = None;

        // 3. 多重检测策略
        // 自定义映射检测
        if let Some(mut result) = self.detect_by_custom_mapping(file_path) {
            result.file_size = Some(file_size);
            result.detection_time_ms = start_time.elapsed().as_millis() as u64;
            result.fallback_methods = fallback_methods.clone();
            result.security_status = self.assess_security_status(&result.format, file_path);

            if result.confidence >= 0.9 {
                debug!(
                    "High confidence detection successful: custom_mapping ({})",
                    result.confidence
                );
                return Ok(result);
            }
            best_result = Some(result);
        } else {
            fallback_methods.push(DetectionMethod::CustomMapping);
        }

        // 魔数检测
        if best_result.as_ref().is_none_or(|r| r.confidence < 0.9) {
            match self.detect_by_magic_number_sync(file_path) {
                Ok(Some(mut result)) => {
                    result.file_size = Some(file_size);
                    result.detection_time_ms = start_time.elapsed().as_millis() as u64;
                    result.fallback_methods = fallback_methods.clone();
                    result.security_status = self.assess_security_status(&result.format, file_path);

                    if best_result.is_none()
                        || result.confidence > best_result.as_ref().unwrap().confidence
                    {
                        best_result = Some(result);
                    }

                    if best_result.as_ref().unwrap().confidence >= 0.9 {
                        debug!(
                            "High confidence detection successful: magic_number ({})",
                            best_result.as_ref().unwrap().confidence
                        );
                        return Ok(best_result.unwrap());
                    }
                }
                Ok(None) => {
                    fallback_methods.push(DetectionMethod::MagicNumber);
                }
                Err(e) => {
                    warn!("Magic number detection failed: {}", e);
                    fallback_methods.push(DetectionMethod::MagicNumber);
                }
            }
        }

        // MIME类型检测
        if best_result.as_ref().is_none_or(|r| r.confidence < 0.9) {
            if let Some(mime) = mime_type {
                if let Some(mut result) = self.detect_by_mime_type(mime) {
                    result.file_size = Some(file_size);
                    result.detection_time_ms = start_time.elapsed().as_millis() as u64;
                    result.fallback_methods = fallback_methods.clone();
                    result.security_status = self.assess_security_status(&result.format, file_path);

                    if best_result.is_none()
                        || result.confidence > best_result.as_ref().unwrap().confidence
                    {
                        best_result = Some(result);
                    }
                } else {
                    fallback_methods.push(DetectionMethod::MimeType);
                }
            } else {
                fallback_methods.push(DetectionMethod::MimeType);
            }
        }

        // 扩展名检测
        if best_result.as_ref().is_none_or(|r| r.confidence < 0.9) {
            if let Some(mut result) = self.detect_by_extension(file_path) {
                result.file_size = Some(file_size);
                result.detection_time_ms = start_time.elapsed().as_millis() as u64;
                result.fallback_methods = fallback_methods.clone();
                result.security_status = self.assess_security_status(&result.format, file_path);

                if best_result.is_none()
                    || result.confidence > best_result.as_ref().unwrap().confidence
                {
                    best_result = Some(result);
                }
            } else {
                fallback_methods.push(DetectionMethod::FileExtension);
            }
        }

        // 内容分析检测
        if best_result.as_ref().is_none_or(|r| r.confidence < 0.9) {
            match self.detect_by_content_analysis(file_path) {
                Ok(Some(mut result)) => {
                    result.file_size = Some(file_size);
                    result.detection_time_ms = start_time.elapsed().as_millis() as u64;
                    result.fallback_methods = fallback_methods.clone();
                    result.security_status = self.assess_security_status(&result.format, file_path);

                    if best_result.is_none()
                        || result.confidence > best_result.as_ref().unwrap().confidence
                    {
                        best_result = Some(result);
                    }
                }
                Ok(None) => {
                    fallback_methods.push(DetectionMethod::ContentAnalysis);
                }
                Err(e) => {
                    warn!("Content analysis detection failed: {}", e);
                    fallback_methods.push(DetectionMethod::ContentAnalysis);
                }
            }
        }

        // 4. 返回最佳结果或默认结果
        let mut final_result = best_result.unwrap_or_else(|| DetectionResult {
            format: DocumentFormat::Other("unknown".to_string()),
            confidence: 0.1,
            detection_method: DetectionMethod::FallbackDetection,
            recommended_engine: ParserEngine::MarkItDown,
            file_size: Some(file_size),
            mime_type: mime_type.map(|s| s.to_string()),
            security_status: SecurityStatus::Unknown,
            detection_time_ms: start_time.elapsed().as_millis() as u64,
            fallback_methods: fallback_methods.clone(),
        });

        // 确保所有字段都正确设置
        final_result.file_size = Some(file_size);
        final_result.mime_type = mime_type.map(|s| s.to_string());
        final_result.detection_time_ms = start_time.elapsed().as_millis() as u64;
        if final_result.fallback_methods.is_empty() {
            final_result.fallback_methods = fallback_methods;
        }
        if final_result.security_status == SecurityStatus::Safe {
            final_result.security_status =
                self.assess_security_status(&final_result.format, file_path);
        }

        debug!("File format detection completed: {:?}", final_result);
        Ok(final_result)
    }

    /// 异步检测文件格式
    pub async fn detect_format_async(
        &self,
        file_path: &str,
        mime_type: Option<&str>,
    ) -> Result<DetectionResult> {
        let start_time = Instant::now();
        debug!("Start asynchronous detection of file format: {}", file_path);

        // 1. 安全检查
        self.validate_file_security(file_path)?;

        // 2. 获取文件大小
        let metadata = tokio::fs::metadata(file_path)
            .await
            .context("无法获取文件元数据")?;
        let file_size = metadata.len();

        if file_size > self.max_file_size {
            bail!("文件大小超过限制: {} bytes", file_size);
        }

        let mut fallback_methods = Vec::new();
        let mut best_result: Option<DetectionResult> = None;

        // 3. 异步多重检测策略
        if let Some(result) = self.detect_by_custom_mapping(file_path) {
            best_result = Some(result);
        }

        if best_result.is_none() || best_result.as_ref().unwrap().confidence < 0.9 {
            if let Ok(Some(result)) = self.detect_by_magic_number_async(file_path).await {
                if best_result.is_none()
                    || result.confidence > best_result.as_ref().unwrap().confidence
                {
                    best_result = Some(result);
                }
            } else {
                fallback_methods.push(DetectionMethod::MagicNumber);
            }
        }

        if best_result.is_none() || best_result.as_ref().unwrap().confidence < 0.9 {
            if let Some(mime) = mime_type {
                if let Some(result) = self.detect_by_mime_type(mime) {
                    if best_result.is_none()
                        || result.confidence > best_result.as_ref().unwrap().confidence
                    {
                        best_result = Some(result);
                    }
                } else {
                    fallback_methods.push(DetectionMethod::MimeType);
                }
            }
        }

        if best_result.is_none() || best_result.as_ref().unwrap().confidence < 0.9 {
            if let Some(result) = self.detect_by_extension(file_path) {
                if best_result.is_none()
                    || result.confidence > best_result.as_ref().unwrap().confidence
                {
                    best_result = Some(result);
                }
            } else {
                fallback_methods.push(DetectionMethod::FileExtension);
            }
        }

        // 4. 设置最终结果属性
        let mut final_result = best_result.unwrap_or_else(|| DetectionResult {
            format: DocumentFormat::Other("unknown".to_string()),
            confidence: 0.1,
            detection_method: DetectionMethod::FallbackDetection,
            recommended_engine: ParserEngine::MarkItDown,
            file_size: Some(file_size),
            mime_type: mime_type.map(|s| s.to_string()),
            security_status: SecurityStatus::Unknown,
            detection_time_ms: start_time.elapsed().as_millis() as u64,
            fallback_methods: fallback_methods.clone(),
        });

        final_result.file_size = Some(file_size);
        final_result.mime_type = mime_type.map(|s| s.to_string());
        final_result.detection_time_ms = start_time.elapsed().as_millis() as u64;
        final_result.fallback_methods = fallback_methods;
        final_result.security_status = self.assess_security_status(&final_result.format, file_path);

        debug!(
            "Asynchronous file format detection completed: {:?}",
            final_result
        );
        Ok(final_result)
    }

    /// 通过自定义映射检测
    fn detect_by_custom_mapping(&self, file_path: &str) -> Option<DetectionResult> {
        let extension = Path::new(file_path).extension()?.to_str()?.to_lowercase();

        self.custom_mappings
            .get(&extension)
            .map(|format| DetectionResult {
                format: format.clone(),
                confidence: 1.0,
                detection_method: DetectionMethod::CustomMapping,
                recommended_engine: Self::select_engine_for_format(format),
                file_size: None,
                mime_type: None,
                security_status: SecurityStatus::Safe,
                detection_time_ms: 0,
                fallback_methods: Vec::new(),
            })
    }

    /// 通过文件扩展名检测
    fn detect_by_extension(&self, file_path: &str) -> Option<DetectionResult> {
        let extension = Path::new(file_path).extension()?.to_str()?.to_lowercase();

        let format = DocumentFormat::from_extension(&extension);

        // 如果是Other格式，说明不支持
        if matches!(format, DocumentFormat::Other(_)) {
            return None;
        }

        // 根据扩展名的常见程度调整置信度
        let confidence = match extension.as_str() {
            "pdf" | "docx" | "xlsx" | "pptx" => 0.9,
            "doc" | "xls" | "ppt" => 0.8,
            "jpg" | "jpeg" | "png" => 0.85,
            "mp3" | "wav" => 0.8,
            "txt" | "md" => 0.7,
            _ => 0.6,
        };

        Some(DetectionResult {
            format: format.clone(),
            confidence,
            detection_method: DetectionMethod::FileExtension,
            recommended_engine: Self::select_engine_for_format(&format),
            file_size: None,
            mime_type: None,
            security_status: SecurityStatus::Safe,
            detection_time_ms: 0,
            fallback_methods: Vec::new(),
        })
    }

    /// 通过MIME类型检测
    fn detect_by_mime_type(&self, mime_type: &str) -> Option<DetectionResult> {
        let format = DocumentFormat::from_mime_type(mime_type);

        // 如果是Other格式，说明不支持
        if matches!(format, DocumentFormat::Other(_)) {
            return None;
        }

        // 根据MIME类型的准确性调整置信度
        let confidence = match mime_type {
            "application/pdf" => 0.95,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => 0.9,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => 0.9,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => 0.9,
            "image/jpeg" | "image/png" => 0.85,
            "audio/mpeg" | "audio/wav" => 0.8,
            "text/plain" => 0.6, // 可能不准确
            _ => 0.7,
        };

        Some(DetectionResult {
            format: format.clone(),
            confidence,
            detection_method: DetectionMethod::MimeType,
            recommended_engine: Self::select_engine_for_format(&format),
            file_size: None,
            mime_type: Some(mime_type.to_string()),
            security_status: SecurityStatus::Safe,
            detection_time_ms: 0,
            fallback_methods: Vec::new(),
        })
    }

    /// 通过文件头魔数检测
    fn detect_by_magic_number(&self, file_path: &str) -> Result<Option<DetectionResult>> {
        let mut file = File::open(file_path)?;
        let mut buffer = [0u8; 16]; // 读取前16字节
        let bytes_read = file.read(&mut buffer)?;

        if bytes_read < 4 {
            return Ok(None);
        }

        let format = match &buffer[0..4] {
            [0x25, 0x50, 0x44, 0x46] => Some(DocumentFormat::PDF), // %PDF
            [0x50, 0x4B, 0x03, 0x04] | [0x50, 0x4B, 0x05, 0x06] => {
                // ZIP格式，可能是Office文档
                self.detect_office_format(file_path)?
            }
            [0xFF, 0xD8, 0xFF, _] => Some(DocumentFormat::Image), // JPEG
            [0x89, 0x50, 0x4E, 0x47] => Some(DocumentFormat::Image), // PNG
            [0x47, 0x49, 0x46, 0x38] => Some(DocumentFormat::Image), // GIF
            [0x42, 0x4D, _, _] => Some(DocumentFormat::Image),    // BMP
            [0x49, 0x44, 0x33, _] => Some(DocumentFormat::Audio), // MP3 with ID3
            [0xFF, 0xFB, _, _] | [0xFF, 0xF3, _, _] | [0xFF, 0xF2, _, _] => {
                Some(DocumentFormat::Audio)
            } // MP3
            [0x52, 0x49, 0x46, 0x46] => {
                // RIFF格式，可能是WAV
                if bytes_read >= 8 && &buffer[8..12] == b"WAVE" {
                    Some(DocumentFormat::Audio)
                } else {
                    None
                }
            }
            _ => None,
        };

        Ok(format.map(|f| DetectionResult {
            format: f.clone(),
            confidence: 0.95,
            detection_method: DetectionMethod::MagicNumber,
            recommended_engine: Self::select_engine_for_format(&f),
            file_size: None,
            mime_type: None,
            security_status: SecurityStatus::Safe,
            detection_time_ms: 0,
            fallback_methods: Vec::new(),
        }))
    }

    /// 检测Office文档格式
    fn detect_office_format(&self, file_path: &str) -> Result<Option<DocumentFormat>> {
        let extension = Path::new(file_path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_lowercase());

        match extension.as_deref() {
            Some("docx") | Some("doc") => Ok(Some(DocumentFormat::Word)),
            Some("xlsx") | Some("xls") => Ok(Some(DocumentFormat::Excel)),
            Some("pptx") | Some("ppt") => Ok(Some(DocumentFormat::PowerPoint)),
            _ => Ok(None),
        }
    }

    /// 为格式选择推荐的解析引擎
    fn select_engine_for_format(format: &DocumentFormat) -> ParserEngine {
        match format {
            DocumentFormat::PDF => ParserEngine::MinerU,
            DocumentFormat::Word
            | DocumentFormat::Excel
            | DocumentFormat::PowerPoint
            | DocumentFormat::Image
            | DocumentFormat::Audio
            | DocumentFormat::HTML
            | DocumentFormat::Text
            | DocumentFormat::Txt
            | DocumentFormat::Md => ParserEngine::MarkItDown,
            DocumentFormat::Other(_) => ParserEngine::MarkItDown,
        }
    }

    /// 批量检测文件格式
    pub fn detect_batch(&self, files: &[(String, Option<String>)]) -> Vec<Result<DetectionResult>> {
        files
            .iter()
            .map(|(path, mime)| self.detect_format(path, mime.as_deref()))
            .collect()
    }

    /// 获取支持的格式列表
    pub fn get_supported_formats() -> Vec<DocumentFormat> {
        vec![
            DocumentFormat::PDF,
            DocumentFormat::Word,
            DocumentFormat::Excel,
            DocumentFormat::PowerPoint,
            DocumentFormat::Image,
            DocumentFormat::Audio,
            DocumentFormat::HTML,
            DocumentFormat::Text,
            DocumentFormat::Txt,
            DocumentFormat::Md,
        ]
    }

    /// 检查格式是否支持
    pub fn is_format_supported(format: &DocumentFormat) -> bool {
        !matches!(format, DocumentFormat::Other(_))
    }

    /// 获取格式的置信度阈值
    pub fn get_confidence_threshold() -> f32 {
        0.7
    }

    /// 获取魔数签名定义
    pub fn get_magic_signatures() -> Vec<MagicSignature> {
        vec![
            MagicSignature {
                signature: vec![0x25, 0x50, 0x44, 0x46], // %PDF
                offset: 0,
                format: DocumentFormat::PDF,
                confidence: 0.95,
                description: "PDF文档".to_string(),
            },
            MagicSignature {
                signature: vec![0x50, 0x4B, 0x03, 0x04], // ZIP
                offset: 0,
                format: DocumentFormat::Word, // 可能是Office文档
                confidence: 0.8,
                description: "ZIP压缩文件（可能是Office文档）".to_string(),
            },
            MagicSignature {
                signature: vec![0xFF, 0xD8, 0xFF], // JPEG
                offset: 0,
                format: DocumentFormat::Image,
                confidence: 0.95,
                description: "JPEG图像".to_string(),
            },
            MagicSignature {
                signature: vec![0x89, 0x50, 0x4E, 0x47], // PNG
                offset: 0,
                format: DocumentFormat::Image,
                confidence: 0.95,
                description: "PNG图像".to_string(),
            },
            MagicSignature {
                signature: vec![0x47, 0x49, 0x46, 0x38], // GIF
                offset: 0,
                format: DocumentFormat::Image,
                confidence: 0.95,
                description: "GIF图像".to_string(),
            },
            MagicSignature {
                signature: vec![0x42, 0x4D], // BMP
                offset: 0,
                format: DocumentFormat::Image,
                confidence: 0.9,
                description: "BMP图像".to_string(),
            },
            MagicSignature {
                signature: vec![0x49, 0x44, 0x33], // MP3 with ID3
                offset: 0,
                format: DocumentFormat::Audio,
                confidence: 0.9,
                description: "MP3音频（带ID3标签）".to_string(),
            },
            MagicSignature {
                signature: vec![0xFF, 0xFB], // MP3
                offset: 0,
                format: DocumentFormat::Audio,
                confidence: 0.85,
                description: "MP3音频".to_string(),
            },
            MagicSignature {
                signature: vec![0x52, 0x49, 0x46, 0x46], // RIFF (WAV)
                offset: 0,
                format: DocumentFormat::Audio,
                confidence: 0.8,
                description: "RIFF格式（可能是WAV音频）".to_string(),
            },
        ]
    }

    /// 验证检测结果
    pub fn validate_detection_result(
        &self,
        result: &DetectionResult,
        file_path: &str,
    ) -> Result<bool> {
        // 检查置信度
        if result.confidence < 0.1 || result.confidence > 1.0 {
            return Ok(false);
        }

        // 检查格式与文件扩展名的一致性
        if let Some(extension) = Path::new(file_path)
            .extension()
            .and_then(|ext| ext.to_str())
        {
            let expected_format = DocumentFormat::from_extension(extension);
            if !matches!(expected_format, DocumentFormat::Other(_))
                && expected_format != result.format
            {
                // 如果扩展名和检测结果不一致，降低置信度
                warn!(
                    "The format detection result is inconsistent with the file extension: detection={:?}, extension={:?}",
                    result.format, expected_format
                );
            }
        }

        // 检查推荐引擎是否正确
        let expected_engine = Self::select_engine_for_format(&result.format);
        if result.recommended_engine != expected_engine {
            warn!(
                "Incorrect recommendation engine: detect={:?}, expect={:?}",
                result.recommended_engine, expected_engine
            );
        }

        Ok(true)
    }

    /// 获取详细的检测统计信息
    pub fn get_detection_stats(&self) -> HashMap<String, u64> {
        let mut stats = HashMap::new();
        stats.insert("total_detections".to_string(), 0);
        stats.insert("successful_detections".to_string(), 0);
        stats.insert("failed_detections".to_string(), 0);
        stats.insert("high_confidence_detections".to_string(), 0);
        stats.insert("low_confidence_detections".to_string(), 0);
        stats
    }
}

impl SecurityConfig {
    /// 使用全局配置创建安全配置
    pub fn with_global_config() -> Self {
        // 安全地获取文件大小限制，如果全局配置未初始化则使用默认值
        let max_allowed_size = std::panic::catch_unwind(|| {
            get_file_size_limit(&FileSizePurpose::FormatDetector).bytes()
        })
        .unwrap_or(100 * 1024 * 1024);

        Self {
            enable_size_check: true,
            enable_malware_detection: false,
            enable_filename_validation: true,
            max_allowed_size,
            dangerous_extensions: vec![
                "exe".to_string(),
                "bat".to_string(),
                "cmd".to_string(),
                "scr".to_string(),
                "com".to_string(),
                "pif".to_string(),
                "vbs".to_string(),
                "js".to_string(),
                "jar".to_string(),
            ],
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self::with_global_config()
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            enable_cache: true,
            cache_size_limit: 1000,
            enable_parallel_detection: true,
            detection_timeout_ms: 5000,
        }
    }
}

impl Default for FormatDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extension_detection() {
        let config = crate::tests::test_helpers::create_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        let result = detector.detect_by_extension("test.pdf").unwrap();
        assert!(matches!(result.format, DocumentFormat::PDF));
        assert_eq!(result.detection_method, DetectionMethod::FileExtension);
        assert!(result.confidence > 0.8);
    }

    #[test]
    fn test_mime_type_detection() {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        let result = detector.detect_by_mime_type("application/pdf").unwrap();
        assert!(matches!(result.format, DocumentFormat::PDF));
        assert_eq!(result.detection_method, DetectionMethod::MimeType);
    }

    #[test]
    fn test_custom_mapping() {
        let config = crate::tests::test_helpers::create_test_config();
        crate::config::init_global_config(config).unwrap();
        let mut detector = FormatDetector::new();
        detector.add_custom_mapping("custom".to_string(), DocumentFormat::Text);

        let result = detector.detect_by_custom_mapping("test.custom").unwrap();
        assert!(matches!(result.format, DocumentFormat::Text));
        assert_eq!(result.detection_method, DetectionMethod::CustomMapping);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn test_pdf_magic_number() -> Result<()> {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 创建一个临时PDF文件
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"%PDF-1.4\n")?;
        temp_file.flush()?;

        let result = detector.detect_format(temp_file.path().to_str().unwrap(), None)?;
        assert!(matches!(result.format, DocumentFormat::PDF));
        assert_eq!(result.detection_method, DetectionMethod::MagicNumber);

        Ok(())
    }

    #[test]
    fn test_jpeg_magic_number() -> Result<()> {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 创建一个临时JPEG文件
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(&[0xFF, 0xD8, 0xFF, 0xE0])?;
        temp_file.flush()?;

        let result = detector.detect_format(temp_file.path().to_str().unwrap(), None)?;
        assert!(matches!(result.format, DocumentFormat::Image));
        assert_eq!(result.detection_method, DetectionMethod::MagicNumber);

        Ok(())
    }

    #[test]
    fn test_png_magic_number() -> Result<()> {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 创建一个临时PNG文件
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])?;
        temp_file.flush()?;

        let result = detector.detect_format(temp_file.path().to_str().unwrap(), None)?;
        assert!(matches!(result.format, DocumentFormat::Image));
        assert_eq!(result.detection_method, DetectionMethod::MagicNumber);

        Ok(())
    }

    #[test]
    fn test_security_validation() -> Result<()> {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 测试安全文件
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"Hello, world!")?;
        temp_file.flush()?;

        let result = detector.validate_file_security(temp_file.path().to_str().unwrap());
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_dangerous_extension_detection() {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        let result = detector.validate_file_security("malware.exe");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("危险文件扩展名"));
    }

    #[test]
    fn test_content_analysis_html() -> Result<()> {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 创建一个临时HTML文件
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(
            b"<!DOCTYPE html><html><head><title>Test</title></head><body>Hello</body></html>",
        )?;
        temp_file.flush()?;

        let result = detector.detect_format(temp_file.path().to_str().unwrap(), None)?;
        assert!(matches!(result.format, DocumentFormat::HTML));
        assert_eq!(result.detection_method, DetectionMethod::ContentAnalysis);

        Ok(())
    }

    #[test]
    fn test_content_analysis_markdown() -> Result<()> {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 创建一个临时Markdown文件
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"# Test\n\nThis is a test markdown file.")?;
        temp_file.flush()?;

        let result = detector.detect_format(temp_file.path().to_str().unwrap(), None)?;
        assert!(matches!(result.format, DocumentFormat::Md));
        assert_eq!(result.detection_method, DetectionMethod::ContentAnalysis);

        Ok(())
    }

    #[test]
    fn test_engine_selection() {
        assert_eq!(
            FormatDetector::select_engine_for_format(&DocumentFormat::PDF),
            ParserEngine::MinerU
        );
        assert_eq!(
            FormatDetector::select_engine_for_format(&DocumentFormat::Word),
            ParserEngine::MarkItDown
        );
    }

    #[test]
    fn test_supported_formats() {
        let formats = FormatDetector::get_supported_formats();
        assert!(!formats.is_empty());
        assert!(formats.contains(&DocumentFormat::PDF));
        assert!(formats.contains(&DocumentFormat::Word));
    }

    #[test]
    fn test_format_support_check() {
        assert!(FormatDetector::is_format_supported(&DocumentFormat::PDF));
        assert!(FormatDetector::is_format_supported(&DocumentFormat::Word));
        assert!(!FormatDetector::is_format_supported(
            &DocumentFormat::Other("unknown".to_string())
        ));
    }

    #[test]
    fn test_confidence_threshold() {
        let threshold = FormatDetector::get_confidence_threshold();
        assert!(threshold > 0.0 && threshold <= 1.0);
    }

    #[test]
    fn test_magic_signatures() {
        let signatures = FormatDetector::get_magic_signatures();
        assert!(!signatures.is_empty());

        // 检查PDF签名
        let pdf_sig = signatures
            .iter()
            .find(|s| matches!(s.format, DocumentFormat::PDF));
        assert!(pdf_sig.is_some());
        assert_eq!(pdf_sig.unwrap().signature, vec![0x25, 0x50, 0x44, 0x46]);
    }

    #[test]
    fn test_security_config_default() {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 测试默认安全配置
        let security_config = &detector.security_config;
        assert_eq!(security_config.max_allowed_size, 200 * 1024 * 1024); // 200MB
        assert_eq!(security_config.dangerous_extensions.len(), 9); // 默认包含9个危险扩展名
    }

    #[test]
    fn test_performance_config_default() {
        let config = PerformanceConfig::default();
        assert!(config.enable_cache);
        assert!(config.enable_parallel_detection);
        assert!(config.detection_timeout_ms > 0);
    }

    #[test]
    fn test_batch_detection() -> Result<()> {
        let config = crate::tests::test_helpers::create_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 创建临时文件
        let mut pdf_file = NamedTempFile::new()?;
        pdf_file.write_all(b"%PDF-1.4\n")?;
        pdf_file.flush()?;

        let mut txt_file = NamedTempFile::new()?;
        txt_file.write_all(b"Hello, world!")?;
        txt_file.flush()?;

        let files = vec![
            (
                pdf_file.path().to_str().unwrap().to_string(),
                Some("application/pdf".to_string()),
            ),
            (
                txt_file.path().to_str().unwrap().to_string(),
                Some("text/plain".to_string()),
            ),
        ];

        let results = detector.detect_batch(&files);
        assert_eq!(results.len(), 2);

        // 检查第一个结果（PDF）
        assert!(results[0].is_ok());
        let pdf_result = results[0].as_ref().unwrap();
        assert!(matches!(pdf_result.format, DocumentFormat::PDF));

        Ok(())
    }

    #[tokio::test]
    async fn test_async_detection() -> Result<()> {
        let config = crate::tests::test_helpers::create_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 创建一个临时PDF文件
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"%PDF-1.4\n")?;
        temp_file.flush()?;

        let result = detector
            .detect_format_async(temp_file.path().to_str().unwrap(), Some("application/pdf"))
            .await?;

        assert!(matches!(result.format, DocumentFormat::PDF));
        assert!(result.confidence > 0.9);
        assert!(result.file_size.is_some());
        assert!(result.detection_time_ms >= 0);

        Ok(())
    }

    #[test]
    fn test_detection_result_validation() -> Result<()> {
        let config = crate::tests::test_helpers::create_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        let valid_result = DetectionResult {
            format: DocumentFormat::PDF,
            confidence: 0.95,
            detection_method: DetectionMethod::MagicNumber,
            recommended_engine: ParserEngine::MinerU,
            file_size: Some(1024),
            mime_type: Some("application/pdf".to_string()),
            security_status: SecurityStatus::Safe,
            detection_time_ms: 100,
            fallback_methods: Vec::new(),
        };

        assert!(detector.validate_detection_result(&valid_result, "test.pdf")?);

        let invalid_result = DetectionResult {
            format: DocumentFormat::PDF,
            confidence: 1.5, // 无效的置信度
            detection_method: DetectionMethod::MagicNumber,
            recommended_engine: ParserEngine::MinerU,
            file_size: Some(1024),
            mime_type: Some("application/pdf".to_string()),
            security_status: SecurityStatus::Safe,
            detection_time_ms: 100,
            fallback_methods: Vec::new(),
        };

        assert!(!detector.validate_detection_result(&invalid_result, "test.pdf")?);

        Ok(())
    }

    #[test]
    fn test_security_status_assessment() {
        let config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(config).unwrap();
        let detector = FormatDetector::new();

        // 测试安全状态评估
        let status = detector.assess_security_status(&DocumentFormat::PDF, "test.pdf");
        assert_eq!(status, SecurityStatus::Safe);

        // 测试危险文件
        let status =
            detector.assess_security_status(&DocumentFormat::Other("exe".to_string()), "test.exe");
        assert_eq!(
            status,
            SecurityStatus::Dangerous("危险文件扩展名: exe".to_string())
        );
    }
}
