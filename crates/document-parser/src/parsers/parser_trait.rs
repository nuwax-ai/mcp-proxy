use crate::error::AppError;
use crate::models::{DocumentFormat, ParseResult};
use async_trait::async_trait;

/// 文档解析器特征
#[async_trait]
pub trait DocumentParser: Send + Sync {
    /// 解析文档
    async fn parse(&self, file_path: &str) -> Result<ParseResult, AppError>;

    /// 带取消令牌解析文档（默认忽略令牌优雅退化）。
    ///
    /// 任务级取消打通用：DocumentService 持文档 task_id 级令牌经
    /// [`DualEngineParser::parse_document_auto_with_cancel`] 下传，引擎实现
    /// 转发给自身 `parse_with_progress` 的取消参数——execute 层 select 轮询
    /// 令牌并 kill 子进程。未适配的引擎（含测试 mock）走默认实现不受影响。
    async fn parse_with_cancel(
        &self,
        file_path: &str,
        cancel: Option<crate::parsers::mineru_parser::CancellationToken>,
    ) -> Result<ParseResult, AppError> {
        let _ = cancel;
        self.parse(file_path).await
    }

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
