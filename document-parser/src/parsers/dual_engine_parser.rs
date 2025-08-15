use std::sync::Arc;

use crate::error::AppError;
use crate::models::{DocumentFormat, ParseResult};
use crate::config::{MinerUConfig as ConfigMinerUConfig, MarkItDownConfig as ConfigMarkItDownConfig};
use super::mineru_parser::MinerUConfig;
use super::markitdown_parser::MarkItDownConfig;
use super::parser_trait::DocumentParser;
use super::{MinerUParser, MarkItDownParser};

/// 双引擎解析器管理器
pub struct DualEngineParser {
    mineru_parser: Arc<MinerUParser>,
    markitdown_parser: Arc<MarkItDownParser>,
}

impl DualEngineParser {
    /// 创建新的双引擎解析器
    pub fn new(mineru_config: &ConfigMinerUConfig, markitdown_config: &ConfigMarkItDownConfig) -> Self {
        Self::with_timeout(mineru_config, markitdown_config, 3600) // 默认60分钟超时
    }

    /// 创建带指定超时的双引擎解析器
    pub fn with_timeout(mineru_config: &ConfigMinerUConfig, markitdown_config: &ConfigMarkItDownConfig, default_timeout_seconds: u32) -> Self {
        // 转换配置类型
        let mineru_parser_config = MinerUConfig {
            python_path: mineru_config.get_effective_python_path(),
            backend: mineru_config.backend.clone(),
            max_concurrent: mineru_config.max_concurrent,
            queue_size: mineru_config.queue_size,
            timeout: if mineru_config.timeout == 0 { default_timeout_seconds } else { mineru_config.timeout },
            batch_size: mineru_config.batch_size,
            quality_level: mineru_config.quality_level.clone(),
        };
        
        let markitdown_parser_config = MarkItDownConfig::with_global_config();
        let markitdown_parser_config = MarkItDownConfig {
             python_path: markitdown_config.get_effective_python_path(),
             enable_plugins: markitdown_config.enable_plugins,
             timeout_seconds: (if markitdown_config.timeout == 0 { default_timeout_seconds } else { markitdown_config.timeout }) as u64,
             supported_formats: markitdown_parser_config.supported_formats,
             output_format: markitdown_parser_config.output_format,
             quality_settings: markitdown_parser_config.quality_settings,
         };
        
        let mineru_parser = Arc::new(MinerUParser::new(mineru_parser_config));
        let markitdown_parser = Arc::new(MarkItDownParser::new(markitdown_parser_config));
        
        Self {
            mineru_parser,
            markitdown_parser,
        }
    }

    /// 创建自动检测当前目录虚拟环境的双引擎解析器
    pub fn with_auto_venv_detection() -> Result<Self, AppError> {
        let mineru_parser = Arc::new(MinerUParser::with_auto_venv_detection()?);
        let markitdown_parser = Arc::new(MarkItDownParser::with_auto_venv_detection()?);
        
        Ok(Self {
            mineru_parser,
            markitdown_parser,
        })
    }

    /// 检查解析器是否正常
    pub fn is_ok(&self) -> bool {
        // 简单的健康检查，可以根据需要扩展
        true
    }
    
    /// 根据格式选择合适的解析器
    pub fn get_parser_for_format(&self, format: &DocumentFormat) -> Arc<dyn DocumentParser> {
        match format {
            DocumentFormat::PDF => self.mineru_parser.clone() as Arc<dyn DocumentParser>,
            _ => self.markitdown_parser.clone() as Arc<dyn DocumentParser>,
        }
    }
    
    /// 解析文档（自动选择引擎）
    pub async fn parse_document(&self, file_path: &str, format: &DocumentFormat) -> Result<ParseResult, AppError> {
        // 检查格式是否支持
        if !self.supports_format(format) {
            return Err(AppError::UnsupportedFormat(format!(
                "不支持的文件格式: {:?}", format
            )));
        }
        
        let parser = self.get_parser_for_format(format);
        parser.parse(file_path, format).await
    }
    
    /// 检查是否支持指定格式
    pub fn supports_format(&self, format: &DocumentFormat) -> bool {
        // 基于当前 `DocumentFormat` 定义进行判断
        matches!(format,
            DocumentFormat::PDF
            | DocumentFormat::Word
            | DocumentFormat::Excel
            | DocumentFormat::PowerPoint
            | DocumentFormat::Image
            | DocumentFormat::Audio
            | DocumentFormat::HTML
            | DocumentFormat::Text
            | DocumentFormat::Txt
            | DocumentFormat::Md
            | DocumentFormat::Other(_)
        )
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
    
    /// 健康检查
    pub async fn health_check(&self) -> Result<(), AppError> {
        // 检查MinerU解析器
        if let Err(e) = self.mineru_parser.health_check().await {
            log::warn!("MinerU解析器健康检查失败: {}", e);
        }
        
        // 检查MarkItDown解析器
        if let Err(e) = self.markitdown_parser.health_check().await {
            log::warn!("MarkItDown解析器健康检查失败: {}", e);
        }
        
        Ok(())
    }
    
    /// 获取解析器统计信息
    pub fn get_parser_stats(&self) -> ParserStats {
        ParserStats {
            mineru_name: self.mineru_parser.get_name().to_string(),
            mineru_description: self.mineru_parser.get_description().to_string(),
            markitdown_name: self.markitdown_parser.get_name().to_string(),
            markitdown_description: self.markitdown_parser.get_description().to_string(),
            supported_formats: Self::get_supported_formats(),
        }
    }
}

#[async_trait::async_trait]
impl DocumentParser for DualEngineParser {
    /// 解析文档
    async fn parse(&self, file_path: &str, format: &DocumentFormat) -> Result<ParseResult, AppError> {
        self.parse_document(file_path, format).await
    }
    
    /// 检查是否支持指定格式
    fn supports_format(&self, format: &DocumentFormat) -> bool {
        self.supports_format(format)
    }
    
    /// 获取解析器名称
    fn get_name(&self) -> &'static str {
        "DualEngineParser"
    }
    
    /// 获取解析器描述
    fn get_description(&self) -> &'static str {
        "双引擎文档解析器，支持MinerU和MarkItDown"
    }
    
    /// 健康检查
    async fn health_check(&self) -> Result<(), AppError> {
        self.health_check().await
    }
}

/// 解析器统计信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParserStats {
    pub mineru_name: String,
    pub mineru_description: String,
    pub markitdown_name: String,
    pub markitdown_description: String,
    pub supported_formats: Vec<DocumentFormat>,
}