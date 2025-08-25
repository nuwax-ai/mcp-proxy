pub mod config;
pub mod request;
pub mod worker;
mod http_result;
pub mod cluster;
pub mod metadata_store;

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
    TranscriptionResponse, Segment, HealthResponse, ModelsResponse, ModelInfo,
    ProcessedAudio, AudioFormat, AudioFormatResult, DetectionMethod,
    ModelDownloadStatus, DownloadStatus, DaemonStatus,
};

// Rename conflicting types from request module
pub use request::{
    TranscriptionRequest as HttpTranscriptionRequest,
    AudioMetadata as HttpAudioMetadata,
};

// Worker module exports (for internal processing)
pub use worker::{
    TranscriptionTask, TranscriptionResult, WorkerProcessedAudio,
};

// Rename conflicting types from worker module  
pub use worker::{
    TranscriptionRequest as WorkerTranscriptionRequest,
    AudioMetadata as WorkerAudioMetadata,
};