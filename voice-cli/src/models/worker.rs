use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::VoiceCliError;

/// Transcription task for worker processing
#[derive(Debug)]
pub struct TranscriptionTask {
    /// Unique task identifier
    pub task_id: String,
    /// Audio data to transcribe
    pub audio_data: Bytes,
    /// Original filename (for format detection)
    pub filename: String,
    /// Optional model override
    pub model: Option<String>,
    /// Optional response format
    pub response_format: Option<String>,
    /// Channel to send result back to handler
    pub result_sender: tokio::sync::oneshot::Sender<TranscriptionResult>,
}

/// Result from transcription worker
#[derive(Debug)]
pub struct TranscriptionResult {
    /// Task identifier
    pub task_id: String,
    /// Whether the transcription succeeded
    pub success: bool,
    /// Transcription response if successful
    pub response: Option<crate::models::TranscriptionResponse>,
    /// Error if transcription failed
    pub error: Option<VoiceCliError>,
    /// Time taken for processing (seconds)
    pub processing_time: f32,
}

/// Processed audio file information for workers
#[derive(Debug)]
pub struct WorkerProcessedAudio {
    /// Path to the processed/converted audio file
    pub file_path: PathBuf,
    /// Original audio format
    pub original_format: crate::models::AudioFormat,
    /// List of temporary files to cleanup
    pub cleanup_files: Vec<PathBuf>,
}

impl Drop for WorkerProcessedAudio {
    fn drop(&mut self) {
        // Cleanup temporary files
        for file_path in &self.cleanup_files {
            if file_path.exists() {
                if let Err(e) = std::fs::remove_file(file_path) {
                    tracing::warn!("Failed to cleanup temporary file {}: {}", file_path.display(), e);
                }
            }
        }
        
        // Also cleanup the main processed file if it exists
        if self.file_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.file_path) {
                tracing::warn!("Failed to cleanup processed audio file {}: {}", self.file_path.display(), e);
            }
        }
    }
}

/// Audio metadata information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioMetadata {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u16,
    /// Duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Original audio format
    pub format: Option<String>,
}

/// Internal transcription request (after multipart extraction)
#[derive(Debug)]
pub struct TranscriptionRequest {
    /// Original filename
    pub filename: String,
    /// Optional model override
    pub model: Option<String>,
    /// Optional response format
    pub response_format: Option<String>,
}