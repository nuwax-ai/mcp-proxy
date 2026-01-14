use crate::error::AppError;
use crate::models::DocumentFormat;

/// 从文件路径检测文档格式
pub fn detect_format_from_path(file_path: &str) -> Result<DocumentFormat, AppError> {
    let extension = super::file_utils::get_file_extension(file_path)
        .ok_or_else(|| AppError::UnsupportedFormat("无法识别文件扩展名".to_string()))?;

    Ok(DocumentFormat::from_extension(&extension))
}

/// 从MIME类型检测文档格式
pub fn detect_format_from_mime(mime_type: &str) -> DocumentFormat {
    DocumentFormat::from_mime_type(mime_type)
}

/// 检查格式是否支持
pub fn is_format_supported(format: &DocumentFormat) -> bool {
    format.is_supported()
}

/// 获取支持格式列表
pub fn get_supported_formats() -> Vec<DocumentFormat> {
    vec![
        DocumentFormat::PDF,
        DocumentFormat::Word,
        DocumentFormat::Excel,
        DocumentFormat::PowerPoint,
        DocumentFormat::Image,
        DocumentFormat::Audio,
        DocumentFormat::HTML,
        DocumentFormat::Text,
        DocumentFormat::Txt,
        DocumentFormat::Md,
    ]
}
