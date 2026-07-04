use crate::models::{DocumentFormat, ParserEngine};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 解析结果
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ParseResult {
    pub markdown_content: String,
    pub format: DocumentFormat,
    pub engine: ParserEngine,
    pub processing_time: Option<f64>, // 处理时间（秒）
    pub word_count: Option<usize>,    // 字数统计
    pub error_count: Option<usize>,   // 错误数量
    /// MinerU 输出目录的绝对路径
    pub output_dir: Option<String>,
    /// MinerU 任务工作目录（包含输出目录）的绝对路径
    pub work_dir: Option<String>,
}

impl ParseResult {
    /// 创建新的解析结果
    pub fn new(markdown_content: String, format: DocumentFormat, engine: ParserEngine) -> Self {
        let word_count = markdown_content.split_whitespace().count();

        Self {
            markdown_content,
            format,
            engine,
            processing_time: None,
            word_count: Some(word_count),
            error_count: Some(0),
            output_dir: None,
            work_dir: None,
        }
    }

    /// 设置处理时间
    pub fn set_processing_time(&mut self, time_seconds: f64) {
        self.processing_time = Some(time_seconds);
    }

    /// 设置错误数量
    pub fn set_error_count(&mut self, count: usize) {
        self.error_count = Some(count);
    }

    /// 获取处理时间描述
    pub fn get_processing_time_description(&self) -> String {
        match self.processing_time {
            Some(time) if time < 1.0 => format!("{:.0}ms", time * 1000.0),
            Some(time) => format!("{time:.1}s"),
            None => "未知".to_string(),
        }
    }

    /// 检查是否成功
    pub fn is_success(&self) -> bool {
        self.error_count.unwrap_or(0) == 0
    }

    /// 获取统计信息
    pub fn get_statistics(&self) -> String {
        format!(
            "字数: {}, 处理时间: {}, 引擎: {}",
            self.word_count.unwrap_or(0),
            self.get_processing_time_description(),
            self.engine.get_name()
        )
    }
}
