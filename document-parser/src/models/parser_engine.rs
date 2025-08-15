use serde::{Deserialize, Serialize};
use crate::models::DocumentFormat;

/// 解析引擎枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ParserEngine {
    MinerU,      // PDF专用
    MarkItDown,  // 其他格式
}

impl ParserEngine {
    /// 根据文档格式选择解析引擎
    pub fn select_for_format(format: &DocumentFormat) -> Self {
        match format {
            DocumentFormat::PDF => ParserEngine::MinerU,
            _ => ParserEngine::MarkItDown, // 其他所有格式使用MarkItDown
        }
    }

    /// 获取引擎名称
    pub fn get_name(&self) -> &'static str {
        match self {
            ParserEngine::MinerU => "MinerU",
            ParserEngine::MarkItDown => "MarkItDown",
        }
    }

    /// 获取引擎描述
    pub fn get_description(&self) -> &'static str {
        match self {
            ParserEngine::MinerU => "专业PDF解析引擎，支持图片提取、表格识别、布局保持",
            ParserEngine::MarkItDown => "多格式文档解析引擎，支持Word、Excel、PowerPoint、图片、音频等",
        }
    }

    /// 检查是否支持指定格式
    pub fn supports_format(&self, format: &DocumentFormat) -> bool {
        match self {
            ParserEngine::MinerU => matches!(format, DocumentFormat::PDF),
            ParserEngine::MarkItDown => !matches!(format, DocumentFormat::PDF),
        }
    }
}
