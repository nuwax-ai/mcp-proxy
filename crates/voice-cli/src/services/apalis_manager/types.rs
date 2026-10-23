//! 流水线任务类型：初始/处理后/完成三个阶段的任务结构与转换。

use super::*;

/// 任务类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskType {
    /// 文件上传任务
    FileUpload,
    /// URL下载任务
    UrlDownload,
}

/// 初始转录任务 - 流水线的第一步
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionTask {
    pub task_id: String,
    pub audio_file_path: PathBuf,
    pub original_filename: String,
    pub model: Option<String>,
    pub response_format: Option<String>,
    /// 目标语种（`None` = 自动检测）。P1 透传到 STT 引擎
    #[serde(default)]
    pub language: Option<String>,
    /// 初始提示。P1 透传到 STT 引擎
    #[serde(default)]
    pub initial_prompt: Option<String>,
    pub created_at: DateTime<Utc>,
    /// 任务类型
    pub task_type: TaskType,
    /// URL地址（仅对UrlDownload类型有效）
    pub url: Option<String>,
}

/// 音频预处理完成的任务 - 流水线的第二步
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioProcessedTask {
    pub task_id: String,
    pub processed_audio_path: PathBuf,
    pub original_filename: String,
    pub model: Option<String>,
    pub response_format: Option<String>,
    /// 目标语种（`None` = 自动检测）。P1 透传到 STT 引擎
    #[serde(default)]
    pub language: Option<String>,
    /// 初始提示。P1 透传到 STT 引擎
    #[serde(default)]
    pub initial_prompt: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 转录完成的任务 - 流水线的第三步
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionCompletedTask {
    pub task_id: String,
    pub transcription_result: TranscriptionResponse,
    pub response_format: Option<String>,
    pub metadata: Option<crate::models::request::AudioVideoMetadata>,
    pub created_at: DateTime<Utc>,
}

impl From<AsyncTranscriptionTask> for TranscriptionTask {
    fn from(task: AsyncTranscriptionTask) -> Self {
        Self {
            task_id: task.task_id,
            audio_file_path: task.audio_file_path,
            original_filename: task.original_filename,
            model: task.model,
            response_format: task.response_format,
            language: task.language,
            initial_prompt: task.initial_prompt,
            created_at: task.created_at,
            task_type: TaskType::FileUpload,
            url: None,
        }
    }
}

/// 任务状态更新
#[derive(Debug, Clone)]
pub struct TaskStatusUpdate {
    pub task_id: String,
    pub status: TaskStatus,
}
