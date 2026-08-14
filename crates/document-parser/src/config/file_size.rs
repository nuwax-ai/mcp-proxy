//! 文件大小域：FileSize 序列化类型、全局大小限制配置与用途枚举、大小字符串解析。

use super::*;

/// 文件大小单位
#[derive(Debug, Clone)]
pub struct FileSize(pub u64);

impl<'de> Deserialize<'de> for FileSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        parse_file_size(&s)
            .map(FileSize)
            .map_err(serde::de::Error::custom)
    }
}

impl Serialize for FileSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format_file_size_human(self.0))
    }
}

fn format_file_size_human(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB && bytes.is_multiple_of(GB) {
        format!("{}GB", bytes / GB)
    } else if bytes >= MB && bytes.is_multiple_of(MB) {
        format!("{}MB", bytes / MB)
    } else if bytes >= KB && bytes.is_multiple_of(KB) {
        format!("{}KB", bytes / KB)
    } else {
        format!("{bytes}B")
    }
}

impl FileSize {
    pub fn bytes(&self) -> u64 {
        self.0
    }

    pub fn mb(&self) -> u64 {
        self.0 / (1024 * 1024)
    }

    pub fn from_mb(mb: u64) -> Self {
        Self(mb * 1024 * 1024)
    }
}

/// 全局文件大小配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalFileSizeConfig {
    /// 统一的最大文件大小限制
    pub max_file_size: FileSize,
    /// 大文档阈值（用于流式处理）
    pub large_document_threshold: FileSize,
}

impl Default for GlobalFileSizeConfig {
    fn default() -> Self {
        Self {
            max_file_size: FileSize(500 * 1024 * 1024), // 500MB
            large_document_threshold: FileSize(50 * 1024 * 1024), // 50MB
        }
    }
}

impl GlobalFileSizeConfig {
    /// 创建新的全局文件大小配置实例
    pub fn new() -> Self {
        get_global_file_size_config().clone()
    }

    /// 验证配置的有效性
    pub fn validate(&self) -> Result<(), ConfigError> {
        let configs = [
            ("max_file_size", self.max_file_size.bytes()),
            (
                "large_document_threshold",
                self.large_document_threshold.bytes(),
            ),
        ];

        for (name, size) in configs {
            if size == 0 {
                return Err(ConfigError::Validation {
                    field: format!("file_size_config.{name}"),
                    message: "文件大小不能为0".to_string(),
                });
            }

            if size > 10 * 1024 * 1024 * 1024 {
                // 10GB
                return Err(ConfigError::Validation {
                    field: format!("file_size_config.{name}"),
                    message: "文件大小不能超过10GB".to_string(),
                });
            }
        }

        Ok(())
    }

    /// 获取指定用途的文件大小限制
    pub fn get_max_size_for(&self, _purpose: FileSizePurpose) -> u64 {
        // 统一使用同一个文件大小限制
        self.max_file_size.bytes()
    }

    /// 检查文件大小是否超过指定用途的限制
    pub fn is_size_allowed(&self, file_size: u64, purpose: FileSizePurpose) -> bool {
        file_size <= self.get_max_size_for(purpose)
    }

    /// 获取大文档阈值
    pub fn get_large_document_threshold(&self) -> u64 {
        self.large_document_threshold.bytes()
    }

    /// 检查是否为大文档
    pub fn is_large_document(&self, file_size: u64) -> bool {
        file_size >= self.get_large_document_threshold()
    }

    /// 获取指定用途的文件大小限制（返回FileSize结构体）
    pub fn get_file_size_limit(&self, _purpose: &FileSizePurpose) -> &FileSize {
        // 统一使用同一个文件大小限制
        &self.max_file_size
    }
}

/// 文件大小用途枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSizePurpose {
    /// 默认用途
    Default,
    /// 文档解析器
    DocumentParser,
    /// MinerU解析器
    MinerU,
    /// MarkItDown解析器
    MarkItDown,
    /// 图片处理器
    ImageProcessor,
    /// 文件上传
    Upload,
    /// 格式检测器
    FormatDetector,
    /// 内容验证
    ContentValidation,
    /// 缓存
    Cache,
}

/// 解析文件大小字符串 (例如: "100MB", "1GB", "500KB")
pub fn parse_file_size(size_str: &str) -> Result<u64, String> {
    let size_str = size_str.trim().to_uppercase();

    if let Some(pos) = size_str.find(|c: char| c.is_alphabetic()) {
        let (number_part, unit_part) = size_str.split_at(pos);
        let number: f64 = number_part
            .parse()
            .map_err(|_| format!("无效的数字: {number_part}"))?;

        let multiplier = match unit_part {
            "B" => 1,
            "KB" => 1024,
            "MB" => 1024 * 1024,
            "GB" => 1024 * 1024 * 1024,
            "TB" => 1024_u64.pow(4),
            _ => return Err(format!("不支持的单位: {unit_part}")),
        };

        Ok((number * multiplier as f64) as u64)
    } else {
        // 如果没有单位，假设是字节
        size_str
            .parse::<u64>()
            .map_err(|_| format!("无效的文件大小: {size_str}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_size_parsing() {
        assert_eq!(parse_file_size("100B").unwrap(), 100);
        assert_eq!(parse_file_size("1KB").unwrap(), 1024);
        assert_eq!(parse_file_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_file_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(
            parse_file_size("2.5MB").unwrap(),
            (2.5 * 1024.0 * 1024.0) as u64
        );

        // 测试无效格式
        assert!(parse_file_size("invalid").is_err());
        assert!(parse_file_size("100XB").is_err());
        assert!(parse_file_size("").is_err());
    }
}
