use serde::{Deserialize, Serialize};
use crate::models::{DocumentFormat, ParserEngine};

/// 解析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub markdown_content: String,
    pub images: Vec<String>, // 图片路径列表
    pub format: DocumentFormat,
    pub engine: ParserEngine,
    pub processing_time: Option<f64>, // 处理时间（秒）
    pub word_count: Option<usize>,   // 字数统计
    pub error_count: Option<usize>,  // 错误数量
}

impl ParseResult {
    /// 创建新的解析结果
    pub fn new(
        markdown_content: String,
        format: DocumentFormat,
        engine: ParserEngine,
    ) -> Self {
        let word_count = markdown_content.split_whitespace().count();
        
        Self {
            markdown_content,
            images: Vec::new(),
            format,
            engine,
            processing_time: None,
            word_count: Some(word_count),
            error_count: Some(0),
        }
    }

    /// 添加图片
    pub fn add_image(&mut self, image_path: String) {
        self.images.push(image_path);
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
            Some(time) => format!("{:.1}s", time),
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
            "字数: {}, 图片: {}, 处理时间: {}, 引擎: {}",
            self.word_count.unwrap_or(0),
            self.images.len(),
            self.get_processing_time_description(),
            self.engine.get_name()
        )
    }
}
