use std::path::Path;
use tracing::warn;
use url::Url;

/// MIME 类型到文件扩展名的映射
/// 专注于音视频格式，其他类型统一使用 bin 扩展名
pub fn mime_type_to_extension(content_type: &str) -> &'static str {
    match content_type {
        // 音频类型
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "audio/flac" => "flac",
        "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        "audio/aac" => "aac",
        "audio/opus" => "opus",
        "audio/webm" => "webm",
        "audio/x-matroska" => "mka",
        "audio/x-ms-wma" => "wma",
        "audio/x-wav" => "wav",
        "audio/vnd.wave" => "wav",
        "audio/x-aiff" => "aiff",
        "audio/aiff" => "aiff",
        "audio/x-caf" => "caf",
        "audio/x-m4a" => "m4a",
        "audio/x-mpeg" => "mp3",
        "audio/x-ogg" => "ogg",
        
        // 视频类型
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/x-msvideo" => "avi",
        "video/quicktime" => "mov",
        "video/x-ms-wmv" => "wmv",
        "video/x-flv" => "flv",
        "video/3gpp" => "3gp",
        "video/3gpp2" => "3g2",
        "video/mpeg" => "mpeg",
        "video/x-mpeg" => "mpeg",
        
        // 其他所有类型统一使用 bin 扩展名
        _ => {
            // 对于非音视频类型，直接使用默认扩展名，不输出警告日志
            "bin"
        }
    }
}

/// 从 URL 中提取文件扩展名
pub fn extract_extension_from_url(url: &str) -> Option<&'static str> {
    if let Ok(parsed_url) = Url::parse(url) {
        parsed_url
            .path_segments()
            .and_then(|segments| segments.last())
            .and_then(|filename| {
                if let Some(dot_index) = filename.rfind('.') {
                    let ext = &filename[dot_index + 1..];
                    // 将常见扩展名映射为标准格式
                    match ext.to_lowercase().as_str() {
                        "mp3" => Some("mp3"),
                        "wav" => Some("wav"),
                        "flac" => Some("flac"),
                        "m4a" => Some("m4a"),
                        "mp4" => Some("mp4"),
                        "mov" => Some("mov"),
                        "avi" => Some("avi"),
                        "mkv" => Some("mkv"),
                        "webm" => Some("webm"),
                        "ogg" => Some("ogg"),
                        "aac" => Some("aac"),
                        "opus" => Some("opus"),
                        "wma" => Some("wma"),
                        "flv" => Some("flv"),
                        "3gp" => Some("3gp"),
                        "caf" => Some("caf"),
                        "aiff" => Some("aiff"),
                        "bin" => Some("bin"),
                        _ => None,
                    }
                } else {
                    None
                }
            })
    } else {
        None
    }
}

/// 从文件路径推断 MIME 类型
pub fn infer_mime_type_from_path<P: AsRef<Path>>(path: P) -> Option<&'static str> {
    infer::get_from_path(path)
        .ok()
        .flatten()
        .map(|kind| kind.mime_type())
}

/// 从文件扩展名获取 MIME 类型
pub fn extension_to_mime_type(extension: &str) -> &'static str {
    match extension.to_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "ogg" => "audio/ogg",
        "aac" => "audio/aac",
        "opus" => "audio/opus",
        "wma" => "audio/x-ms-wma",
        "flv" => "video/x-flv",
        "3gp" => "video/3gpp",
        "caf" => "audio/x-caf",
        "aiff" => "audio/x-aiff",
        "bin" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// 智能获取文件扩展名（优先使用 MIME 类型，回退到 URL 扩展名）
pub fn get_file_extension(content_type: &str, url: &str) -> &'static str {
    let extension = mime_type_to_extension(content_type);
    
    // 如果得到的是默认扩展名，尝试从 URL 中获取更精确的扩展名
    if extension == "bin" {
        if let Some(url_extension) = extract_extension_from_url(url) {
            return url_extension;
        }
    }
    
    extension
}

/// 检查是否为支持的音频格式
pub fn is_supported_audio_format(mime_type: &str) -> bool {
    matches!(mime_type,
        "audio/mpeg" |      // MP3
        "audio/wav" |       // WAV
        "audio/flac" |      // FLAC
        "audio/mp4" |       // M4A
        "audio/ogg" |       // OGG
        "audio/aac" |       // AAC
        "audio/opus" |      // Opus
        "audio/webm" |      // WebM Audio
        "audio/x-matroska" | // Matroska Audio
        "audio/x-wav" |     // WAV (alternative)
        "audio/vnd.wave" |  // WAV (alternative)
        "audio/x-aiff" |    // AIFF
        "audio/aiff" |      // AIFF (alternative)
        "audio/x-caf"       // CAF
    )
}

/// 检查是否为支持的视频格式
pub fn is_supported_video_format(mime_type: &str) -> bool {
    matches!(mime_type,
        "video/mp4" |        // MP4
        "video/webm" |       // WebM
        "video/x-matroska" | // MKV
        "video/x-msvideo" |  // AVI
        "video/quicktime" |  // MOV
        "video/x-ms-wmv" |   // WMV
        "video/x-flv" |      // FLV
        "video/3gpp" |       // 3GP
        "video/3gpp2" |      // 3G2
        "video/mpeg" |       // MPEG
        "video/x-mpeg"       // MPEG (alternative)
    )
}

/// 检查是否为支持的媒体格式
pub fn is_supported_media_format(mime_type: &str) -> bool {
    is_supported_audio_format(mime_type) || is_supported_video_format(mime_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_type_to_extension() {
        assert_eq!(mime_type_to_extension("audio/mpeg"), "mp3");
        assert_eq!(mime_type_to_extension("audio/wav"), "wav");
        assert_eq!(mime_type_to_extension("video/mp4"), "mp4");
        assert_eq!(mime_type_to_extension("unknown/type"), "bin");
        assert_eq!(mime_type_to_extension("text/plain"), "bin");
        assert_eq!(mime_type_to_extension("application/json"), "bin");
        assert_eq!(mime_type_to_extension("image/jpeg"), "bin");
    }

    #[test]
    fn test_extract_extension_from_url() {
        assert_eq!(extract_extension_from_url("https://example.com/test.mp3"), Some("mp3"));
        assert_eq!(extract_extension_from_url("https://example.com/test.wav"), Some("wav"));
        assert_eq!(extract_extension_from_url("https://example.com/test?format=mp3"), None);
        assert_eq!(extract_extension_from_url("invalid-url"), None);
    }

    #[test]
    fn test_extension_to_mime_type() {
        assert_eq!(extension_to_mime_type("mp3"), "audio/mpeg");
        assert_eq!(extension_to_mime_type("wav"), "audio/wav");
        assert_eq!(extension_to_mime_type("mp4"), "video/mp4");
        assert_eq!(extension_to_mime_type("unknown"), "application/octet-stream");
    }

    #[test]
    fn test_get_file_extension() {
        assert_eq!(get_file_extension("audio/mpeg", "https://example.com/test.mp3"), "mp3");
        assert_eq!(get_file_extension("unknown/type", "https://example.com/test.wav"), "wav");
        assert_eq!(get_file_extension("unknown/type", "https://example.com/test"), "bin");
    }

    #[test]
    fn test_supported_formats() {
        assert!(is_supported_audio_format("audio/mpeg"));
        assert!(is_supported_audio_format("audio/wav"));
        assert!(!is_supported_audio_format("video/mp4"));
        
        assert!(is_supported_video_format("video/mp4"));
        assert!(is_supported_video_format("video/webm"));
        assert!(!is_supported_video_format("audio/mpeg"));
        
        assert!(is_supported_media_format("audio/mpeg"));
        assert!(is_supported_media_format("video/mp4"));
        assert!(!is_supported_media_format("unknown/type"));
    }
}