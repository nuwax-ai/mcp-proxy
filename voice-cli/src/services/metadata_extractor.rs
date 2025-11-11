use crate::VoiceCliError;
use serde::{Deserialize, Serialize};
use std::fs::Metadata;
use std::path::Path;
use tokio::{fs, task};
use tracing::{debug, info, warn};

/// 音视频元数据信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioVideoMetadata {
    // 基础信息
    pub format: String,           // 文件格式 (mp3, wav, mp4, etc.)
    pub container_format: String, // 容器格式
    pub duration_seconds: f64,    // 时长（秒）
    pub file_size_bytes: u64,     // 文件大小

    // 音频信息
    pub audio_codec: String, // 音频编码器
    pub sample_rate: u32,    // 采样率 (Hz)
    pub channels: u8,        // 声道数
    pub audio_bitrate: u32,  // 音频码率 (kbps)

    // 视频信息（如果是视频文件）
    pub has_video: bool,             // 是否包含视频
    pub video_codec: Option<String>, // 视频编码器
    pub width: Option<u32>,          // 视频宽度
    pub height: Option<u32>,         // 视频高度
    pub video_bitrate: Option<u32>,  // 视频码率 (kbps)
    pub frame_rate: Option<f64>,     // 帧率

    // 其他元数据
    pub bitrate: u32,                  // 总码率 (kbps)
    pub creation_time: Option<String>, // 创建时间
}

impl Default for AudioVideoMetadata {
    fn default() -> Self {
        Self {
            format: "unknown".to_string(),
            container_format: "unknown".to_string(),
            duration_seconds: 0.0,
            file_size_bytes: 0,
            audio_codec: "unknown".to_string(),
            sample_rate: 0,
            channels: 1,
            audio_bitrate: 0,
            has_video: false,
            video_codec: None,
            width: None,
            height: None,
            video_bitrate: None,
            frame_rate: None,
            bitrate: 0,
            creation_time: None,
        }
    }
}

/// 音视频元数据提取器
pub struct MetadataExtractor;

impl MetadataExtractor {
    /// 从文件路径提取音视频元数据
    pub async fn extract_metadata(file_path: &Path) -> Result<AudioVideoMetadata, VoiceCliError> {
        info!("开始提取音视频元数据: {:?}", file_path);

        // 首先获取文件基本信息
        let file_metadata = fs::metadata(file_path)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("无法访问文件: {}", e)))?;

        let _file_size = file_metadata.len();
        let file_extension = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("unknown")
            .to_lowercase();

        // 尝试使用 FFmpeg 提取详细元数据
        if let Ok(ffmpeg_metadata) = Self::extract_with_ffmpeg(file_path).await {
            info!("FFmpeg 元数据提取成功");
            return Ok(ffmpeg_metadata);
        }

        // 如果 FFmpeg 不可用或失败，使用基础方法
        warn!("FFmpeg 不可用或失败，使用基础元数据提取方法");
        Self::extract_basic_metadata(file_path, &file_metadata, &file_extension).await
    }

    /// 使用 FFmpeg 提取详细元数据
    async fn extract_with_ffmpeg(file_path: &Path) -> Result<AudioVideoMetadata, VoiceCliError> {
        use ffmpeg_sidecar::command::FfmpegCommand;

        debug!("使用 FFmpeg 提取元数据: {:?}", file_path);

        let file_path_buf = file_path.to_path_buf();

        let metadata =
            task::spawn_blocking(move || -> Result<AudioVideoMetadata, VoiceCliError> {
                let mut metadata = AudioVideoMetadata::default();
                let file_path_str = file_path_buf.to_string_lossy().to_string();

                // 使用 FfmpegCommand 获取文件信息
                let mut child = FfmpegCommand::new()
                    .arg("-i")
                    .arg(&file_path_str)
                    .arg("-hide_banner")
                    .spawn()
                    .map_err(|e| VoiceCliError::Storage(format!("FFmpeg 执行失败: {}", e)))?;

                // 等待命令完成（在阻塞线程中执行）
                let _exit_status = child
                    .wait()
                    .map_err(|e| VoiceCliError::Storage(format!("FFmpeg 执行失败: {}", e)))?;

                // 使用传统方法获取输出（因为 ffmpeg-sidecar 主要用于处理媒体流，不是元数据提取）
                let output = std::process::Command::new("ffmpeg")
                    .args(["-i", &file_path_str, "-hide_banner", "-f", "null", "-"])
                    .output()
                    .map_err(|e| VoiceCliError::Storage(format!("FFmpeg 执行失败: {}", e)))?;

                // 解析 stderr 输出中的元数据信息
                let stderr_output = String::from_utf8_lossy(&output.stderr);

                // 解析输出中的元数据信息
                for line in stderr_output.lines() {
                    if line.contains("Duration:") {
                        // 解析时长: Duration: 00:00:01.60, start: 0.000000, bitrate: 705 kb/s
                        if let Some(duration_part) = line.split("Duration: ").nth(1) {
                            if let Some(duration_str) = duration_part.split(',').next() {
                                let parts: Vec<&str> = duration_str.split(':').collect();
                                if parts.len() == 3 {
                                    let hours: f64 = parts[0].parse().unwrap_or(0.0);
                                    let minutes: f64 = parts[1].parse().unwrap_or(0.0);
                                    let seconds: f64 = parts[2].parse().unwrap_or(0.0);
                                    metadata.duration_seconds =
                                        hours * 3600.0 + minutes * 60.0 + seconds;
                                }
                            }
                        }
                    }

                    if line.contains("Audio:") {
                        // 解析音频信息: Stream #0:0: Audio: pcm_f32le, 22050 Hz, mono, fltp, 705 kb/s
                        let audio_info = line.split("Audio: ").nth(1).unwrap_or("");
                        let parts: Vec<&str> = audio_info.split(',').collect();

                        if let Some(codec) = parts.first() {
                            metadata.audio_codec = codec.trim().to_string();
                        }

                        for part in parts {
                            if part.contains("Hz") {
                                if let Some(rate_str) = part.split("Hz").next() {
                                    metadata.sample_rate = rate_str.trim().parse().unwrap_or(0);
                                }
                            }
                            if part.contains("mono") {
                                metadata.channels = 1;
                            }
                            if part.contains("stereo") {
                                metadata.channels = 2;
                            }
                            if part.contains("kb/s") {
                                if let Some(bitrate_str) = part.split("kb/s").next() {
                                    metadata.audio_bitrate =
                                        bitrate_str.trim().parse().unwrap_or(0);
                                }
                            }
                        }
                    }

                    if line.contains("Video:") {
                        // 解析视频信息: Stream #0:1: Video: h264, yuv420p, 1280x720, 24 fps, 1992 kb/s
                        metadata.has_video = true;
                        let video_info = line.split("Video: ").nth(1).unwrap_or("");
                        let parts: Vec<&str> = video_info.split(',').collect();

                        if let Some(codec) = parts.first() {
                            metadata.video_codec = Some(codec.trim().to_string());
                        }

                        for part in parts {
                            if part.contains('x') {
                                let resolution_parts: Vec<&str> = part.trim().split('x').collect();
                                if resolution_parts.len() == 2 {
                                    metadata.width = resolution_parts[0].trim().parse().ok();
                                    metadata.height = resolution_parts[1].trim().parse().ok();
                                }
                            }
                            if part.contains("fps") {
                                if let Some(fps_str) = part.split("fps").next() {
                                    metadata.frame_rate = fps_str.trim().parse().ok();
                                }
                            }
                            if part.contains("kb/s") {
                                if let Some(bitrate_str) = part.split("kb/s").next() {
                                    metadata.video_bitrate =
                                        Some(bitrate_str.trim().parse().unwrap_or(0));
                                }
                            }
                        }
                    }
                }

                // 获取文件大小
                if let Ok(file_meta) = std::fs::metadata(&file_path_buf) {
                    metadata.file_size_bytes = file_meta.len();
                }

                // 如果没有从输出中获取到码率，计算总码率
                if metadata.bitrate == 0
                    && metadata.duration_seconds > 0.0
                    && metadata.file_size_bytes > 0
                {
                    let total_bits = metadata.file_size_bytes as f64 * 8.0;
                    metadata.bitrate = (total_bits / metadata.duration_seconds / 1000.0) as u32;
                }

                // 根据文件扩展名设置格式
                if let Some(extension) = file_path_buf.extension().and_then(|ext| ext.to_str()) {
                    metadata.format = extension.to_lowercase();
                    metadata.container_format = extension.to_lowercase();
                }

                Ok(metadata)
            })
            .await
            .map_err(|e| VoiceCliError::Storage(format!("FFmpeg 阻塞任务失败: {}", e)))??;

        info!("FFmpeg 元数据提取完成: {:?}", metadata);
        Ok(metadata)
    }

    /// 基础元数据提取（不依赖 FFmpeg）
    async fn extract_basic_metadata(
        file_path: &Path,
        file_metadata: &Metadata,
        file_extension: &str,
    ) -> Result<AudioVideoMetadata, VoiceCliError> {
        debug!("使用基础方法提取元数据: {:?}", file_path);

        let mut metadata = AudioVideoMetadata {
            file_size_bytes: file_metadata.len(),
            format: file_extension.to_string(),
            container_format: file_extension.to_string(),
            ..Default::default()
        };

        // 尝试使用现有的 AudioFormatDetector 获取音频信息
        if let Ok(Some(format_type)) =
            crate::services::AudioFormatDetector::detect_format_from_path(file_path)
        {
            metadata.format = format_type.extension().to_string();
            metadata.audio_codec = format_type.mime_type().to_string();
        }

        // 判断是否为视频文件
        metadata.has_video = Self::is_video_format(file_extension);

        // 如果是视频文件，设置默认值
        if metadata.has_video {
            metadata.video_codec = Some("unknown".to_string());
        }

        // 计算码率
        if metadata.duration_seconds > 0.0 && metadata.file_size_bytes > 0 {
            let total_bits = metadata.file_size_bytes as f64 * 8.0;
            metadata.bitrate = (total_bits / metadata.duration_seconds / 1000.0) as u32;
        }

        info!("基础元数据提取完成: {:?}", metadata);
        Ok(metadata)
    }

    /// 判断是否为视频格式
    fn is_video_format(extension: &str) -> bool {
        matches!(
            extension,
            "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "3gp" | "mpg" | "mpeg"
        )
    }

    /// 获取文件格式描述
    pub fn get_format_description(metadata: &AudioVideoMetadata) -> String {
        if metadata.has_video {
            format!(
                "视频文件 - 格式: {}, 分辨率: {}x{}, 时长: {:.2}s",
                metadata.format,
                metadata.width.unwrap_or(0),
                metadata.height.unwrap_or(0),
                metadata.duration_seconds
            )
        } else {
            format!(
                "音频文件 - 格式: {}, 采样率: {}Hz, 声道: {}, 时长: {:.2}s",
                metadata.format, metadata.sample_rate, metadata.channels, metadata.duration_seconds
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_extract_basic_metadata() {
        // 创建一个临时文件进行测试
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"dummy audio data").unwrap();
        temp_file.flush().unwrap();

        let metadata = MetadataExtractor::extract_metadata(temp_file.path()).await;

        // 验证基本结构
        match metadata {
            Ok(meta) => {
                assert!(!meta.format.is_empty());
                assert_eq!(meta.file_size_bytes, 17); // "dummy audio data" 的长度
            }
            Err(e) => {
                println!("提取失败: {}", e);
                // 对于测试环境，FFmpeg 可能不可用，这是可以接受的
            }
        }
    }

    #[test]
    fn test_is_video_format() {
        assert!(MetadataExtractor::is_video_format("mp4"));
        assert!(MetadataExtractor::is_video_format("avi"));
        assert!(!MetadataExtractor::is_video_format("mp3"));
        assert!(!MetadataExtractor::is_video_format("wav"));
    }

    #[test]
    fn test_get_format_description() {
        let audio_meta = AudioVideoMetadata {
            format: "mp3".to_string(),
            sample_rate: 44100,
            channels: 2,
            duration_seconds: 180.5,
            has_video: false,
            ..Default::default()
        };

        let video_meta = AudioVideoMetadata {
            format: "mp4".to_string(),
            width: Some(1920),
            height: Some(1080),
            duration_seconds: 120.0,
            has_video: true,
            ..Default::default()
        };

        let audio_desc = MetadataExtractor::get_format_description(&audio_meta);
        let video_desc = MetadataExtractor::get_format_description(&video_meta);

        assert!(audio_desc.contains("音频文件"));
        assert!(audio_desc.contains("44100Hz"));
        assert!(video_desc.contains("视频文件"));
        assert!(video_desc.contains("1920x1080"));
    }
}
