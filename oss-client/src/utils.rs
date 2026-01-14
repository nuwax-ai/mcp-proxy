//! 工具函数模块

use std::collections::HashMap;
use std::path::Path;

use tracing::debug;

/// MIME类型映射表
fn get_mime_type_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();

    // 图片类型
    map.insert("jpg", "image/jpeg");
    map.insert("jpeg", "image/jpeg");
    map.insert("png", "image/png");
    map.insert("gif", "image/gif");
    map.insert("webp", "image/webp");
    map.insert("svg", "image/svg+xml");
    map.insert("bmp", "image/bmp");
    map.insert("tiff", "image/tiff");
    map.insert("ico", "image/x-icon");

    // 文档类型
    map.insert("pdf", "application/pdf");
    map.insert("doc", "application/msword");
    map.insert(
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );
    map.insert("xls", "application/vnd.ms-excel");
    map.insert(
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    );
    map.insert("ppt", "application/vnd.ms-powerpoint");
    map.insert(
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    );

    // 文本类型
    map.insert("txt", "text/plain");
    map.insert("html", "text/html");
    map.insert("htm", "text/html");
    map.insert("css", "text/css");
    map.insert("js", "application/javascript");
    map.insert("json", "application/json");
    map.insert("xml", "application/xml");
    map.insert("csv", "text/csv");
    map.insert("md", "text/markdown");

    // 压缩文件
    map.insert("zip", "application/zip");
    map.insert("rar", "application/x-rar-compressed");
    map.insert("7z", "application/x-7z-compressed");
    map.insert("tar", "application/x-tar");
    map.insert("gz", "application/gzip");

    // 音频类型
    map.insert("mp3", "audio/mpeg");
    map.insert("wav", "audio/wav");
    map.insert("ogg", "audio/ogg");
    map.insert("m4a", "audio/mp4");
    map.insert("aac", "audio/aac");
    map.insert("flac", "audio/flac");

    // 视频类型
    map.insert("mp4", "video/mp4");
    map.insert("avi", "video/x-msvideo");
    map.insert("mov", "video/quicktime");
    map.insert("wmv", "video/x-ms-wmv");
    map.insert("flv", "video/x-flv");
    map.insert("webm", "video/webm");

    map
}

/// 检测文件MIME类型
pub fn detect_mime_type(file_path: &str) -> String {
    let path = Path::new(file_path);
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mime_map = get_mime_type_map();
    mime_map
        .get(extension.as_str())
        .unwrap_or(&"application/octet-stream")
        .to_string()
}

/// 根据扩展名检测MIME类型
pub fn detect_mime_type_by_extension(extension: &str) -> String {
    let ext = extension.trim_start_matches('.').to_lowercase();
    let mime_map = get_mime_type_map();
    mime_map
        .get(ext.as_str())
        .unwrap_or(&"application/octet-stream")
        .to_string()
}

/// 判断是否为图片文件
pub fn is_image_file(file_path: &str) -> bool {
    let mime_type = detect_mime_type(file_path);
    mime_type.starts_with("image/")
}

/// 判断是否为文档文件
pub fn is_document_file(file_path: &str) -> bool {
    let mime_type = detect_mime_type(file_path);
    mime_type.starts_with("application/")
        && (mime_type.contains("pdf")
            || mime_type.contains("word")
            || mime_type.contains("excel")
            || mime_type.contains("powerpoint")
            || mime_type.contains("document"))
}

/// 判断是否为音频文件
pub fn is_audio_file(file_path: &str) -> bool {
    let mime_type = detect_mime_type(file_path);
    mime_type.starts_with("audio/")
}

/// 判断是否为视频文件
pub fn is_video_file(file_path: &str) -> bool {
    let mime_type = detect_mime_type(file_path);
    mime_type.starts_with("video/")
}

/// 清理文件名，移除特殊字符
pub fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 格式化文件大小
pub fn format_file_size(size: u64) -> String {
    let size = size as f64;
    if size < 1024.0 {
        format!("{size} B")
    } else if size < 1024.0 * 1024.0 {
        format!("{:.2} KB", size / 1024.0)
    } else if size < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} MB", size / (1024.0 * 1024.0))
    } else if size < 1024.0 * 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", size / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.2} TB", size / (1024.0 * 1024.0 * 1024.0 * 1024.0))
    }
}

/// 解析文件大小字符串（如"100MB"）为字节数
pub fn parse_file_size(size_str: &str) -> Result<u64, String> {
    let size_str = size_str.trim().to_uppercase();

    if size_str.is_empty() {
        return Err("文件大小字符串不能为空".to_string());
    }

    // 提取数字部分和单位部分
    let (number_part, unit_part) = if size_str.ends_with("TB") {
        (&size_str[..size_str.len() - 2], "TB")
    } else if size_str.ends_with("GB") {
        (&size_str[..size_str.len() - 2], "GB")
    } else if size_str.ends_with("MB") {
        (&size_str[..size_str.len() - 2], "MB")
    } else if size_str.ends_with("KB") {
        (&size_str[..size_str.len() - 2], "KB")
    } else if size_str.ends_with("B") {
        (&size_str[..size_str.len() - 1], "B")
    } else {
        // 没有单位，默认为字节
        (size_str.as_str(), "B")
    };

    let number: f64 = number_part
        .parse()
        .map_err(|_| format!("无效的数字: {number_part}"))?;

    if number < 0.0 {
        return Err("文件大小不能为负数".to_string());
    }

    let bytes = match unit_part {
        "B" => number,
        "KB" => number * 1024.0,
        "MB" => number * 1024.0 * 1024.0,
        "GB" => number * 1024.0 * 1024.0 * 1024.0,
        "TB" => number * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return Err(format!("不支持的单位: {unit_part}")),
    };

    Ok(bytes as u64)
}

/// 生成随机文件名
pub fn generate_random_filename(extension: Option<&str>) -> String {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let uid = uuid::Uuid::new_v4().to_string()[..8].to_string();

    match extension {
        Some(ext) => {
            let clean_ext = ext.trim_start_matches('.');
            format!("{timestamp}_{uid}.{clean_ext}")
        }
        None => format!("{timestamp}_{uid}"),
    }
}

/// 提取文件扩展名
pub fn get_file_extension(file_path: &str) -> Option<String> {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
}

/// 获取文件名（不包含路径）
pub fn get_filename(file_path: &str) -> Option<String> {
    Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

/// 获取文件名（不包含扩展名）
pub fn get_filename_without_extension(file_path: &str) -> Option<String> {
    Path::new(file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
}

/// 替换OSS域名前缀，解决跨域问题
///
/// 将阿里云OSS的域名替换为自定义域名，避免跨域问题
///
/// # 参数
/// * `url` - 原始OSS URL
///
/// # 返回值
/// * 替换后的URL，如果没有匹配的域名则返回原URL
///
/// # 示例
/// ```
/// use oss_client::utils::replace_oss_domain;
///
/// let original_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image.jpg";
/// let replaced_url = replace_oss_domain(original_url);
/// assert_eq!(replaced_url, "https://statics-ali.nuwax.com/image.jpg");
/// ```
pub fn replace_oss_domain(url: &str) -> String {
    //把固定的公网 bucket 替换为自定义域名
    const OLD_DOMAIN: &str = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com";
    const NEW_DOMAIN: &str = "https://statics-ali.nuwax.com";
    debug!("替换OSS域名: {}", url);
    let new_url = if url.starts_with(OLD_DOMAIN) {
        url.replacen(OLD_DOMAIN, NEW_DOMAIN, 1)
    } else {
        url.to_string()
    };
    debug!("替换后的OSS域名: {}", new_url);
    new_url
}

/// 批量替换OSS域名前缀
///
/// 对多个URL进行域名替换
///
/// # 参数
/// * `urls` - URL列表的引用
///
/// # 返回值
/// * 替换后的URL列表
///
/// # 示例
/// ```
/// use oss_client::utils::replace_oss_domains_batch;
///
/// let urls = vec![
///     "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image1.jpg".to_string(),
///     "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image2.jpg".to_string(),
/// ];
/// let replaced_urls = replace_oss_domains_batch(&urls);
/// assert_eq!(replaced_urls[0], "https://statics-ali.nuwax.com/image1.jpg");
/// assert_eq!(replaced_urls[1], "https://statics-ali.nuwax.com/image2.jpg");
/// ```
pub fn replace_oss_domains_batch(urls: &[String]) -> Vec<String> {
    urls.iter().map(|url| replace_oss_domain(url)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mime_type() {
        assert_eq!(detect_mime_type("test.jpg"), "image/jpeg");
        assert_eq!(detect_mime_type("test.png"), "image/png");
        assert_eq!(detect_mime_type("test.pdf"), "application/pdf");
        assert_eq!(
            detect_mime_type("test.docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(detect_mime_type("test.mp3"), "audio/mpeg");
        assert_eq!(detect_mime_type("test.mp4"), "video/mp4");
        assert_eq!(detect_mime_type("test.unknown"), "application/octet-stream");
        assert_eq!(detect_mime_type("test"), "application/octet-stream");
    }

    #[test]
    fn test_detect_mime_type_by_extension() {
        assert_eq!(detect_mime_type_by_extension("jpg"), "image/jpeg");
        assert_eq!(detect_mime_type_by_extension(".png"), "image/png");
        assert_eq!(detect_mime_type_by_extension("PDF"), "application/pdf");
        assert_eq!(
            detect_mime_type_by_extension("unknown"),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_file_type_detection() {
        assert!(is_image_file("test.jpg"));
        assert!(is_image_file("test.png"));
        assert!(!is_image_file("test.pdf"));

        assert!(is_document_file("test.pdf"));
        assert!(is_document_file("test.docx"));
        assert!(!is_document_file("test.jpg"));

        assert!(is_audio_file("test.mp3"));
        assert!(is_audio_file("test.wav"));
        assert!(!is_audio_file("test.jpg"));

        assert!(is_video_file("test.mp4"));
        assert!(is_video_file("test.avi"));
        assert!(!is_video_file("test.jpg"));
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("test file.txt"), "test_file.txt");
        assert_eq!(sanitize_filename("test@#$%file.txt"), "test____file.txt");
        assert_eq!(
            sanitize_filename("normal-file_name.txt"),
            "normal-file_name.txt"
        );

        // 中文字符实际上会被保留，因为它们通过了is_alphanumeric()检查
        let result = sanitize_filename("中文文件名.txt");
        assert!(result.contains(".txt"));
        assert!(result.contains("中文文件名"));

        // 特殊字符会被替换为下划线
        assert_eq!(sanitize_filename("file@name.txt"), "file_name.txt");
        assert_eq!(
            sanitize_filename("file name with spaces.txt"),
            "file_name_with_spaces.txt"
        );
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(512), "512 B");
        assert_eq!(format_file_size(1024), "1.00 KB");
        assert_eq!(format_file_size(1536), "1.50 KB");
        assert_eq!(format_file_size(1024 * 1024), "1.00 MB");
        assert_eq!(format_file_size(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(format_file_size(1024_u64.pow(4)), "1.00 TB");
    }

    #[test]
    fn test_parse_file_size() {
        assert_eq!(parse_file_size("100").unwrap(), 100);
        assert_eq!(parse_file_size("100B").unwrap(), 100);
        assert_eq!(parse_file_size("1KB").unwrap(), 1024);
        assert_eq!(parse_file_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_file_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_file_size("1TB").unwrap(), 1024_u64.pow(4));

        assert_eq!(
            parse_file_size("1.5MB").unwrap(),
            (1.5 * 1024.0 * 1024.0) as u64
        );
        assert_eq!(parse_file_size("100mb").unwrap(), 100 * 1024 * 1024);

        assert!(parse_file_size("").is_err());
        assert!(parse_file_size("abc").is_err());
        assert!(parse_file_size("-100MB").is_err());
    }

    #[test]
    fn test_generate_random_filename() {
        let filename1 = generate_random_filename(Some("txt"));
        let filename2 = generate_random_filename(Some(".jpg"));
        let filename3 = generate_random_filename(None);

        assert!(filename1.ends_with(".txt"));
        assert!(filename2.ends_with(".jpg"));
        assert!(!filename3.contains("."));

        // 确保生成的文件名不同
        assert_ne!(filename1, filename2);
    }

    #[test]
    fn test_file_path_utilities() {
        assert_eq!(get_file_extension("test.txt"), Some("txt".to_string()));
        assert_eq!(get_file_extension("test.TAR.GZ"), Some("gz".to_string()));
        assert_eq!(get_file_extension("test"), None);

        assert_eq!(
            get_filename("/path/to/test.txt"),
            Some("test.txt".to_string())
        );
        assert_eq!(get_filename("test.txt"), Some("test.txt".to_string()));

        assert_eq!(
            get_filename_without_extension("/path/to/test.txt"),
            Some("test".to_string())
        );
        assert_eq!(
            get_filename_without_extension("test"),
            Some("test".to_string())
        );
    }

    #[test]
    fn test_replace_oss_domain() {
        // 测试正常替换
        let original_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image.jpg";
        let replaced_url = replace_oss_domain(original_url);
        assert_eq!(replaced_url, "https://statics-ali.nuwax.com/image.jpg");

        // 测试带路径的URL
        let original_url_with_path =
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/folder/subfolder/image.png";
        let replaced_url_with_path = replace_oss_domain(original_url_with_path);
        assert_eq!(
            replaced_url_with_path,
            "https://statics-ali.nuwax.com/folder/subfolder/image.png"
        );

        // 测试带查询参数的URL
        let original_url_with_query = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image.jpg?version=1.0&size=large";
        let replaced_url_with_query = replace_oss_domain(original_url_with_query);
        assert_eq!(
            replaced_url_with_query,
            "https://statics-ali.nuwax.com/image.jpg?version=1.0&size=large"
        );

        // 测试不匹配的域名
        let other_url = "https://other-domain.com/image.jpg";
        let unchanged_url = replace_oss_domain(other_url);
        assert_eq!(unchanged_url, other_url);

        // 测试空字符串
        let empty_url = "";
        let unchanged_empty = replace_oss_domain(empty_url);
        assert_eq!(unchanged_empty, "");
    }

    #[test]
    fn test_replace_oss_domains_batch() {
        let urls = vec![
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image1.jpg".to_string(),
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image2.jpg".to_string(),
            "https://other-domain.com/image3.jpg".to_string(),
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/folder/image4.png"
                .to_string(),
        ];

        let replaced_urls = replace_oss_domains_batch(&urls);

        assert_eq!(replaced_urls[0], "https://statics-ali.nuwax.com/image1.jpg");
        assert_eq!(replaced_urls[1], "https://statics-ali.nuwax.com/image2.jpg");
        assert_eq!(replaced_urls[2], "https://other-domain.com/image3.jpg"); // 不匹配的域名保持不变
        assert_eq!(
            replaced_urls[3],
            "https://statics-ali.nuwax.com/folder/image4.png"
        );

        // 测试空列表
        let empty_urls: Vec<String> = vec![];
        let replaced_empty = replace_oss_domains_batch(&empty_urls);
        assert_eq!(replaced_empty.len(), 0);
    }
}
