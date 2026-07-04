use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// OSS数据
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OssData {
    pub markdown_url: String,
    pub markdown_object_key: Option<String>,
    pub images: Vec<ImageInfo>,
    pub bucket: String,
}

/// 图片信息
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageInfo {
    pub original_path: String,     // 原始本地路径
    pub original_filename: String, // 原始文件名（不含路径）
    pub oss_object_key: String,    // OSS对象键名
    pub oss_url: String,           // OSS下载URL
    pub file_size: u64,            // 文件大小
    pub mime_type: String,         // MIME类型
    pub width: Option<u32>,        // 图片宽度
    pub height: Option<u32>,       // 图片高度
}

impl ImageInfo {
    /// 创建新的图片信息
    pub fn new(original_path: String, oss_url: String, file_size: u64, mime_type: String) -> Self {
        let original_filename = std::path::Path::new(&original_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let oss_object_key = format!("images/{original_filename}");

        Self {
            original_path,
            original_filename,
            oss_object_key,
            oss_url,
            file_size,
            mime_type,
            width: None,
            height: None,
        }
    }

    /// 从完整信息创建图片信息
    pub fn with_full_info(
        original_path: String,
        original_filename: String,
        oss_object_key: String,
        oss_url: String,
        file_size: u64,
        mime_type: String,
    ) -> Self {
        Self {
            original_path,
            original_filename,
            oss_object_key,
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

    /// 检查文件名是否匹配（支持多种匹配方式）
    pub fn filename_matches(&self, reference: &str) -> bool {
        // 完全匹配
        if self.original_filename == reference {
            return true;
        }

        // 忽略扩展名匹配
        let ref_without_ext = std::path::Path::new(reference)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let self_without_ext = std::path::Path::new(&self.original_filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        if ref_without_ext == self_without_ext {
            return true;
        }

        // 路径匹配（如果reference包含路径）
        if reference.contains('/') || reference.contains('\\') {
            let ref_filename = std::path::Path::new(reference)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if ref_filename == self.original_filename {
                return true;
            }
        }

        false
    }
}
