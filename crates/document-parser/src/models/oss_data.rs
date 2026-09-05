use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 产物存储后端判别（编译期穷举，替代裸字符串匹配）
///
/// serde 序列化值为小写 `"oss"` / `"custom"`——与既已落盘的 sled 记录
/// （`storage_type: Some("custom")` 字符串）逐字节兼容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum StorageType {
    /// 阿里云 OSS
    Oss,
    /// 自定义上传后端（nuwax 风格 REST）
    Custom,
}

/// OSS数据
///
/// 自定义上传后端（nuwax 风格）的任务复用本结构存储产物信息，此时
/// `markdown_url` 为后端返回的 URL、`markdown_object_key` 为后端 key、
/// `bucket` 为后端 base_url——以 [`OssData::storage_type`] 判别（数据自描述），
/// 消费方勿按字段名望文生义。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OssData {
    pub markdown_url: String,
    pub markdown_object_key: Option<String>,
    pub images: Vec<ImageInfo>,
    pub bucket: String,
    /// 存储后端判别：`None` = OSS（旧记录兼容）；`Some(Custom)` = 自定义上传后端
    #[serde(default)]
    pub storage_type: Option<StorageType>,
}

impl OssData {
    /// 是否为自定义上传后端产物
    pub fn is_custom_storage(&self) -> bool {
        self.storage_type == Some(StorageType::Custom)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legacy_oss_data_without_storage_type_deserializes() {
        // 旧 sled 记录没有 storage_type 字段 → None（判别为 OSS）
        let json = r#"{
            "markdown_url": "https://bucket.endpoint/x.md",
            "markdown_object_key": "processed_markdown/t/t.md",
            "images": [],
            "bucket": "my-bucket"
        }"#;
        let data: OssData = serde_json::from_str(json).unwrap();
        assert!(data.storage_type.is_none());
        assert!(!data.is_custom_storage());
    }

    #[test]
    fn test_custom_storage_round_trip() {
        let data = OssData {
            markdown_url: "https://agent.example.com/api/f/s3/x.md".to_string(),
            markdown_object_key: Some("s3/default/x.md".to_string()),
            images: vec![],
            bucket: "https://agent.example.com".to_string(),
            storage_type: Some(StorageType::Custom),
        };
        // 序列化值与既有落盘记录逐字节兼容（小写字符串 "custom"）
        let json = serde_json::to_string(&data).unwrap();
        assert!(
            json.contains(r#""storage_type":"custom""#),
            "序列化必须保持兼容: {json}"
        );
        let parsed: OssData = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_custom_storage());
        assert_eq!(parsed.storage_type, Some(StorageType::Custom));
    }

    #[test]
    fn test_oss_variant_serializes_lowercase() {
        let data = OssData {
            markdown_url: String::new(),
            markdown_object_key: None,
            images: vec![],
            bucket: "b".to_string(),
            storage_type: Some(StorageType::Oss),
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains(r#""storage_type":"oss""#));
        assert!(
            !OssData {
                storage_type: Some(StorageType::Oss),
                ..data
            }
            .is_custom_storage()
        );
    }
}
