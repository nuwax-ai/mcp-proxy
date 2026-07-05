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

/// TTS 异步合成请求（`POST /api/v1/tasks/tts`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TtsAsyncRequest {
    /// 要合成的文本（必填）
    pub text: String,
    /// 音色 id（`None` = 用 `tts.engine.default_sid`）
    #[serde(default)]
    pub sid: Option<i32>,
    /// 语速（`None` = 用 `tts.engine.default_speed`）
    #[serde(default)]
    pub speed: Option<f32>,
    /// 时长缩放（model-level；`None` = 用 `tts.engine.default_length_scale`）
    #[serde(default)]
    pub length_scale: Option<f32>,
    /// 语言提示（可选）
    #[serde(default)]
    pub language: Option<String>,
    /// 输出格式：`wav` / `pcm_s16le`
    #[serde(default)]
    pub format: Option<String>,
}

/// TTS 异步任务（apalis `Job`，序列化存 SQLite `tts_tasks`）。
///
/// 注意：`task_id` 唯一；apalis worker 取出后调 `tts_pipeline_worker` 合成。
/// 字段都 `Serialize + Deserialize` 以便 apalis 持久化 + 重启恢复。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsTask {
    pub task_id: String,
    pub text: String,
    pub sid: i32,
    pub speed: f32,
    pub length_scale: f32,
    pub language: Option<String>,
    /// 输出格式：`wav` / `pcm_s16le`
    pub format: String,
    /// 模型 id（对应 `{models_dir}/{model_id}/`）
    pub model: String,
    pub created_at: DateTime<Utc>,
}

impl TtsTask {
    /// 估算处理时长（秒）：Kokoro CPU RTF≈0.3，按文本字数粗估（仅用于响应，非真实）。
    pub fn estimate_duration_secs(&self) -> u32 {
        let chars = self.text.chars().count() as f32;
        // 假设 ~12 字符/秒语音 + RTF 0.3 → 合成耗时 ≈ chars/12 * 0.3
        ((chars / 12.0) * 0.3).ceil().clamp(1.0, 300.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_with_text(text: &str) -> TtsTask {
        TtsTask {
            task_id: "t1".into(),
            text: text.into(),
            sid: 0,
            speed: 1.0,
            length_scale: 1.0,
            language: None,
            format: "wav".into(),
            model: "kokoro".into(),
            created_at: DateTime::parse_from_rfc3339("2026-07-05T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn estimate_at_least_one_second() {
        // 即使很短的文本也至少 1s（clamp 下界）
        assert_eq!(task_with_text("hi").estimate_duration_secs(), 1);
        assert_eq!(task_with_text("a").estimate_duration_secs(), 1);
    }

    #[test]
    fn estimate_grows_with_text_length() {
        let short = task_with_text("hello").estimate_duration_secs();
        let long = task_with_text(&"a".repeat(1200)).estimate_duration_secs();
        assert!(long > short, "{long} should > {short}");
    }

    #[test]
    fn estimate_clamps_to_max_300() {
        // 超长文本不应超过 300s 上界
        let huge = task_with_text(&"a".repeat(100_000)).estimate_duration_secs();
        assert_eq!(huge, 300);
    }

    #[test]
    fn estimate_counts_chars_not_bytes() {
        // 中文多字节：按字符数算（"你好" = 2 chars，不是 6 bytes）
        let zh = task_with_text("你好").estimate_duration_secs();
        let en = task_with_text("ab").estimate_duration_secs();
        assert_eq!(zh, en, "等字符数应等时长");
    }
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
