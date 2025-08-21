use crate::error::AppError;
use crate::models::{DocumentFormat, ParseResult};
use async_trait::async_trait;

/// 文档解析器特征
#[async_trait]
pub trait DocumentParser: Send + Sync {
    /// 解析文档
    async fn parse(&self, file_path: &str) -> Result<ParseResult, AppError>;

    /// 检查是否支持指定格式
    fn supports_format(&self, format: &DocumentFormat) -> bool;

    /// 获取解析器名称
    fn get_name(&self) -> &'static str;

    /// 获取解析器描述
    fn get_description(&self) -> &'static str;

    /// 健康检查
    async fn health_check(&self) -> Result<(), AppError>;
}

/// 解析器工厂
pub struct ParserFactory;

impl ParserFactory {
    /// 根据格式选择合适的解析器
    pub fn get_parser_for_format(format: &DocumentFormat) -> crate::models::ParserEngine {
        use crate::models::ParserEngine;

        match format {
            DocumentFormat::PDF => ParserEngine::MinerU,
            _ => ParserEngine::MarkItDown,
        }
    }

    /// 检查格式是否支持
    pub fn is_format_supported(format: &DocumentFormat) -> bool {
        // 基于当前 `DocumentFormat` 定义进行判断
        matches!(
            format,
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
}
