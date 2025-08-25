use serde::{Deserialize, Serialize};
use bytes::Bytes;
use utoipa::ToSchema;


/// Request structure for transcription (internal use after extracting from multipart)
#[derive(Debug)]
pub struct TranscriptionRequest {
    pub audio_data: Bytes,
    pub filename: Option<String>,
    pub model: Option<String>,
    pub response_format: Option<String>,
}

/// Response structure for transcription API
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TranscriptionResponse {
    #[schema(example = "Hello, this is a test transcription.")]
    pub text: String,
    #[schema(example = json!([{"start": 0.0, "end": 2.5, "text": "Hello world", "confidence": 0.95}]))]
    pub segments: Vec<Segment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "en")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 2.5)]
    pub duration: Option<f32>,
    #[schema(example = 0.8)]
    pub processing_time: f32,
}

/// Individual segment in transcription
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Segment {
    /// Start time of the segment in seconds
    #[schema(example = 0.0)]
    pub start: f32,
    /// End time of the segment in seconds
    #[schema(example = 2.5)]
    pub end: f32,
    /// Text content of this segment
    #[schema(example = "Hello, this is a test transcription.")]
    pub text: String,
    /// Confidence score for this segment (0.0-1.0)
    #[schema(example = 0.95)]
    pub confidence: f32,
}

/// Health check response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Current service status
    #[schema(example = "healthy")]
    pub status: String,
    /// List of currently loaded models
    #[schema(example = json!(["base", "small"]))]
    pub models_loaded: Vec<String>,
    /// Service uptime in seconds
    #[schema(example = 3600)]
    pub uptime: u64,
    /// Service version
    #[schema(example = "0.1.0")]
    pub version: String,
}

/// Models list response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelsResponse {
    /// All supported model names
    #[schema(example = json!(["tiny", "base", "small", "medium", "large"]))]
    pub available_models: Vec<String>,
    /// Currently loaded models in memory
    #[schema(example = json!(["base"]))]
    pub loaded_models: Vec<String>,
    /// Detailed information about each model
    pub model_info: std::collections::HashMap<String, ModelInfo>,
}

/// Information about a specific model
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelInfo {
    /// Model file size on disk
    #[schema(example = "142 MB")]
    pub size: String,
    /// Memory usage when loaded
    #[schema(example = "388 MB")]
    pub memory_usage: String,
    /// Current model status
    #[schema(example = "loaded")]
    pub status: String,
}

/// Processed audio data
#[derive(Debug)]
pub struct ProcessedAudio {
    pub data: Bytes,
    pub converted: bool,
    pub original_format: Option<String>,
}

/// Audio format detection result
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioFormat {
    // Core audio formats (commonly supported by Symphonia)
    Wav,
    Mp3,
    Flac,
    M4a,
    Aac,
    Ogg,
    Webm,
    Opus,
    
    // Extended audio formats (FFmpeg supported)
    Amr,        // Adaptive Multi-Rate (mobile)
    Wma,        // Windows Media Audio
    Ra,         // RealAudio
    Au,         // Sun/Unix audio
    Aiff,       // Apple's uncompressed format
    Caf,        // Core Audio Format
    
    // Video formats (audio extraction via FFmpeg)
    ThreeGp,    // 3GP mobile format
    Mp4,        // MPEG-4 container
    Mov,        // QuickTime format
    Avi,        // Audio Video Interleave
    Mkv,        // Matroska container
    
    Unknown,
}

/// Audio metadata extracted during format detection
#[derive(Debug, Clone)]
pub struct AudioMetadata {
    pub duration: Option<std::time::Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bit_depth: Option<u8>,
    pub bitrate: Option<u32>,
    pub codec_info: String,
}

/// Enhanced audio format detection result with metadata
#[derive(Debug, Clone)]
pub struct AudioFormatResult {
    pub format: AudioFormat,
    pub confidence: f32,
    pub metadata: Option<AudioMetadata>,
    pub detection_method: DetectionMethod,
}

/// Method used for format detection
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DetectionMethod {
    SymphoniaProbe,
    FileExtension,
    ContentType,
}

impl AudioFormat {
    /// Enhanced filename-based format detection with support for extended formats
    pub fn from_filename(filename: &str) -> Self {
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            // Core audio formats
            "wav" => AudioFormat::Wav,
            "mp3" => AudioFormat::Mp3,
            "flac" => AudioFormat::Flac,
            "m4a" => AudioFormat::M4a,
            "aac" => AudioFormat::Aac,
            "ogg" => AudioFormat::Ogg,
            "webm" => AudioFormat::Webm,
            "opus" => AudioFormat::Opus,
            
            // Extended audio formats
            "amr" => AudioFormat::Amr,
            "wma" => AudioFormat::Wma,
            "ra" | "ram" => AudioFormat::Ra,
            "au" | "snd" => AudioFormat::Au,
            "aiff" | "aif" => AudioFormat::Aiff,
            "caf" => AudioFormat::Caf,
            
            // Video formats (audio extraction)
            "3gp" | "3g2" => AudioFormat::ThreeGp,
            "mp4" => AudioFormat::Mp4,
            "mov" => AudioFormat::Mov,
            "avi" => AudioFormat::Avi,
            "mkv" | "mka" => AudioFormat::Mkv,
            
            _ => AudioFormat::Unknown,
        }
    }

    /// Check if format is supported for transcription
    pub fn is_supported(&self) -> bool {
        !matches!(self, AudioFormat::Unknown)
    }

    /// Check if format requires FFmpeg conversion to WAV
    pub fn needs_conversion(&self) -> bool {
        !matches!(self, AudioFormat::Wav)
    }

    /// Get string representation of the format
    pub fn to_string(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::M4a => "m4a",
            AudioFormat::Aac => "aac",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Webm => "webm",
            AudioFormat::Opus => "opus",
            AudioFormat::Amr => "amr",
            AudioFormat::Wma => "wma",
            AudioFormat::Ra => "ra",
            AudioFormat::Au => "au",
            AudioFormat::Aiff => "aiff",
            AudioFormat::Caf => "caf",
            AudioFormat::ThreeGp => "3gp",
            AudioFormat::Mp4 => "mp4",
            AudioFormat::Mov => "mov",
            AudioFormat::Avi => "avi",
            AudioFormat::Mkv => "mkv",
            AudioFormat::Unknown => "unknown",
        }
    }

    /// Get MIME type for the format
    pub fn get_mime_type(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "audio/mpeg",
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Aac => "audio/aac",
            AudioFormat::Ogg => "audio/ogg",
            AudioFormat::M4a => "audio/mp4",
            AudioFormat::Webm => "audio/webm",
            AudioFormat::Opus => "audio/opus",
            AudioFormat::Amr => "audio/amr",
            AudioFormat::Wma => "audio/x-ms-wma",
            AudioFormat::Ra => "audio/vnd.rn-realaudio",
            AudioFormat::Au => "audio/basic",
            AudioFormat::Aiff => "audio/aiff",
            AudioFormat::Caf => "audio/x-caf",
            AudioFormat::ThreeGp => "audio/3gpp",
            AudioFormat::Mp4 => "video/mp4",
            AudioFormat::Mov => "video/quicktime",
            AudioFormat::Avi => "video/x-msvideo",
            AudioFormat::Mkv => "video/x-matroska",
            AudioFormat::Unknown => "application/octet-stream",
        }
    }

    /// Get FFmpeg input format specifier
    pub fn get_ffmpeg_input_format(&self) -> Option<&'static str> {
        match self {
            AudioFormat::Mp3 => Some("mp3"),
            AudioFormat::Wav => Some("wav"),
            AudioFormat::Flac => Some("flac"),
            AudioFormat::Aac => Some("aac"),
            AudioFormat::M4a => Some("m4a"),
            AudioFormat::Ogg => Some("ogg"),
            AudioFormat::Webm => Some("webm"),
            AudioFormat::Opus => Some("opus"),
            AudioFormat::Amr => Some("amr"),
            AudioFormat::Wma => Some("asf"),
            AudioFormat::Ra => Some("rm"),
            AudioFormat::Au => Some("au"),
            AudioFormat::Aiff => Some("aiff"),
            AudioFormat::Caf => Some("caf"),
            AudioFormat::ThreeGp => Some("3gp"),
            AudioFormat::Mp4 => Some("mp4"),
            AudioFormat::Mov => Some("mov"),
            AudioFormat::Avi => Some("avi"),
            AudioFormat::Mkv => Some("matroska"),
            AudioFormat::Unknown => None,
        }
    }

    /// Check if format requires FFmpeg conversion
    pub fn requires_ffmpeg_conversion(&self) -> bool {
        !matches!(self, AudioFormat::Wav)
    }

    /// Convert from Symphonia codec type
    pub fn from_symphonia_codec(codec_type: symphonia::core::codecs::CodecType) -> Self {
        use symphonia::core::codecs::*;
        
        // Match specific Symphonia codec types to our AudioFormat enum
        match codec_type {
            CODEC_TYPE_NULL => AudioFormat::Unknown,
            
            // PCM codecs (usually WAV)
            CODEC_TYPE_PCM_S16LE | CODEC_TYPE_PCM_S16BE |
            CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S24BE |
            CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_S32BE |
            CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_F32BE |
            CODEC_TYPE_PCM_F64LE | CODEC_TYPE_PCM_F64BE |
            CODEC_TYPE_PCM_U8 => AudioFormat::Wav,
            
            // MP3 codec
            CODEC_TYPE_MP3 => AudioFormat::Mp3,
            
            // FLAC codec
            CODEC_TYPE_FLAC => AudioFormat::Flac,
            
            // AAC codec
            CODEC_TYPE_AAC => AudioFormat::Aac,
            
            // Vorbis (usually in OGG container)
            CODEC_TYPE_VORBIS => AudioFormat::Ogg,
            
            // Opus codec
            CODEC_TYPE_OPUS => AudioFormat::Opus,
            
            // Default to Unknown for unsupported codecs
            _ => AudioFormat::Unknown,
        }
    }

    /// Get corresponding Symphonia codec type
    pub fn to_symphonia_codec(&self) -> Option<symphonia::core::codecs::CodecType> {
        // For MVP, return None for all formats since codec constant mapping is complex
        // In a full implementation, this would map to appropriate Symphonia codec constants
        match self {
            AudioFormat::Unknown => None,
            _ => Some(symphonia::core::codecs::CODEC_TYPE_NULL), // Placeholder
        }
    }
}

/// Model download status
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelDownloadStatus {
    pub model_name: String,
    pub status: DownloadStatus,
    pub progress: Option<f32>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DownloadStatus {
    NotStarted,
    Downloading,
    Completed,
    Failed,
    Exists,
}

/// Daemon status information
#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime: Option<u64>,
    pub memory_usage: Option<String>,
    pub cpu_usage: Option<f32>,
}