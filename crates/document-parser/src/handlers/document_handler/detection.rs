//! 文档格式检测簇
//!
//! 通过文件扩展名与文件内容（魔数）综合检测文档格式，供上传流程使用。

use crate::error::AppError;
use crate::models::DocumentFormat;
use crate::utils::file_utils::get_file_extension;
use tracing::warn;

/// 增强的文档格式检测
pub(crate) fn detect_document_format_enhanced(
    file_path: &str,
    first_chunk: Option<&[u8]>,
    expected_extension: &str,
) -> Result<DocumentFormat, AppError> {
    // 1. 通过文件扩展名检测
    let extension_format = if let Some(extension) = get_file_extension(file_path) {
        DocumentFormat::from_extension(&extension)
    } else {
        DocumentFormat::from_extension(expected_extension)
    };

    // 2. 通过文件内容检测（魔数）
    let content_format = if let Some(chunk) = first_chunk {
        detect_format_by_magic_number_enhanced(chunk).unwrap_or(extension_format.clone())
    } else {
        extension_format.clone()
    };

    // 3. 验证格式一致性
    if !formats_compatible(&extension_format, &content_format) {
        warn!(
            "File extension and content format mismatch: {:?} vs {:?}",
            extension_format, content_format
        );

        // 如果内容检测更可靠，使用内容格式
        if is_reliable_magic_number_detection(&content_format) {
            return Ok(content_format);
        }
    }

    // 4. 返回最终格式
    Ok(extension_format)
}

/// 检查两种格式是否兼容
fn formats_compatible(format1: &DocumentFormat, format2: &DocumentFormat) -> bool {
    match (format1, format2) {
        (DocumentFormat::Text, DocumentFormat::Txt)
        | (DocumentFormat::Txt, DocumentFormat::Text)
        | (DocumentFormat::Text, DocumentFormat::Md)
        | (DocumentFormat::Md, DocumentFormat::Text) => true,
        (a, b) => a == b,
    }
}

/// 检查是否为可靠的魔数检测
fn is_reliable_magic_number_detection(format: &DocumentFormat) -> bool {
    matches!(
        format,
        DocumentFormat::PDF | DocumentFormat::Image | DocumentFormat::Audio
    )
}

/// 增强的魔数检测文件格式
fn detect_format_by_magic_number_enhanced(data: &[u8]) -> Result<DocumentFormat, AppError> {
    if data.len() < 4 {
        return Err(AppError::Validation("文件数据不足以检测格式".to_string()));
    }

    // PDF: %PDF
    if data.starts_with(b"%PDF") {
        return Ok(DocumentFormat::PDF);
    }

    // ZIP-based formats: PK\x03\x04 或 PK\x05\x06 或 PK\x07\x08
    if data.len() >= 4 && data.starts_with(b"PK") {
        // 进一步检测ZIP内容类型
        return detect_zip_based_format(data);
    }

    // 图片格式
    if let Ok(format) = detect_image_format(data) {
        return Ok(format);
    }

    // 音频格式
    if let Ok(format) = detect_audio_format(data) {
        return Ok(format);
    }

    // HTML/XML格式
    if let Ok(format) = detect_text_format(data) {
        return Ok(format);
    }

    Err(AppError::Validation("无法通过文件内容检测格式".to_string()))
}

/// 检测ZIP格式的具体类型
fn detect_zip_based_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    // 这里可以通过读取ZIP文件的目录结构来判断具体格式
    // 简化实现，返回Word格式作为默认
    if data.len() >= 30 {
        // 检查是否包含Office文档的特征
        let data_str = String::from_utf8_lossy(&data[0..std::cmp::min(512, data.len())]);
        if data_str.contains("word/") {
            return Ok(DocumentFormat::Word);
        } else if data_str.contains("xl/") {
            return Ok(DocumentFormat::Excel);
        } else if data_str.contains("ppt/") {
            return Ok(DocumentFormat::PowerPoint);
        }
    }

    // 默认返回Word格式
    Ok(DocumentFormat::Word)
}

/// 检测图片格式
fn detect_image_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    if data.len() < 8 {
        return Err(AppError::Validation("数据不足".to_string()));
    }

    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(DocumentFormat::Image);
    }

    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok(DocumentFormat::Image);
    }

    // GIF: GIF87a 或 GIF89a
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Ok(DocumentFormat::Image);
    }

    // BMP: BM
    if data.starts_with(b"BM") {
        return Ok(DocumentFormat::Image);
    }

    // TIFF: II*\0 或 MM\0*
    if data.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
        return Ok(DocumentFormat::Image);
    }

    Err(AppError::Validation("不是图片格式".to_string()))
}

/// 检测音频格式
fn detect_audio_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    if data.len() < 4 {
        return Err(AppError::Validation("数据不足".to_string()));
    }

    // MP3: ID3 或 FF FB/FF F3/FF F2
    if data.starts_with(b"ID3") || (data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0)
    {
        return Ok(DocumentFormat::Audio);
    }

    // WAV: RIFF....WAVE
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WAVE" {
        return Ok(DocumentFormat::Audio);
    }

    // M4A/AAC: ftyp
    if data.len() >= 8 && &data[4..8] == b"ftyp" {
        return Ok(DocumentFormat::Audio);
    }

    Err(AppError::Validation("不是音频格式".to_string()))
}

/// 检测文本格式
fn detect_text_format(data: &[u8]) -> Result<DocumentFormat, AppError> {
    let data_str = String::from_utf8_lossy(data).to_lowercase();

    // HTML
    if data_str.contains("<html") || data_str.contains("<!doctype html") {
        return Ok(DocumentFormat::HTML);
    }

    // XML
    if data_str.starts_with("<?xml") {
        return Ok(DocumentFormat::HTML); // 将XML归类为HTML处理
    }

    // Markdown (简单检测)
    if data_str.contains("# ") || data_str.contains("## ") || data_str.contains("```") {
        return Ok(DocumentFormat::Md);
    }

    // 默认文本
    Ok(DocumentFormat::Text)
}
