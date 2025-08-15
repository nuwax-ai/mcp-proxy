use std::path::Path;
use crate::error::AppError;
use regex::Regex;

/// 检查文件是否存在
pub fn file_exists(file_path: &str) -> bool {
    Path::new(file_path).exists()
}

/// 获取文件大小
pub fn get_file_size(file_path: &str) -> Result<u64, AppError> {
    let metadata = std::fs::metadata(file_path)
        .map_err(|e| AppError::File(format!("无法获取文件元数据: {e}")))?;
    Ok(metadata.len())
}

/// 获取文件扩展名
pub fn get_file_extension(file_path: &str) -> Option<String> {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_lowercase())
}

/// 创建临时目录
pub fn create_temp_dir(dir_path: &str) -> Result<(), AppError> {
    if !Path::new(dir_path).exists() {
        std::fs::create_dir_all(dir_path)
            .map_err(|e| AppError::File(format!("无法创建临时目录: {e}")))?
    }
    Ok(())
}

/// 验证文件大小
pub fn validate_file_size(file_size: u64, max_size: u64) -> Result<(), AppError> {
    if file_size > max_size {
        return Err(AppError::Validation(format!(
            "文件大小超过限制: {} > {} 字节", file_size, max_size
        )));
    }
    Ok(())
}

/// 清理文件名，移除不安全字符
pub fn sanitize_filename(filename: &str) -> Result<String, AppError> {
    if filename.is_empty() {
        return Err(AppError::Validation("文件名不能为空".to_string()));
    }
    
    // 移除路径分隔符和其他不安全字符
    let unsafe_chars = Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).unwrap();
    let sanitized = unsafe_chars.replace_all(filename, "_").to_string();
    
    // 移除开头和结尾的点和空格
    let sanitized = sanitized.trim_matches(|c| c == '.' || c == ' ').to_string();
    
    if sanitized.is_empty() {
        return Err(AppError::Validation("清理后的文件名为空".to_string()));
    }
    
    // 限制文件名长度
    if sanitized.len() > 255 {
        let truncated = sanitized.chars().take(255).collect::<String>();
        Ok(truncated)
    } else {
        Ok(sanitized)
    }
}
