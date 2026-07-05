use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// TTS 同步合成请求（sherpa-onnx Kokoro）。
///
/// 重新设计的 `/api/v1/tts` 请求体（旧 Python IndexTTS 的 pitch/volume/reference_audio 砍掉）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TtsSyncRequest {
    /// 要合成的文本（必填）
    pub text: String,
    /// 音色 id（Kokoro voices.bin 多 speaker 索引；`None` = 用 `tts.engine.default_sid`）
    #[serde(default)]
    pub sid: Option<i32>,
    /// 音色名（可选别名，v1 暂不解析名字→sid，保留接口）
    #[serde(default)]
    pub voice: Option<String>,
    /// 语速（1.0 = 原速；`None` = 用 `tts.engine.default_speed`）
    #[serde(default)]
    pub speed: Option<f32>,
    /// 时长缩放（model-level，仅引擎首次加载生效；`None` = 用 `tts.engine.default_length_scale`）
    #[serde(default)]
    pub length_scale: Option<f32>,
    /// 语言提示（kokoro-multi-lang 自动检测时可选，如 `"zh"`/`"en"`）
    #[serde(default)]
    pub language: Option<String>,
    /// 输出格式：`wav`（默认，含 RIFF 头）/ `pcm_s16le`（裸 PCM）
    #[serde(default)]
    pub format: Option<String>,
}

/// TTS 任务响应（P4 异步任务用；P3 同步接口不返回此结构）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TtsTaskResponse {
    pub task_id: String,
    pub message: String,
    pub estimated_duration: Option<u32>, // 预估处理时间（秒）
}

/// TTS处理阶段
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum TtsProcessingStage {
    TextPreprocessing,
    VoiceSynthesis,
    AudioPostProcessing,
    ResultFormatting,
}

impl TtsProcessingStage {
    pub fn step_name(&self) -> &'static str {
        match self {
            TtsProcessingStage::TextPreprocessing => "text_preprocessing_step",
            TtsProcessingStage::VoiceSynthesis => "voice_synthesis_step",
            TtsProcessingStage::AudioPostProcessing => "audio_post_processing_step",
            TtsProcessingStage::ResultFormatting => "result_formatting_step",
        }
    }
}

/// TTS任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum TtsTaskStatus {
    Pending {
        queued_at: DateTime<Utc>,
    },
    Processing {
        stage: TtsProcessingStage,
        started_at: DateTime<Utc>,
        progress_details: Option<TtsProgressDetails>,
    },
    Completed {
        completed_at: DateTime<Utc>,
        processing_time: chrono::Duration,
        audio_file_path: String,
        file_size: u64,
        duration_seconds: f32,
    },
    Failed {
        error: TtsTaskError,
        failed_at: DateTime<Utc>,
        retry_count: u32,
        is_recoverable: bool,
    },
    Cancelled {
        cancelled_at: DateTime<Utc>,
        reason: Option<String>,
    },
}

/// TTS进度详情
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct TtsProgressDetails {
    pub current_stage: TtsProcessingStage,
    pub stage_progress: Option<f32>, // 0.0 to 1.0
    pub estimated_remaining: Option<chrono::Duration>,
    pub text_length: usize,
    pub processed_chars: usize,
}

/// TTS任务错误
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum TtsTaskError {
    TextProcessingFailed {
        message: String,
        is_recoverable: bool,
    },
    SynthesisFailed {
        model: String,
        message: String,
        is_recoverable: bool,
    },
    AudioProcessingFailed {
        stage: TtsProcessingStage,
        message: String,
        is_recoverable: bool,
    },
    StorageError {
        operation: String,
        message: String,
    },
    TimeoutError {
        stage: TtsProcessingStage,
        timeout_duration: chrono::Duration,
    },
    CancellationRequested,
}

impl TtsTaskError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            TtsTaskError::TextProcessingFailed { is_recoverable, .. } => *is_recoverable,
            TtsTaskError::SynthesisFailed { is_recoverable, .. } => *is_recoverable,
            TtsTaskError::AudioProcessingFailed { is_recoverable, .. } => *is_recoverable,
            TtsTaskError::StorageError { .. } => true,
            TtsTaskError::TimeoutError { .. } => true,
            TtsTaskError::CancellationRequested => false,
        }
    }
}

impl std::fmt::Display for TtsTaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TtsTaskError::TextProcessingFailed { message, .. } => {
                write!(f, "文本处理失败: {}", message)
            }
            TtsTaskError::SynthesisFailed { model, message, .. } => {
                write!(f, "语音合成失败 ({}): {}", model, message)
            }
            TtsTaskError::AudioProcessingFailed { stage, message, .. } => {
                write!(f, "音频处理失败 ({}): {}", stage.step_name(), message)
            }
            TtsTaskError::StorageError { operation, message } => {
                write!(f, "存储错误 ({}): {}", operation, message)
            }
            TtsTaskError::TimeoutError {
                stage,
                timeout_duration,
            } => {
                write!(
                    f,
                    "超时错误 ({}): {} 秒",
                    stage.step_name(),
                    timeout_duration.num_seconds()
                )
            }
            TtsTaskError::CancellationRequested => {
                write!(f, "任务已被取消")
            }
        }
    }
}

/// TTS任务优先级 (复用现有的TaskPriority)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema, Default)]
pub enum TaskPriority {
    Low = 1,
    #[default]
    Normal = 2,
    High = 3,
}
