pub mod config;
mod http_result;
pub mod request;
pub mod stepped_task;
pub mod tts;

// Re-export config types
pub use config::*;

// Re-export HTTP result types
pub use http_result::*;

// Request module exports (for HTTP API)
pub use request::{
    AudioFormat, AudioFormatResult, DaemonStatus, DetectionMethod, DownloadStatus, HealthResponse,
    ModelDownloadStatus, ModelInfo, ModelsResponse, ProcessedAudio, Segment, TranscriptionResponse,
    AudioMetadata, TranscriptionRequest,
};

// Stepped task module exports
pub use stepped_task::{
    AsyncTranscriptionTask, AudioProcessedTask, ProcessingStage, ProcessingStageInfo,
    ProgressDetails, SerializableSegment, SerializableTranscriptionResult, TaskError,
    TaskPriority, TaskStatus, TranscriptionCompletedTask,
};

// TTS module exports
pub use tts::{
    TtsSyncRequest, TtsAsyncRequest, TtsTaskResponse, TtsProcessingStage, TtsTaskStatus,
    TtsProgressDetails, TtsTaskError, TaskPriority as TtsTaskPriority,
};

// 简化的任务响应类型
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 异步任务提交响应
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AsyncTaskResponse {
    pub task_id: String,
    pub status: TaskStatus,
    pub estimated_completion: Option<DateTime<Utc>>,
}

/// 简化任务状态枚举
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum SimpleTaskStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

impl From<&TaskStatus> for SimpleTaskStatus {
    fn from(status: &TaskStatus) -> Self {
        match status {
            TaskStatus::Pending { .. } => SimpleTaskStatus::Pending,
            TaskStatus::Processing { .. } => SimpleTaskStatus::Processing,
            TaskStatus::Completed { .. } => SimpleTaskStatus::Completed,
            TaskStatus::Failed { .. } => SimpleTaskStatus::Failed,
            TaskStatus::Cancelled { .. } => SimpleTaskStatus::Cancelled,
        }
    }
}

/// 任务状态查询响应
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaskStatusResponse {
    pub task_id: String,
    pub status: SimpleTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 任务取消响应
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelResponse {
    pub task_id: String,
    pub cancelled: bool,
    pub message: String,
}

/// 任务重试响应
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetryResponse {
    pub task_id: String,
    pub retried: bool,
    pub message: String,
}

/// 任务删除响应
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteResponse {
    pub task_id: String,
    pub deleted: bool,
    pub message: String,
}

/// 任务统计响应
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaskStatsResponse {
    pub total_tasks: u32,
    pub pending_tasks: u32,
    pub processing_tasks: u32,
    pub completed_tasks: u32,
    pub failed_tasks: u32,
    pub cancelled_tasks: u32,
    pub average_processing_time_ms: Option<f64>,
    pub failed_task_ids: Vec<String>,
}
