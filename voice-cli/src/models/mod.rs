pub mod cluster;
pub mod config;
mod http_result;
pub mod metadata_store;
pub mod request;
pub mod worker;

// Re-export config types
pub use config::*;

// Re-export HTTP result types
pub use http_result::*;

// Re-export cluster types
pub use cluster::*;

// Re-export metadata store types
pub use metadata_store::*;

// Explicit re-exports to avoid conflicts
// Request module exports (for HTTP API)
pub use request::{
    AudioFormat, AudioFormatResult, DaemonStatus, DetectionMethod, DownloadStatus, HealthResponse,
    ModelDownloadStatus, ModelInfo, ModelsResponse, ProcessedAudio, Segment, TranscriptionResponse,
};

// Rename conflicting types from request module
pub use request::{
    AudioMetadata as HttpAudioMetadata, TranscriptionRequest as HttpTranscriptionRequest,
};

// Worker module exports (for internal processing)
pub use worker::{TranscriptionResult, TranscriptionTask, WorkerProcessedAudio};

// Rename conflicting types from worker module
pub use worker::{
    AudioMetadata as WorkerAudioMetadata, TranscriptionRequest as WorkerTranscriptionRequest,
};
