use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use utoipa::ToSchema;

use crate::models::AudioFormat;

/// Initial task submitted to the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncTranscriptionTask {
    pub task_id: String,
    pub audio_file_path: PathBuf, // Path to the audio file on disk
    pub original_filename: String, // Original filename from upload
    pub model: Option<String>,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
    pub priority: TaskPriority,
}

impl AsyncTranscriptionTask {
    pub fn new(
        task_id: String,
        audio_file_path: PathBuf,
        original_filename: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> Self {
        Self {
            task_id,
            audio_file_path,
            original_filename,
            model,
            response_format,
            created_at: Utc::now(),
            priority: TaskPriority::Normal,
        }
    }
}

/// After Step 1: Audio format processing completed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioProcessedTask {
    pub task_id: String,
    pub processed_audio_path: PathBuf,
    pub original_format: AudioFormat,
    pub model: Option<String>,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
    pub audio_duration: Option<f32>,
    pub cleanup_files: Vec<PathBuf>, // Files to cleanup after processing
}

/// After Step 2: Whisper transcription completed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionCompletedTask {
    pub task_id: String,
    pub transcription_result: SerializableTranscriptionResult,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
    pub processing_stages: Vec<ProcessingStageInfo>,
}

/// Serializable version of voice_toolkit::stt::TranscriptionResult
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTranscriptionResult {
    pub text: String,
    pub segments: Vec<SerializableSegment>,
    pub language: Option<String>,
    pub audio_duration: u64, // milliseconds
}

/// Serializable version of voice_toolkit segment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableSegment {
    pub start_time: u64, // milliseconds
    pub end_time: u64,   // milliseconds
    pub text: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingStageInfo {
    pub stage: ProcessingStage,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum TaskPriority {
    Low = 1,
    Normal = 2,
    High = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum ProcessingStage {
    AudioFormatDetection,
    AudioConversion,
    WhisperTranscription,
    ResultProcessing,
}

impl ProcessingStage {
    pub fn step_name(&self) -> &'static str {
        match self {
            ProcessingStage::AudioFormatDetection => "audio_format_step",
            ProcessingStage::AudioConversion => "audio_format_step", // Same step handles both
            ProcessingStage::WhisperTranscription => "whisper_transcription_step",
            ProcessingStage::ResultProcessing => "result_formatting_step",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum TaskStatus {
    Pending {
        queued_at: DateTime<Utc>,
    },
    Processing {
        stage: ProcessingStage,
        started_at: DateTime<Utc>,
        progress_details: Option<ProgressDetails>,
    },
    Completed {
        completed_at: DateTime<Utc>,
        processing_time: Duration,
        result_summary: Option<String>,
    },
    Failed {
        error: TaskError,
        failed_at: DateTime<Utc>,
        retry_count: u32,
        is_recoverable: bool,
    },
    Cancelled {
        cancelled_at: DateTime<Utc>,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct ProgressDetails {
    pub current_stage: ProcessingStage,
    pub stage_progress: Option<f32>, // 0.0 to 1.0
    pub estimated_remaining: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub enum TaskError {
    AudioProcessingFailed {
        stage: ProcessingStage,
        message: String,
        is_recoverable: bool,
    },
    TranscriptionFailed {
        model: String,
        message: String,
        is_recoverable: bool,
    },
    StorageError {
        operation: String,
        message: String,
    },
    TimeoutError {
        stage: ProcessingStage,
        timeout_duration: Duration,
    },
    CancellationRequested,
}

impl TaskError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            TaskError::AudioProcessingFailed { is_recoverable, .. } => *is_recoverable,
            TaskError::TranscriptionFailed { is_recoverable, .. } => *is_recoverable,
            TaskError::StorageError { .. } => true, // Storage errors are usually recoverable
            TaskError::TimeoutError { .. } => true, // Timeout errors are recoverable
            TaskError::CancellationRequested => false, // Cancellation is not recoverable
        }
    }
}

impl Default for TaskPriority {
    fn default() -> Self {
        TaskPriority::Normal
    }
}

impl AsyncTranscriptionTask {
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }
}

impl AudioProcessedTask {
    pub fn from_async_task(
        task: AsyncTranscriptionTask,
        processed_audio_path: PathBuf,
        original_format: AudioFormat,
        audio_duration: Option<f32>,
        cleanup_files: Vec<PathBuf>,
    ) -> Self {
        Self {
            task_id: task.task_id,
            processed_audio_path,
            original_format,
            model: task.model,
            response_format: task.response_format,
            created_at: task.created_at,
            audio_duration,
            cleanup_files,
        }
    }
}

impl TranscriptionCompletedTask {
    pub fn from_audio_processed_task(
        task: AudioProcessedTask,
        transcription_result: SerializableTranscriptionResult,
        processing_stages: Vec<ProcessingStageInfo>,
    ) -> Self {
        Self {
            task_id: task.task_id,
            transcription_result,
            response_format: task.response_format,
            created_at: task.created_at,
            processing_stages,
        }
    }
}

// Conversion functions between voice_toolkit types and serializable types
impl From<voice_toolkit::stt::TranscriptionResult> for SerializableTranscriptionResult {
    fn from(result: voice_toolkit::stt::TranscriptionResult) -> Self {
        Self {
            text: result.text,
            segments: result.segments.into_iter().map(|s| SerializableSegment::from_voice_toolkit_segment(s)).collect(),
            language: result.language,
            audio_duration: result.audio_duration,
        }
    }
}

impl From<crate::models::Segment> for SerializableSegment {
    fn from(segment: crate::models::Segment) -> Self {
        Self {
            start_time: (segment.start * 1000.0) as u64, // Convert seconds to milliseconds
            end_time: (segment.end * 1000.0) as u64,     // Convert seconds to milliseconds
            text: segment.text,
            confidence: segment.confidence,
        }
    }
}

impl SerializableSegment {
    pub fn from_voice_toolkit_segment(segment: voice_toolkit::stt::TranscriptionSegment) -> Self {
        Self {
            start_time: segment.start_time * 1000, // Convert seconds to milliseconds (assuming start_time is in seconds as u64)
            end_time: segment.end_time * 1000,     // Convert seconds to milliseconds (assuming end_time is in seconds as u64)
            text: segment.text,
            confidence: segment.confidence,
        }
    }
}

impl From<SerializableTranscriptionResult> for crate::models::TranscriptionResponse {
    fn from(result: SerializableTranscriptionResult) -> Self {
        Self {
            text: result.text,
            segments: result
                .segments
                .into_iter()
                .map(|s| crate::models::Segment {
                    start: s.start_time as f32 / 1000.0, // Convert from ms to seconds
                    end: s.end_time as f32 / 1000.0,
                    text: s.text,
                    confidence: s.confidence,
                })
                .collect(),
            language: result.language,
            duration: Some(result.audio_duration as f32 / 1000.0), // Convert from ms to seconds
            processing_time: 0.0, // Will be set by the handler
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_priority_default() {
        assert_eq!(TaskPriority::default(), TaskPriority::Normal);
    }

    #[test]
    fn test_async_transcription_task_creation() {
        let task = AsyncTranscriptionTask::new(
            "test-task-1".to_string(),
            PathBuf::from("/tmp/test.mp3"),
            "test.mp3".to_string(),
            Some("base".to_string()),
            Some("json".to_string()),
        );

        assert_eq!(task.task_id, "test-task-1");
        assert_eq!(task.audio_file_path, PathBuf::from("/tmp/test.mp3"));
        assert_eq!(task.original_filename, "test.mp3");
        assert_eq!(task.model, Some("base".to_string()));
        assert_eq!(task.response_format, Some("json".to_string()));
        assert_eq!(task.priority, TaskPriority::Normal);
    }

    #[test]
    fn test_task_with_priority() {
        let task = AsyncTranscriptionTask::new(
            "test-task-1".to_string(),
            PathBuf::from("/tmp/test.mp3"),
            "test.mp3".to_string(),
            None,
            None,
        )
        .with_priority(TaskPriority::High);

        assert_eq!(task.priority, TaskPriority::High);
    }

    #[test]
    fn test_processing_stage_step_name() {
        assert_eq!(
            ProcessingStage::AudioFormatDetection.step_name(),
            "audio_format_step"
        );
        assert_eq!(
            ProcessingStage::AudioConversion.step_name(),
            "audio_format_step"
        );
        assert_eq!(
            ProcessingStage::WhisperTranscription.step_name(),
            "whisper_transcription_step"
        );
        assert_eq!(
            ProcessingStage::ResultProcessing.step_name(),
            "result_formatting_step"
        );
    }

    #[test]
    fn test_task_error_is_recoverable() {
        let recoverable_error = TaskError::AudioProcessingFailed {
            stage: ProcessingStage::AudioConversion,
            message: "Conversion failed".to_string(),
            is_recoverable: true,
        };
        assert!(recoverable_error.is_recoverable());

        let non_recoverable_error = TaskError::CancellationRequested;
        assert!(!non_recoverable_error.is_recoverable());

        let timeout_error = TaskError::TimeoutError {
            stage: ProcessingStage::WhisperTranscription,
            timeout_duration: Duration::from_secs(3600),
        };
        assert!(timeout_error.is_recoverable());
    }

    #[test]
    fn test_serializable_transcription_result_conversion() {
        use crate::models::TranscriptionResponse;

        let serializable_result = SerializableTranscriptionResult {
            text: "Hello world".to_string(),
            segments: vec![SerializableSegment {
                start_time: 0,
                end_time: 2000, // 2 seconds in ms
                text: "Hello world".to_string(),
                confidence: 0.95,
            }],
            language: Some("en".to_string()),
            audio_duration: 2000, // 2 seconds in ms
        };

        let response: TranscriptionResponse = serializable_result.into();
        assert_eq!(response.text, "Hello world");
        assert_eq!(response.segments.len(), 1);
        assert_eq!(response.segments[0].start, 0.0);
        assert_eq!(response.segments[0].end, 2.0); // Converted to seconds
        assert_eq!(response.language, Some("en".to_string()));
        assert_eq!(response.duration, Some(2.0)); // Converted to seconds
    }
}