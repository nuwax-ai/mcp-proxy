use crate::error::AppError;
use crate::models::DocumentFormat;
use std::collections::HashSet;
use url::Url;

/// 请求验证器
pub struct RequestValidator;

impl RequestValidator {
    /// 验证文件大小
    pub fn validate_file_size(size: u64, max_size: u64) -> Result<(), AppError> {
        if size == 0 {
            return Err(AppError::Validation("文件大小不能为0".to_string()));
        }
        if size > max_size {
            return Err(AppError::Validation(format!(
                "文件大小超过限制: {size} > {max_size} 字节"
            )));
        }
        Ok(())
    }

    /// 验证文件扩展名
    pub fn validate_file_extension(
        filename: &str,
        allowed_extensions: &[String],
    ) -> Result<String, AppError> {
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase())
            .ok_or_else(|| AppError::Validation("无法确定文件扩展名".to_string()))?;

        if !allowed_extensions.contains(&extension) {
            return Err(AppError::Validation(format!(
                "不支持的文件格式: .{extension}"
            )));
        }

        Ok(extension)
    }

    /// 验证URL格式
    pub fn validate_url(url_str: &str) -> Result<Url, AppError> {
        let url =
            Url::parse(url_str).map_err(|e| AppError::Validation(format!("无效的URL格式: {e}")))?;

        // 检查协议
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AppError::Validation("只支持HTTP和HTTPS协议".to_string()));
        }

        // 检查主机
        let host = url
            .host_str()
            .ok_or_else(|| AppError::Validation("URL缺少主机名".to_string()))?;

        // 防止访问本地地址
        if Self::is_local_address(host) {
            return Err(AppError::Validation("不允许访问本地地址".to_string()));
        }

        Ok(url)
    }

    /// 验证URL格式（仅验证，不返回解析后的URL）
    pub fn validate_url_format(url_str: &str) -> Result<(), AppError> {
        let _url =
            Url::parse(url_str).map_err(|e| AppError::Validation(format!("无效的URL格式: {e}")))?;

        // 检查协议
        if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
            return Err(AppError::Validation("只支持HTTP和HTTPS协议".to_string()));
        }

        // 检查是否包含主机名（简单检查）
        if !url_str.contains("://") || url_str.split("://").nth(1).unwrap_or("").is_empty() {
            return Err(AppError::Validation("URL缺少主机名".to_string()));
        }

        Ok(())
    }

    /// 验证OSS路径
    pub fn validate_oss_path(oss_path: &str) -> Result<(), AppError> {
        if oss_path.is_empty() {
            return Err(AppError::Validation("OSS路径不能为空".to_string()));
        }

        // 检查路径格式
        if oss_path.starts_with('/') || oss_path.contains("../") || oss_path.contains("..\\") {
            return Err(AppError::Validation("OSS路径格式无效".to_string()));
        }

        // 检查路径长度
        if oss_path.len() > 1024 {
            return Err(AppError::Validation("OSS路径过长".to_string()));
        }

        Ok(())
    }

    /// 验证任务ID格式
    pub fn validate_task_id(task_id: &str) -> Result<(), AppError> {
        if task_id.is_empty() {
            return Err(AppError::Validation("任务ID不能为空".to_string()));
        }

        // 检查UUID格式
        if uuid::Uuid::parse_str(task_id).is_err() {
            return Err(AppError::Validation("任务ID格式无效".to_string()));
        }

        Ok(())
    }

    /// 验证分页参数
    pub fn validate_pagination(
        page: Option<usize>,
        page_size: Option<usize>,
    ) -> Result<(usize, usize), AppError> {
        let page = page.unwrap_or(1);
        let page_size = page_size.unwrap_or(10);

        if page == 0 {
            return Err(AppError::Validation("页码必须大于0".to_string()));
        }

        if page_size == 0 || page_size > 100 {
            return Err(AppError::Validation("每页大小必须在1-100之间".to_string()));
        }

        Ok((page, page_size))
    }

    /// 验证排序参数
    pub fn validate_sort_params(
        sort_by: Option<&str>,
        sort_order: Option<&str>,
    ) -> Result<(String, String), AppError> {
        let allowed_sort_fields =
            HashSet::from(["created_at", "updated_at", "progress", "file_size"]);

        let sort_by = sort_by.unwrap_or("created_at");
        if !allowed_sort_fields.contains(sort_by) {
            return Err(AppError::Validation(format!("不支持的排序字段: {sort_by}")));
        }

        let sort_order = sort_order.unwrap_or("desc");
        if !matches!(sort_order, "asc" | "desc") {
            return Err(AppError::Validation("排序方向必须是asc或desc".to_string()));
        }

        Ok((sort_by.to_string(), sort_order.to_string()))
    }

    /// 验证文档格式
    pub fn validate_document_format(format: &DocumentFormat) -> Result<(), AppError> {
        // 这里可以添加特定格式的验证逻辑
        match format {
            DocumentFormat::PDF
            | DocumentFormat::Word
            | DocumentFormat::Excel
            | DocumentFormat::PowerPoint
            | DocumentFormat::Image
            | DocumentFormat::Audio
            | DocumentFormat::HTML
            | DocumentFormat::Text
            | DocumentFormat::Txt
            | DocumentFormat::Md
            | DocumentFormat::Other(_) => Ok(()),
        }
    }

    /// 验证Markdown内容
    pub fn validate_markdown_content(content: &str) -> Result<(), AppError> {
        if content.is_empty() {
            return Err(AppError::Validation("Markdown内容不能为空".to_string()));
        }

        // 检查内容长度（最大10MB）
        const MAX_CONTENT_SIZE: usize = 10 * 1024 * 1024;
        if content.len() > MAX_CONTENT_SIZE {
            return Err(AppError::Validation(format!(
                "Markdown内容过长: {} > {} 字节",
                content.len(),
                MAX_CONTENT_SIZE
            )));
        }

        Ok(())
    }

    /// 验证TOC配置
    pub fn validate_toc_config(
        enable_toc: Option<bool>,
        max_toc_depth: Option<usize>,
    ) -> Result<(bool, usize), AppError> {
        let enable_toc = enable_toc.unwrap_or(true);
        let max_depth = max_toc_depth.unwrap_or(3);

        if enable_toc && (max_depth == 0 || max_depth > 10) {
            return Err(AppError::Validation(
                "TOC最大深度必须在1-10之间".to_string(),
            ));
        }

        Ok((enable_toc, max_depth))
    }

    /// 验证分页参数
    pub fn validate_pagination_params(page: usize, page_size: usize) -> Result<(), AppError> {
        if page == 0 {
            return Err(AppError::Validation("页码必须大于0".to_string()));
        }
        if page_size == 0 || page_size > 100 {
            return Err(AppError::Validation("每页大小必须在1-100之间".to_string()));
        }
        Ok(())
    }

    /// 检查是否为本地地址
    fn is_local_address(host: &str) -> bool {
        //todo: 找rust生态的库,看能否用更简单的方式实现"检查是否为本地地址"
        matches!(
            host,
            "localhost"
                | "127.0.0.1"
                | "::1"
                | "0.0.0.0"
                | "10.0.0.0"
                | "172.16.0.0"
                | "192.168.0.0"
        ) || host.starts_with("10.")
            || host.starts_with("172.16.")
            || host.starts_with("172.17.")
            || host.starts_with("172.18.")
            || host.starts_with("172.19.")
            || host.starts_with("172.20.")
            || host.starts_with("172.21.")
            || host.starts_with("172.22.")
            || host.starts_with("172.23.")
            || host.starts_with("172.24.")
            || host.starts_with("172.25.")
            || host.starts_with("172.26.")
            || host.starts_with("172.27.")
            || host.starts_with("172.28.")
            || host.starts_with("172.29.")
            || host.starts_with("172.30.")
            || host.starts_with("172.31.")
            || host.starts_with("192.168.")
    }
}

/// 文件名清理工具
pub struct FileNameSanitizer;

impl FileNameSanitizer {
    /// 清理文件名
    pub fn sanitize(filename: &str) -> Result<String, AppError> {
        if filename.is_empty() {
            return Err(AppError::Validation("文件名不能为空".to_string()));
        }

        // 移除危险字符
        let sanitized = filename
            .chars()
            .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            .collect::<String>();

        if sanitized.is_empty() {
            return Err(AppError::Validation("文件名包含过多非法字符".to_string()));
        }

        // 检查长度
        if sanitized.len() > 255 {
            return Err(AppError::Validation("文件名过长".to_string()));
        }

        // 避免保留名称
        let reserved_names = HashSet::from([
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]);

        let name_without_ext = std::path::Path::new(&sanitized)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&sanitized)
            .to_uppercase();

        if reserved_names.contains(name_without_ext.as_str()) {
            return Err(AppError::Validation(
                "文件名不能使用系统保留名称".to_string(),
            ));
        }

        Ok(sanitized)
    }
}
