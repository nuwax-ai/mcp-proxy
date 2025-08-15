use serde::{Deserialize, Serialize};

/// 文档格式枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DocumentFormat {
    PDF,
    Word,
    Excel,
    PowerPoint,
    Image,
    Audio,
    HTML,
    Text,
    Txt,
    Md,
    Other(String),
}

impl DocumentFormat {
    /// 从文件扩展名获取格式
    pub fn from_extension(extension: &str) -> Self {
        match extension.to_lowercase().as_str() {
            "pdf" => DocumentFormat::PDF,
            "docx" | "doc" => DocumentFormat::Word,
            "xlsx" | "xls" => DocumentFormat::Excel,
            "pptx" | "ppt" => DocumentFormat::PowerPoint,
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" => DocumentFormat::Image,
            "mp3" | "wav" | "m4a" | "aac" => DocumentFormat::Audio,
            "html" | "htm" => DocumentFormat::HTML,
            "txt" | "csv" | "json" | "xml" | "md" => DocumentFormat::Text,
            _ => DocumentFormat::Other(extension.to_string()),
        }
    }

    /// 从MIME类型获取格式
    pub fn from_mime_type(mime_type: &str) -> Self {
        match mime_type {
            "application/pdf" => DocumentFormat::PDF,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => DocumentFormat::Word,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => DocumentFormat::Excel,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => DocumentFormat::PowerPoint,
            "image/jpeg" | "image/png" | "image/gif" | "image/bmp" | "image/tiff" => DocumentFormat::Image,
            "audio/mpeg" | "audio/wav" | "audio/mp4" | "audio/aac" => DocumentFormat::Audio,
            "text/html" => DocumentFormat::HTML,
            "text/plain" | "text/csv" | "application/json" | "application/xml" | "text/markdown" => DocumentFormat::Text,
            _ => DocumentFormat::Other(mime_type.to_string()),
        }
    }

    /// 获取文件扩展名
    pub fn get_extension(&self) -> &'static str {
        match self {
            DocumentFormat::PDF => "pdf",
            DocumentFormat::Word => "docx",
            DocumentFormat::Excel => "xlsx",
            DocumentFormat::PowerPoint => "pptx",
            DocumentFormat::Image => "jpg",
            DocumentFormat::Audio => "mp3",
            DocumentFormat::HTML => "html",
            DocumentFormat::Text => "txt",
            DocumentFormat::Txt => "txt",
            DocumentFormat::Md => "md",
            DocumentFormat::Other(_) => "bin",
        }
    }

    /// 获取MIME类型
    pub fn get_mime_type(&self) -> &'static str {
        match self {
            DocumentFormat::PDF => "application/pdf",
            DocumentFormat::Word => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            DocumentFormat::Excel => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            DocumentFormat::PowerPoint => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            DocumentFormat::Image => "image/jpeg",
            DocumentFormat::Audio => "audio/mpeg",
            DocumentFormat::HTML => "text/html",
            DocumentFormat::Text => "text/plain",
            DocumentFormat::Txt => "text/plain",
            DocumentFormat::Md => "text/markdown",
            DocumentFormat::Other(_) => "application/octet-stream",
        }
    }

    /// 检查是否支持该格式
    pub fn is_supported(&self) -> bool {
        !matches!(self, DocumentFormat::Other(_))
    }
}

impl std::fmt::Display for DocumentFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentFormat::PDF => write!(f, "pdf"),
            DocumentFormat::Word => write!(f, "word"),
            DocumentFormat::Excel => write!(f, "excel"),
            DocumentFormat::PowerPoint => write!(f, "powerpoint"),
            DocumentFormat::Image => write!(f, "image"),
            DocumentFormat::Audio => write!(f, "audio"),
            DocumentFormat::HTML => write!(f, "html"),
            DocumentFormat::Text => write!(f, "text"),
            DocumentFormat::Txt => write!(f, "txt"),
            DocumentFormat::Md => write!(f, "md"),
            DocumentFormat::Other(s) => write!(f, "other({})", s),
        }
    }
}
