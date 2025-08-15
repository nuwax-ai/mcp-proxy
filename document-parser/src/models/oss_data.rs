use serde::{Deserialize, Serialize};

/// OSS数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OssData {
    pub markdown_url: String,
    pub markdown_object_key: Option<String>,
    pub images: Vec<ImageInfo>,
    pub bucket: String,
}

/// 图片信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub original_path: String,
    pub oss_url: String,
    pub file_size: u64,
    pub mime_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl ImageInfo {
    /// 创建新的图片信息
    pub fn new(original_path: String, oss_url: String, file_size: u64, mime_type: String) -> Self {
        Self {
            original_path,
            oss_url,
            file_size,
            mime_type,
            width: None,
            height: None,
        }
    }

    /// 设置图片尺寸
    pub fn with_dimensions(mut self, width: u32, height: u32) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    /// 获取文件大小（格式化）
    pub fn get_formatted_size(&self) -> String {
        if self.file_size < 1024 {
            format!("{} B", self.file_size)
        } else if self.file_size < 1024 * 1024 {
            format!("{:.1} KB", self.file_size as f64 / 1024.0)
        } else {
            format!("{:.1} MB", self.file_size as f64 / (1024.0 * 1024.0))
        }
    }
}
