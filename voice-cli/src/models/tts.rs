use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use utoipa::ToSchema;

/// TTS同步请求
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TtsSyncRequest {
    /// 要合成的文本
    pub text: String,
    /// 语音模型 (可选)
    pub model: Option<String>,
    /// 语速 (0.5-2.0, 默认1.0)
    pub speed: Option<f32>,
    /// 音调 (-20到20, 默认0)
    pub pitch: Option<i32>,
    /// 音量 (0.5-2.0, 默认1.0)
    pub volume: Option<f32>,
    /// 输出音频格式 (mp3, wav, etc.)
    pub format: Option<String>,
}

/// TTS异步请求
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TtsAsyncRequest {
    /// 要合成的文本
    pub text: String,
    /// 语音模型 (可选)
    pub model: Option<String>,
    /// 语速 (0.5-2.0, 默认1.0)
    pub speed: Option<f32>,
    /// 音调 (-20到20, 默认0)
    pub pitch: Option<i32>,
    /// 音量 (0.5-2.0, 默认1.0)
    pub volume: Option<f32>,
    /// 输出音频格式 (mp3, wav, etc.)
    pub format: Option<String>,
    /// 任务优先级
    pub priority: Option<TaskPriority>,
}

/// TTS任务响应
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum TaskPriority {
    Low = 1,
    Normal = 2,
    High = 3,
}

impl Default for TaskPriority {
    fn default() -> Self {
        TaskPriority::Normal
    }
}
